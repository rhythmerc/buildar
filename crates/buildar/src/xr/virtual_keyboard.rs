//! Meta's own keyboard in the room (`XR_META_virtual_keyboard`), for typing a name - in place of the panel's own
//! keyboard where the headset has it.
//!
//! For now a probe, run when a name is first to be typed (`Wanted`: the player is in the headset, the app in focus -
//! making the keyboard waits on the runtime, and with the app out of focus it waited past Android's patience): it
//! asks whether the headset has the keyboard, makes one, and fetches the model the runtime draws it with (`XR_FB_render_model`, "/model_meta/keyboard/virtual"): it is saved to the app's
//! files as `virtual-keyboard.glb` and summed up in the log (its images and how they're named, its animations, its
//! nodes), for the drawing of it to be made to fit.
use bevy::prelude::*;
use bevy_mod_openxr::resources::{OxrInstance, OxrSystemId};
use bevy_mod_openxr::session::OxrSession;
use openxr::sys;
use openxr::sys::Handle as _;

pub(crate) struct VirtualKeyboardPlugin;

impl Plugin for VirtualKeyboardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Wanted>().add_systems(Update, probe.run_if(resource_exists::<OxrSession>));
    }
}

/// A name is to be typed: the keyboard is wanted (set by the course details' rename).
#[derive(Resource, Default)]
pub(crate) struct Wanted(pub bool);

/// The extension, asked for when the session is made (dropped where the headset doesn't offer it).
pub(crate) fn enable(exts: &mut openxr::ExtensionSet) {
    exts.meta_virtual_keyboard = true;
    exts.fb_render_model = true;
}

/// Where the probe is: frames waited, the keyboard being made (on a thread of its own: the call waits on the
/// runtime, which may want frames to go on meanwhile) or made, model fetches tried, and whether it's done.
#[derive(Default)]
struct Probe {
    wait: u32,
    making: Option<(std::sync::mpsc::Receiver<sys::Result>, std::sync::Arc<std::sync::atomic::AtomicU64>, std::time::Instant)>,
    keyboard: Option<sys::VirtualKeyboardMETA>,
    tries: u32,
    done: bool,
}

const TRIES: u32 = 10;

/// (On the app's main thread - `NonSendMarker` - which made the session: the runtime lends that thread's Java handle
/// to a worker of its own while the call waits, so the call must come from it, the thread then held still. From
/// another thread the handle is used by two at once, and the runtime's Java calls fail: Android aborts the app.)
fn probe(
    _main: bevy::ecs::system::NonSendMarker,
    session: Res<OxrSession>,
    instance: Res<OxrInstance>,
    system: Option<Res<OxrSystemId>>,
    wanted: Res<Wanted>,
    mut state: Local<Probe>,
) {
    if state.done || !wanted.0 {
        return;
    }
    state.wait += 1;
    // (a second apart)
    if state.wait % 72 != 1 {
        return;
    }
    let Some(fns) = instance.exts().meta_virtual_keyboard else {
        info!("VKB: the headset doesn't offer XR_META_virtual_keyboard");
        state.done = true;
        return;
    };
    let ok = |result: sys::Result, what: &str| if result.into_raw() >= 0 { Ok(()) } else { Err(format!("{what}: {result:?}")) };
    // (being made: done yet?)
    if let Some((rx, handle, started)) = &state.making {
        match rx.try_recv() {
            Ok(result) => {
                let ms = started.elapsed().as_secs_f64() * 1000.0;
                match ok(result, "making the keyboard") {
                    Ok(()) => {
                        info!("VKB: keyboard made ({ms:.0} ms, on its own thread)");
                        state.keyboard = Some(sys::VirtualKeyboardMETA::from_raw(handle.load(std::sync::atomic::Ordering::Acquire)));
                    }
                    Err(why) => {
                        warn!("VKB: {why} ({ms:.0} ms)");
                        state.done = true;
                    }
                }
                state.making = None;
            }
            Err(_) => {
                info!("VKB: still making the keyboard ({:.1} s)", started.elapsed().as_secs_f64());
                return;
            }
        }
    }
    if state.keyboard.is_none() {
        // (does this headset have it)
        if let Some(system) = system.as_ref() {
            let mut vk = sys::SystemVirtualKeyboardPropertiesMETA { ty: sys::SystemVirtualKeyboardPropertiesMETA::TYPE, next: std::ptr::null_mut(), supports_virtual_keyboard: sys::FALSE };
            // SAFETY: the properties are filled in, the chain is one struct of ours.
            let mut props: sys::SystemProperties = unsafe { std::mem::zeroed() };
            props.ty = sys::SystemProperties::TYPE;
            props.next = (&raw mut vk).cast();
            let got = unsafe { (instance.fp().get_system_properties)(instance.as_raw(), system.0, &mut props) };
            info!("VKB: system properties {got:?}: supports_virtual_keyboard {}", vk.supports_virtual_keyboard.into_raw());
            if vk.supports_virtual_keyboard == sys::FALSE {
                state.done = true;
                return;
            }
        }
        // (on the main thread: the runtime uses the Java handle of the thread that made the session, whatever thread
        // calls it - a thread of our own, attached or not, made Android abort the app)
        info!("VKB: making the keyboard (main thread: {:?})…", std::thread::current().name());
        let started = std::time::Instant::now();
        let info = sys::VirtualKeyboardCreateInfoMETA { ty: sys::VirtualKeyboardCreateInfoMETA::TYPE, next: std::ptr::null() };
        let mut keyboard = sys::VirtualKeyboardMETA::NULL;
        // SAFETY: the session is live; the handle is written.
        let result = unsafe { (fns.create_virtual_keyboard)(session.as_raw(), &info, &mut keyboard) };
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        match ok(result, "making the keyboard") {
            Ok(()) => {
                info!("VKB: keyboard made ({ms:.0} ms)");
                state.keyboard = Some(keyboard);
            }
            Err(why) => {
                warn!("VKB: {why} ({ms:.0} ms)");
                state.done = true;
                return;
            }
        }
    }
    state.tries += 1;
    match super::controller_shapes::model(&session, "/model_meta/keyboard/virtual") {
        Ok(bytes) => {
            state.done = true;
            info!("VKB: model {} bytes", bytes.len());
            #[cfg(target_os = "android")]
            if let Ok(dir) = crate::android::files_dir() {
                let path = dir.join("virtual-keyboard.glb");
                match std::fs::write(&path, &bytes) {
                    Ok(()) => info!("VKB: saved to {}", path.display()),
                    Err(e) => warn!("VKB: saving: {e}"),
                }
            }
            summarise(&bytes);
        }
        Err(why) if state.tries >= TRIES => {
            warn!("VKB: the model: {why}");
            state.done = true;
        }
        Err(why) => info!("VKB: the model not yet ({why})"),
    }
}

/// The model summed up in the log.
fn summarise(bytes: &[u8]) {
    let Ok(gltf) = gltf::Gltf::from_slice_without_validation(bytes) else {
        warn!("VKB: not a glTF");
        return;
    };
    let doc = &gltf.document;
    info!(
        "VKB: {} scenes, {} nodes, {} meshes, {} materials, {} textures, {} images, {} animations; extensions used {:?}, required {:?}",
        doc.scenes().count(),
        doc.nodes().count(),
        doc.meshes().count(),
        doc.materials().count(),
        doc.textures().count(),
        doc.images().count(),
        doc.animations().count(),
        doc.extensions_used().collect::<Vec<_>>(),
        doc.extensions_required().collect::<Vec<_>>(),
    );
    for image in doc.images() {
        let source = match image.source() {
            gltf::image::Source::Uri { uri, mime_type } => format!("uri {uri} ({mime_type:?})"),
            gltf::image::Source::View { view, mime_type } => format!("view {} bytes ({mime_type})", view.length()),
        };
        info!("VKB: image {} {:?}: {source}", image.index(), image.name());
    }
    for material in doc.materials() {
        let base = material.pbr_metallic_roughness().base_color_texture().map(|t| t.texture().source().index());
        info!("VKB: material {:?}: alpha {:?}, base texture image {base:?}, unlit {}", material.name(), material.alpha_mode(), material.unlit());
    }
    for animation in doc.animations() {
        let targets: Vec<String> = animation.channels().take(4).map(|c| format!("{:?}@{:?}", c.target().property(), c.target().node().name())).collect();
        info!("VKB: animation {} {:?}: {} channels, e.g. {targets:?}", animation.index(), animation.name(), animation.channels().count());
    }
    for node in doc.nodes().take(40) {
        info!("VKB: node {} {:?} mesh {:?} children {}", node.index(), node.name(), node.mesh().map(|m| m.index()), node.children().count());
    }
}
