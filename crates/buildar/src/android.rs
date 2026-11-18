//! Quest (Android) process setup. The app's external files directory
//! (`/sdcard/Android/data/<package>/files`) stands in for the desktop
//! package folder.
//!
//! Skate 3's data is the player's own converted installation, imported in the headset or pushed with
//! `scripts/quest/push-assets.sh` into `files/assets/` (`crate::skate3::assets`). Public releases carry nothing else.
use std::path::PathBuf;

pub(crate) fn files_dir() -> Result<PathBuf, String> {
    bevy::android::ANDROID_APP
        .get()
        .ok_or("Android app handle not set; start through bevy_main")?
        .external_data_path()
        .ok_or_else(|| "Android external files directory is unavailable".into())
}

/// Must run before any thread reads the environment.
pub(crate) fn prepare() -> Result<(), String> {
    let files = files_dir()?;
    eprintln!("SKATE_ANDROID files={}", files.display());
    // Every engine cache path is `$LOCALAPPDATA/Skate3RustEngine/...`.
    // SAFETY: called first thing in `run`, before the engine starts threads.
    unsafe {
        std::env::set_var("LOCALAPPDATA", &files);
    }
    Ok(())
}
