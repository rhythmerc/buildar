//! The skate engines one app carries, and switching between them while it runs (the headset app): SK8, the
//! clean-room engine (`crate::clean`); Skate 3, the faithful port on the player's imported data; THUG2, the
//! decomp's engine on the player's own game files (`crate::thug2`).
//!
//! Each engine's plugins are added inside [`add_engine`], which makes everything they schedule that engine's:
//! - Their per-frame systems (every schedule but the start-up ones) run only while it is the running engine.
//! - Their start-up systems (`PreStartup`, `Startup`, `PostStartup`) are set aside and run the first time the
//!   engine is chosen, not at launch: an engine never picked never starts.
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
use bevy::ecs::schedule::{ConditionWithAccess, InternedScheduleLabel, InternedSystemSet, ScheduleLabel, SystemKey};
use bevy::ecs::system::{RunSystemError, ScheduleSystem, SystemIn, SystemParamValidationError, SystemStateFlags};
use bevy::ecs::world::DeferredWorld;
use bevy::ecs::world::unsafe_world_cell::UnsafeWorldCell;
use bevy::prelude::*;
use bevy::utils::prelude::DebugName;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// A skate engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Engine {
    /// The clean-room engine (no game data).
    Sk8,
    /// The faithful port of Skate 3, on the player's imported data.
    Skate3,
    /// THUG2's own engine (the decomp), on the player's game files.
    Thug2,
}

impl Engine {
    pub(crate) const ALL: [Engine; 3] = [Engine::Sk8, Engine::Skate3, Engine::Thug2];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Engine::Sk8 => "SK8",
            Engine::Skate3 => "Skate 3",
            Engine::Thug2 => "THUG2",
        }
    }

    /// Its name in files (`engine.txt`, a park's engine tag).
    pub(crate) fn key(self) -> &'static str {
        match self {
            Engine::Sk8 => "sk8",
            Engine::Skate3 => "skate3",
            Engine::Thug2 => "thug2",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Engine> {
        Engine::ALL.into_iter().find(|e| e.key() == key.trim())
    }
}

/// Whether an engine can run now; `Err` says why not (for the menu).
pub(crate) type Availability = fn(&World) -> Result<(), String>;

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

/// Spawned by an engine: hidden and silenced while another runs.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EngineOwned(pub Engine);

/// Sent after a switch (also the first engine's start, with `from` none), once the new engine is running.
#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct EngineSwitched {
    pub from: Option<Engine>,
    pub to: Engine,
}

/// What of an engine's settings a park keeps (THUG2's world size and game speed: what the park was built to), and
/// how to put them back: each engine that has any registers them. Saved with the park, applied when it's opened.
#[derive(Resource, Default)]
pub(crate) struct ParkSettings(pub HashMap<Engine, (fn(&World) -> serde_json::Value, fn(&mut World, &serde_json::Value))>);

impl ParkSettings {
    /// `engine`'s settings now (none: it keeps none).
    pub(crate) fn of(world: &World, engine: Engine) -> Option<serde_json::Value> {
        let (get, _) = *world.get_resource::<ParkSettings>()?.0.get(&engine)?;
        Some(get(world))
    }

    /// `engine`'s settings put back as `settings` say.
    pub(crate) fn apply(world: &mut World, engine: Engine, settings: &serde_json::Value) {
        let Some(&(_, set)) = world.get_resource::<ParkSettings>().and_then(|p| p.0.get(&engine)) else { return };
        set(world, settings);
    }
}

/// Run before a switch with the engine being left (the XR layer notes where the skater is).
#[derive(Resource, Default)]
pub(crate) struct BeforeSwitch(pub Vec<fn(&mut World, Engine)>);

/// The run condition of an engine's systems.
pub(crate) fn running(engine: Engine) -> impl Fn(Option<Res<Engines>>) -> bool + Clone {
    move |engines: Option<Res<Engines>>| engines.is_some_and(|e| e.current == Some(engine))
}

const STARTUP: [fn() -> InternedScheduleLabel; 3] =
    [|| PreStartup.intern(), || Startup.intern(), || PostStartup.intern()];

/// Adds `engine` to the app: `plugins` adds its plugins and systems, which become the engine's (see the module
/// notes). `available` says whether it can run (checked when it is chosen).
pub(crate) fn add_engine(app: &mut App, engine: Engine, available: Availability, plugins: impl FnOnce(&mut App)) {
    if !app.world().contains_resource::<Engines>() {
        app.insert_resource(Engines::new())
            .init_resource::<BeforeSwitch>()
            .add_message::<EngineSwitched>()
            .add_systems(First, (scripted, switch).chain())
            .add_observer(tag::<Visibility>)
            .add_observer(tag::<bevy::audio::PlaybackSettings>);
    }
    // The start-up schedules set aside, so the engine's own land in fresh ones.
    let kept: Vec<Schedule> = {
        let mut schedules = app.world_mut().resource_mut::<Schedules>();
        STARTUP.iter().filter_map(|label| schedules.remove(label())).collect()
    };
    let before = system_keys(app);
    plugins(app);
    let mut own_startup = Vec::new();
    {
        let mut schedules = app.world_mut().resource_mut::<Schedules>();
        for label in STARTUP {
            if let Some(mut schedule) = schedules.remove(label()) {
                own(&mut schedule, engine, None);
                own_startup.push(schedule);
            }
        }
        for schedule in kept {
            schedules.insert(schedule);
        }
        let mut gated = Vec::new();
        for (_, schedule) in schedules.iter_mut() {
            let label = schedule.label();
            // (the start-up schedules here are the shared ones, put back: the engine's own were taken above. Their
            // systems aren't new - `before` was taken with them set aside - and made the engine's, they marked
            // everything the shared layer starts with as the engine's: the menu, hidden when it was left)
            if is_transition(label) || STARTUP.iter().any(|startup| startup() == label) {
                continue;
            }
            let old = before.get(&label);
            if own(schedule, engine, old) > 0 {
                gated.push(format!("{label:?}"));
            }
        }
        info!("engines: {} added; its systems in {}", engine.name(), gated.join(", "));
    }
    let mut engines = app.world_mut().resource_mut::<Engines>();
    engines.carried.push((engine, available));
    engines.startup.insert(engine, own_startup);
    if engines.wanted.is_none() {
        engines.wanted = Some(engine);
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

/// State transition schedules (`OnEnter(..)` and the like) are left alone: a run condition there would read the
/// state already changed.
fn is_transition(label: InternedScheduleLabel) -> bool {
    let name = format!("{label:?}");
    name.starts_with("OnEnter") || name.starts_with("OnExit") || name.starts_with("OnTransition")
}

/// The systems of `schedule` not in `old` made `engine`'s: wrapped (so what they spawn is known) and, unless it is
/// a start-up schedule (`old` none), run only while the engine runs. How many.
fn own(schedule: &mut Schedule, engine: Engine, old: Option<&HashSet<SystemKey>>) -> usize {
    let gate = old.is_some() || !schedule_is_startup(schedule);
    let systems = &mut schedule.graph_mut().systems;
    let new: Vec<SystemKey> =
        systems.iter().map(|(key, _, _)| key).filter(|key| old.is_none_or(|o| !o.contains(key))).collect();
    for &key in &new {
        if let Some(node) = systems.get_mut(key) {
            let inner = std::mem::replace(&mut node.system, Box::new(IntoSystem::into_system(|| {})));
            node.system = Box::new(Owned { engine, inner });
        }
        if gate && let Some(conditions) = systems.get_conditions_mut(key) {
            conditions.push(ConditionWithAccess::new(Box::new(IntoSystem::into_system(running(engine)))));
        }
    }
    new.len()
}

fn schedule_is_startup(schedule: &Schedule) -> bool {
    STARTUP.iter().any(|label| label() == schedule.label())
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
            .add_systems(Update, |mut ran: ResMut<Ran>| ran.0.push("a"));
        });
        add_engine(&mut app, Engine::Thug2, |_| Ok(()), |app| {
            app.add_systems(Startup, |mut ran: ResMut<Ran>| ran.0.push("b start"))
                .add_systems(Update, |mut ran: ResMut<Ran>, mut commands: Commands, mut once: Local<bool>| {
                    ran.0.push("b");
                    if !std::mem::replace(&mut *once, true) {
                        commands.spawn((Thing, Visibility::Inherited, Name::new("b")));
                    }
                });
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
        assert_eq!(sorted(frame(&mut app)), vec!["a", "shared"]);

        app.world_mut().resource_mut::<Engines>().request(Engine::Thug2);
        let switched = frame(&mut app);
        assert!(switched.contains(&"b start") && switched.contains(&"b") && !switched.contains(&"a"), "{switched:?}");
        assert_eq!(app.world().resource::<Engines>().current(), Some(Engine::Thug2));

        app.world_mut().resource_mut::<Engines>().request(Engine::Sk8);
        let back = sorted(frame(&mut app));
        assert_eq!(back, vec!["a", "shared"], "no second start");
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
