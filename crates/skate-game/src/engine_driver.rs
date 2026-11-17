//! One driver for every engine (`engine_core::EngineCore`): what hosting a game takes, whichever game it is.
//!
//! - **Start:** picked the first time, the engine is made on a thread of its own from its factory (the game's files
//!   loaded there). A marker file is there while it starts: still there at the next launch, the start crashed the app
//!   (the engine's adapter says so). Once made, its models are spawned and the player is put in front of the player's
//!   eyes, or where a placement asked while it started.
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
use crate::model::{FlatModel, PoseHistories};
use crate::xr::link::{CurrentPark, CurrentStage, PlacePlayer, SkaterLink};
use bevy::prelude::*;
use engine_core::audio::Voice;
use engine_core::model::BodyGroup;
use engine_core::{EngineCore, EngineFactory, Focus, ModelSource, Pose, Step};
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

/// Steps a frame at most: behind more than that, the engine's clock lets the time go.
const MAX_STEPS_PER_FRAME: u32 = 4;
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
    poses: PoseHistories<()>,
    scratch: Vec<Pose>,
    heard: Heard,
    voices: Voices,
    heard_now: Vec<Voice>,
    /// Where the player was after the last two steps.
    focus: [Option<Focus>; 2],
    /// A placement asked before the engine ran.
    pending: Option<PlacePlayer>,
    model: Option<(FlatModel, Vec<BodyGroup>)>,
    hud: engine_core::hud::HudFrame,
    _engine: PhantomData<M>,
}

impl<M: Hosted> Driver<M> {
    fn new(factory: EngineFactory, marker: Option<PathBuf>) -> Self {
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
            heard_now: Vec::new(),
            focus: [None; 2],
            pending: None,
            model: None,
            hud: default(),
            _engine: PhantomData,
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
    pub(crate) fn poses(&self) -> &PoseHistories<()> {
        &self.poses
    }

    #[cfg(test)]
    pub(crate) fn steps(&self) -> u64 {
        self.steps
    }

    #[cfg(test)]
    pub(crate) fn clips(&self) -> usize {
        self.voices.clips()
    }

    #[cfg(test)]
    pub(crate) fn model(&self) -> Option<&FlatModel> {
        self.model.as_ref().map(|(m, _)| m)
    }

    /// Where the player was after the last two steps.
    #[cfg(test)]
    pub(crate) fn focus(&self) -> [Option<Focus>; 2] {
        self.focus
    }
}

/// Hosts the engine `M` made by `factory` (`marker`: the file there while it starts). Call it inside the engine's
/// `engines::add_engine`.
pub(crate) fn install<M: Hosted>(app: &mut App, factory: EngineFactory, marker: Option<PathBuf>) {
    crate::engine_audio::add_voice_source(app);
    app.insert_resource(Driver::<M>::new(factory, marker))
        .add_systems(Startup, (boot::<M>, spawn_stand_in::<M>))
        .add_systems(
            Update,
            (
                finish_boot::<M>,
                input::<M>,
                place::<M>,
                offer::<M>,
                want_hud::<M>,
                step::<M>,
                publish::<M>.before(crate::skate_audio::clean::SkaterSoundSet),
                play::<M>,
                draw_hud::<M>,
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
        );
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
    let worker = std::thread::Builder::new().name(format!("{name}-boot").to_lowercase()).spawn(move || {
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
        let mut hud = world.resource_mut::<crate::xr::ui::hud::TrickHud>();
        if hud.name.is_empty() {
            hud.name = format!("Starting {name}...");
        }
        return;
    }
    let handle = {
        let mut driver = world.resource_mut::<Driver<M>>();
        let phase = driver.phase.get_mut().unwrap_or_else(|p| p.into_inner());
        let Phase::Booting(handle) = std::mem::replace(phase, Phase::Idle(None)) else { return };
        handle
    };
    let made = handle.join().unwrap_or_else(|_| Err(format!("{name} stopped while it started")));
    world.resource_mut::<crate::xr::ui::hud::TrickHud>().name.clear();
    match made {
        Ok(mut core) => {
            let mut spawned = None;
            for model in core.models() {
                match &model.source {
                    ModelSource::Meshes { meshes, textures, joint_count } if spawned.is_none() => {
                        let flat = crate::model::spawn_meshes(world, &format!("{name} player"), meshes, textures, *joint_count);
                        spawned = Some((flat, model.groups.clone()));
                    }
                    _ => warn!("{name}: a model the driver can't draw yet"),
                }
            }
            let pending = {
                let mut driver = world.resource_mut::<Driver<M>>();
                driver.model = spawned;
                let pending = driver.pending.take();
                if let Some(place) = pending {
                    core.place_player(place.0);
                }
                *driver.phase.get_mut().unwrap_or_else(|p| p.into_inner()) = Phase::Running(core);
                pending
            };
            if pending.is_none() {
                crate::xr::ui::pages::put_skater_ahead(world);
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
    let Some(core) = running::<M>(&mut driver.phase) else { return };
    core.set_input(&touch.0, **held);
    let view = rig.and_then(|r| r.root_transform()).zip(eyes.iter().next()).map(|(root, eye)| {
        let turn = root.rotation * eye.rotation;
        [turn * Vec3::X, turn * Vec3::Y, turn * Vec3::NEG_Z]
    });
    core.set_view(view);
}

fn place<M: Hosted>(mut requests: MessageReader<PlacePlayer>, mut driver: ResMut<Driver<M>>) {
    let driver = &mut *driver;
    for r in requests.read() {
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
    let wanted = huds.is_some() && settings.is_some_and(|s| s.hud) && !panel.is_some_and(|p| p.visible);
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
            d.poses.offer(pose, ());
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

/// The engine's voices from where they are, at its level and pitch. While the player is busy elsewhere its loops wait
/// too.
#[allow(clippy::type_complexity)]
fn play<M: Hosted>(
    mut commands: Commands,
    held: Res<engine_api::PadHeld>,
    mut assets: ResMut<Assets<VoicePcm>>,
    mut emitters: Query<(Entity, &VoiceEmitter, &mut Transform, Option<&mut bevy::audio::SpatialAudioSink>)>,
    mut driver: ResMut<Driver<M>>,
) {
    let d = &mut *driver;
    if running::<M>(&mut d.phase).is_none() {
        return;
    }
    // (not in our world yet: what it started can't be placed)
    if d.focus[1].is_none() {
        d.heard.forget_started();
        return;
    }
    d.voices.play(&mut d.heard, held.pause, d.volume, &mut commands, &mut assets, &mut emitters);
}

fn draw_hud<M: Hosted>(mut driver: ResMut<Driver<M>>, mut renderer: crate::engine_hud::HudRenderer) {
    let d = &mut *driver;
    let phase = d.phase.get_mut().unwrap_or_else(|p| p.into_inner());
    let Phase::Running(core) = phase else { return };
    if core.hud(&mut d.hud) {
        renderer.show(M::ENGINE, &d.hud, |id| core.hud_texture(id));
    }
}

/// The models where the engine's poses have them, between its last two steps ([`crate::model::flat_blend`]), and
/// their bodies with them.
fn pose_models<M: Hosted>(
    mut driver: ResMut<Driver<M>>,
    mut q: Query<(&mut Transform, &mut Visibility)>,
    mut bodies: ResMut<engine_api::PlayerBodies>,
) {
    let d = &mut *driver;
    let Some((model, groups)) = &d.model else { return };
    let view = d.poses.get(0).and_then(|h| h.view_at(d.clock + Duration::from_secs_f32(d.accum.max(0.0))));
    let shown = d.focus[1].is_some() && view.is_some();
    if let Ok((_, mut visibility)) = q.get_mut(model.root) {
        *visibility = if shown { Visibility::Inherited } else { Visibility::Hidden };
    }
    let (true, Some((previous, current, alpha))) = (shown, view) else {
        bodies.set(M::ENGINE, Vec::new());
        return;
    };
    let (placed, matrices) = crate::model::flat_blend(previous, current, alpha, model.joints.len());
    // (the parts' own transform - the root's - is where the model is: skinning ignores it, but culling places the
    // parts' bind-pose bounds with it)
    if let Ok((mut t, _)) = q.get_mut(model.root) {
        *t = Transform::from_matrix(placed);
    }
    // (its bodies where they are drawn: each point carried by its joint as blended)
    let point = |p: engine_core::model::Point| Some(matrices.get(usize::from(p.joint))?.transform_point3(p.at));
    bodies.set(M::ENGINE, crate::model::bodies(groups, &[], point));
    for (&joint, matrix) in model.joints.iter().zip(&matrices) {
        if let Ok((mut transform, _)) = q.get_mut(joint) {
            *transform = Transform::from_matrix(*matrix);
        }
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
    let (Some(now), None) = (driver.focus[1], &driver.model) else {
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
