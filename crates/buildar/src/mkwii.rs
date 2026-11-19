//! Mario Kart Wii in the headset: the game's own code from the player's disc (WiiCompiled's translation, built as
//! `libmkw_engine`), free driving on the room and a track's pieces. What it knows of the game is `mkw_engine`'s (the
//! game as an `engine_core::EngineCore`: `mkw_engine::engine`); the app's generic driver hosts it
//! (`crate::engine_driver`). Here: where the player's files are, loading the library from there, its controls.
//!
//! The files (`mkw_engine::DATA`): the extracted disc (`sys/`, `files/`) with `libmkw_engine.so` built from it, in
//! `<app files>/mkwii` (`MKW_DISC_DIR` elsewhere; `MKW_ENGINE_LIB` names the library). Android won't load code from
//! the shared storage the app's files are on, so the library is copied into the app's own storage first.
use crate::engines::{self, Engine};
use bevy::prelude::*;
use std::path::{Path, PathBuf};

/// Mario Kart Wii to the driver.
pub(crate) struct Mkwii;

impl crate::engine_driver::Hosted for Mkwii {
    const ENGINE: Engine = Engine::Mkwii;
}

type Driver = crate::engine_driver::Driver<Mkwii>;

const LIBRARY: &str = "libmkw_engine.so";

/// Where its files are.
#[derive(Resource, Clone)]
struct MkwiiFiles {
    disc: PathBuf,
}

fn disc_folder(files: &Path) -> PathBuf {
    std::env::var_os("MKW_DISC_DIR").map(PathBuf::from).unwrap_or_else(|| mkw_engine::DATA.kept_in(files))
}

fn library_in(disc: &Path) -> PathBuf {
    std::env::var_os("MKW_ENGINE_LIB").map(PathBuf::from).unwrap_or_else(|| disc.join(LIBRARY))
}

fn available(world: &World) -> Result<(), String> {
    if let Some(why) = world.get_resource::<Driver>().and_then(Driver::failed) {
        return Err(why);
    }
    let files = world.resource::<MkwiiFiles>();
    if mkw_engine::DATA.in_folder(&files.disc) && library_in(&files.disc).is_file() {
        Ok(())
    } else {
        Err("needs your game files".into())
    }
}

/// The library where it can be loaded from: on Android, a copy in the app's own storage (the copy refreshed when the
/// player's changes).
fn loadable(library: &Path) -> Result<PathBuf, String> {
    #[cfg(target_os = "android")]
    {
        let own = bevy::android::ANDROID_APP
            .get()
            .and_then(|app| app.internal_data_path())
            .ok_or("the app's own storage is unavailable")?
            .join(LIBRARY);
        let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).ok();
        if size(&own) != size(library) {
            std::fs::copy(library, &own).map_err(|e| format!("{}: {e}", own.display()))?;
        }
        Ok(own)
    }
    #[cfg(not(target_os = "android"))]
    {
        Ok(library.to_path_buf())
    }
}

/// Adds Mario Kart Wii to the app's engines.
pub(crate) fn install(app: &mut App, files: &Path) {
    let disc = disc_folder(files);
    let data = files.join("mkwii-data");
    let factory_disc = disc.clone();
    let factory: engine_core::EngineFactory = Box::new(move || {
        // (the disc's parts the game never reads gone from the player's copy in the app's files: their space given
        // back; a disc named elsewhere, MKW_DISC_DIR, is left as it is)
        let own_copy = std::env::var_os("MKW_DISC_DIR").is_none();
        match if own_copy { mkw_engine::leave_out_unused(&factory_disc) } else { Ok(0) } {
            Ok(0) => {}
            Ok(freed) => info!("Mario Kart Wii: {} MB of unused disc files removed", freed >> 20),
            Err(e) => warn!("Mario Kart Wii: unused disc files not removed: {e}"),
        }
        let library = loadable(&library_in(&factory_disc))?;
        info!("Mario Kart Wii starting: {} ({})", factory_disc.display(), library.display());
        mkw_engine::engine::factory(library, factory_disc, data)()
    });
    app.insert_resource(MkwiiFiles { disc })
        .insert_resource(PlayerSettings::load(&files.join(SETTINGS_FILE)))
        .init_resource::<TrackSettings>()
        .insert_resource(SettingsFile(files.join(SETTINGS_FILE)));
    app.init_resource::<engines::CourseSettings>();
    app.world_mut().resource_mut::<engines::CourseSettings>().0.insert(Engine::Mkwii, (park_settings, apply_park_settings));
    app.init_resource::<engines::CourseSteppers>();
    app.world_mut().resource_mut::<engines::CourseSteppers>().0.insert(Engine::Mkwii, TRACK_STEPPERS.to_vec());
    app.init_resource::<engines::EngineToggles>();
    app.world_mut().resource_mut::<engines::EngineToggles>().0.insert(Engine::Mkwii, (PLAYER_TOGGLES.to_vec(), (player_toggle, set_player_toggle)));
    crate::xr::ui::import::offer(app, Engine::Mkwii, mkw_engine::DATA);
    app.world_mut().resource_mut::<crate::xr::ui::ControlsSheet>().set(Engine::Mkwii, CONTROLS.iter().map(|&(a, b)| engine_api::Control::new(a, b)).collect());
    crate::engine_hud::install(app);
    app.init_resource::<engine_api::PlayerBodies>();
    engines::add_engine(app, Engine::Mkwii, available, |app| {
        crate::engine_driver::install::<Mkwii>(app, factory, None, None);
        app.add_systems(Update, tune.in_set(engines::EngineSet(Engine::Mkwii)));
    });
}

/// What the controllers do (`mkw_engine::engine::pad`).
const CONTROLS: &[(&str, &str)] = &[
    ("Right trigger / A", "drive (part way: slower)"),
    ("Left trigger / B", "brake, reverse"),
    ("Left stick", "steer"),
    ("Right grip", "drift (hop)"),
];

// Its settings are of two kinds: the player's own (`PlayerSettings`: how they drive, on every track, kept in
// `mkwii.json`) and the open track's (`TrackSettings`: how the kart goes on it, kept with the track as its engine
// settings - `engines::CourseSettings` - and stepped in the course settings dialog, `TRACK_STEPPERS`; back to the
// kart's own on a track that kept none).

/// The player's own: how they drive, on every track.
#[derive(Resource, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct PlayerSettings {
    /// The kart drifts by itself when it turns hard (no mini-turbos); off, manual: hop and steer.
    drift_automatic: bool,
    /// The right trigger as an analog throttle (how far it is pulled, how fast the kart goes); off, all or nothing.
    analog_throttle: bool,
}

impl Default for PlayerSettings {
    fn default() -> Self {
        Self { drift_automatic: false, analog_throttle: true }
    }
}

const SETTINGS_FILE: &str = "mkwii.json";

impl PlayerSettings {
    /// (an older file's track settings in it are left: tracks keep those)
    fn load(file: &Path) -> Self {
        std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
}

#[derive(Resource)]
struct SettingsFile(PathBuf);

/// The open track's: the kart's top speed and acceleration on it, multiples of its own (1: as the game has them).
/// Kept with the track (`park_settings`), never in the player's file.
#[derive(Resource, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct TrackSettings {
    speed: f32,
    acceleration: f32,
}

impl Default for TrackSettings {
    fn default() -> Self {
        Self { speed: 1.0, acceleration: 1.0 }
    }
}

/// The track's settings in the course settings dialog.
const TRACK_STEPPERS: [engines::CourseStepper; 2] = [
    engines::CourseStepper { key: "speed", label: "Top speed", help: "100% is the kart's own", range: (0.1, 2.0, 0.01), default: 1.0, percent: true },
    engines::CourseStepper { key: "acceleration", label: "Acceleration", help: "100% is the kart's own", range: (0.1, 3.0, 0.01), default: 1.0, percent: true },
];

impl TrackSettings {
    /// Within the steppers' ranges.
    fn clamped(self) -> Self {
        let within = |v: f32, s: &engines::CourseStepper| if v.is_finite() { v.clamp(s.range.0, s.range.1) } else { s.default };
        Self { speed: within(self.speed, &TRACK_STEPPERS[0]), acceleration: within(self.acceleration, &TRACK_STEPPERS[1]) }
    }
}

/// The open track's settings for it to keep (`xr::build::parks::tag_engine` saves them with it).
fn park_settings(world: &World) -> serde_json::Value {
    serde_json::to_value(*world.resource::<TrackSettings>()).unwrap_or_default()
}

/// A track's settings in play (as it opens, or stepped; one that kept none, `{}`: the kart's own).
fn apply_park_settings(world: &mut World, settings: &serde_json::Value) {
    let track = serde_json::from_value::<TrackSettings>(settings.clone()).unwrap_or_default().clamped();
    world.insert_resource(track);
}

/// Its settings to the engine (the driver gives them again each time it is made).
fn tune(player: Res<PlayerSettings>, track: Res<TrackSettings>, mut driver: ResMut<Driver>) {
    use mkw_engine::engine::settings;
    let flag = |on: bool| if on { 1.0 } else { 0.0 };
    driver.set_setting(settings::DRIFT_AUTOMATIC, flag(player.drift_automatic));
    driver.set_setting(settings::ANALOG_THROTTLE, flag(player.analog_throttle));
    driver.set_setting(settings::SPEED, track.speed);
    driver.set_setting(settings::ACCELERATION, track.acceleration);
}

/// The player's settings in its engine settings dialog (the gear by its dropdown).
const PLAYER_TOGGLES: [engines::EngineToggle; 2] = [
    engines::EngineToggle { key: "analog_throttle", label: "Analog throttle", help: "Pull the right trigger part way to drive slower" },
    engines::EngineToggle { key: "drift_automatic", label: "Automatic drift", help: "Off: hop with the right grip and steer, for mini-turbos" },
];

fn player_toggle(world: &World, key: &str) -> bool {
    let p = world.resource::<PlayerSettings>();
    match key {
        "analog_throttle" => p.analog_throttle,
        "drift_automatic" => p.drift_automatic,
        _ => false,
    }
}

fn set_player_toggle(world: &mut World, key: &str, on: bool) {
    let mut p = *world.resource::<PlayerSettings>();
    match key {
        "analog_throttle" => p.analog_throttle = on,
        "drift_automatic" => p.drift_automatic = on,
        _ => return,
    }
    world.insert_resource(p);
    let file = world.resource::<SettingsFile>().0.clone();
    if let Ok(json) = serde_json::to_vec_pretty(&p) {
        let _ = std::fs::write(&file, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Manual drift and an analog throttle by default; a saved choice read back (and a broken file: the default).
    #[test]
    fn the_players_settings_are_their_own() {
        let p = PlayerSettings::default();
        assert!(!p.drift_automatic && p.analog_throttle);
        let dir = std::env::temp_dir().join(format!("mkwii-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // (an older file: the drift and the track settings it once held, those left for the track)
        std::fs::write(dir.join("old.json"), br#"{ "drift_automatic": true, "speed": 1.5, "acceleration": 2.0 }"#).unwrap();
        let p = PlayerSettings::load(&dir.join("old.json"));
        assert!(p.drift_automatic && p.analog_throttle);
        std::fs::write(dir.join("broken.json"), b"{").unwrap();
        assert_eq!(PlayerSettings::load(&dir.join("broken.json")), PlayerSettings::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A track keeps its speed and acceleration; one put back (out of range: clamped) leaves the player's settings
    /// alone; a track that kept none, the kart's own.
    #[test]
    fn a_track_keeps_its_speed_and_acceleration() {
        let mut world = World::new();
        world.insert_resource(PlayerSettings { drift_automatic: true, analog_throttle: false });
        world.insert_resource(TrackSettings::default());
        apply_park_settings(&mut world, &serde_json::json!({ "speed": 1.5, "acceleration": 9.0 }));
        assert_eq!(*world.resource::<TrackSettings>(), TrackSettings { speed: 1.5, acceleration: TRACK_STEPPERS[1].range.1 });
        assert_eq!(park_settings(&world), serde_json::json!({ "speed": 1.5, "acceleration": 3.0 }));
        assert_eq!(*world.resource::<PlayerSettings>(), PlayerSettings { drift_automatic: true, analog_throttle: false });
        apply_park_settings(&mut world, &serde_json::json!({}));
        assert_eq!(*world.resource::<TrackSettings>(), TrackSettings::default());
        // (the steppers' defaults and keys are the track settings')
        assert_eq!((TRACK_STEPPERS[0].default, TRACK_STEPPERS[1].default), (1.0, 1.0));
        let keys = serde_json::to_value(TrackSettings::default()).unwrap();
        assert!(TRACK_STEPPERS.iter().all(|s| keys.get(s.key).is_some()));
    }
}
