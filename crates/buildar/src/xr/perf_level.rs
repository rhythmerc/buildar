//! The CPU's performance level asked of the runtime (`XR_EXT_performance_settings`): sustained high, for the CPU
//! only (the GPU is left to the runtime). Left to itself the runtime ran the app's CPU at level 2 or 3 from launch to
//! launch; at 2 the frame's chain (update, render, the GPU) ran past two periods and the runtime queued a frame more
//! (its predicted display time ~70 ms out, not ~36). Dev flag `cpu_high` while it's tried.
use super::dev_flags::DevFlags;
use bevy::prelude::*;
use bevy_mod_openxr::resources::OxrInstance;
use bevy_mod_openxr::session::OxrSession;
use openxr::sys;

pub(crate) struct PerfLevelPlugin;

impl Plugin for PerfLevelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, ask);
    }
}

/// The extension, asked for when the session is made.
pub(crate) fn enable(exts: &mut openxr::ExtensionSet) {
    exts.ext_performance_settings = true;
}

fn ask(flags: Option<Res<DevFlags>>, session: Option<Res<OxrSession>>, instance: Option<Res<OxrInstance>>, mut asked: Local<bool>) {
    let (Some(session), Some(instance)) = (session, instance) else {
        *asked = false;
        return;
    };
    if std::mem::replace(&mut *asked, true) || !flags.is_some_and(|f| f.has("cpu_high")) {
        return;
    }
    let Some(fns) = instance.exts().ext_performance_settings else {
        warn!("PERF: no XR_EXT_performance_settings");
        return;
    };
    // SAFETY: the session is live.
    let result = unsafe {
        (fns.perf_settings_set_performance_level)(session.as_raw(), sys::PerfSettingsDomainEXT::CPU, sys::PerfSettingsLevelEXT::SUSTAINED_HIGH)
    };
    info!("PERF: CPU sustained high asked: {result:?}");
}
