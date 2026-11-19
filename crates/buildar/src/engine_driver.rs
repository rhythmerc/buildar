//! One driver for every engine (`engine_core::EngineCore`): what hosting a game takes, whichever game it is.
//!
//! - **Start:** picked the first time, the engine is made on a thread of its own from its factory (the game's files
//!   loaded there). A marker file is there while it starts: still there at the next launch, the start crashed the app
//!   (the engine's adapter says so). Once made, its models are spawned. How far along it is: [`Readiness`].
//! - **Placed:** its player isn't in the room until the player puts it there (a placement: the menu's, the aiming
//!   chord, carried over from the last engine). Until then it runs unseen and unheard: its model and stand-in hidden,
//!   its sounds held, where it is not given out.
//! - **Frames:** the controllers and the player's eyes in, placements, the stage and park offered (the engine takes the
//!   latest when it can); then the engine stepped a whole number of its periods behind real time (several a frame at
//!   most; its clock scaled by its speed), not while the player is busy elsewhere (`PadHeld::pause`) or while the
//!   engine waits. Each step's poses are numbered and stamped with its clock (`crate::model::PoseHistories` draws
//!   between the last two), its voices heard, where the player is noted.
//! - **Out:** where the skater is (the skate mode's link, trick text and sounds), the voices played from where they
//!   are, the HUD drawn by the app's renderer, the models posed (their bodies' contact shadows with them), and a
//!   stand-in where the player is while an engine has no model.
//!
//! Installed once an engine ([`install`]), inside `engines::add_engine` and in its set: what it spawns is that
//! engine's (hidden and paused while another runs), and its start-up runs when the engine is first picked.
use crate::engine_audio::{Heard, VoiceEmitter, VoicePcm, Voices};
use crate::engines::{Engine, EngineSet};
use crate::model::{FlatModel, JointBinding, PoseHistories};
use crate::xr::link::{CurrentPark, CurrentStage, PlacePlayer, SkaterLink};
use bevy::prelude::*;
use engine_core::audio::Voice;
use engine_core::model::BodyGroup;
use std::collections::BTreeMap;
use engine_core::{EngineCore, EngineFactory, Focus, ModelSource, Pose, Step};
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

/// Steps a frame at most: behind more than that, the engine's clock lets the time go.
const MAX_STEPS_PER_FRAME: u32 = 4;
/// The stack an engine is made on (the old Skate 3 loader's).
const BOOT_STACK: usize = 64 << 20;
/// A frame longer than this (a hitch) counts as this long.
const MAX_FRAME_S: f32 = 0.1;

/// An engine the driver hosts: which one it is (each its own driver).
pub(crate) trait Hosted: Send + Sync + 'static {
    const ENGINE: Engine;
}

type Booted = Result<Box<dyn EngineCore>, String>;

enum Phase {
    Idle(Option<EngineFactory>),
    Booting(JoinHandle<Booted>),
    Running(Box<dyn EngineCore>),
    Failed(String),
}

/// One engine hosted.
#[derive(Resource)]
pub(crate) struct Driver<M: Hosted> {
    phase: Mutex<Phase>,
    /// Made while the engine starts, gone once it has.
    marker: Option<PathBuf>,
    /// Its clock: its steps' periods summed, the time towards the next step (seconds), and the steps run.
    clock: Duration,
    accum: f32,
    steps: u64,
    period: Duration,
    /// Its clock against real time (1: the game's own speed), and its sounds' level: the adapter's settings.
    pub(crate) speed: f32,
    pub(crate) volume: f32,
    poses: PoseHistories,
    scratch: Vec<Pose>,
    heard: Heard,
    voices: Voices,
    /// Where its sounds are heard from: its player (`aim`).
    emitter: std::sync::Arc<crate::binaural::Emitter>,
    heard_now: Vec<Voice>,
    /// Where the player was after the last two steps.
    focus: [Option<Focus>; 2],
    /// A placement asked before the engine ran.
    pending: Option<PlacePlayer>,
    /// Its player has been put in the room (once, ever: it stays there after).
    placed: bool,
    model: Option<Drawn>,
    /// Where its glTF models load from (an asset source the host registered for its data).
    assets: Option<&'static str>,
    /// The player's choices of its settings, and those not given to it yet.
    settings: BTreeMap<String, f32>,
    unsaid: Vec<String>,
    stream: Option<crate::engine_audio::EngineStream>,
    hud: engine_core::hud::HudFrame,
    _engine: PhantomData<M>,
}

impl<M: Hosted> Driver<M> {
    fn new(factory: EngineFactory, marker: Option<PathBuf>, assets: Option<&'static str>) -> Self {
        Self {
            phase: Mutex::new(Phase::Idle(Some(factory))),
            marker,
            clock: Duration::ZERO,
            accum: 0.0,
            steps: 0,
            period: Duration::from_secs(1) / 60,
            speed: 1.0,
            volume: 1.0,
            poses: PoseHistories::default(),
            scratch: Vec::new(),
            heard: Heard::default(),
            voices: Voices::default(),
            emitter: Default::default(),
            heard_now: Vec::new(),
            focus: [None; 2],
            pending: None,
            placed: false,
            model: None,
            assets,
            settings: BTreeMap::new(),
            unsaid: Vec::new(),
            stream: None,
            hud: default(),
            _engine: PhantomData,
        }
    }

    /// One of the engine's settings, as the player chose it: given to it now, or once it is made.
    pub(crate) fn set_setting(&mut self, key: &str, value: f32) {
        if self.settings.get(key) != Some(&value) {
            self.settings.insert(key.to_owned(), value);
            self.unsaid.push(key.to_owned());
        }
    }

    /// Why the engine stopped, if it did.
    pub(crate) fn failed(&self) -> Option<String> {
        match &*self.phase.lock().unwrap_or_else(|p| p.into_inner()) {
            Phase::Failed(why) => Some(why.clone()),
            _ => None,
        }
    }

    /// The engine's clock now: its last step's time and the time since, towards the next.
    pub(crate) fn now(&self) -> Duration {
        self.clock + Duration::from_secs_f32(self.accum.max(0.0))
    }

    #[cfg(test)]
    pub(crate) fn poses(&self) -> &PoseHistories {
        &self.poses
    }

    #[cfg(test)]
    pub(crate) fn steps(&self) -> u64 {
        self.steps
    }

    /// It runs (made, not failed).
    #[cfg(test)]
    pub(crate) fn running(&mut self) -> bool {
        running::<M>(&mut self.phase).is_some()
    }

    /// The poses the last step left, as stamped.
    #[cfg(test)]
    pub(crate) fn last_poses(&self) -> &[Pose] {
        &self.scratch
    }

    #[cfg(test)]
    pub(crate) fn clips(&self) -> usize {
        self.voices.clips()
    }

    #[cfg(test)]
    pub(crate) fn model(&self) -> Option<&FlatModel> {
        match &self.model {
            Some(Drawn::Flat(m, _)) => Some(m),
            _ => None,
        }
    }

    /// Where the player was after the last two steps.
    #[cfg(test)]
    pub(crate) fn focus(&self) -> [Option<Focus>; 2] {
        self.focus
    }
}

/// How far the running engine's player is from being in the room (the loading card's: `xr::loading_card`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ready {
    /// Being made (its files loaded).
    Starting,
    /// Made, its game still loading inside it: not ready for its player yet (`EngineCore::ready_to_place`).
    Preparing,
    /// Running, its player not put in the room yet.
    ToPlace,
    /// In the room.
    Placed,
}

/// The running engine and how ready it is (none while no hosted engine runs, or after it failed).
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Readiness(pub Option<(Engine, Ready)>);

/// A model spawned: given as meshes, or a glTF scene (its skins bound to the pose's joints by name once loaded).
enum Drawn {
    Flat(FlatModel, Vec<BodyGroup>),
    Gltf { root: Entity, names: Vec<String>, basis: Mat4, parents: Vec<Option<u16>>, groups: Vec<BodyGroup>, bindings: Option<Vec<JointBinding>> },
}

/// Hosts the engine `M` made by `factory` (`marker`: the file there while it starts; `assets`: the asset source its
/// glTF models load from). Call it inside the engine's `engines::add_engine`.
pub(crate) fn install<M: Hosted>(app: &mut App, factory: EngineFactory, marker: Option<PathBuf>, assets: Option<&'static str>) {
    crate::engine_audio::add_voice_source(app);
    crate::engine_audio::add_stream_source(app);
    app.insert_resource(Driver::<M>::new(factory, marker, assets))
        .init_resource::<Readiness>()
        .add_systems(Startup, (boot::<M>, spawn_stand_in::<M>))
        .add_systems(
            Update,
            (
                finish_boot::<M>,
                bind_gltf::<M>,
                input::<M>,
                place::<M>,
                offer::<M>,
                want_hud::<M>,
                step::<M>,
                publish::<M>.before(crate::skate_audio::clean::SkaterSoundSet),
                aim::<M>,
                play::<M>,
                play_stream::<M>,
                draw_hud::<M>,
                report::<M>,
            )
                .chain()
                .after(crate::xr::ui::UiSet)
                .in_set(EngineSet(M::ENGINE)),
        )
        .add_systems(
            PostUpdate,
            (place_stand_in::<M>, pose_models::<M>.in_set(engine_api::BodiesSet))
                .before(bevy::transform::TransformSystems::Propagate)
                .in_set(EngineSet(M::ENGINE)),
        )
        .add_systems(PostUpdate, publish_focus::<M>.in_set(crate::xr::link::FocusSet).in_set(EngineSet(M::ENGINE)));
}

/// Where its player is, for the shared layer ([`engine_api::PlayerFocus`]) - an engine of a mode whose player isn't
/// a skater (a skate engine's is the skater link's: `xr::link`): its focus after its last step.
fn publish_focus<M: Hosted>(driver: Res<Driver<M>>, mut focus: ResMut<crate::xr::link::PlayerFocus>) {
    if M::ENGINE.mode() != engine_api::Mode::Skate {
        focus.set(M::ENGINE, driver.focus[1].filter(|_| driver.placed));
    }
}

/// How ready it is, for the loading card.
fn report<M: Hosted>(mut driver: ResMut<Driver<M>>, mut readiness: ResMut<Readiness>) {
    let d = &mut *driver;
    let ready = match d.phase.get_mut().unwrap_or_else(|p| p.into_inner()) {
        Phase::Idle(_) | Phase::Booting(_) => Some(Ready::Starting),
        Phase::Failed(_) => None,
        // (a game still loading after it is made - Mario Kart Wii's race - says when it can take its player)
        Phase::Running(core) if !core.ready_to_place() => Some(Ready::Preparing),
        Phase::Running(_) if !d.placed => Some(Ready::ToPlace),
        Phase::Running(_) => Some(Ready::Placed),
    };
    let now = Readiness(ready.map(|r| (M::ENGINE, r)));
    if *readiness != now {
        info!("{}: {:?}", M::ENGINE.name(), ready);
    }
    readiness.set_if_neq(now);
}

fn boot<M: Hosted>(mut driver: ResMut<Driver<M>>) {
    let driver = &mut *driver;
    let phase = driver.phase.get_mut().unwrap_or_else(|p| p.into_inner());
    let Phase::Idle(factory) = phase else { return };
    let Some(factory) = factory.take() else { return };
    let name = M::ENGINE.name();
    info!("{name} starting");
    let marker = driver.marker.clone();
    if let Some(marker) = &marker {
        let _ = std::fs::write(marker, b"");
    }
    // (loading a game's files can recurse deep: Skate 3's sim needs more than a thread's default stack)
    let worker = std::thread::Builder::new().name(format!("{name}-boot").to_lowercase()).stack_size(BOOT_STACK).spawn(move || {
        let made = factory();
        // (made or not, it didn't crash the app)
        if let Some(marker) = marker {
            let _ = std::fs::remove_file(marker);
        }
        made
    });
    *phase = match worker {
        Ok(handle) => Phase::Booting(handle),
        Err(e) => Phase::Failed(format!("{name} didn't start: {e}")),
    };
}

/// The engine made: running, its models spawned, and the player put in front of the player unless a place is waiting.
fn finish_boot<M: Hosted>(world: &mut World) {
    let name = M::ENGINE.name();
    let finished = {
        let mut driver = world.resource_mut::<Driver<M>>();
        match driver.phase.get_mut().unwrap_or_else(|p| p.into_inner()) {
            Phase::Booting(handle) => handle.is_finished(),
            _ => return,
        }
    };
    if !finished {
        return;
    }
    let handle = {
        let mut driver = world.resource_mut::<Driver<M>>();
        let phase = driver.phase.get_mut().unwrap_or_else(|p| p.into_inner());
        let Phase::Booting(handle) = std::mem::replace(phase, Phase::Idle(None)) else { return };
        handle
    };
    let made = handle.join().unwrap_or_else(|_| Err(format!("{name} stopped while it started")));
    match made {
        Ok(mut core) => {
            let mut spawned = None;
            let assets = world.resource::<Driver<M>>().assets;
            for model in core.models().into_iter().take(1) {
                spawned = match model.source {
                    ModelSource::Meshes { meshes, textures, joint_count } => {
                        let flat = crate::model::spawn_meshes(world, &format!("{name} player"), &meshes, &textures, joint_count);
                        Some(Drawn::Flat(flat, model.groups))
                    }
                    ModelSource::Gltf { path, joint_names, basis } => {
                        let mut at = bevy::asset::AssetPath::from(path);
                        if let Some(source) = assets {
                            at = at.with_source(source);
                        }
                        let scene = world.resource::<AssetServer>().load(GltfAssetLabel::Scene(0).from_asset(at));
                        let root = world
                            .spawn((Transform::default(), Visibility::Hidden, Name::new(format!("{name} player"))))
                            .with_children(|p| {
                                p.spawn(SceneRoot(scene));
                            })
                            .id();
                        Some(Drawn::Gltf { root, names: joint_names, basis, parents: model.parents, groups: model.groups, bindings: None })
                    }
                };
            }
            let stream = core.stream().map(|factory| crate::engine_audio::EngineStream::start(name, factory));
            {
                let mut driver = world.resource_mut::<Driver<M>>();
                driver.model = spawned;
                driver.stream = stream;
                // (the player's choices of its settings, each time it is made)
                for (key, value) in &driver.settings {
                    core.set_setting(key, *value);
                }
                driver.unsaid.clear();
                // (a placement asked while it started; else none: its player waits to be put down)
                if let Some(place) = driver.pending.take() {
                    core.place_player(place.0);
                }
                *driver.phase.get_mut().unwrap_or_else(|p| p.into_inner()) = Phase::Running(core);
            }
        }
        Err(why) => {
            error!("{name}: {why}");
            world.resource_mut::<crate::xr::ui::XrUi>().say(format!("{name}: {why}"));
            *world.resource_mut::<Driver<M>>().phase.get_mut().unwrap_or_else(|p| p.into_inner()) = Phase::Failed(why);
        }
    }
}

/// The running engine, if it is.
fn running<M: Hosted>(phase: &mut Mutex<Phase>) -> Option<&mut Box<dyn EngineCore>> {
    match phase.get_mut().unwrap_or_else(|p| p.into_inner()) {
        Phase::Running(core) => Some(core),
        _ => None,
    }
}

/// The controllers, and what the player sees from (their eyes in the world: right, up, forward).
fn input<M: Hosted>(
    touch: Res<engine_api::LatestTouch>,
    held: Res<engine_api::PadHeld>,
    rig: Option<Res<crate::xr::rig::Rig>>,
    eyes: Query<&Transform, With<bevy_mod_xr::camera::XrCamera>>,
    mut driver: ResMut<Driver<M>>,
) {
    let driver = &mut *driver;
    let Some(core) = running::<M>(&mut driver.phase) else { return };
    for key in driver.unsaid.drain(..) {
        if let Some(value) = driver.settings.get(&key) {
            core.set_setting(&key, *value);
        }
    }
    core.set_input(&touch.0, **held);
    let view = rig.and_then(|r| r.root_transform()).zip(eyes.iter().next()).map(|(root, eye)| {
        let turn = root.rotation * eye.rotation;
        [turn * Vec3::X, turn * Vec3::Y, turn * Vec3::NEG_Z]
    });
    core.set_view(view);
}

fn place<M: Hosted>(
    mut requests: MessageReader<PlacePlayer>,
    parks: Option<Res<crate::xr::build::parks::Parks>>,
    mut driver: ResMut<Driver<M>>,
    mut played: Local<Option<String>>,
) {
    let driver = &mut *driver;
    // (another course opened: the player is put in it afresh, not carried over from the last; a place asked for
    // with it is read after)
    let open = parks.and_then(|p| p.current.as_ref().map(|c| c.id.clone()));
    if let Some(open) = open
        && played.as_ref() != Some(&open)
    {
        if played.is_some() {
            info!("{}: another course open: to be placed again", M::ENGINE.name());
        }
        driver.placed = false;
        driver.pending = None;
        *played = Some(open);
    }
    for r in requests.read() {
        info!("{}: placed at {:.2} (asked)", M::ENGINE.name(), r.0.point);
        driver.placed = true;
        match running::<M>(&mut driver.phase) {
            Some(core) => core.place_player(r.0),
            None => driver.pending = Some(*r),
        }
    }
}

/// The stage (and for a skating engine its park, first) offered every frame: the engine takes them when it can.
fn offer<M: Hosted>(stage: Res<CurrentStage>, park: Res<CurrentPark>, mut driver: ResMut<Driver<M>>) {
    let Some(core) = running::<M>(&mut driver.phase) else { return };
    if let Some(skate) = core.skate() {
        skate.set_park(&park);
    }
    core.set_stage(&stage);
}

/// The HUD is seen: on, and no menu over it.
fn want_hud<M: Hosted>(
    settings: Option<Res<crate::xr::ar_settings::ArSettings>>,
    panel: Option<Res<crate::xr::panel::XrPanel>>,
    huds: Option<Res<crate::engine_hud::EngineHuds>>,
    mut driver: ResMut<Driver<M>>,
) {
    let wanted = huds.is_some() && settings.is_some_and(|s| s.hud) && !panel.is_some_and(|p| p.visible) && driver.placed;
    if let Some(core) = running::<M>(&mut driver.phase) {
        core.want_hud(wanted);
    }
}

fn step<M: Hosted>(time: Res<Time<Real>>, held: Res<engine_api::PadHeld>, mut driver: ResMut<Driver<M>>) {
    let d = &mut *driver;
    let phase = d.phase.get_mut().unwrap_or_else(|p| p.into_inner());
    let Phase::Running(core) = phase else { return };
    // (waiting, its clock stands still: the player is drawn where it was)
    if held.pause || !core.wants_steps() {
        return;
    }
    d.accum += time.delta_secs().min(MAX_FRAME_S) * d.speed;
    let mut steps = 0;
    let mut failed = None;
    while steps < MAX_STEPS_PER_FRAME {
        let period = core.period();
        if d.accum < period.as_secs_f32() {
            break;
        }
        d.accum -= period.as_secs_f32();
        steps += 1;
        if let Err(e) = core.step() {
            failed = Some(e);
            break;
        }
        d.clock += period;
        d.period = period;
        d.steps += 1;
        // (every step's poses, numbered and stamped, for the model layer to draw between the last two)
        d.scratch.clear();
        core.poses(&mut d.scratch);
        for pose in &mut d.scratch {
            pose.generation = d.steps;
            pose.step = Step { at: d.clock, period };
            d.poses.offer(pose);
        }
        d.focus = [d.focus[1], core.focus()];
        d.heard_now.clear();
        core.voices(&mut d.heard_now);
        d.heard.after_step(std::mem::take(&mut d.heard_now), |id| core.clip(id));
    }
    if let Some(why) = failed {
        let name = M::ENGINE.name();
        error!("{name}: {why}");
        *phase = Phase::Failed(why);
        return;
    }
    if steps == MAX_STEPS_PER_FRAME {
        d.accum = d.accum.min(d.period.as_secs_f32());
    }
}

/// Where the skater is, its trick text and its sounds (a skating engine's), once it is in our world.
fn publish<M: Hosted>(
    held: Res<engine_api::PadHeld>,
    mut link: ResMut<SkaterLink>,
    mut trick: ResMut<crate::xr::ui::hud::TrickHud>,
    sounds: Option<ResMut<crate::skate_audio::clean::SkaterSounds>>,
    mut driver: ResMut<Driver<M>>,
) {
    if !driver.placed {
        return;
    }
    let Some(skate) = running::<M>(&mut driver.phase).and_then(|core| core.skate()) else { return };
    if let Some(now) = skate.link() {
        **link = now;
    }
    if let Some(next) = skate.trick()
        && **trick != next
    {
        **trick = next;
    }
    if let (Some(said), Some(mut sounds)) = (skate.sounds(), sounds) {
        *sounds = crate::skate_audio::clean::SkaterSounds { engine: Some(M::ENGINE), paused: held.pause, sounds: said };
    }
}

/// Its sounds' emitter at its player (its feet, after its last step), as the head hears it - every engine's sound,
/// voices and mix alike, comes from where its player is.
fn aim<M: Hosted>(
    driver: Res<Driver<M>>,
    listener: Query<&Transform, With<bevy::audio::SpatialListener>>,
    scale: Res<bevy::audio::DefaultSpatialScale>,
) {
    if let (Some(focus), Ok(head)) = (driver.focus[1], listener.single()) {
        crate::binaural::place(&driver.emitter, focus.feet, head, scale.0.0.x);
    }
}

/// The engine's voices, from its player at its level and pitch. While the player is busy elsewhere its loops wait
/// too.
fn play<M: Hosted>(
    mut commands: Commands,
    held: Res<engine_api::PadHeld>,
    mut assets: ResMut<Assets<VoicePcm>>,
    mut playing: Query<(Entity, &VoiceEmitter, Option<&mut bevy::audio::AudioSink>)>,
    mut driver: ResMut<Driver<M>>,
) {
    let d = &mut *driver;
    if running::<M>(&mut d.phase).is_none() {
        return;
    }
    // (not in our world yet: what it started can't be placed)
    if d.focus[1].is_none() || !d.placed {
        d.heard.forget_started();
        return;
    }
    d.voices.play(&mut d.heard, held.pause, d.volume, &d.emitter, &mut commands, &mut assets, &mut playing);
}

fn draw_hud<M: Hosted>(mut driver: ResMut<Driver<M>>, mut renderer: crate::engine_hud::HudRenderer) {
    let d = &mut *driver;
    let phase = d.phase.get_mut().unwrap_or_else(|p| p.into_inner());
    let Phase::Running(core) = phase else { return };
    if core.hud(&mut d.hud) {
        renderer.show(M::ENGINE, &d.hud, |id| core.hud_texture(id));
    }
}

/// A glTF model's skins bound to the pose's joints once its scene is in (each visible rig, its authored inverse binds
/// kept; a skinned mesh isn't culled by its bind-pose bounds).
#[allow(clippy::type_complexity)]
fn bind_gltf<M: Hosted>(
    mut commands: Commands,
    mut driver: ResMut<Driver<M>>,
    skins: Query<(Entity, &bevy::mesh::skinning::SkinnedMesh, Has<bevy::camera::visibility::NoFrustumCulling>)>,
    nodes: Query<(&Name, &Transform)>,
    parents: Query<&ChildOf>,
    visibility: Query<&Visibility>,
) {
    let Some(Drawn::Gltf { root, names, bindings: bindings @ None, .. }) = &mut driver.model else { return };
    let root = *root;
    let under = |e: Entity| parents.iter_ancestors(e).any(|a| a == root);
    for (entity, _, unculled) in &skins {
        if !unculled && under(entity) {
            commands.entity(entity).insert(bevy::camera::visibility::NoFrustumCulling);
        }
    }
    let rigs = skins.iter().filter_map(|(entity, skin, _)| {
        let shown = !parents
            .iter_ancestors(entity)
            .take_while(|a| *a != root)
            .any(|a| visibility.get(a).is_ok_and(|v| *v == Visibility::Hidden));
        (under(entity) && shown).then_some(skin.joints.as_slice())
    });
    match crate::model::bind_skins(
        names,
        rigs,
        |e| nodes.get(e).ok().map(|(name, transform)| (name.as_str().to_owned(), transform.to_matrix())),
        |e| parents.iter_ancestors(e).collect(),
        |e| e == root,
    ) {
        Ok(bound) if bound.is_empty() => {}
        Ok(bound) => {
            info!("{}: its model bound ({} joints)", M::ENGINE.name(), bound.len());
            *bindings = Some(bound);
        }
        Err(why) => {
            error!("{}: its model can't be drawn: {why}", M::ENGINE.name());
            *bindings = Some(Vec::new());
        }
    }
}

/// The model where the engine's poses have it, between its last two steps, and its bodies with it: as meshes, each
/// joint's matrix ([`crate::model::flat_blend`]); as a glTF, its root and its bound joints' local transforms
/// ([`crate::model::joint_locals`]).
fn pose_models<M: Hosted>(
    mut driver: ResMut<Driver<M>>,
    mut q: Query<(&mut Transform, &mut Visibility)>,
    mut bodies: ResMut<engine_api::PlayerBodies>,
) {
    let d = &mut *driver;
    let now = d.clock + Duration::from_secs_f32(d.accum.max(0.0));
    let Some(model) = &d.model else { return };
    let view = d.poses.get(0).and_then(|h| h.view_at(now));
    let (root, ready) = match model {
        Drawn::Flat(flat, _) => (flat.root, true),
        Drawn::Gltf { root, bindings, .. } => (*root, bindings.as_ref().is_some_and(|b| !b.is_empty())),
    };
    let shown = ready && d.placed && d.focus[1].is_some() && view.is_some();
    if let Ok((_, mut visibility)) = q.get_mut(root) {
        *visibility = if shown { Visibility::Inherited } else { Visibility::Hidden };
    }
    let (true, Some((previous, current, alpha))) = (shown, view) else {
        bodies.set(M::ENGINE, Vec::new());
        return;
    };
    match model {
        Drawn::Flat(flat, groups) => {
            let (placed, matrices) = crate::model::flat_blend(previous, current, alpha, flat.joints.len());
            // (the parts' own transform - the root's - is where the model is: skinning ignores it, but culling places
            // the parts' bind-pose bounds with it)
            if let Ok((mut t, _)) = q.get_mut(flat.root) {
                *t = Transform::from_matrix(placed);
            }
            // (its bodies where they are drawn: each point carried by its joint as blended)
            let point = |p: engine_core::model::Point| Some(matrices.get(usize::from(p.joint))?.transform_point3(p.at));
            bodies.set(M::ENGINE, crate::model::bodies(groups, &[], point));
            for (&joint, matrix) in flat.joints.iter().zip(&matrices) {
                if let Ok((mut transform, _)) = q.get_mut(joint) {
                    *transform = Transform::from_matrix(*matrix);
                }
            }
        }
        Drawn::Gltf { root, basis, parents, groups, bindings, .. } => {
            let bindings = bindings.as_deref().unwrap_or_default();
            let placed = crate::model::blend(previous.root, current.root, alpha);
            if let Ok((mut t, _)) = q.get_mut(*root) {
                *t = placed;
            }
            let locals = crate::model::joint_locals(bindings, &previous.joints, &current.joints, *basis, alpha);
            for (binding, transform) in bindings.iter().zip(locals) {
                if let Ok((mut node, _)) = q.get_mut(binding.entity) {
                    *node = transform;
                }
            }
            // (its bodies where they are drawn: each point between the two steps, carried by the root between them)
            let point = |p: engine_core::model::Point| {
                let j = usize::from(p.joint);
                let (a, b) = (previous.joints.get(j)?.transform_point3(p.at), current.joints.get(j)?.transform_point3(p.at));
                Some(placed.transform_point(a.lerp(b, alpha)))
            };
            bodies.set(M::ENGINE, crate::model::bodies(groups, parents, point));
        }
    }
}

/// The engine's own mix, from its player; held with the engine.
fn play_stream<M: Hosted>(
    mut commands: Commands,
    held: Res<engine_api::PadHeld>,
    mut assets: ResMut<Assets<crate::engine_audio::EngineMix>>,
    mut driver: ResMut<Driver<M>>,
) {
    let d = &mut *driver;
    if let Some(stream) = &mut d.stream {
        stream.update(&mut commands, &mut assets, held.pause || !d.placed, &d.emitter);
    }
}

/// Where the player is while the engine has no model of it: a board and a body.
#[derive(Component)]
pub(crate) struct StandIn(Engine);

/// Its deck's height over the player's feet, metres.
const DECK_HEIGHT: f32 = 0.08;

fn spawn_stand_in<M: Hosted>(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let board = materials.add(StandardMaterial { base_color: Color::srgb(0.08, 0.08, 0.09), perceptual_roughness: 0.6, ..default() });
    let body = materials.add(StandardMaterial { base_color: Color::srgb(0.93, 0.45, 0.12), perceptual_roughness: 0.8, ..default() });
    let head = materials.add(StandardMaterial { base_color: Color::srgb(0.85, 0.68, 0.55), perceptual_roughness: 0.8, ..default() });
    commands
        .spawn((StandIn(M::ENGINE), Transform::default(), Visibility::Hidden, Name::new(format!("{} stand-in", M::ENGINE.name()))))
        .with_children(|s| {
            s.spawn((Mesh3d(meshes.add(Cuboid::new(0.21, 0.025, 0.8))), MeshMaterial3d(board), Transform::from_xyz(0.0, DECK_HEIGHT, 0.0)));
            s.spawn((Mesh3d(meshes.add(Capsule3d::new(0.17, 1.05))), MeshMaterial3d(body), Transform::from_xyz(0.0, DECK_HEIGHT + 0.7, 0.0)));
            s.spawn((Mesh3d(meshes.add(Sphere::new(0.12))), MeshMaterial3d(head), Transform::from_xyz(0.0, DECK_HEIGHT + 1.5, 0.0)));
        });
}

/// The stand-in where the player is, between the engine's last two steps.
fn place_stand_in<M: Hosted>(driver: Res<Driver<M>>, mut stand_in: Query<(&StandIn, &mut Transform, &mut Visibility)>) {
    let Some((_, mut transform, mut visibility)) = stand_in.iter_mut().find(|(s, ..)| s.0 == M::ENGINE) else { return };
    let (Some(now), None, true) = (driver.focus[1], &driver.model, driver.placed) else {
        *visibility = Visibility::Hidden;
        return;
    };
    let before = driver.focus[0].unwrap_or(now);
    // (the same time since the last step as the model layer's, in steps)
    let alpha = Step { at: driver.clock, period: driver.period }.alpha(driver.now()).clamp(0.0, 1.0);
    let at = before.feet.lerp(now.feet, alpha);
    let up = now.up.normalize_or(Vec3::Y);
    let forward = now.forward.normalize_or(Vec3::Z);
    let right = up.cross(forward).normalize_or(Vec3::X);
    *transform = Transform::from_translation(at).with_rotation(Quat::from_mat3(&Mat3::from_cols(right, up, right.cross(up))));
    *visibility = Visibility::Inherited;
}
