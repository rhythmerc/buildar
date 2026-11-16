#[cfg(feature = "retail")]
use bevy::prelude::Resource;
#[cfg(feature = "retail")]
use std::path::PathBuf;

#[cfg(feature = "retail")]
#[derive(Resource)]
pub(crate) struct Config {
    pub asset_root: PathBuf,
    pub verification_capture: Option<PathBuf>,
    pub map: Option<skate_data::skate_map::SkateMap>,
    pub map_path: Option<PathBuf>,
    pub difficulty: crate::difficulty::Difficulty,
    pub check_assets: bool,
    pub start_paused: bool,
    pub map_fingerprint: u64,
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
            verification_capture: None,
            map: None,
            map_path: None,
            difficulty: crate::difficulty::Difficulty::default(),
            check_assets: false,
            start_paused: false,
            map_fingerprint: 0,
            xr: None,
        };
        let mut difficulty_override = None;
        let mut explicit_map = false;
        let mut args = std::env::args_os().skip(1);
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--assets") => {
                    config.asset_root = args.next().ok_or("--assets requires a directory")?.into()
                }
                Some("--map") => {
                    let path = PathBuf::from(args.next().ok_or("--map requires a .skate file")?);
                    config.map = Some(skate_data::skate_map::SkateMap::load(&path)?);
                    config.map_path = Some(path.canonicalize().map_err(|e| e.to_string())?);
                    explicit_map = true;
                }
                Some("--test-world") => {
                    explicit_map = true;
                    config.map = None;
                    config.map_path = None;
                }
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
                Some("--start-paused") => config.start_paused = true,
                Some("--difficulty") => {
                    let value = args
                        .next()
                        .ok_or("--difficulty requires easy, normal or hardcore")?;
                    difficulty_override = Some(crate::difficulty::Difficulty::parse(
                        &value.to_string_lossy(),
                    )?);
                }
                Some("--verify") => {
                    config.verification_capture = Some(
                        args.next()
                            .ok_or("--verify requires an output PNG path")?
                            .into(),
                    )
                }
                _ => {
                    return Err(format!(
                        "Unknown argument {arg:?}. Usage: skate3rust [--assets DIRECTORY] [--map MAP.skate | --test-world] [--difficulty easy|normal|hardcore] [--verify CAPTURE.png] [--check-assets] [--start-paused] [--xr [--xr-scale K]]"
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
        if !explicit_map {
            if let Some(path) = crate::map_library::default_map(&config.asset_root)? {
                config.map = Some(skate_data::skate_map::SkateMap::load(&path)?);
                config.map_path = Some(path.canonicalize().map_err(|e| e.to_string())?);
            }
        }
        config.map_fingerprint = map_fingerprint(config.map_path.as_deref())?;
        if let Some(path) = &mut config.verification_capture {
            if path.extension().and_then(|x| x.to_str()) != Some("png") {
                return Err("--verify output must be a PNG file".into());
            }
            if !path.is_absolute() {
                *path = std::env::current_dir()
                    .map_err(|e| e.to_string())?
                    .join(&path);
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        Ok(config)
    }
}

/// 64-bit FNV-1a (the map fingerprint's hash; the same function as the former `skate_net::hash`).
#[cfg(feature = "retail")]
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |hash, b| (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3))
}

/// Hash map bytes identically for startup and background map replacement.
#[cfg(feature = "retail")]
pub(crate) fn map_fingerprint(path: Option<&std::path::Path>) -> Result<u64, String> {
    Ok(if let Some(path) = path {
        use std::io::Read;
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut hash = 0xcbf29ce484222325u64;
        let mut buffer = [0; 65536];
        loop {
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            for b in &buffer[..n] {
                hash = (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3);
            }
        }
        hash
    } else {
        fnv1a(b"skate-test-world-v1")
    })
}
