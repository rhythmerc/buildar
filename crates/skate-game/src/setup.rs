//! Where the converted Skate 3 assets are: the headset's import folder on Android; elsewhere
//! `assets/` in the working directory (the desktop dev harness), which `--assets DIRECTORY`
//! overrides (config.rs).
use std::path::PathBuf;

pub(crate) fn asset_root() -> Result<PathBuf, String> {
    #[cfg(target_os = "android")]
    return crate::android::asset_root();
    #[allow(unreachable_code)]
    Ok(PathBuf::from("assets"))
}
