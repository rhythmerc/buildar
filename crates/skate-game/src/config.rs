#[cfg(feature = "retail")]
use bevy::prelude::Resource;
#[cfg(feature = "retail")]
use std::path::PathBuf;

#[cfg(feature = "retail")]
#[derive(Resource)]
pub(crate) struct Config {
    pub asset_root: PathBuf,
    pub difficulty: crate::difficulty::Difficulty,
    pub check_assets: bool,
    /// `--xr [--xr-scale K]`: OpenXR passthrough miniature.
    pub xr: Option<XrOptions>,
}

/// OpenXR view options. Physics never sees these; they only scale the view.
#[derive(Clone, Copy, Debug)]
pub(crate) struct XrOptions {
    /// Real-world size / miniature size. 12 makes a 1.8 m skater 15 cm tall.
    pub scale: f32,
}

impl Default for XrOptions {
    fn default() -> Self {
        Self { scale: 12.0 }
    }
}

#[cfg(feature = "retail")]
impl Config {
    pub fn from_env() -> Result<Self, String> {
        let mut config = Self {
            asset_root: crate::setup::asset_root()?,
            difficulty: crate::difficulty::Difficulty::default(),
            check_assets: false,
            xr: None,
        };
        let mut difficulty_override = None;
        let mut args = std::env::args_os().skip(1);
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--assets") => {
                    config.asset_root = args.next().ok_or("--assets requires a directory")?.into()
                }
                // (the only world there is: Skate 3 has no maps)
                Some("--test-world") => {}
                Some("--xr") => {
                    if !cfg!(feature = "xr") {
                        return Err("--xr needs a build with the `xr` feature".into());
                    }
                    config.xr.get_or_insert_default();
                }
                Some("--xr-scale") => {
                    let scale: f32 = args
                        .next()
                        .ok_or("--xr-scale requires a number")?
                        .to_string_lossy()
                        .parse()
                        .map_err(|_| "Invalid --xr-scale")?;
                    if !(1.0..=1000.0).contains(&scale) {
                        return Err("--xr-scale must be between 1 and 1000".into());
                    }
                    config.xr.get_or_insert_default().scale = scale;
                }
                Some("--check-assets") => config.check_assets = true,
                Some("--difficulty") => {
                    let value = args
                        .next()
                        .ok_or("--difficulty requires easy, normal or hardcore")?;
                    difficulty_override = Some(crate::difficulty::Difficulty::parse(
                        &value.to_string_lossy(),
                    )?);
                }
                _ => {
                    return Err(format!(
                        "Unknown argument {arg:?}. Usage: skate3rust [--assets DIRECTORY] [--test-world] [--difficulty easy|normal|hardcore] [--check-assets] [--xr [--xr-scale K]]"
                    ));
                }
            }
        }
        // The Quest build has no command line and is always the XR game.
        #[cfg(target_os = "android")]
        config.xr.get_or_insert_default();
        config.asset_root = config
            .asset_root
            .canonicalize()
            .map_err(|e| format!("Asset root {}: {e}", config.asset_root.display()))?;
        config.difficulty = match difficulty_override {
            Some(mode) => mode,
            None => crate::difficulty::Difficulty::load(&config.asset_root)?,
        };
        Ok(config)
    }
}
