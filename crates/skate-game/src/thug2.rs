//! THUG2 in the headset: the game's own engine (`thug2-sys`: thug2-decomp's code, run headless) as the app's third
//! engine, skating the room and the parks.
//!
//! It runs on the player's own THUG2 files, in `<app files>/thug2` (the folder holding `Data`; `THUG2_GAME_DIR`
//! elsewhere); without them the engine can't be picked. The game draws nothing itself: its skater - the model it built
//! from those files, posed by its own animation every frame - is drawn here ([`spawn_skater`], [`pose_skater`]); a
//! stand-in only if the model can't be read.
//!
//! - Start: picked the first time, the game boots on a thread of its own (its scripts and an empty level), then the
//!   skater is put in front of the player, or where the last engine's skater was.
//! - World: the room and the park pieces become its collision ([`triangles`]), the rails its rails. The game's units
//!   are inches, y up, right-handed like ours, so metres map onto it by scale; it is offset ([`OFFSET`]) clear of
//!   the empty level's own scene.
//! - Frames: the game steps at its 60 Hz from the headset's frames (several a frame at most), and waits while a menu
//!   or build mode has the controllers.
//! - Pad: the Touch controllers as a PlayStation 2 pad ([`CONTROLS`]).
use crate::engines::{self, Engine, EngineSet};
use crate::xr::link::{ArWorld, PieceSurfaces, SkaterLink, TeleportSkater};
use crate::xr::touch::TouchState;
use bevy::prelude::*;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use thug2_sys as t2;

/// The game's frame.
const STEP: f32 = 1.0 / 60.0;
const MAX_STEPS_PER_FRAME: u32 = 4;
/// Game frames run at boot, before the skater is put anywhere (the level puts it at its own start first).
const SETTLE_FRAMES: u32 = 120;
/// Where our origin is in the game's world (inches): clear of the empty level's sky.
const OFFSET: Vec3 = Vec3::new(50_000.0, 0.0, 50_000.0);
/// A face whose normal is less up than this is upright (a wall: steeper than 75°); one more up than `RAMP_UP` is level
/// ground (under 18°), between them a ramp. A piece's transition steeper than `VERT_UP` (45°), joined to the ramp below
/// it, is vert: the game's vert air, spine transfers and hips need it (they look for vert faces steeper than 45°).
const WALL_UP: f32 = 0.26;
const RAMP_UP: f32 = 0.95;
const VERT_UP: f32 = 0.707;
/// Where the deck is above the skater's feet, metres.
const DECK_HEIGHT: f32 = 0.08;
const PRESSED: f32 = 0.5;

/// The Touch controllers as THUG2's pad.
const CONTROLS: [(&str, &str); 8] = [
    ("Either stick", "steer, push (up), trick directions"),
    ("A / B", "ollie (X) / grind (triangle)"),
    ("X / Y", "flip trick (square) / grab trick (circle)"),
    ("Grips", "L1 / R1: spin left / right"),
    ("Triggers", "L2 / R2: revert, nollie, switch"),
    ("Left grip + trigger", "aim, let go to put the skater there"),
    ("Left menu (tap)", "this menu"),
    ("Left menu (hold 1 s)", "build mode"),
];

/// Where the game's files are, and where it may write.
#[derive(Resource, Clone)]
struct Thug2Files {
    game: PathBuf,
    cache: PathBuf,
}

/// The game's folder for the app's files folder.
fn game_folder(files: &Path) -> PathBuf {
    std::env::var_os("THUG2_GAME_DIR").map(PathBuf::from).unwrap_or_else(|| files.join("thug2"))
}

fn has_data(game: &Path) -> bool {
    std::fs::read_dir(game).into_iter().flatten().flatten().any(|e| {
        e.file_name().to_string_lossy().eq_ignore_ascii_case("data") && e.path().is_dir()
    })
}

fn available(world: &World) -> Result<(), String> {
    if let Some(Thug2 { phase: Phase::Failed(why), .. }) = world.get_resource::<Thug2>() {
        return Err(why.clone());
    }
    let files = world.resource::<Thug2Files>();
    if has_data(&files.game) {
        Ok(())
    } else {
        Err("needs your THUG2 files: docs/THUG2.md".into())
    }
}

/// Adds THUG2 to the app's engines.
pub(crate) fn install(app: &mut App, files: &Path) {
    let cache = files.join("thug2-cache");
    // (the game stopped the app while it started last time - a file missing, most likely: the app opens on another
    // engine, and says so)
    let crashed = cache.join(STARTING).exists();
    if crashed {
        warn!("THUG2 stopped the app while it started last time: opening on another engine");
        let _ = std::fs::remove_file(cache.join(STARTING));
    }
    {
        use bevy::audio::AddAudioSource;
        app.add_audio_source::<Thug2Pcm>();
    }
    app.insert_resource(Thug2Files { game: game_folder(files), cache })
        .insert_resource(Tuning::load(&files.join("thug2.json")))
        .insert_resource(TuningFile(files.join("thug2.json")));
    app.world_mut().resource_mut::<crate::xr::ui::UiProviders>().0.push(std::sync::Arc::new(Thug2Pages));
    app.init_resource::<engines::CourseSettings>();
    app.world_mut().resource_mut::<engines::CourseSettings>().0.insert(Engine::Thug2, (park_settings, apply_park_settings));
    app.world_mut()
        .resource_mut::<crate::xr::ui::ControlsSheet>()
        .set(Engine::Thug2, CONTROLS.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect());
    crate::engine_hud::install(app);
    engines::add_engine(app, Engine::Thug2, available, |app| {
        app.init_resource::<Thug2>()
            .init_resource::<Thug2Sounds>()
            .add_systems(Startup, (boot, spawn_stand_in, hud_parts))
            .add_systems(
                Update,
                (finish_boot, read_pad, teleport, apply_world, want_hud, step, publish, play_sounds, draw_hud)
                    .chain()
                    .after(crate::xr::ui::UiSet)
                    .in_set(EngineSet(Engine::Thug2)),
            )
            .add_systems(
                PostUpdate,
                (place_stand_in, pose_skater)
                    .before(bevy::transform::TransformSystems::Propagate)
                    .in_set(EngineSet(Engine::Thug2)),
            );
    });
    // (outside THUG2's systems: it shoots whichever engine's skater - another's skinned skater, to compare)
    if std::env::var_os("THUG2_SHOT").is_some() {
        app.add_systems(Update, shoot.after(crate::xr::ui::UiSet));
    }
    if crashed {
        app.world_mut().resource_mut::<engines::Engines>().not_at_launch(Engine::Thug2);
        app.add_systems(Update, |mut ui: ResMut<crate::xr::ui::XrUi>, mut said: Local<bool>| {
            if !std::mem::replace(&mut *said, true) {
                ui.say("THUG2 crashed while starting last time: check its files (docs/THUG2.md)");
            }
        });
    }
}

/// Made while the game starts, gone once it runs: still there at launch, the last start crashed the app.
const STARTING: &str = "starting";

type Booted = Result<(t2::Engine, Option<t2::Skater>, Option<t2::SkaterModel>), String>;

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Booting(JoinHandle<Booted>),
    Running(t2::Engine),
    Failed(String),
}

#[derive(Resource, Default)]
struct Thug2 {
    phase: Phase,
    pad: t2::Pad,
    /// A menu or build mode has the controllers: the game waits.
    hold: bool,
    accum: f32,
    previous: Option<t2::Skater>,
    current: Option<t2::Skater>,
    /// The `ArWorld` revision the game has, and the world size it was given at.
    world_given: Option<u64>,
    scale_given: f32,
    /// Where to put the skater, waiting for the game or the world.
    spawn: Option<t2::Spawn>,
    /// The game has stepped since it was given our world (the skater is in it, not at the level's own start).
    placed: bool,
    /// The skater's pose after the last step (for its drawn model).
    pose: t2::Pose,
    /// What the game played in the steps since the app last played it (`play_sounds`).
    heard: Heard,
    /// The game takes its 2D down (`want_hud`), and the last step's (`draw_hud`).
    hud_on: bool,
    hud: t2::Hud,
}

/// The game's sound effects as its steps leave them.
#[derive(Default)]
struct Heard {
    /// The voices playing after the last step.
    voices: Vec<t2::Voice>,
    /// The voices the game played from their start in the steps since (a short one may have run out already).
    started: Vec<t2::Voice>,
    /// The samples of sounds the game played for the first time.
    new_sounds: Vec<(u32, t2::Sound)>,
    fetched: std::collections::HashSet<u32>,
    starts: std::collections::HashMap<u32, u32>,
    scratch: Vec<t2::Voice>,
}

impl Heard {
    /// After a step: the voices it started, and the samples of sounds new to the app.
    fn after_step(&mut self, engine: &mut t2::Engine) {
        engine.voices(&mut self.scratch);
        for v in &self.scratch {
            if self.starts.insert(v.voice, v.starts) == Some(v.starts) {
                continue;
            }
            self.started.retain(|s| s.voice != v.voice);
            self.started.push(*v);
            if self.fetched.insert(v.sound) {
                match engine.sound(v.sound) {
                    Some(sound) => self.new_sounds.push((v.sound, sound)),
                    None => warn!("THUG2: sound {} has no samples", v.sound),
                }
            }
        }
        std::mem::swap(&mut self.voices, &mut self.scratch);
    }
}

fn boot(files: Res<Thug2Files>, mut thug2: ResMut<Thug2>) {
    let Thug2Files { game, cache } = files.clone();
    let _ = std::fs::create_dir_all(&cache);
    info!("THUG2 starting from {}", game.display());
    let _ = std::fs::write(cache.join(STARTING), b"");
    let worker = std::thread::Builder::new().name("thug2-boot".into()).spawn(move || -> Booted {
        let started = std::time::Instant::now();
        let mut engine = t2::boot(&game, Some(&cache), &["-level", "Default"]).map_err(|e| e.to_string())?;
        let mut skater = None;
        for _ in 0..SETTLE_FRAMES {
            skater = engine.step(&t2::PAD_AT_REST).map_err(|e| e.to_string())?;
        }
        // (the skater's look, as the game built it: drawn here in place of the stand-in)
        let model = engine.skater_model();
        info!(
            "THUG2 ready in {:.0} ms (skater {}, model: {} meshes, {} textures)",
            started.elapsed().as_secs_f32() * 1000.0,
            skater.is_some(),
            model.as_ref().map_or(0, |m| m.meshes.len()),
            model.as_ref().map_or(0, |m| m.textures.len())
        );
        let _ = std::fs::remove_file(cache.join(STARTING));
        Ok((engine, skater, model))
    });
    thug2.phase = match worker {
        Ok(handle) => Phase::Booting(handle),
        Err(e) => Phase::Failed(format!("THUG2 didn't start: {e}")),
    };
}

/// The game ready: running, and the skater put in front of the player unless a place is waiting already.
fn finish_boot(world: &mut World) {
    let finished = match &world.resource::<Thug2>().phase {
        Phase::Booting(handle) => handle.is_finished(),
        _ => return,
    };
    if !finished {
        let mut hud = world.resource_mut::<crate::xr::ui::hud::TrickHud>();
        if hud.name.is_empty() {
            hud.name = "Starting THUG2...".into();
        }
        return;
    }
    let Phase::Booting(handle) = std::mem::take(&mut world.resource_mut::<Thug2>().phase) else { return };
    let result = handle.join().unwrap_or_else(|_| Err("THUG2 stopped while it started".into()));
    world.resource_mut::<crate::xr::ui::hud::TrickHud>().name.clear();
    match result {
        Ok((engine, skater, model)) => {
            if let Some(model) = model {
                spawn_skater(world, &model);
            }
            let waiting = {
                let mut thug2 = world.resource_mut::<Thug2>();
                thug2.phase = Phase::Running(engine);
                thug2.previous = skater;
                thug2.current = skater;
                thug2.spawn.is_some()
            };
            if !waiting {
                crate::xr::ui::pages::put_skater_ahead(world);
            }
        }
        Err(why) => {
            error!("THUG2: {why}");
            world.resource_mut::<crate::xr::ui::XrUi>().say(format!("THUG2: {why}"));
            world.resource_mut::<Thug2>().phase = Phase::Failed(why);
        }
    }
}

fn read_pad(touch: Res<engine_api::LatestTouch>, held: Res<engine_api::PadHeld>, mut thug2: ResMut<Thug2>) {
    let t = &touch.0;
    thug2.hold = held.pause;
    thug2.pad = if held.held || t.menu { t2::PAD_AT_REST } else { pad(t) };
}

/// The controllers as the game's pad: sticks 0-255 (0x80 at rest, up and left 0).
fn pad(t: &TouchState) -> t2::Pad {
    use t2::buttons::*;
    let axis = |v: f32| (128.0 + v.clamp(-1.0, 1.0) * 127.0).round() as u8;
    let mut buttons = 0;
    for (on, bit) in [
        // (laid out as a pad's: ollie and grind on the right face buttons, flip and grab on the left's, the grips
        // the shoulder buttons and the triggers the triggers)
        (t.a, CROSS),
        (t.b, TRIANGLE),
        (t.x, SQUARE),
        (t.y, CIRCLE),
        (t.left_grip > PRESSED, L1),
        (t.right_grip > PRESSED, R1),
        (t.left_trigger > PRESSED, L2),
        (t.right_trigger > PRESSED, R2),
    ] {
        if on {
            buttons |= bit;
        }
    }
    // Either stick is the game's direction stick (the pad's left; its right stick does nothing in the game): the
    // further pushed one, so a thumb can leave its stick for a button (the left one for flip and grab, X / Y).
    let length = |v: [f32; 2]| v[0] * v[0] + v[1] * v[1];
    let stick = if length(t.right_stick) > length(t.left_stick) { t.right_stick } else { t.left_stick };
    t2::Pad { left_x: axis(stick[0]), left_y: axis(-stick[1]), right_x: 0x80, right_y: 0x80, buttons }
}

/// THUG2's HUD (`thug2_engine::hud`: its parts and where they go) drawn by the app's HUD renderer. Its parts are
/// there from the start, empty (the corner readout gives way to them).
fn hud_parts(mut renderer: crate::engine_hud::HudRenderer) {
    let mut frame = engine_core::hud::HudFrame::default();
    thug2_engine::hud::frame(&t2::Hud::default(), false, &mut frame);
    renderer.show(Engine::Thug2, &frame, |_| None);
}

/// The game takes its 2D down only while it's seen: the HUD on and no menu over it.
fn want_hud(
    settings: Option<Res<crate::xr::ar_settings::ArSettings>>,
    panel: Option<Res<crate::xr::panel::XrPanel>>,
    huds: Option<Res<crate::engine_hud::EngineHuds>>,
    mut thug2: ResMut<Thug2>,
) {
    let wanted = huds.is_some() && settings.is_some_and(|s| s.hud) && !panel.is_some_and(|p| p.visible);
    let thug2 = &mut *thug2;
    let Phase::Running(engine) = &mut thug2.phase else { return };
    if thug2.hud_on != wanted {
        engine.hud_enable(wanted);
        thug2.hud_on = wanted;
        thug2.hud = t2::Hud::default();
    }
}

/// The last step's 2D drawn, and where the skater is for the balance meter at it.
fn draw_hud(
    mut thug2: ResMut<Thug2>,
    mut renderer: crate::engine_hud::HudRenderer,
    mut focus: ResMut<crate::engine_hud::HudFocus>,
    mut frame: Local<engine_core::hud::HudFrame>,
) {
    let thug2 = &mut *thug2;
    let scale = thug2.scale_given.max(1e-3);
    focus.set(
        Engine::Thug2,
        thug2.current.filter(|_| thug2.placed).map(|s| engine_core::Focus {
            feet: from_game(s.position, scale),
            up: Vec3::from_array(s.up).normalize_or(Vec3::Y),
            height: thug2_engine::hud::SKATER_HEIGHT / scale,
        }),
    );
    let Phase::Running(engine) = &mut thug2.phase else { return };
    let balancing = thug2.current.is_some_and(|s| s.balance.is_some());
    thug2_engine::hud::frame(&thug2.hud, balancing, &mut frame);
    renderer.show(Engine::Thug2, &frame, |id| engine.hud_texture(thug2_engine::hud::game_texture(id)).map(thug2_engine::hud::texture));
}

/// Our metres as the game's units: `scale` is the world size ([`Tuning::scale`]: how much bigger the room is to the
/// game than it is to the player).
fn to_game(p: Vec3, scale: f32) -> [f32; 3] {
    (p * t2::INCHES_PER_METRE * scale + OFFSET).to_array()
}

fn from_game(p: [f32; 3], scale: f32) -> Vec3 {
    (Vec3::from_array(p) - OFFSET) / (t2::INCHES_PER_METRE * scale)
}

/// How THUG2 plays in the room, kept in `thug2.json` beside the app's other settings.
#[derive(Resource, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Tuning {
    /// The game's clock: 1 is the game's own speed, less slows everything (jumps float).
    speed: f32,
    /// The world size: the room is this much bigger to the game than it is, so the skater crosses it more slowly,
    /// and is drawn that much smaller (ramps and rails keep their size to the player).
    scale: f32,
    /// The game's sound effects' level: 1 is as the game sets them.
    volume: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self { speed: 1.0, scale: 1.0, volume: 1.0 }
    }
}

const SPEED_RANGE: (f32, f32, f32) = (0.4, 1.0, 0.05);
const SCALE_RANGE: (f32, f32, f32) = (1.0, 4.0, 0.25);
const VOLUME_RANGE: (f32, f32, f32) = (0.0, 1.5, 0.05);

impl Tuning {
    fn load(file: &Path) -> Self {
        let t: Tuning = std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Tuning {
            speed: t.speed.clamp(SPEED_RANGE.0, SPEED_RANGE.1),
            scale: t.scale.clamp(SCALE_RANGE.0, SCALE_RANGE.1),
            volume: t.volume.clamp(VOLUME_RANGE.0, VOLUME_RANGE.1),
        }
    }
}

#[derive(Resource)]
struct TuningFile(PathBuf);

/// `t` in use, and kept for the next launch.
fn keep(world: &mut World, t: Tuning) {
    world.insert_resource(t);
    let file = world.resource::<TuningFile>().0.clone();
    if let Ok(json) = serde_json::to_vec_pretty(&t) {
        let _ = std::fs::write(&file, json);
    }
}

/// What a park keeps of THUG2's settings: the world size and game speed it was built for (the
/// sound's level is the player's own).
#[derive(serde::Serialize, serde::Deserialize)]
struct ParkTuning {
    scale: f32,
    speed: f32,
}

fn park_settings(world: &World) -> serde_json::Value {
    let t = world.resource::<Tuning>();
    serde_json::to_value(ParkTuning { scale: t.scale, speed: t.speed }).unwrap_or_default()
}

fn apply_park_settings(world: &mut World, settings: &serde_json::Value) {
    let Ok(park) = serde_json::from_value::<ParkTuning>(settings.clone()) else { return };
    let mut t = *world.resource::<Tuning>();
    t.scale = park.scale.clamp(SCALE_RANGE.0, SCALE_RANGE.1);
    t.speed = park.speed.clamp(SPEED_RANGE.0, SPEED_RANGE.1);
    if t != *world.resource::<Tuning>() {
        keep(world, t);
    }
}

/// THUG2's page in the headset menu (shown while it runs).
struct Thug2Pages;

const PAGE: crate::xr::ui::PageId = "thug2";

impl crate::xr::ui::UiProvider for Thug2Pages {
    fn page(&self, id: crate::xr::ui::PageId, world: &World) -> Option<crate::xr::ui::UiPage> {
        use crate::xr::ui::{UiPage, Widget};
        if id != PAGE {
            return None;
        }
        let t = *world.resource::<Tuning>();
        let mut w = vec![
            Widget::slider("scale", "World size", t.scale, SCALE_RANGE, format!("{:.2}x", t.scale)),
            Widget::slider("speed", "Game speed", t.speed, SPEED_RANGE, format!("{:.0}%", t.speed * 100.0)),
            Widget::button("reset", "Back to the game's own (1x, 100%)"),
            Widget::slider("volume", "Sound volume", t.volume, VOLUME_RANGE, format!("{:.0}%", t.volume * 100.0)),
        ];
        w.extend([
            Widget::info(""),
            Widget::info("World size: the room is that much bigger to the game. The skater"),
            Widget::info("crosses it slower and is drawn smaller; ramps and rails stay put."),
            Widget::info("Game speed: the game's clock. Slower keeps sizes, but jumps float."),
        ]);
        Some(UiPage::new("THUG2", w))
    }

    fn act(
        &self,
        page: crate::xr::ui::PageId,
        widget: &'static str,
        act: crate::xr::ui::Act,
        world: &mut World,
    ) -> Option<crate::xr::ui::Outcome> {
        use crate::xr::ui::{Act, Outcome};
        if page != PAGE {
            return None;
        }
        let mut t = *world.resource::<Tuning>();
        match (widget, act) {
            ("scale", Act::Set(v)) => t.scale = v,
            ("speed", Act::Set(v)) => t.speed = v,
            ("volume", Act::Set(v)) => t.volume = v,
            ("reset", Act::Press) => t = Tuning { volume: t.volume, ..Tuning::default() },
            _ => return Some(Outcome::Stay),
        }
        keep(world, t);
        Some(Outcome::Stay)
    }

    fn start_items(&self, _world: &World) -> Vec<crate::xr::ui::Widget> {
        vec![crate::xr::ui::Widget::link("THUG2: world size, game speed, sound", PAGE)]
    }

    fn engine(&self) -> Option<Engine> {
        Some(Engine::Thug2)
    }
}

fn teleport(mut requests: MessageReader<TeleportSkater>, tuning: Res<Tuning>, mut thug2: ResMut<Thug2>) {
    for r in requests.read() {
        let spawn = t2::Spawn {
            point: to_game(r.point + Vec3::Y * 0.1, tuning.scale),
            heading_degrees: t2::heading_of(r.forward.to_array()),
        };
        let given = thug2.world_given.is_some();
        match &mut thug2.phase {
            Phase::Running(engine) if given => engine.teleport(spawn),
            _ => thug2.spawn = Some(spawn),
        }
    }
}

/// The room and pieces given to the game when they change (not mid-grind: the rails would go from under it), once
/// there is somewhere to put the skater.
fn apply_world(world: Res<ArWorld>, tuning: Res<Tuning>, mut thug2: ResMut<Thug2>) {
    let grinding = thug2.current.is_some_and(|s| s.state == t2::State::Rail);
    let rescaled = thug2.world_given.is_some() && thug2.scale_given != tuning.scale;
    if world.revision == 0 || (thug2.world_given == Some(world.revision) && !rescaled) || grinding {
        return;
    }
    // (a new world size: the skater put back where it is in the room, in the new units)
    if rescaled && thug2.spawn.is_none()
        && let Some(s) = thug2.current
    {
        let at = from_game(s.position, thug2.scale_given);
        thug2.spawn = Some(t2::Spawn {
            point: to_game(at + Vec3::Y * 0.1, tuning.scale),
            heading_degrees: t2::heading_of(s.forward),
        });
    }
    if thug2.spawn.is_none() && thug2.world_given.is_none() {
        return;
    }
    let spawn = thug2.spawn.take();
    let Phase::Running(engine) = &mut thug2.phase else {
        thug2.spawn = spawn;
        return;
    };
    let faces = triangles(&world, tuning.scale);
    let rails: Vec<Vec<[f32; 3]>> =
        world.rails.iter().map(|r| r.iter().map(|&p| to_game(p, tuning.scale)).collect()).collect();
    engine.set_world(&faces, spawn);
    engine.set_rails(&rails);
    info!("THUG2 world: {} faces, {} rails (AR revision {})", faces.len(), rails.len(), world.revision);
    thug2.world_given = Some(world.revision);
    thug2.scale_given = tuning.scale;
}

fn step(time: Res<Time<Real>>, tuning: Res<Tuning>, mut thug2: ResMut<Thug2>) {
    let thug2 = &mut *thug2;
    let Phase::Running(engine) = &mut thug2.phase else { return };
    if thug2.hold || thug2.world_given.is_none() {
        thug2.accum = 0.0;
        return;
    }
    thug2.accum += time.delta_secs().min(0.1) * tuning.speed;
    let mut steps = 0;
    while thug2.accum >= STEP && steps < MAX_STEPS_PER_FRAME {
        thug2.accum -= STEP;
        steps += 1;
        match engine.step(&thug2.pad) {
            Ok(skater) => {
                thug2.previous = thug2.current;
                thug2.current = skater;
                if steps == MAX_STEPS_PER_FRAME || thug2.accum < STEP {
                    // (the last step this frame: its pose is the one drawn, and its 2D)
                    engine.skater_pose(&mut thug2.pose);
                    if thug2.hud_on {
                        engine.hud(&mut thug2.hud);
                    }
                }
                // (the world and the skater's place are taken at the start of a step: from now it is in ours)
                thug2.placed = true;
                thug2.heard.after_step(engine);
            }
            Err(e) => {
                error!("THUG2: {e}");
                thug2.phase = Phase::Failed(e.to_string());
                return;
            }
        }
    }
    if steps == MAX_STEPS_PER_FRAME {
        thug2.accum = thug2.accum.min(STEP);
    }
}

/// The game's sound effects as an audio source of THUG2's own (the decoded samples' source, `StreamPcm`, is the
/// Skate 3 tier's, which may not be in the app).
#[derive(Asset, TypePath, Clone)]
struct Thug2Pcm(crate::stream_pcm::StreamPcm);

impl bevy::audio::Decodable for Thug2Pcm {
    type DecoderItem = f32;
    type Decoder = crate::stream_pcm::PcmPlayback;

    fn decoder(&self) -> Self::Decoder {
        bevy::audio::Decodable::decoder(&self.0)
    }
}

/// The sounds' samples, and each as a clip once and looping.
#[derive(Resource, Default)]
struct Thug2Sounds {
    samples: std::collections::HashMap<u32, (std::sync::Arc<Vec<i16>>, u16, u32)>,
    clips: std::collections::HashMap<(u32, bool), Handle<Thug2Pcm>>,
}

/// An emitter playing one of the game's voices.
#[derive(Component)]
struct VoiceEmitter {
    voice: u32,
    looping: bool,
}

/// Where the skater's sounds come from: its board, a little up from its feet (metres at its own size).
const SOUND_HEIGHT: f32 = 0.3;

/// The game's sound effects from the skater, heard through the headset (`xr::spatial_audio`): each voice the game
/// plays, an emitter at the skater playing its sound at the game's level and pitch. A voice played again from its
/// start is a new emitter; a looping one goes when the game stops it, one played once when it has run out. While the
/// game waits (a menu, build mode) its loops wait too.
#[allow(clippy::type_complexity)]
fn play_sounds(
    mut commands: Commands,
    mut thug2: ResMut<Thug2>,
    tuning: Res<Tuning>,
    mut sounds: ResMut<Thug2Sounds>,
    mut assets: ResMut<Assets<Thug2Pcm>>,
    mut emitters: Query<(Entity, &VoiceEmitter, &mut Transform, Option<&mut bevy::audio::SpatialAudioSink>)>,
) {
    use bevy::audio::{AudioSinkPlayback, Volume};
    let thug2 = &mut *thug2;
    for (id, sound) in thug2.heard.new_sounds.drain(..) {
        sounds.samples.insert(id, (std::sync::Arc::new(sound.samples), sound.channels, sound.rate));
    }
    let started = std::mem::take(&mut thug2.heard.started);
    let Some(s) = thug2.current.filter(|_| thug2.placed) else { return };
    let scale = thug2.scale_given.max(1e-3);
    let at = from_game(s.position, scale) + Vec3::from_array(s.up).normalize_or(Vec3::Y) * (SOUND_HEIGHT / scale);
    let voices = &thug2.heard.voices;
    for (entity, emitter, mut transform, sink) in &mut emitters {
        let now = voices.iter().find(|v| v.voice == emitter.voice);
        let replaced = started.iter().any(|v| v.voice == emitter.voice);
        if replaced || (emitter.looping && now.is_none()) {
            commands.entity(entity).despawn();
            continue;
        }
        transform.translation = at;
        if let (Some(v), Some(mut sink)) = (now, sink) {
            sink.set_volume(Volume::Linear(v.gain * tuning.volume));
            sink.set_speed(v.speed);
            match (thug2.hold, sink.is_paused()) {
                (true, false) => sink.pause(),
                (false, true) => sink.play(),
                _ => {}
            }
        }
    }
    if thug2.hold {
        return;
    }
    for v in started {
        let Some((samples, channels, rate)) = sounds.samples.get(&v.sound).cloned() else { continue };
        let clip = sounds
            .clips
            .entry((v.sound, v.looping))
            .or_insert_with(|| {
                assets.add(Thug2Pcm(crate::stream_pcm::StreamPcm::from_i16(samples, channels, rate, v.looping)))
            })
            .clone();
        let settings = if v.looping { PlaybackSettings::ONCE } else { PlaybackSettings::DESPAWN };
        commands.spawn((
            VoiceEmitter { voice: v.voice, looping: v.looping },
            AudioPlayer(clip),
            settings.with_spatial(true).with_volume(Volume::Linear(v.gain * tuning.volume)).with_speed(v.speed),
            Transform::from_translation(at),
            Name::new("THUG2 voice"),
        ));
    }
}

fn publish(thug2: Res<Thug2>, mut link: ResMut<SkaterLink>) {
    let Some(s) = thug2.current.filter(|_| thug2.placed) else { return };
    let up = Vec3::from_array(s.up);
    let scale = thug2.scale_given;
    link.deck = Some((from_game(s.position, scale) + up * DECK_HEIGHT / scale, Vec3::from_array(s.forward)));
    link.grinding = s.state == t2::State::Rail;
    link.size = 1.0 / scale.max(1e-3);
}

/// The room and pieces as the game's collision faces (game units, offset). The game rides a face only from its
/// front, so each is turned the way it is met:
/// - ground (level to a steep ramp): facing up, skatable;
/// - a piece's transition steeper than 45° above a ramp (a quarter pipe's top half): vert, facing the ramp, so the
///   skater goes straight up and comes back down into it - or transfers to a vert face behind the coping (a spine);
/// - any other upright face (room walls, furniture and box sides): both ways, wallridable.
pub(crate) fn triangles(world: &ArWorld, scale: f32) -> Vec<t2::Triangle> {
    let mut out = Vec::new();
    for t in world.room() {
        face(&mut out, t, None, scale);
    }
    for piece in &world.pieces {
        piece_faces(&mut out, piece, scale);
    }
    out
}

fn normal(t: [Vec3; 3]) -> Vec3 {
    (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero()
}

/// One face; `vert`: it is a quarter pipe's top, facing (across) `vert`.
fn face(out: &mut Vec<t2::Triangle>, t: [Vec3; 3], vert: Option<Vec3>, scale: f32) {
    use t2::face::*;
    let n = normal(t);
    if n == Vec3::ZERO {
        return;
    }
    let flipped = [t[0], t[2], t[1]];
    let game = |t: [Vec3; 3], flags: u16| t2::Triangle { v: t.map(|p| to_game(p, scale)), flags, terrain: 0 };
    if let Some(toward) = vert {
        let t = if n.with_y(0.0).dot(toward.with_y(0.0)) >= 0.0 { t } else { flipped };
        out.push(game(t, SKATABLE | VERT));
    } else if n.y.abs() >= WALL_UP {
        // (ground, or a slope or overhang turned to face up)
        out.push(game(if n.y > 0.0 { t } else { flipped }, SKATABLE));
    } else {
        out.push(game(t, SKATABLE | WALL));
        out.push(game(flipped, SKATABLE | WALL));
    }
}

/// A piece's faces, its quarter pipes' tops found: a quad steeper than 45° whose lower edge is the upper edge of a
/// gentler ramp quad (or of another such top) is vert, facing that ramp.
fn piece_faces(out: &mut Vec<t2::Triangle>, piece: &PieceSurfaces, scale: f32) {
    let quads: Vec<[Vec3; 4]> = piece.quads.iter().chain(&piece.rail_quads).copied().collect();
    let up_normal = |q: &[Vec3; 4]| {
        let n = normal([q[0], q[1], q[2]]);
        if n.y < 0.0 { -n } else { n }
    };
    // (a quad's two lowest corners)
    let bottom = |q: &[Vec3; 4]| {
        let mut c = *q;
        c.sort_by(|a, b| a.y.total_cmp(&b.y));
        [c[0], c[1]]
    };
    let has = |q: &[Vec3; 4], p: Vec3| q.iter().any(|c| c.distance_squared(p) < 1e-4);
    // Each quad's vert facing: none, or the ramp's way.
    let mut vert: Vec<Option<Vec3>> = vec![None; quads.len()];
    for _ in 0..quads.len().min(32) {
        let mut changed = false;
        for (i, q) in quads.iter().enumerate() {
            if vert[i].is_some() || up_normal(q).y >= VERT_UP {
                continue;
            }
            let [a, b] = bottom(q);
            let below = quads.iter().enumerate().find_map(|(j, r)| {
                let n = up_normal(r);
                let ramp = n.y >= VERT_UP && n.y < RAMP_UP;
                let joins = j != i && has(r, a) && has(r, b);
                if !joins {
                    None
                } else if ramp {
                    Some(n)
                } else {
                    vert[j]
                }
            });
            if let Some(toward) = below.filter(|n| n.with_y(0.0).length_squared() > 1e-6) {
                vert[i] = Some(toward);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (q, v) in quads.iter().zip(vert) {
        let [a, b, c, d] = *q;
        face(out, [a, b, c], v, scale);
        face(out, [a, c, d], v, scale);
    }
}

/// The skater's stand-in: a board and a body.
#[derive(Component)]
struct StandIn;

fn spawn_stand_in(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let board = materials.add(StandardMaterial { base_color: Color::srgb(0.08, 0.08, 0.09), perceptual_roughness: 0.6, ..default() });
    let body = materials.add(StandardMaterial { base_color: Color::srgb(0.93, 0.45, 0.12), perceptual_roughness: 0.8, ..default() });
    let head = materials.add(StandardMaterial { base_color: Color::srgb(0.85, 0.68, 0.55), perceptual_roughness: 0.8, ..default() });
    commands
        .spawn((StandIn, Transform::default(), Visibility::Hidden, Name::new("THUG2 skater")))
        .with_children(|s| {
            s.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.21, 0.025, 0.8))),
                MeshMaterial3d(board),
                Transform::from_xyz(0.0, DECK_HEIGHT, 0.0),
            ));
            s.spawn((
                Mesh3d(meshes.add(Capsule3d::new(0.17, 1.05))),
                MeshMaterial3d(body),
                Transform::from_xyz(0.0, DECK_HEIGHT + 0.7, 0.0),
            ));
            s.spawn((
                Mesh3d(meshes.add(Sphere::new(0.12))),
                MeshMaterial3d(head),
                Transform::from_xyz(0.0, DECK_HEIGHT + 1.5, 0.0),
            ));
        });
}

/// The stand-in where the skater is, between the game's last two frames.
fn place_stand_in(
    thug2: Res<Thug2>,
    model: Option<Res<SkaterJoints>>,
    mut stand_in: Query<(&mut Transform, &mut Visibility), With<StandIn>>,
) {
    let Ok((mut transform, mut visibility)) = stand_in.single_mut() else { return };
    let (Some(now), true, None) = (thug2.current, thug2.placed, model) else {
        *visibility = Visibility::Hidden;
        return;
    };
    let before = thug2.previous.unwrap_or(now);
    let alpha = (thug2.accum / STEP).clamp(0.0, 1.0);
    let scale = thug2.scale_given;
    let at = from_game(before.position, scale).lerp(from_game(now.position, scale), alpha);
    let up = Vec3::from_array(now.up).normalize_or(Vec3::Y);
    let forward = Vec3::from_array(now.forward).normalize_or(Vec3::Z);
    let right = up.cross(forward).normalize_or(Vec3::X);
    *transform = Transform::from_translation(at)
        .with_rotation(Quat::from_mat3(&Mat3::from_cols(right, up, right.cross(up))))
        .with_scale(Vec3::splat(1.0 / scale));
    *visibility = Visibility::Inherited;
}

/// The game's skater drawn: its meshes skinned to joints set from its pose each frame.
#[derive(Resource)]
struct SkaterJoints {
    joints: Vec<Entity>,
    root: Entity,
}

#[derive(Component)]
struct SkaterPart;

/// The skater's model as Bevy meshes: bind-pose vertices (game units), skinned by joints whose matrices are the game's
/// palette carried into our world (`pose_skater`) - inverse bind poses identity, as the palette has them in.
fn spawn_skater(world: &mut World, model: &t2::SkaterModel) {
    use bevy::asset::RenderAssetUsages;
    use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
    use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
    use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    const JOINTS: usize = 64;
    let images: Vec<Handle<Image>> = {
        let mut images = world.resource_mut::<Assets<Image>>();
        model
            .textures
            .iter()
            .map(|t| {
                let mut image = Image::new(
                    Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 },
                    TextureDimension::D2,
                    t.rgba.clone(),
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::RENDER_WORLD,
                );
                image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                    address_mode_u: ImageAddressMode::Repeat,
                    address_mode_v: ImageAddressMode::Repeat,
                    ..ImageSamplerDescriptor::linear()
                });
                images.add(image)
            })
            .collect()
    };
    let bindposes = world
        .resource_mut::<Assets<SkinnedMeshInverseBindposes>>()
        .add(SkinnedMeshInverseBindposes::from(vec![Mat4::IDENTITY; JOINTS]));
    let root = world.spawn((Name::new("THUG2 skater"), Transform::default(), Visibility::Hidden)).id();
    let joints: Vec<Entity> = (0..JOINTS).map(|_| world.spawn((Transform::default(), Visibility::default())).id()).collect();
    for part in &model.meshes {
        if part.indices.is_empty() {
            continue;
        }
        let v = &part.vertices;
        let mut indices = part.indices.clone();
        if wound_backwards(part) {
            for t in indices.chunks_exact_mut(3) {
                t.swap(1, 2);
            }
        }
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, v.iter().map(|v| v.position).collect::<Vec<_>>());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, v.iter().map(|v| v.normal).collect::<Vec<_>>());
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, v.iter().map(|v| v.uv).collect::<Vec<_>>());
        // (BGRA bytes, 0x80 unchanged: the game doubles them)
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            v.iter()
                .map(|v| {
                    let [b, g, r, a] = v.color.to_le_bytes();
                    [r, g, b, a].map(|c| (c as f32 / 128.0).min(1.0))
                })
                .collect::<Vec<[f32; 4]>>(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_INDEX,
            VertexAttributeValues::Uint16x4(v.iter().map(|v| [v.bones[0], v.bones[1], v.bones[2], 0].map(u16::from)).collect()),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_WEIGHT,
            v.iter()
                .map(|v| {
                    let sum: f32 = v.weights[..3].iter().sum();
                    let w = if sum > 1e-6 { v.weights[..3].iter().map(|w| w / sum).collect::<Vec<_>>() } else { vec![1.0, 0.0, 0.0] };
                    [w[0], w[1], w[2], 0.0]
                })
                .collect::<Vec<[f32; 4]>>(),
        );
        mesh.insert_indices(Indices::U16(indices));
        let colour = part.color.map(|c| (c * 2.0).min(1.0));
        let material = StandardMaterial {
            base_color: Color::srgba(colour[0], colour[1], colour[2], 1.0),
            base_color_texture: part.texture.and_then(|t| images.get(t).cloned()),
            alpha_mode: if part.blended {
                AlphaMode::Blend
            } else if part.alpha_ref > 0 {
                AlphaMode::Mask((part.alpha_ref as f32 / 255.0).max(0.02))
            } else {
                AlphaMode::Opaque
            },
            double_sided: part.two_sided,
            cull_mode: if part.two_sided { None } else { Some(bevy::render::render_resource::Face::Back) },
            perceptual_roughness: 0.85,
            ..default()
        };
        let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
        let material = world.resource_mut::<Assets<StandardMaterial>>().add(material);
        let part = world
            .spawn((
                SkaterPart,
                Mesh3d(mesh),
                MeshMaterial3d(material),
                SkinnedMesh { inverse_bindposes: bindposes.clone(), joints: joints.clone() },
                // (bounds of the bind pose: a pose reaches past them)
                bevy::camera::visibility::NoFrustumCulling,
                Transform::default(),
            ))
            .id();
        world.entity_mut(root).add_child(part);
    }
    world.insert_resource(SkaterJoints { joints, root });
}

/// A mesh's triangles turned the other way to ours (front: counter-clockwise): most of them face away from their
/// vertices' normals.
fn wound_backwards(part: &t2::SkaterMesh) -> bool {
    let mut against = 0i64;
    for t in part.indices.chunks_exact(3) {
        let [a, b, c] = [0, 1, 2].map(|k| &part.vertices[t[k] as usize]);
        let (pa, pb, pc) = (Vec3::from_array(a.position), Vec3::from_array(b.position), Vec3::from_array(c.position));
        let geometric = (pb - pa).cross(pc - pa);
        let normal = Vec3::from_array(a.normal) + Vec3::from_array(b.normal) + Vec3::from_array(c.normal);
        against += if geometric.dot(normal) < 0.0 { 1 } else { -1 };
    }
    against > 0
}

/// Each joint where the game's pose has its bone, in our world: (game to ours) x (draw matrix) x (palette), the
/// game's row-vector matrices read as our column ones (transposed); the whole moved by where the skater is between
/// the game's last two frames.
fn pose_skater(thug2: Res<Thug2>, joints: Option<Res<SkaterJoints>>, mut q: Query<(&mut Transform, &mut Visibility)>) {
    let Some(joints) = joints else { return };
    let shown = thug2.placed && thug2.current.is_some() && !thug2.pose.palette.is_empty();
    if let Ok((_, mut visibility)) = q.get_mut(joints.root) {
        *visibility = if shown { Visibility::Inherited } else { Visibility::Hidden };
    }
    if !shown {
        return;
    }
    let scale = thug2.scale_given;
    let to_ours = Mat4::from_scale(Vec3::splat(1.0 / (t2::INCHES_PER_METRE * scale))) * Mat4::from_translation(-OFFSET);
    // (between the last two frames: the game steps at 60 Hz, the headset draws faster)
    let now = thug2.current.unwrap();
    let before = thug2.previous.unwrap_or(now);
    let alpha = (thug2.accum / STEP).clamp(0.0, 1.0);
    let back = Vec3::from_array(before.position).lerp(Vec3::from_array(now.position), alpha) - Vec3::from_array(now.position);
    let mut draw = thug2.pose.draw;
    (draw[3], draw[7], draw[11], draw[15]) = (0.0, 0.0, 0.0, 1.0);
    let placed = to_ours * Mat4::from_translation(back) * Mat4::from_cols_array(&draw);
    // (the parts' own transform - the root's - is where the model is: skinning ignores it, but culling places the
    // parts' bind-pose bounds with it)
    if let Ok((mut t, _)) = q.get_mut(joints.root) {
        *t = Transform::from_matrix(placed);
    }
    for (i, &joint) in joints.joints.iter().enumerate() {
        // (the game clears its palette's w column - its skinning reads only the rest, as for a point: w is 1 again
        // here, or the product would drop the translations of the matrices before it)
        let palette = thug2.pose.palette.get(i).map_or(Mat4::IDENTITY, |m| {
            let mut m = *m;
            (m[3], m[7], m[11], m[15]) = (0.0, 0.0, 0.0, 1.0);
            Mat4::from_cols_array(&m)
        });
        if let Ok((mut transform, _)) = q.get_mut(joint) {
            *transform = Transform::from_matrix(placed * palette);
        }
    }
}

/// `THUG2_SHOT=<file.png>` (testing without a headset): a camera put a little way in front of the skater, looking at
/// it, rendered to an image and saved once the skater has been in the world `THUG2_SHOT_AFTER` seconds (default 2;
/// `THUG2_SHOT_SK8`: SK8's skater). The camera is made at launch: in this app a camera made later drew none of the meshes
/// that were there before it.
fn shoot(
    mut commands: Commands,
    engines: Res<engines::Engines>,
    link: Res<SkaterLink>,
    time: Res<Time<Real>>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<&mut Transform, With<ShotImage>>,
    mut since: Local<Option<f32>>,
    mut done: Local<u8>,
) {
    use bevy::render::view::screenshot::{Screenshot, save_to_disk};
    // (the camera made at the start, before anything of the skater exists - aimed once the skater is placed)
    if cameras.is_empty() && *done == 0 {
        let image = images.add(Image::new_target_texture(
            800,
            800,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            None,
        ));
        commands.spawn((
            Camera3d::default(),
            Camera { order: 50, clear_color: ClearColorConfig::Custom(Color::srgb(0.25, 0.27, 0.3)), ..default() },
            bevy::camera::RenderTarget::Image(image.clone().into()),
            Transform::default(),
            ShotImage(image),
            crate::xr::panel::XrPanelCamera,
            bevy::render::view::NoIndirectDrawing,
        ));
        *done = 1;
        return;
    }
    let wanted = if std::env::var_os("THUG2_SHOT_SK8").is_some() { Engine::Sk8 } else { Engine::Thug2 };
    if engines.current() != Some(wanted) || *done >= 12 {
        return;
    }
    let Some((deck, nose)) = link.deck else { return };
    let started = *since.get_or_insert(time.elapsed_secs());
    let after: f32 = std::env::var("THUG2_SHOT_AFTER").ok().and_then(|v| v.parse().ok()).unwrap_or(2.0);
    let side = nose.cross(Vec3::Y).normalize_or(Vec3::X);
    let wide: f32 = if std::env::var_os("THUG2_SHOT_METER").is_some() { 1.8 } else { 1.0 };
    let at = deck + Vec3::Y * 0.9 * wide.min(1.2);
    let eye = at + (nose * 1.6 + side * 1.4 + Vec3::Y * 0.5) * wide;
    for mut t in &mut cameras {
        *t = Transform::from_translation(eye).looking_at(at, Vec3::Y);
    }
    if time.elapsed_secs() - started < after {
        return;
    }
    *done += 1;
    if *done < 12 {
        return;
    }
    let file = std::env::var("THUG2_SHOT").unwrap_or_default();
    commands.queue(move |world: &mut World| {
        let mut q = world.query::<&ShotImage>();
        if let Some(image) = q.iter(world).next().map(|s| s.0.clone()) {
            world.spawn(Screenshot::image(image)).observe(save_to_disk(file.clone()));
            info!("THUG2 shot: {file}");
        }
    });
}

#[derive(Component)]
struct ShotImage(Handle<Image>);


#[cfg(test)]
mod tests {
    use super::*;

    fn quad(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> [Vec3; 4] {
        [a, b, c, d]
    }

    fn up(t: &t2::Triangle) -> f32 {
        let v = t.v.map(Vec3::from_array);
        normal([v[0], v[1], v[2]]).y
    }

    #[test]
    fn metres_map_to_offset_inches_and_back() {
        let p = Vec3::new(1.0, 2.0, -3.0);
        let g = to_game(p, 1.0);
        assert!((g[1] - 2.0 * 39.370_08).abs() < 1e-3);
        assert!((from_game(g, 1.0) - p).length() < 1e-4);
        // A world twice the size: twice the inches, and back.
        let g2 = to_game(p, 2.0);
        assert!((g2[1] - 4.0 * 39.370_08).abs() < 1e-3);
        assert!((from_game(g2, 2.0) - p).length() < 1e-4);
    }

    #[test]
    fn ground_faces_up_whichever_way_it_was_wound_and_walls_both_ways() {
        let mut out = Vec::new();
        // A floor wound to face down, and a wall.
        face(&mut out, [Vec3::ZERO, Vec3::X, Vec3::Z], None, 1.0);
        assert_eq!(out.len(), 1);
        assert!(up(&out[0]) > 0.99, "turned up");
        assert_eq!(out[0].flags, t2::face::SKATABLE);
        face(&mut out, [Vec3::ZERO, Vec3::Y, Vec3::X], None, 1.0);
        assert_eq!(out.len(), 3, "a wall is two faces");
        assert_eq!(out[1].flags, t2::face::SKATABLE | t2::face::WALL);
    }

    #[test]
    fn a_quarter_pipes_top_is_vert_facing_its_ramp() {
        // A quarter pipe along +z: flat, a 45° ramp, then an upright top (wound facing away, +z).
        let r = 1.0;
        let flat = quad(Vec3::new(0.0, 0.0, -1.0), Vec3::new(1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0));
        let ramp = quad(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 0.5 * r, 0.5 * r), Vec3::new(0.0, 0.5 * r, 0.5 * r));
        let top = quad(Vec3::new(0.0, 0.5 * r, 0.5 * r), Vec3::new(1.0, 0.5 * r, 0.5 * r), Vec3::new(1.0, 1.5 * r, 0.5 * r), Vec3::new(0.0, 1.5 * r, 0.5 * r));
        // A box side elsewhere, on nothing: a wall.
        let side = quad(Vec3::new(3.0, 0.0, 0.0), Vec3::new(3.0, 0.5, 0.0), Vec3::new(3.0, 0.5, 1.0), Vec3::new(3.0, 0.0, 1.0));
        let piece = PieceSurfaces { quads: vec![flat, ramp, top, side], ..default() };
        let mut out = Vec::new();
        piece_faces(&mut out, &piece, 1.0);
        let verts: Vec<&t2::Triangle> = out.iter().filter(|t| t.flags & t2::face::VERT != 0).collect();
        assert_eq!(verts.len(), 2, "the top's two triangles");
        for t in verts {
            let v = t.v.map(Vec3::from_array);
            assert!(normal([v[0], v[1], v[2]]).z < -0.9, "facing back down the ramp (-z)");
        }
        let walls = out.iter().filter(|t| t.flags & t2::face::WALL != 0).count();
        assert_eq!(walls, 4, "the box side, both ways");
        assert!(out.iter().filter(|t| t.flags == t2::face::SKATABLE).all(|t| up(t) > 0.3));
    }

    /// The adapter on the real engine (`THUG2_GAME_DIR`: the folder holding the player's `Data`; skipped without
    /// it): a room floor given, the skater put on it, pushed forward with the left stick; then a second engine
    /// picked and the stand-in hidden, and THUG2 picked again carrying on.
    #[test]
    fn skates_the_room_and_switches_away_and_back() {
        use crate::engines::Engines;
        if std::env::var_os("THUG2_GAME_DIR").is_none() {
            eprintln!("THUG2_GAME_DIR not set: skipped");
            return;
        }
        let files = std::env::temp_dir().join(format!("thug2-adapter-test-{}", std::process::id()));
        std::fs::create_dir_all(&files).unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default(), bevy::mesh::MeshPlugin))
            .init_asset::<StandardMaterial>()
            .init_asset::<Image>()
            .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
            // (the game's sounds are played: an output, though there may be no device)
            .add_plugins(bevy::audio::AudioPlugin::default())
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f64(1.0 / 60.0)))
            .init_resource::<SkaterLink>()
            .init_resource::<crate::xr::ui::ControlsSheet>()
            .init_resource::<crate::xr::ui::UiProviders>()
            .init_resource::<crate::xr::actions::LatestTouch>()
            .init_resource::<engine_api::PadHeld>()
            .init_resource::<crate::xr::build::BuildMode>()
            .init_resource::<crate::xr::ui::UiOwnsControllers>()
            .init_resource::<crate::xr::spawn::PlaceSkater>()
            .init_resource::<crate::xr::ui::hud::TrickHud>()
            .init_resource::<crate::xr::ui::XrUi>()
            .add_message::<TeleportSkater>();
        // A 20 m room floor (wound facing down: it is turned up) and a rail.
        let mut room = ArWorld { revision: 1, ..default() };
        room.room_quads.push([Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 0.0, -10.0), Vec3::new(10.0, 0.0, 10.0), Vec3::new(-10.0, 0.0, 10.0)]);
        room.rails.push(vec![Vec3::new(1.0, 0.3, 0.0), Vec3::new(1.0, 0.3, 2.0)]);
        app.insert_resource(room);
        engines::add_engine(&mut app, Engine::Sk8, |_| Ok(()), |app| {
            app.add_systems(Update, || {});
        });
        install(&mut app, &files);
        app.world_mut().resource_mut::<Engines>().request(Engine::Thug2);
        app.world_mut().write_message(TeleportSkater { point: Vec3::new(0.0, 0.0, -2.0), forward: Vec3::Z });
        let mut deck = None;
        for _ in 0..600 {
            app.update();
            deck = app.world().resource::<SkaterLink>().deck;
            if deck.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (start, nose) = deck.expect("the skater in the room");
        assert!((start - Vec3::new(0.0, DECK_HEIGHT, -2.0)).length() < 0.05, "put where asked: {start}");
        assert!(nose.z > 0.9, "facing +z: {nose}");
        app.world_mut().resource_mut::<crate::xr::actions::LatestTouch>().0.left_stick = [0.0, 1.0];
        for _ in 0..60 {
            app.update();
        }
        let (pushed, _) = app.world().resource::<SkaterLink>().deck.unwrap();
        assert!(pushed.z > start.z + 2.0, "pushed along +z: {start} -> {pushed}");
        assert!(pushed.y.abs() < 0.2, "on the floor: {pushed}");
        // Its sound: rolling, a looping emitter at the skater, playing the game's own samples.
        let mut q = app.world_mut().query::<(&VoiceEmitter, &Transform)>();
        let rolling = q.iter(app.world()).find(|(e, _)| e.looping).map(|(_, t)| t.translation).expect("the rolling loop plays");
        assert!((rolling - pushed).length() < 0.6, "from the skater: {rolling} vs {pushed}");
        assert!(!app.world().resource::<Thug2Sounds>().samples.is_empty());
        // A world twice the size: the skater stays where it is in the room, and is drawn half size.
        let here = app.world().resource::<SkaterLink>().deck.unwrap().0;
        app.world_mut().resource_mut::<Tuning>().scale = 2.0;
        for _ in 0..3 {
            app.update();
        }
        let there = app.world().resource::<SkaterLink>().deck.unwrap().0;
        assert!((there.xz() - here.xz()).length() < 0.6, "kept its place: {here} -> {there}");
        // The game's own skater drawn: its parts skinned to joints that sit on it (half size now, as the world is
        // twice as big), the stand-in not shown.
        let root = app.world().resource::<SkaterJoints>().root;
        let joint = app.world().resource::<SkaterJoints>().joints[0];
        let parts = app.world_mut().query_filtered::<(), With<SkaterPart>>().iter(app.world()).count();
        assert!(parts > 10, "{parts} parts");
        let root_at = app.world().get::<Transform>(root).unwrap();
        assert!((root_at.translation.xz() - there.xz()).length() < 0.6, "the model where the skater is: {root_at:?}");
        assert!((root_at.scale.x - 0.5 / t2::INCHES_PER_METRE).abs() < 1e-4, "half size: {root_at:?}");
        let joint_at = app.world().get::<Transform>(joint).unwrap().translation;
        assert!((joint_at - there).length() < 1.5, "its joints on it: {joint_at} vs {there}");
        let mut q = app.world_mut().query_filtered::<&Visibility, With<StandIn>>();
        assert_eq!(*q.single(app.world()).unwrap(), Visibility::Hidden);
        // Away to the other engine: the game waits, its stand-in hidden.
        app.world_mut().resource_mut::<Engines>().request(Engine::Sk8);
        app.update();
        let frame = |app: &App| match &app.world().resource::<Thug2>().current {
            Some(s) => s.frame,
            None => 0,
        };
        let left_at = frame(&app);
        assert_eq!(*app.world().get::<Visibility>(root).unwrap(), Visibility::Hidden, "the model hidden");
        for _ in 0..30 {
            app.update();
        }
        assert_eq!(frame(&app), left_at, "the game waits while another engine runs");
        app.world_mut().resource_mut::<Engines>().request(Engine::Thug2);
        for _ in 0..10 {
            app.update();
        }
        assert!(frame(&app) > left_at, "and carries on when picked again");
        assert_eq!(*app.world().get::<Visibility>(root).unwrap(), Visibility::Inherited, "the model shown again");
        let _ = std::fs::remove_dir_all(files);
    }

    #[test]
    fn a_transition_is_vert_from_45_degrees_up() {
        // Up +z: a 30° ramp, then 60°, then 85° (each quad's lower edge the last one's upper edge).
        let mut quads = Vec::new();
        let mut at = Vec3::ZERO;
        for degrees in [30.0f32, 60.0, 85.0] {
            let rise = Vec3::new(0.0, degrees.to_radians().sin(), degrees.to_radians().cos()) * 0.3;
            let next = at + rise;
            quads.push(quad(at, at + Vec3::X, next + Vec3::X, next));
            at = next;
        }
        let piece = PieceSurfaces { quads, ..default() };
        let mut out = Vec::new();
        piece_faces(&mut out, &piece, 1.0);
        let vert: Vec<bool> = out.iter().map(|t| t.flags & t2::face::VERT != 0).collect();
        assert_eq!(vert, vec![false, false, true, true, true, true], "the 30° quad isn't vert, the 60° and 85° are");
        for t in out.iter().filter(|t| t.flags & t2::face::VERT != 0) {
            let v = t.v.map(Vec3::from_array);
            assert!(normal([v[0], v[1], v[2]]).z < 0.0, "facing back down the ramp");
        }
    }

    #[test]
    fn the_pad_is_the_games() {
        let mut t = TouchState::default();
        assert_eq!(pad(&t), t2::Pad { left_x: 128, left_y: 128, right_x: 128, right_y: 128, buttons: 0 });
        t.left_stick = [-1.0, 1.0];
        t.a = true;
        t.x = true;
        let p = pad(&t);
        assert_eq!((p.left_x, p.left_y), (1, 1), "left and up are low");
        assert_eq!(p.buttons, t2::buttons::CROSS | t2::buttons::SQUARE);
        // A pad's layout: B grind, Y grab, grips the shoulders, triggers the triggers.
        let mut u = TouchState { b: true, y: true, left_grip: 1.0, right_trigger: 1.0, ..default() };
        assert_eq!(pad(&u).buttons, t2::buttons::TRIANGLE | t2::buttons::CIRCLE | t2::buttons::L1 | t2::buttons::R2);
        u = TouchState { right_grip: 1.0, left_trigger: 1.0, ..default() };
        assert_eq!(pad(&u).buttons, t2::buttons::R1 | t2::buttons::L2);
        // The left thumb off its stick for X: the right stick steers.
        t.left_stick = [0.0, 0.0];
        t.right_stick = [1.0, -0.2];
        t.y = true;
        let p = pad(&t);
        assert_eq!((p.left_x, p.left_y), (255, 153), "the right stick as the direction");
        assert_eq!((p.right_x, p.right_y), (128, 128));
        assert_eq!(p.buttons & t2::buttons::CIRCLE, t2::buttons::CIRCLE);
        // Both pushed: the further one.
        t.left_stick = [-0.3, 0.0];
        assert_eq!(pad(&t).left_x, 255);
    }

    /// The HUD mid-grind as the app draws it (the app's HUD renderer), saved as pictures to look at, a part each:
    /// `THUG2_GAME_DIR=<folder holding Data> SK8_THUG2_HUD_SHOT=/path/hud.png cargo test ... thug2::tests::a_picture
    /// -- --ignored` (alone: the game boots once a process): `hud-score.png`, `hud-meter.png`, `hud-trick.png`.
    #[test]
    #[ignore = "needs a GPU and the player's THUG2 files; writes pictures"]
    fn a_picture_of_the_hud() {
        let (Some(path), Some(dir)) = (std::env::var_os("SK8_THUG2_HUD_SHOT"), std::env::var_os("THUG2_GAME_DIR")) else {
            return;
        };
        let cache = std::env::temp_dir().join("thug2-hud-shot-cache");
        let mut engine = t2::boot(Path::new(&dir), Some(&cache), &["-level", "Default"]).expect("boot");
        for _ in 0..120 {
            engine.step(&t2::PAD_AT_REST).expect("step");
        }
        // (thug2-drive's test run: a floor and a rail, pushed along, an ollie onto the rail and a nosegrind)
        let (cx, cz) = (50_000.0, 50_000.0);
        let quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]| {
            [
                t2::Triangle { v: [a, b, c], flags: t2::face::SKATABLE, terrain: 0 },
                t2::Triangle { v: [a, c, d], flags: t2::face::SKATABLE, terrain: 0 },
            ]
        };
        let floor = quad([cx - 400.0, 0.0, cz - 400.0], [cx - 400.0, 0.0, cz + 400.0], [cx + 400.0, 0.0, cz + 400.0], [cx + 400.0, 0.0, cz - 400.0]);
        engine.set_world(&floor, Some(t2::Spawn { point: [cx, 20.0, cz - 300.0], heading_degrees: 0.0 }));
        engine.set_rails(&[vec![[cx, 30.0, cz - 100.0], [cx, 30.0, cz + 80.0]]]);
        engine.hud_enable(true);
        let mut skater = None;
        for frame in 120..195 {
            let mut pad = t2::PAD_AT_REST;
            if frame >= 140 {
                pad.left_y = 0;
            }
            if (160..=164).contains(&frame) {
                pad.buttons |= t2::buttons::CROSS;
            }
            if frame >= 166 {
                pad.buttons |= t2::buttons::TRIANGLE;
            }
            skater = engine.step(&pad).expect("step");
        }
        let mut hud = t2::Hud::default();
        engine.hud(&mut hud);
        assert!(!hud.draws.is_empty());
        let mut frame = engine_core::hud::HudFrame::default();
        thug2_engine::hud::frame(&hud, skater.is_some_and(|s| s.balance.is_some()), &mut frame);
        let mut textures = std::collections::HashMap::new();
        for id in frame.draws.iter().filter_map(|d| d.texture) {
            if let Some(texture) = engine.hud_texture(thug2_engine::hud::game_texture(id)) {
                textures.insert(id, thug2_engine::hud::texture(texture));
            }
        }
        let path = std::path::PathBuf::from(path);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("hud").to_string();
        let pictures = crate::engine_hud::tests::pictures(Engine::Thug2, &[(stem, frame)], textures);
        assert_eq!(pictures.len(), 3, "a picture a part");
        crate::engine_hud::tests::save(path.parent().unwrap_or(Path::new(".")), &pictures);
    }
}
