//! Re-points `ndk_context` at the real `NativeActivity` before the engine
//! starts.
//!
//! android-activity 0.6 registers the *Application* in `ndk_context`, and the
//! `openxr` crate hands that object to the runtime as
//! `XrInstanceCreateInfoAndroidKHR::applicationActivity`. Meta's runtime then
//! can't track the Activity's lifecycle: `xrCreateSession` logs "Activity is
//! not yet in the ready state" and the session never leaves IDLE. An Activity
//! is still a `Context`, so every other `ndk_context` user keeps working.
use jni::objects::JObject;
use jni::JavaVM;

pub fn point_ndk_context_at_activity() {
    let Some(app) = bevy::android::ANDROID_APP.get() else {
        eprintln!("buildar_quest: ANDROID_APP not set yet, ndk_context left as Application");
        return;
    };
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    let result = vm.attach_current_thread(|env| -> jni::errors::Result<_> {
        let activity = unsafe { JObject::from_raw(&*env, app.activity_as_ptr().cast()) };
        // Leaked on purpose: ndk_context holds it for the life of the process
        // (singleTask, so one Activity per process).
        Ok(env.new_global_ref(activity)?.into_raw())
    });
    match result {
        Ok(activity_global) => {
            unsafe {
                ndk_context::release_android_context();
                ndk_context::initialize_android_context(
                    app.vm_as_ptr(),
                    activity_global.cast(),
                );
            }
            eprintln!("buildar_quest: ndk_context now points at the NativeActivity");
        }
        Err(e) => eprintln!("buildar_quest: failed to re-point ndk_context: {e}"),
    }
}

/// Runs `f` with this thread attached to the Java VM (`buildar::AttachHook`): a Rust thread calling into Meta's
/// runtime, which calls Java, must be.
pub fn run_attached(f: &mut dyn FnMut()) {
    let Some(app) = bevy::android::ANDROID_APP.get() else {
        f();
        return;
    };
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    let result = vm.attach_current_thread(|_env| -> jni::errors::Result<()> {
        f();
        Ok(())
    });
    if let Err(e) = result {
        eprintln!("buildar_quest: attaching a thread to the VM: {e}");
    }
}
