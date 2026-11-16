//! The skate engines one app carries, and switching between them while it runs (the headset app): SK8, the
//! clean-room engine (`crate::clean`); Skate 3, the faithful port on the player's imported data; THUG2, the
//! decomp's engine on the player's own game files (`crate::thug2`).
//!
//! Each engine's plugins are added inside [`add_engine`], and what is the engine's is said, not guessed:
//! - Its per-frame systems are the ones it puts in [`EngineSet`]`(engine)` (any schedule): they run only while it
//!   is the running engine. Anything else added meanwhile (a Bevy plugin, an audio source, a shared helper's
//!   systems) stays shared.
//! - Its start-up systems (`PreStartup`, `Startup`, `PostStartup` added while it registers) are set aside and run
//!   the first time the engine is chosen, not at launch: an engine never picked never starts.
//! - What they spawn is the engine's ([`EngineOwned`]): while another engine runs, its things are hidden and its
//!   sounds paused, and they come back as they were when it is picked again. Nothing is unloaded: going back is
//!   instant and its game carries on where it was.
//!
//! The shared XR layer (room, parks, build mode, menus) runs whatever the engine. Switching is [`Engines::request`];
//! it happens at the start of the next frame (`First`), and hooks ([`EngineSwitched`]) let the XR layer carry the
//! skater's spot over to the new engine.
//!
//! An engine's systems run one at a time inside a wrapper that names the engine while it runs and while its
//! commands are applied, so everything it spawns can be told apart (an observer on `Visibility` and on
//! `PlaybackSettings` tags it).
use bevy::ecs::change_detection::{CheckChangeTicks, Tick};
use bevy::ecs::query::FilteredAccessSet;
use bevy::ecs::schedule::{InternedScheduleLabel, InternedSystemSet, ScheduleLabel, SystemKey};
use bevy::ecs::system::{RunSystemError, ScheduleSystem, SystemIn, SystemParamValidationError, SystemStateFlags};
use bevy::ecs::world::DeferredWorld;
use bevy::ecs::world::unsafe_world_cell::UnsafeWorldCell;
use bevy::prelude::*;
use bevy::utils::prelude::DebugName;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
pub(crate) use engine_api::{Availability, BeforeSwitch, Engine, EngineOwned, EngineSet, EngineSwitched};
#[cfg(feature = "xr")]
pub(crate) use engine_api::{CourseSettings, Mode};

/// The engines the app carries and which one runs.
#[derive(Resource)]
pub(crate) struct Engines {
    current: Option<Engine>,
    wanted: Option<Engine>,
    carried: Vec<(Engine, Availability)>,
    started: HashSet<Engine>,
    /// Each engine's set-aside start-up schedules, run on its first turn.
    startup: HashMap<Engine, Vec<Schedule>>,
    /// What was hidden when its engine was left, and how it was shown.
    hidden: HashMap<Entity, Visibility>,
    /// Sounds paused likewise.
    paused: HashSet<Entity>,
    /// Where the choice is kept between launches.
    file: Option<PathBuf>,
    not_at_launch: HashSet<Engine>,
    /// The systems made an engine's through its set (each once), and the schedules its set is gated in.
    wrapped: HashSet<(InternedScheduleLabel, SystemKey)>,
    gated_sets: HashSet<(InternedScheduleLabel, Engine)>,
}

impl Engines {
    fn new() -> Self {
        Self {
            current: None,
            wanted: None,
            carried: Vec::new(),
            started: HashSet::new(),
            startup: HashMap::new(),
            hidden: HashMap::new(),
            paused: HashSet::new(),
            file: None,
            not_at_launch: HashSet::new(),
            wrapped: HashSet::new(),
            gated_sets: HashSet::new(),
        }
    }

    /// The running engine (none before the first frame).
    pub(crate) fn current(&self) -> Option<Engine> {
        self.current
    }

    /// The running engine, once any switch asked for has happened (none while one is pending).
    pub(crate) fn settled(&self) -> Option<Engine> {
        self.current.filter(|c| self.wanted.is_none_or(|w| w == *c))
    }

    /// Whether this app has `engine` at all (Skate 3 needs its data imported before launch; THUG2 a build with it).
    pub(crate) fn carries(&self, engine: Engine) -> bool {
        self.carried.iter().any(|(e, _)| *e == engine)
    }

    /// Whether `engine` can be switched to now.
    pub(crate) fn available(&self, engine: Engine, world: &World) -> Result<(), String> {
        match self.carried.iter().find(|(e, _)| *e == engine) {
            Some((_, check)) => check(world),
            None => Err(format!("{} isn't in this app", engine.name())),
        }
    }

    /// Switch to `engine` at the start of the next frame (and launch on it from now on).
    pub(crate) fn request(&mut self, engine: Engine) {
        self.wanted = Some(engine);
    }

    /// Where the choice is kept (`<folder>/engine.txt`), and the engine to start on: the one kept there, else
    /// `fallback` (also when the kept one is barred from starting the app: [`Engines::not_at_launch`]).
    pub(crate) fn remember_in(&mut self, folder: PathBuf, fallback: Engine) {
        let file = folder.join("engine.txt");
        let kept = std::fs::read_to_string(&file).ok().and_then(|s| Engine::from_key(&s));
        self.wanted = Some(kept.filter(|e| !self.not_at_launch.contains(e)).unwrap_or(fallback));
        self.file = Some(file);
    }

    /// Don't start the app on `engine` this launch (it crashed the app while starting last time): it can still be
    /// picked from the menu.
    #[cfg(any(feature = "thug2", test))]
    pub(crate) fn not_at_launch(&mut self, engine: Engine) {
        self.not_at_launch.insert(engine);
    }
}

// The engine whose systems are running (or whose commands are being applied) on this thread.
thread_local! {
    static RUNNING: Cell<Option<Engine>> = const { Cell::new(None) };
}

fn as_engine<T>(engine: Engine, f: impl FnOnce() -> T) -> T {
    let before = RUNNING.with(|r| r.replace(Some(engine)));
    let out = f();
    RUNNING.with(|r| r.set(before));
    out
}

/// The run condition of an engine's systems.
pub(crate) fn running(engine: Engine) -> impl Fn(Option<Res<Engines>>) -> bool + Clone {
    move |engines: Option<Res<Engines>>| engines.is_some_and(|e| e.current == Some(engine))
}

const STARTUP: [fn() -> InternedScheduleLabel; 3] =
    [|| PreStartup.intern(), || Startup.intern(), || PostStartup.intern()];

fn is_startup(label: InternedScheduleLabel) -> bool {
    STARTUP.iter().any(|startup| startup() == label)
}

/// Adds `engine` to the app: `plugins` adds its plugins and systems. Its per-frame systems are the ones it puts in
/// [`EngineSet`]`(engine)`; the start-up systems added here are its own too, run on its first turn (see the module
/// notes). `available` says whether it can run (checked when it is chosen).
pub(crate) fn add_engine(app: &mut App, engine: Engine, available: Availability, plugins: impl FnOnce(&mut App)) {
    registry(app);
    let kept = set_startup_aside(app);
    let before = system_keys(app);
    plugins(app);
    let startup = take_startup(app, engine, kept);
    let owned = own_set(app, engine);
    // (what it added outside its set is shared: listed, so a system left out by mistake can be seen)
    let shared: Vec<String> = new_systems(app, &before).into_iter().filter(|name| !owned.contains(name)).collect();
    info!("engines: {} added; {} systems its own", engine.name(), owned.len());
    if !shared.is_empty() {
        debug!("engines: {} added these shared (outside its set): {}", engine.name(), shared.join(", "));
    }
    carry(app, engine, available, startup);
}

/// The registry, the first time an engine is added.
fn registry(app: &mut App) {
    if !app.world().contains_resource::<Engines>() {
        app.insert_resource(Engines::new())
            .init_resource::<BeforeSwitch>()
            .add_message::<EngineSwitched>()
            .add_systems(First, (scripted, switch).chain())
            .add_observer(tag::<Visibility>)
            .add_observer(tag::<bevy::audio::PlaybackSettings>)
            .add_plugins(EnginesPlugin);
    }
}

/// The start-up schedules set aside, so the engine's own land in fresh ones.
fn set_startup_aside(app: &mut App) -> Vec<Schedule> {
    let mut schedules = app.world_mut().resource_mut::<Schedules>();
    STARTUP.iter().filter_map(|label| schedules.remove(label())).collect()
}

/// The engine's start-up schedules (made while it registered: all its own), and the shared ones put back.
fn take_startup(app: &mut App, engine: Engine, kept: Vec<Schedule>) -> Vec<Schedule> {
    let mut schedules = app.world_mut().resource_mut::<Schedules>();
    let mut own_startup = Vec::new();
    for label in STARTUP {
        if let Some(mut schedule) = schedules.remove(label()) {
            own(&mut schedule, engine);
            own_startup.push(schedule);
        }
    }
    for schedule in kept {
        schedules.insert(schedule);
    }
    own_startup
}

fn carry(app: &mut App, engine: Engine, available: Availability, startup: Vec<Schedule>) {
    let mut engines = app.world_mut().resource_mut::<Engines>();
    engines.carried.push((engine, available));
    engines.startup.insert(engine, startup);
    if engines.wanted.is_none() {
        engines.wanted = Some(engine);
    }
}

/// Makes what is in `EngineSet(engine)`, in every schedule, the engine's (once each: systems added to the set later
/// are made its own when this runs again). The names of its systems.
fn own_set(app: &mut App, engine: Engine) -> HashSet<String> {
    let set = EngineSet(engine).intern();
    let mut names = HashSet::new();
    app.world_mut().resource_scope(|world, mut engines: Mut<Engines>| {
        let mut schedules = world.resource_mut::<Schedules>();
        for (_, schedule) in schedules.iter_mut() {
            let label = schedule.label();
            let Some(key) = schedule.graph().system_sets.get_key(set) else { continue };
            let members = members(schedule.graph(), key);
            let systems = &mut schedule.graph_mut().systems;
            for system in members {
                let Some(node) = systems.get_mut(system) else { continue };
                names.insert(node.system.name().to_string());
                if !engines.wrapped.insert((label, system)) {
                    continue;
                }
                let inner = std::mem::replace(&mut node.system, Box::new(IntoSystem::into_system(|| {})));
                node.system = Box::new(Owned { engine, inner });
            }
            if engines.gated_sets.insert((label, engine)) {
                schedule.configure_sets(EngineSet(engine).run_if(running(engine)));
            }
        }
    });
    names
}

/// The systems under a set, through the sets in it.
fn members(graph: &bevy::ecs::schedule::ScheduleGraph, set: bevy::ecs::schedule::SystemSetKey) -> Vec<SystemKey> {
    use bevy::ecs::schedule::NodeId;
    let hierarchy = graph.hierarchy().graph();
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    let mut open = vec![NodeId::Set(set)];
    while let Some(node) = open.pop() {
        if !seen.insert(node) || !hierarchy.contains_node(node) {
            continue;
        }
        for child in hierarchy.neighbors_directed(node, bevy::ecs::schedule::graph::Direction::Outgoing).collect::<Vec<_>>() {
            match child {
                NodeId::System(system) => found.push(system),
                NodeId::Set(_) => open.push(child),
            }
        }
    }
    found
}

/// Runs [`own_set`] again for every engine once the plugins are all built: systems put in an engine's set after it
/// registered (a plugin's `finish`, a later `add_systems`) are its own too.
struct EnginesPlugin;

impl Plugin for EnginesPlugin {
    fn build(&self, _: &mut App) {}

    fn cleanup(&self, app: &mut App) {
        let engines: Vec<Engine> = app.world().resource::<Engines>().carried.iter().map(|(e, _)| *e).collect();
        for engine in engines {
            own_set(app, engine);
        }
    }
}

/// Every schedule's systems, by schedule.
fn system_keys(app: &mut App) -> HashMap<InternedScheduleLabel, HashSet<SystemKey>> {
    let schedules = app.world_mut().resource_mut::<Schedules>();
    schedules
        .iter()
        .map(|(_, schedule)| (schedule.label(), schedule.graph().systems.iter().map(|(key, _, _)| key).collect()))
        .collect()
}

/// The names of the per-frame systems added since `before`.
fn new_systems(app: &mut App, before: &HashMap<InternedScheduleLabel, HashSet<SystemKey>>) -> Vec<String> {
    let schedules = app.world_mut().resource_mut::<Schedules>();
    let mut names = Vec::new();
    for (_, schedule) in schedules.iter() {
        let label = schedule.label();
        if is_startup(label) {
            continue;
        }
        let old = before.get(&label);
        for (key, system, _) in schedule.graph().systems.iter() {
            if old.is_none_or(|o| !o.contains(&key)) {
                names.push(system.name().to_string());
            }
        }
    }
    names
}

/// Every system of one of the engine's own start-up schedules made `engine`'s: wrapped, so what they spawn is known
/// (they run on its first turn: no condition needed).
fn own(schedule: &mut Schedule, engine: Engine) {
    let systems = &mut schedule.graph_mut().systems;
    let keys: Vec<SystemKey> = systems.iter().map(|(key, _, _)| key).collect();
    for key in keys {
        if let Some(node) = systems.get_mut(key) {
            let inner = std::mem::replace(&mut node.system, Box::new(IntoSystem::into_system(|| {})));
            node.system = Box::new(Owned { engine, inner });
        }
    }
}

/// An engine's system: the engine named while it runs and while its commands are applied.
struct Owned {
    engine: Engine,
    inner: ScheduleSystem,
}

impl System for Owned {
    type In = ();
    type Out = ();

    fn name(&self) -> DebugName {
        self.inner.name()
    }

    fn type_id(&self) -> std::any::TypeId {
        self.inner.type_id()
    }

    fn flags(&self) -> SystemStateFlags {
        self.inner.flags()
    }

    unsafe fn run_unsafe(&mut self, input: SystemIn<'_, Self>, world: UnsafeWorldCell) -> Result<(), RunSystemError> {
        // SAFETY: the caller's guarantees, passed on unchanged.
        as_engine(self.engine, || unsafe { self.inner.run_unsafe(input, world) })
    }

    fn apply_deferred(&mut self, world: &mut World) {
        as_engine(self.engine, || self.inner.apply_deferred(world));
    }

    fn queue_deferred(&mut self, world: DeferredWorld) {
        as_engine(self.engine, || self.inner.queue_deferred(world));
    }

    unsafe fn validate_param_unsafe(&mut self, world: UnsafeWorldCell) -> Result<(), SystemParamValidationError> {
        // SAFETY: as above.
        unsafe { self.inner.validate_param_unsafe(world) }
    }

    fn initialize(&mut self, world: &mut World) -> FilteredAccessSet {
        self.inner.initialize(world)
    }

    fn check_change_tick(&mut self, check: CheckChangeTicks) {
        self.inner.check_change_tick(check);
    }

    fn default_system_sets(&self) -> Vec<InternedSystemSet> {
        self.inner.default_system_sets()
    }

    fn get_last_run(&self) -> Tick {
        self.inner.get_last_run()
    }

    fn set_last_run(&mut self, last_run: Tick) {
        self.inner.set_last_run(last_run);
    }
}

/// Marks what an engine spawns (while one of its systems runs or its commands are applied).
fn tag<C: Component>(add: On<Add, C>, mut commands: Commands) {
    if let Some(engine) = RUNNING.with(Cell::get) {
        commands.entity(add.entity).try_insert(EngineOwned(engine));
    }
}

/// `SK8_ENGINE_SCRIPT="5:thug2,12:skate3,20:sk8"`: switches at those seconds after launch (testing switching
/// without the menu).
fn scripted(time: Res<Time<Real>>, mut engines: ResMut<Engines>, mut script: Local<Option<Vec<(f32, Engine)>>>) {
    let script = script.get_or_insert_with(|| {
        let text = std::env::var("SK8_ENGINE_SCRIPT").unwrap_or_default();
        let mut steps: Vec<(f32, Engine)> = text
            .split(',')
            .filter_map(|item| {
                let (at, key) = item.split_once(':')?;
                Some((at.trim().parse().ok()?, Engine::from_key(key)?))
            })
            .collect();
        steps.sort_by(|a, b| b.0.total_cmp(&a.0));
        steps
    });
    while script.last().is_some_and(|(at, _)| time.elapsed_secs() >= *at) {
        let (at, engine) = script.pop().unwrap();
        info!("engines: SK8_ENGINE_SCRIPT at {at} s: {}", engine.name());
        engines.request(engine);
    }
}

/// Carries out a requested switch (and starts the first engine on the first frame).
fn switch(world: &mut World) {
    let (current, wanted) = {
        let engines = world.resource::<Engines>();
        (engines.current, engines.wanted)
    };
    let Some(wanted) = wanted.filter(|w| Some(*w) != current) else { return };
    // (an engine that can't run now - its data gone - leaves things as they are: the first frame falls back to the
    // first engine added)
    let check = world.resource::<Engines>().available(wanted, world);
    if let Err(why) = check {
        warn!("engines: can't run {}: {why}", wanted.name());
        let fallback = world.resource::<Engines>().carried.first().map(|(e, _)| *e);
        let mut engines = world.resource_mut::<Engines>();
        engines.wanted = if current.is_none() { fallback } else { current };
        return;
    }
    if let Some(old) = current {
        let hooks = world.resource::<BeforeSwitch>().0.clone();
        for hook in hooks {
            hook(world, old);
        }
        leave(world, old);
    }
    enter(world, wanted);
    info!("engines: {} -> {}", current.map_or("(start)", Engine::name), wanted.name());
    world.write_message(EngineSwitched { from: current, to: wanted });
    let file = world.resource::<Engines>().file.clone();
    if let Some(file) = file
        && let Err(e) = std::fs::write(&file, wanted.key())
    {
        warn!("engines: {}: {e}", file.display());
    }
}

fn leave(world: &mut World, engine: Engine) {
    use bevy::audio::{AudioSink, AudioSinkPlayback, SpatialAudioSink};
    let mut shown = world.query::<(Entity, &EngineOwned, &mut Visibility)>();
    let mut hidden = Vec::new();
    for (entity, owner, mut visibility) in shown.iter_mut(world) {
        if owner.0 == engine {
            hidden.push((entity, *visibility));
            *visibility = Visibility::Hidden;
        }
    }
    let mut paused = Vec::new();
    let mut sinks = world.query::<(Entity, &EngineOwned, Option<&AudioSink>, Option<&SpatialAudioSink>)>();
    for (entity, owner, sink, spatial) in sinks.iter(world) {
        if owner.0 != engine {
            continue;
        }
        let playing = sink.is_some_and(|s| !s.is_paused()) || spatial.is_some_and(|s| !s.is_paused());
        if playing {
            if let Some(s) = sink {
                s.pause();
            }
            if let Some(s) = spatial {
                s.pause();
            }
            paused.push(entity);
        }
    }
    let mut engines = world.resource_mut::<Engines>();
    engines.hidden.extend(hidden);
    engines.paused.extend(paused);
    engines.current = None;
}

fn enter(world: &mut World, engine: Engine) {
    use bevy::audio::{AudioSink, AudioSinkPlayback, SpatialAudioSink};
    world.resource_mut::<Engines>().current = Some(engine);
    if world.resource_mut::<Engines>().started.insert(engine) {
        let schedules = world.resource_mut::<Engines>().startup.remove(&engine).unwrap_or_default();
        // (the clear colour is the XR layer's - passthrough shows wherever nothing is drawn - whatever an engine's
        // start-up sets)
        let clear = world.get_resource::<ClearColor>().cloned();
        for mut schedule in schedules {
            schedule.run(world);
        }
        if let Some(clear) = clear {
            world.insert_resource(clear);
        }
        return;
    }
    let (hidden, paused) = {
        let mut engines = world.resource_mut::<Engines>();
        (std::mem::take(&mut engines.hidden), std::mem::take(&mut engines.paused))
    };
    let mut keep_hidden = HashMap::new();
    for (entity, visibility) in hidden {
        let mine = world.get::<EngineOwned>(entity).is_some_and(|o| o.0 == engine);
        if !mine {
            keep_hidden.insert(entity, visibility);
        } else if let Some(mut v) = world.get_mut::<Visibility>(entity) {
            *v = visibility;
        }
    }
    let mut keep_paused = HashSet::new();
    for entity in paused {
        let mine = world.get::<EngineOwned>(entity).is_some_and(|o| o.0 == engine);
        if !mine {
            if world.get_entity(entity).is_ok() {
                keep_paused.insert(entity);
            }
            continue;
        }
        if let Some(s) = world.get::<AudioSink>(entity) {
            s.play();
        }
        if let Some(s) = world.get::<SpatialAudioSink>(entity) {
            s.play();
        }
    }
    let mut engines = world.resource_mut::<Engines>();
    engines.hidden = keep_hidden;
    engines.paused = keep_paused;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct Ran(Vec<&'static str>);

    #[derive(Component)]
    struct Thing;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<Ran>();
        // A shared system, before the engines; and a shared start-up one, which makes something of the shared layer's.
        app.add_systems(Update, |mut ran: ResMut<Ran>| ran.0.push("shared"));
        app.add_systems(Startup, |mut commands: Commands| {
            commands.spawn((Visibility::Visible, Name::new("shared thing")));
        });
        add_engine(&mut app, Engine::Sk8, |_| Ok(()), |app| {
            app.add_systems(Startup, |mut ran: ResMut<Ran>, mut commands: Commands| {
                ran.0.push("a start");
                commands.spawn((Thing, Visibility::Visible, Name::new("a")));
            })
            .add_systems(Update, (|mut ran: ResMut<Ran>| ran.0.push("a")).in_set(EngineSet(Engine::Sk8)))
            // (a shared helper's system, added while it registers but outside its set: everyone's)
            .add_systems(Update, |mut ran: ResMut<Ran>| ran.0.push("helper"));
        });
        add_engine(&mut app, Engine::Thug2, |_| Ok(()), |app| {
            app.add_systems(Startup, |mut ran: ResMut<Ran>| ran.0.push("b start")).add_systems(
                Update,
                (|mut ran: ResMut<Ran>, mut commands: Commands, mut once: Local<bool>| {
                    ran.0.push("b");
                    if !std::mem::replace(&mut *once, true) {
                        commands.spawn((Thing, Visibility::Inherited, Name::new("b")));
                    }
                })
                .in_set(EngineSet(Engine::Thug2)),
            );
        });
        // And one after.
        app.add_systems(Startup, |mut ran: ResMut<Ran>| ran.0.push("shared start"));
        app
    }

    fn frame(app: &mut App) -> Vec<&'static str> {
        app.update();
        std::mem::take(&mut app.world_mut().resource_mut::<Ran>().0)
    }

    fn sorted(mut v: Vec<&'static str>) -> Vec<&'static str> {
        v.sort();
        v
    }

    fn visibility(app: &mut App, name: &str) -> Visibility {
        let mut q = app.world_mut().query::<(&Name, &Visibility)>();
        q.iter(app.world()).find(|(n, _)| n.as_str() == name).map(|(_, v)| *v).unwrap()
    }

    /// What the shared layer makes at start-up (the menu) is no engine's: switching engines never hides it.
    #[test]
    fn what_the_shared_layer_starts_with_is_no_engines() {
        let mut app = app();
        frame(&mut app);
        let mut owners = app.world_mut().query::<(&Name, Option<&EngineOwned>)>();
        let owner = owners.iter(app.world()).find(|(n, _)| n.as_str() == "shared thing").map(|(_, o)| o.map(|o| o.0));
        assert_eq!(owner, Some(None), "not an engine's");
        for engine in [Engine::Thug2, Engine::Sk8, Engine::Thug2] {
            app.world_mut().resource_mut::<Engines>().request(engine);
            frame(&mut app);
            assert_eq!(visibility(&mut app, "shared thing"), Visibility::Visible, "after switching to {engine:?}");
        }
    }

    #[test]
    fn only_the_running_engine_runs_and_each_starts_on_its_first_turn() {
        let mut app = app();
        let first = frame(&mut app);
        assert!(first.contains(&"shared start") && first.contains(&"a start") && first.contains(&"a"), "{first:?}");
        assert!(!first.contains(&"b start") && !first.contains(&"b"), "{first:?}");
        assert_eq!(sorted(frame(&mut app)), vec!["a", "helper", "shared"]);

        app.world_mut().resource_mut::<Engines>().request(Engine::Thug2);
        let switched = frame(&mut app);
        assert!(switched.contains(&"b start") && switched.contains(&"b") && !switched.contains(&"a"), "{switched:?}");
        assert!(switched.contains(&"helper"), "what an engine added outside its set runs whatever the engine");
        assert_eq!(app.world().resource::<Engines>().current(), Some(Engine::Thug2));

        app.world_mut().resource_mut::<Engines>().request(Engine::Sk8);
        let back = sorted(frame(&mut app));
        assert_eq!(back, vec!["a", "helper", "shared"], "no second start");
    }

    /// A system put in an engine's set after the engine was added (a plugin's `finish`, a later `add_systems`) is
    /// its own too once the plugins are all built.
    #[test]
    fn what_joins_an_engines_set_later_is_its_own() {
        let mut app = app();
        app.add_systems(
            Update,
            (|mut ran: ResMut<Ran>, mut commands: Commands, mut once: Local<bool>| {
                ran.0.push("b later");
                if !std::mem::replace(&mut *once, true) {
                    commands.spawn((Visibility::Visible, Name::new("b later")));
                }
            })
            .in_set(EngineSet(Engine::Thug2)),
        );
        app.finish();
        app.cleanup();
        assert!(!frame(&mut app).contains(&"b later"), "not while SK8 runs");
        app.world_mut().resource_mut::<Engines>().request(Engine::Thug2);
        assert!(frame(&mut app).contains(&"b later"));
        app.world_mut().resource_mut::<Engines>().request(Engine::Sk8);
        frame(&mut app);
        assert_eq!(visibility(&mut app, "b later"), Visibility::Hidden, "what it spawned is THUG2's");
    }

    #[test]
    fn what_an_engine_spawned_is_hidden_while_another_runs() {
        let mut app = app();
        frame(&mut app);
        let owner = {
            let mut q = app.world_mut().query::<(&Name, &EngineOwned)>();
            q.iter(app.world()).find(|(n, _)| n.as_str() == "a").map(|(_, o)| *o)
        };
        assert_eq!(owner, Some(EngineOwned(Engine::Sk8)));
        app.world_mut().resource_mut::<Engines>().request(Engine::Thug2);
        frame(&mut app);
        frame(&mut app);
        assert_eq!(visibility(&mut app, "a"), Visibility::Hidden);
        assert_eq!(visibility(&mut app, "b"), Visibility::Inherited);
        app.world_mut().resource_mut::<Engines>().request(Engine::Sk8);
        frame(&mut app);
        assert_eq!(visibility(&mut app, "a"), Visibility::Visible, "as it was");
        assert_eq!(visibility(&mut app, "b"), Visibility::Hidden);
    }

    #[test]
    fn the_choice_is_kept() {
        let dir = std::env::temp_dir().join(format!("engines-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = app();
        app.world_mut().resource_mut::<Engines>().remember_in(dir.clone(), Engine::Sk8);
        frame(&mut app);
        app.world_mut().resource_mut::<Engines>().request(Engine::Thug2);
        frame(&mut app);
        assert_eq!(std::fs::read_to_string(dir.join("engine.txt")).unwrap(), "thug2");
        let mut again = self::app();
        again.world_mut().resource_mut::<Engines>().remember_in(dir.clone(), Engine::Sk8);
        let first = frame(&mut again);
        assert!(first.contains(&"b start") && !first.contains(&"a start"), "{first:?}");
        // An engine that crashed while starting isn't launched on again (it can still be picked).
        let mut barred = self::app();
        barred.world_mut().resource_mut::<Engines>().not_at_launch(Engine::Thug2);
        barred.world_mut().resource_mut::<Engines>().remember_in(dir.clone(), Engine::Sk8);
        let first = frame(&mut barred);
        assert!(first.contains(&"a start") && !first.contains(&"b start"), "{first:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
