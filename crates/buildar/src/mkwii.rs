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
        let library = loadable(&library_in(&factory_disc))?;
        info!("Mario Kart Wii starting: {} ({})", factory_disc.display(), library.display());
        mkw_engine::engine::factory(library, factory_disc, data)()
    });
    app.insert_resource(MkwiiFiles { disc })
        .insert_resource(Settings::load(&files.join(SETTINGS_FILE)))
        .insert_resource(SettingsFile(files.join(SETTINGS_FILE)));
    app.world_mut().resource_mut::<crate::xr::ui::UiProviders>().0.push(std::sync::Arc::new(MkwiiPages));
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
    ("Right trigger / A", "drive"),
    ("Left trigger / B", "brake, reverse"),
    ("Left stick", "steer"),
    ("Right grip", "drift (hop)"),
];

/// Mario Kart Wii's settings, kept in `mkwii.json` beside the app's other settings.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Settings {
    /// The kart drifts by itself when it turns hard (no mini-turbos); off, manual: hop and steer.
    drift_automatic: bool,
}

const SETTINGS_FILE: &str = "mkwii.json";

impl Settings {
    fn load(file: &Path) -> Self {
        std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
}

#[derive(Resource)]
struct SettingsFile(PathBuf);

/// Its settings to the engine (the driver gives them again each time it is made).
fn tune(settings: Res<Settings>, mut driver: ResMut<Driver>) {
    driver.set_setting(mkw_engine::engine::settings::DRIFT_AUTOMATIC, if settings.drift_automatic { 1.0 } else { 0.0 });
}

/// Mario Kart Wii's page in the headset menu (shown while it runs).
struct MkwiiPages;

const PAGE: crate::xr::ui::PageId = "mkwii";

impl crate::xr::ui::UiProvider for MkwiiPages {
    fn page(&self, id: crate::xr::ui::PageId, world: &World) -> Option<crate::xr::ui::UiPage> {
        use crate::xr::ui::{UiPage, Widget};
        if id != PAGE {
            return None;
        }
        let s = *world.resource::<Settings>();
        Some(UiPage::new(
            "Mario Kart Wii",
            vec![
                Widget::toggle("drift_automatic", "Drift: automatic", s.drift_automatic),
                Widget::info("Off (manual): hop with the right grip and steer to drift, for mini-turbos."),
            ],
        ))
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
        let mut s = *world.resource::<Settings>();
        match (widget, act) {
            ("drift_automatic", Act::Press) => s.drift_automatic = !s.drift_automatic,
            _ => return Some(Outcome::Stay),
        }
        world.insert_resource(s);
        let file = world.resource::<SettingsFile>().0.clone();
        if let Ok(json) = serde_json::to_vec_pretty(&s) {
            let _ = std::fs::write(&file, json);
        }
        Some(Outcome::Stay)
    }

    fn start_items(&self, _world: &World) -> Vec<crate::xr::ui::Widget> {
        vec![crate::xr::ui::Widget::link("Mario Kart Wii: driving", PAGE)]
    }

    fn engine(&self) -> Option<Engine> {
        Some(Engine::Mkwii)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Manual by default; a saved choice read back (and an old or broken file: the default).
    #[test]
    fn the_drift_is_manual_unless_chosen() {
        assert!(!Settings::default().drift_automatic);
        let dir = std::env::temp_dir().join(format!("mkwii-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("auto.json"), br#"{ "drift_automatic": true }"#).unwrap();
        assert!(Settings::load(&dir.join("auto.json")).drift_automatic);
        std::fs::write(dir.join("broken.json"), b"{").unwrap();
        assert!(!Settings::load(&dir.join("broken.json")).drift_automatic);
        let _ = std::fs::remove_dir_all(dir);
    }
}
