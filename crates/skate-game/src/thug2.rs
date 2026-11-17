//! THUG2 in the headset: the game's own engine (`thug2-sys`: thug2-decomp's code, run headless) as the app's third
//! engine, skating the room and the parks.
//!
//! It runs on the player's own THUG2 files, in `<app files>/thug2` (the folder holding `Data`; `THUG2_GAME_DIR`
//! elsewhere); without them the engine can't be picked. What it knows of THUG2 is `thug2_engine`'s (the game as an
//! `engine_core::EngineCore`: `thug2_engine::engine`); the app's generic driver hosts it (`crate::engine_driver`:
//! booting it on a worker, stepping it at its 60 Hz, drawing its skater, its HUD and playing its voices). Here: where
//! its files are, its settings (game speed, sound level) and page, and its HUD's parts from the start.
use crate::engines::{self, Engine};
use bevy::prelude::*;
use std::path::{Path, PathBuf};
use thug2_sys as t2;

/// THUG2 to the driver.
pub(crate) struct Thug2;

impl crate::engine_driver::Hosted for Thug2 {
    const ENGINE: Engine = Engine::Thug2;
}

type Driver = crate::engine_driver::Driver<Thug2>;
/// Where the game's files are.
#[derive(Resource, Clone)]
struct Thug2Files {
    game: PathBuf,
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
    if let Some(why) = world.get_resource::<Driver>().and_then(Driver::failed) {
        return Err(why);
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
    let game = game_folder(files);
    let factory = thug2_engine::engine::factory(game.clone(), cache.clone());
    let marker = cache.join(STARTING);
    let _ = std::fs::create_dir_all(&cache);
    app.insert_resource(Thug2Files { game })
        .insert_resource(Tuning::load(&files.join("thug2.json")))
        .insert_resource(TuningFile(files.join("thug2.json")));
    app.world_mut().resource_mut::<crate::xr::ui::UiProviders>().0.push(std::sync::Arc::new(Thug2Pages));
    app.init_resource::<engines::CourseSettings>();
    app.world_mut().resource_mut::<engines::CourseSettings>().0.insert(Engine::Thug2, (park_settings, apply_park_settings));
    app.world_mut()
        .resource_mut::<crate::xr::ui::ControlsSheet>()
        .set(Engine::Thug2, thug2_engine::input::CONTROLS.to_vec());
    crate::engine_hud::install(app);
    app.init_resource::<engine_api::PlayerBodies>();
    engines::add_engine(app, Engine::Thug2, available, |app| {
        crate::engine_driver::install::<Thug2>(app, factory, Some(marker), None);
        app.add_systems(Startup, hud_parts).add_systems(Update, tune.in_set(engines::EngineSet(Engine::Thug2)));
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

/// THUG2's HUD (`thug2_engine::hud`: its parts and where they go) drawn by the app's HUD renderer. Its parts are
/// there from the start, empty (the corner readout gives way to them).
fn hud_parts(mut renderer: crate::engine_hud::HudRenderer) {
    let mut frame = engine_core::hud::HudFrame::default();
    thug2_engine::hud::frame(&t2::Hud::default(), false, &mut frame);
    renderer.show(Engine::Thug2, &frame, |_| None);
}

/// Its settings to the driver: the game's clock and its sounds' level.
fn tune(tuning: Res<Tuning>, mut driver: ResMut<Driver>) {
    if (driver.speed, driver.volume) != (tuning.speed, tuning.volume) {
        driver.speed = tuning.speed;
        driver.volume = tuning.volume;
    }
}

/// How THUG2 plays in the room, kept in `thug2.json` beside the app's other settings. (A `scale` there, THUG2's old
/// world size, is ignored: the skater size does that for every engine.)
#[derive(Resource, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Tuning {
    /// The game's clock: 1 is the game's own speed, less slows everything (jumps float).
    speed: f32,
    /// The game's sound effects' level: 1 is as the game sets them.
    volume: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self { speed: 1.0, volume: 1.0 }
    }
}

const SPEED_RANGE: (f32, f32, f32) = (0.4, 1.0, 0.05);
const VOLUME_RANGE: (f32, f32, f32) = (0.0, 1.5, 0.05);

impl Tuning {
    fn load(file: &Path) -> Self {
        let t: Tuning = std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Tuning {
            speed: t.speed.clamp(SPEED_RANGE.0, SPEED_RANGE.1),
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

/// What a park keeps of THUG2's settings: the game speed it was built for (the sound's level is the player's own).
/// (A park's `scale`, THUG2's old world size, is ignored.)
#[derive(serde::Serialize, serde::Deserialize)]
struct ParkTuning {
    speed: f32,
}

fn park_settings(world: &World) -> serde_json::Value {
    let t = world.resource::<Tuning>();
    serde_json::to_value(ParkTuning { speed: t.speed }).unwrap_or_default()
}

fn apply_park_settings(world: &mut World, settings: &serde_json::Value) {
    let Ok(park) = serde_json::from_value::<ParkTuning>(settings.clone()) else { return };
    let mut t = *world.resource::<Tuning>();
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
            Widget::slider("speed", "Game speed", t.speed, SPEED_RANGE, format!("{:.0}%", t.speed * 100.0)),
            Widget::button("reset", "Back to the game's own speed (100%)"),
            Widget::slider("volume", "Sound volume", t.volume, VOLUME_RANGE, format!("{:.0}%", t.volume * 100.0)),
        ];
        w.extend([
            Widget::info(""),
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
            ("speed", Act::Set(v)) => t.speed = v,
            ("volume", Act::Set(v)) => t.volume = v,
            ("reset", Act::Press) => t = Tuning { volume: t.volume, ..Tuning::default() },
            _ => return Some(Outcome::Stay),
        }
        keep(world, t);
        Some(Outcome::Stay)
    }

    fn start_items(&self, _world: &World) -> Vec<crate::xr::ui::Widget> {
        vec![crate::xr::ui::Widget::link("THUG2: game speed, sound", PAGE)]
    }

    fn engine(&self) -> Option<Engine> {
        Some(Engine::Thug2)
    }
}

/// `THUG2_SHOT=<file.png>` (testing without a headset): a camera put a little way in front of the skater, looking at
/// it, rendered to an image and saved once the skater has been in the world `THUG2_SHOT_AFTER` seconds (default 2;
/// `THUG2_SHOT_SK8`: SK8's skater). The camera is made at launch: in this app a camera made later drew none of the meshes
/// that were there before it.
fn shoot(
    mut commands: Commands,
    engines: Res<engines::Engines>,
    focus: Res<crate::xr::link::PlayerFocus>,
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
    let Some((deck, nose)) = focus.of(wanted).map(|f| (f.feet, f.forward)) else { return };
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
    use crate::xr::link::{CurrentPark, CurrentStage, PlacePlayer, SkaterLink};

    /// A `thug2.json` and a park's settings from before the world size went (a `scale` in them) still load: the
    /// scale ignored, the rest kept.
    #[test]
    fn old_settings_with_a_world_size_still_load() {
        let dir = std::env::temp_dir().join(format!("thug2-old-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("thug2.json");
        std::fs::write(&file, br#"{ "speed": 0.8, "scale": 2.5, "volume": 0.5 }"#).unwrap();
        assert_eq!(Tuning::load(&file), Tuning { speed: 0.8, volume: 0.5 });
        let mut world = World::new();
        world.insert_resource(Tuning::default());
        world.insert_resource(TuningFile(dir.join("kept.json")));
        apply_park_settings(&mut world, &serde_json::json!({ "scale": 2.5, "speed": 0.6 }));
        assert_eq!(*world.resource::<Tuning>(), Tuning { speed: 0.6, volume: 1.0 });
        assert_eq!(park_settings(&world), serde_json::json!({ "speed": 0.6f32 }));
        let _ = std::fs::remove_dir_all(dir);
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
            .init_resource::<CurrentPark>()
            .init_resource::<crate::xr::ui::ControlsSheet>()
            .init_resource::<crate::xr::ui::UiProviders>()
            .init_resource::<crate::xr::actions::LatestTouch>()
            .init_resource::<engine_api::PadHeld>()
            .init_resource::<crate::xr::build::BuildMode>()
            .init_resource::<crate::xr::ui::UiOwnsControllers>()
            .init_resource::<crate::xr::spawn::PlaceSkater>()
            .init_resource::<crate::xr::ui::hud::TrickHud>()
            .init_resource::<crate::xr::ui::XrUi>()
            .add_message::<PlacePlayer>();
        // A 20 m room floor (wound facing down: it is turned up) and a rail.
        let mut room = engine_core::Stage { revision: 1, ..default() };
        room.room_quads.push([Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 0.0, -10.0), Vec3::new(10.0, 0.0, 10.0), Vec3::new(-10.0, 0.0, 10.0)]);
        app.insert_resource(CurrentStage(room));
        app.world_mut().resource_mut::<CurrentPark>().revision = 1;
        app.world_mut().resource_mut::<CurrentPark>().rails.push(vec![Vec3::new(1.0, 0.3, 0.0), Vec3::new(1.0, 0.3, 2.0)]);
        engines::add_engine(&mut app, Engine::Sk8, |_| Ok(()), |app| {
            app.add_systems(Update, || {});
        });
        install(&mut app, &files);
        app.world_mut().resource_mut::<Engines>().request(Engine::Thug2);
        app.world_mut().write_message(PlacePlayer::new(Vec3::new(0.0, 0.0, -2.0), Vec3::Z));
        let mut deck = None;
        for _ in 0..600 {
            app.update();
            deck = app.world().resource::<SkaterLink>().board.map(|b| (b.position, b.forward));
            if deck.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (start, nose) = deck.expect("the skater in the room");
        assert!((start - Vec3::new(0.0, thug2_engine::stage::DECK_HEIGHT, -2.0)).length() < 0.05, "put where asked: {start}");
        assert!(nose.z > 0.9, "facing +z: {nose}");
        app.world_mut().resource_mut::<crate::xr::actions::LatestTouch>().0.left_stick = [0.0, 1.0];
        for _ in 0..60 {
            app.update();
        }
        let pushed = app.world().resource::<SkaterLink>().board.unwrap().position;
        assert!(pushed.z > start.z + 2.0, "pushed along +z: {start} -> {pushed}");
        assert!(pushed.y.abs() < 0.2, "on the floor: {pushed}");
        // Its sound: rolling, a looping emitter at the skater, playing the game's own samples.
        let mut q = app.world_mut().query::<(&crate::engine_audio::VoiceEmitter, &Transform)>();
        let rolling = q.iter(app.world()).find(|(e, _)| e.looping).map(|(_, t)| t.translation).expect("the rolling loop plays");
        assert!((rolling - pushed).length() < 0.6, "from the skater: {rolling} vs {pushed}");
        assert!(app.world().resource::<Driver>().clips() > 0);
        let there = app.world().resource::<SkaterLink>().board.unwrap().position;
        // (each step's pose taken, numbered by the steps)
        {
            let driver = app.world().resource::<Driver>();
            let history = driver.poses().get(thug2_engine::model::SKATER).expect("its poses");
            assert!(history.pair().is_some(), "a pair");
            assert!(driver.steps() > 60, "{} steps", driver.steps());
            let alpha = history.alpha(driver.now());
            assert!((0.0..=1.0).contains(&alpha), "{alpha}");
        }
        // The game's own skater drawn: its parts skinned to joints that sit on it (life size: the game's inches as
        // metres), the stand-in not shown.
        let (root, joint) = app.world().resource::<Driver>().model().map(|m| (m.root, m.joints[0])).expect("its model");
        let parts = app.world_mut().query_filtered::<(), With<crate::model::ModelPart>>().iter(app.world()).count();
        assert!(parts > 10, "{parts} parts");
        let root_at = app.world().get::<Transform>(root).unwrap();
        assert!((root_at.translation.xz() - there.xz()).length() < 0.6, "the model where the skater is: {root_at:?}");
        assert!((root_at.scale.x - 1.0 / t2::INCHES_PER_METRE).abs() < 1e-4, "life size: {root_at:?}");
        let joint_at = app.world().get::<Transform>(joint).unwrap().translation;
        assert!((joint_at - there).length() < 1.5, "its joints on it: {joint_at} vs {there}");
        // Its board's body, from the joints only the board's parts are skinned to: where the skater link puts the
        // board, between its last two steps' places (it is drawn between them: at full speed in life size a step is
        // some 15 cm), along the way it faces.
        let link = app.world().resource::<SkaterLink>().board.unwrap();
        let (from, to) = {
            let [before, now] = app.world().resource::<Driver>().focus().map(|f| f.unwrap());
            let board = |f: engine_core::Focus| f.feet + f.up * thug2_engine::stage::DECK_HEIGHT;
            (board(before), board(now))
        };
        let bodies = app.world().resource::<engine_api::PlayerBodies>().of(Engine::Thug2).to_vec();
        let [engine_api::BodyShape::Footprint { middle, along, .. }] = bodies.as_slice() else { panic!("the board alone: {bodies:?}") };
        let t = ((*middle - from).dot(to - from) / (to - from).length_squared().max(1e-9)).clamp(0.0, 1.0);
        assert!(
            (*middle - from.lerp(to, t)).length() < 0.15,
            "the board's middle {middle} at the board, between {from} and {to}"
        );
        assert!(along.dot(link.forward.with_y(0.0).normalize()).abs() > 0.9, "along it: {along} vs {}", link.forward);
        let mut q = app.world_mut().query_filtered::<&Visibility, With<crate::engine_driver::StandIn>>();
        assert_eq!(*q.single(app.world()).unwrap(), Visibility::Hidden);
        // Away to the other engine: the game waits, its stand-in hidden.
        app.world_mut().resource_mut::<Engines>().request(Engine::Sk8);
        app.update();
        let frame = |app: &App| app.world().resource::<Driver>().steps();
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
