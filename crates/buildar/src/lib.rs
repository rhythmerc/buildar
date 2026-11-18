#[cfg(target_os = "android")]
mod android;
mod binaural;
mod clean_mix;
mod clean_synth;
mod reverb;
mod config;
mod fonts;
mod skate_audio;
// (an engine's sound, whichever engine: the ones with a stream or voices use it)
#[cfg_attr(not(all(feature = "xr", feature = "retail", feature = "thug2")), allow(dead_code))]
mod engine_audio;
#[cfg_attr(not(all(feature = "xr", feature = "retail")), allow(dead_code))]
mod live_pcm;
// (every engine's host)
#[cfg(all(feature = "xr", any(feature = "retail", feature = "thug2")))]
mod engine_driver;
#[cfg_attr(not(all(feature = "xr", any(feature = "retail", feature = "thug2"))), allow(dead_code))]
mod stream_pcm;
#[cfg(all(feature = "xr", feature = "retail"))]
mod skate3;
#[cfg(feature = "xr")]
mod xr;
mod clean;
#[cfg_attr(not(feature = "xr"), allow(dead_code))]
mod engines;
#[cfg(all(feature = "xr", any(feature = "retail", feature = "thug2")))]
mod engine_hud;
#[cfg(feature = "thug2")]
mod thug2;
#[cfg(feature = "mkwii")]
mod mkwii;
#[cfg(all(feature = "xr", any(feature = "retail", feature = "thug2")))]
mod model;
#[cfg(all(test, feature = "xr", any(feature = "retail", feature = "thug2")))]
mod model_parity;
#[cfg(all(test, feature = "xr", feature = "retail"))]
mod skate3_gate;

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
#[cfg(all(feature = "xr", any(feature = "retail", feature = "thug2")))]
pub(crate) fn storage_access() -> bool {
    STORAGE_HOOKS.get().is_none_or(|h| (h.has_access)())
}

#[cfg(all(feature = "xr", any(feature = "retail", feature = "thug2")))]
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

/// Runs a function with the calling thread attached to the Java VM: a thread of the app's own calling into a runtime
/// that calls Java (Meta's keyboard) must be, or Android aborts the app. Set by the Android shell; elsewhere the
/// function just runs.
pub type AttachHook = fn(&mut dyn FnMut());

static ATTACH_HOOK: std::sync::OnceLock<AttachHook> = std::sync::OnceLock::new();

pub fn set_attach_hook(hook: AttachHook) {
    let _ = ATTACH_HOOK.set(hook);
}

/// `f` run on this thread attached to the Java VM (`AttachHook`). (Unused for now: Meta's keyboard has to be made on
/// the main thread itself, not on an attached one.)
#[allow(dead_code)]
pub(crate) fn attached(f: &mut dyn FnMut()) {
    match ATTACH_HOOK.get() {
        Some(hook) => hook(f),
        None => f(),
    }
}

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

/// The whole game; the `skate3rust` binary and the Quest library both start here.
pub fn run() -> bevy::app::AppExit {
    run_with(None)
}

/// [`run`] on the headset's entry thread. (Nothing loads before the app any more - each engine loads when it is first
/// picked - so `pump` is no longer needed; kept for the entry point's sake.)
pub fn run_with(pump: Option<&mut dyn FnMut()>) -> bevy::app::AppExit {
    let _ = pump;
    #[cfg(target_os = "android")]
    if let Err(error) = android::prepare() {
        eprintln!("{error}");
        return bevy::app::AppExit::error();
    }
    clean::run()
}
