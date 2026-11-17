#[cfg(feature = "retail")]
mod animation;
#[cfg(target_os = "android")]
mod android;
#[cfg(all(feature = "retail", any(target_os = "android", test)))]
mod clean_pack;
#[cfg(feature = "retail")]
mod app;
#[cfg(feature = "retail")]
mod assets;
mod binaural;
mod clean_mix;
mod clean_synth;
mod reverb;
#[cfg(feature = "retail")]
mod camera;
mod config;
mod fonts;
#[cfg(feature = "retail")]
use skate3_sim::difficulty;
#[cfg(feature = "retail")]
mod graph_runtime;
#[cfg(feature = "retail")]
mod grind_world;
#[cfg(feature = "retail")]
mod input;
#[cfg(feature = "retail")]
mod logging;
#[cfg(feature = "retail")]
mod physics;
#[cfg(feature = "retail")]
mod presentation;
#[cfg(feature = "retail")]
mod replay;
#[cfg(feature = "retail")]
mod skate3_hud;
#[cfg(feature = "retail")]
mod setup;
#[cfg(feature = "retail")]
mod skate_audio;
// The store build (no `retail`): only the clean tier's own sounds.
#[cfg(not(feature = "retail"))]
#[path = "skate_audio_store.rs"]
mod skate_audio;
// (an engine's sound, whichever engine: the ones with a stream or voices use it)
#[cfg_attr(not(all(feature = "retail", feature = "thug2")), allow(dead_code))]
mod engine_audio;
#[cfg_attr(not(feature = "retail"), allow(dead_code))]
mod live_pcm;
// (every engine's host: THUG2's so far)
#[cfg(feature = "thug2")]
mod engine_driver;
#[cfg_attr(not(feature = "thug2"), allow(dead_code))]
mod stream_pcm;
#[cfg(feature = "retail")]
mod world;
#[cfg(feature = "xr")]
mod xr;
mod clean;
#[cfg_attr(not(feature = "xr"), allow(dead_code))]
mod engines;
#[cfg(any(feature = "retail", feature = "thug2"))]
mod engine_hud;
#[cfg(feature = "thug2")]
mod thug2;
#[cfg(any(feature = "retail", feature = "thug2"))]
mod model;
#[cfg(test)]
mod model_parity;

/// Platform hooks for shared storage (the headset's `Download` folder),
/// set by the Android entry point: whether the app may read it, and asking
/// the player for it ("All files access").
#[derive(Clone, Copy)]
pub struct StorageHooks {
    pub has_access: fn() -> bool,
    pub request_access: fn(),
}

static STORAGE_HOOKS: std::sync::OnceLock<StorageHooks> = std::sync::OnceLock::new();

pub fn set_storage_hooks(hooks: StorageHooks) {
    let _ = STORAGE_HOOKS.set(hooks);
}

/// Shared storage can be read (always, without hooks: the desktop).
#[cfg(all(feature = "xr", feature = "retail"))]
pub(crate) fn storage_access() -> bool {
    STORAGE_HOOKS.get().is_none_or(|h| (h.has_access)())
}

#[cfg(all(feature = "xr", feature = "retail"))]
pub(crate) fn request_storage_access() {
    if let Some(h) = STORAGE_HOOKS.get() {
        (h.request_access)();
    }
}

/// Platform hooks for the headset's passthrough cameras (room lighting,
/// `xr::room_light`), set by the Android entry point: whether the app may
/// open them, and asking the player for it.
#[derive(Clone, Copy)]
pub struct CameraHooks {
    pub has_access: fn() -> bool,
    pub request_access: fn(),
    /// The left passthrough camera's Camera2 id, if the headset has one.
    pub passthrough_camera: fn() -> Option<String>,
    /// The right one's (a developer's capture records it too).
    pub right_passthrough_camera: fn() -> Option<String>,
}

static CAMERA_HOOKS: std::sync::OnceLock<CameraHooks> = std::sync::OnceLock::new();

pub fn set_camera_hooks(hooks: CameraHooks) {
    let _ = CAMERA_HOOKS.set(hooks);
}

/// The passthrough cameras can be opened (never, without hooks).
#[cfg_attr(not(feature = "xr"), allow(dead_code))]
pub(crate) fn camera_access() -> bool {
    CAMERA_HOOKS.get().is_some_and(|h| (h.has_access)())
}

#[cfg_attr(not(feature = "xr"), allow(dead_code))]
pub(crate) fn request_camera_access() {
    if let Some(h) = CAMERA_HOOKS.get() {
        (h.request_access)();
    }
}

#[cfg_attr(not(all(feature = "xr", target_os = "android")), allow(dead_code))]
pub(crate) fn passthrough_camera() -> Option<String> {
    CAMERA_HOOKS.get().and_then(|h| (h.passthrough_camera)())
}

#[cfg_attr(not(all(feature = "xr", target_os = "android")), allow(dead_code))]
pub(crate) fn right_passthrough_camera() -> Option<String> {
    CAMERA_HOOKS.get().and_then(|h| (h.right_passthrough_camera)())
}

/// Startup loading. `Err` is the exit to return, including the asset
/// check's success.
#[cfg(feature = "retail")]
fn load() -> Result<Loaded, bevy::app::AppExit> {
    eprintln!("REPORT_META stage=configuration_and_installation");
    let config = match bevy::log::info_span!("load_configuration")
        .in_scope(config::Config::from_env)
    {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            return Err(bevy::app::AppExit::error());
        }
    };
    eprintln!(
        "REPORT_META startup=difficulty:{} renderer:Vulkan",
        config.difficulty.key(),
    );
    // The sim reads the bundle and builds itself; each stage is announced as before.
    let mut stage = |name: &str| eprintln!("REPORT_META stage={name}");
    let bundle = match skate3_sim::Bundle::open(&config.asset_root, &mut stage) {
        Ok(bundle) => bundle,
        Err(error) => {
            eprintln!("{error}");
            return Err(bevy::app::AppExit::error());
        }
    };
    let sim = match skate3_sim::Sim::load(
        &bundle,
        &skate3_sim::SimConfig {
            difficulty: config.difficulty,
            map: None,
            spawn_offset: 0.0,
        },
        &mut stage,
    ) {
        Ok(sim) => sim,
        Err(error) => {
            eprintln!("{error}");
            return Err(bevy::app::AppExit::error());
        }
    };
    if config.check_assets {
        if let Err(error) = camera::CameraRuntime::load(&config.asset_root) {
            eprintln!("{error}");
            return Err(bevy::app::AppExit::error());
        }
        eprintln!("SKATE_ASSETS_READY");
        return Err(bevy::app::AppExit::Success);
    }
    // The app keeps the parts as separate resources (see physics.rs).
    let parts = sim.into_parts();
    Ok(Loaded {
        config,
        manifest: bundle.into_manifest(),
        graphs: graph_runtime::StockGraphs(parts.graphs),
        physics: physics::GamePhysics(parts.physics),
        skater: physics::SkaterRuntime(parts.skater),
        controls: physics::PlayerControls(parts.controls),
    })
}

/// The whole game; the `skate3rust` binary and the Quest library both start here.
pub fn run() -> bevy::app::AppExit {
    run_with(None)
}

/// Everything startup loads before the app is built.
#[cfg(feature = "retail")]
struct Loaded {
    config: config::Config,
    manifest: skate_data::GameAssets,
    graphs: graph_runtime::StockGraphs,
    physics: physics::GamePhysics,
    skater: physics::SkaterRuntime,
    controls: physics::PlayerControls,
}

/// [`run`], optionally loading on a worker thread while `pump` is called
/// repeatedly on this one. Android needs that: until this thread services
/// the Activity's looper, the Java UI thread is blocked in onStart and the
/// window is never shown (an ANR on the first key press), and loading takes
/// seconds.
pub fn run_with(pump: Option<&mut dyn FnMut()>) -> bevy::app::AppExit {
    #[cfg(target_os = "android")]
    if let Err(error) = android::prepare() {
        eprintln!("{error}");
        return bevy::app::AppExit::error();
    }
    // The public tier: the clean-room engine, no game data at all. (The store build has nothing
    // else.)
    #[cfg(feature = "retail")]
    if !clean::wanted() {
        return run_retail(pump);
    }
    #[cfg(not(feature = "retail"))]
    let _ = pump;
    clean::run()
}

/// The imported-disc tier: the faithful port on the player's own converted data.
#[cfg(feature = "retail")]
fn run_retail(pump: Option<&mut dyn FnMut()>) -> bevy::app::AppExit {
    if let Err(error) = logging::init() {
        eprintln!("{error}");
        return bevy::app::AppExit::error();
    }
    let _startup = bevy::log::info_span!("startup").entered();
    let loaded = match pump {
        None => load(),
        Some(pump) => std::thread::scope(|scope| {
            let loader = std::thread::Builder::new()
                .name("startup-loader".into())
                .stack_size(64 << 20)
                .spawn_scoped(scope, load)
                .expect("could not start the startup loader thread");
            while !loader.is_finished() {
                pump();
            }
            loader
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        }),
    };
    let Loaded {
        config,
        manifest,
        graphs,
        physics,
        skater,
        controls,
    } = match loaded {
        Ok(loaded) => loaded,
        Err(exit) => return exit,
    };
    eprintln!("REPORT_META stage=renderer_and_app_initialization");
    let mut app = app::build(config, manifest, graphs, physics, skater);
    app.insert_resource(controls);
    // The winit runner would do this first thing; doing it here puts its
    // cost (render/OpenXR plugin setup, before the Android looper is ever
    // serviced) in the startup log.
    if app.plugins_state() == bevy::app::PluginsState::Ready {
        eprintln!("REPORT_META stage=plugins_finish");
        app.finish();
        app.cleanup();
    }
    eprintln!("REPORT_META stage=app_run");
    drop(_startup);
    app.run()
}
