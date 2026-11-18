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
    app.insert_resource(MkwiiFiles { disc });
    crate::xr::ui::import::offer(app, Engine::Mkwii, mkw_engine::DATA);
    app.world_mut().resource_mut::<crate::xr::ui::ControlsSheet>().set(Engine::Mkwii, CONTROLS.iter().map(|&(a, b)| engine_api::Control::new(a, b)).collect());
    crate::engine_hud::install(app);
    app.init_resource::<engine_api::PlayerBodies>();
    engines::add_engine(app, Engine::Mkwii, available, |app| {
        crate::engine_driver::install::<Mkwii>(app, factory, None, None);
    });
}

/// What the controllers do (`mkw_engine::engine::pad`).
const CONTROLS: &[(&str, &str)] = &[
    ("Right trigger / A", "drive"),
    ("Left trigger / B", "brake, reverse"),
    ("Left stick", "steer"),
    ("Right grip", "drift (hop)"),
    ("Left grip", "item"),
];
