//! Switches for measuring on the headset without a rebuild: `dev-flags.txt` in the app's folder (the external files
//! folder on the Quest, written with adb), one flag a line, read at launch. Unknown lines are ignored; no file, no
//! flags. Each flag in use is logged (`DEV FLAGS`).
//!
//! - `single_main`: the update world's schedules run on one thread (no task hand-offs to the compute worker).
//! - `single_render`: the render world's likewise.
//! - `course=<id, name or busiest>`: that course opened in front of the head in a debug box, the room ignored
//!   (`xr::debug_course`; implies `menu_closed`).
//! - `cluster_single`: one light cluster for the eyes, not Bevy's grid of them (a handful of lights).
//! - `matte=N`: occlusion's edge matte N x N (`xr::occlusion`; 768 by default).
//! - `menu_closed`: the menu doesn't open itself at launch (an engine measured as it plays).
//! Read where the plugins are made (`clean::dev_plugins`):
//! - `compute=N`: at least N threads in Bevy's compute pool; `compute=0`, none (the schedules' own threads run
//!   their systems).
//! - `cpu_preprocess`: meshes' instance data built on the CPU, not by GPU preprocessing (six compute passes an eye).
//! - `lean_plugins`: no post-processing, anti-aliasing or deferred lighting plugins (unused; their systems run).
//! - `quiet_spans`: the ECS's and the renderer's per-system tracing spans filtered out of the log.
use bevy::ecs::schedule::{ExecutorKind, Schedules};
use bevy::prelude::*;
use bevy::render::RenderApp;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Resource, Clone, Default, Debug)]
pub(crate) struct DevFlags(pub BTreeSet<String>);

impl DevFlags {
    pub(crate) fn read(folder: &Path) -> Self {
        let text = std::fs::read_to_string(folder.join("dev-flags.txt")).unwrap_or_default();
        Self(text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect())
    }

    pub(crate) fn has(&self, flag: &str) -> bool {
        self.0.contains(flag)
    }

    /// A `name=value` flag's value.
    pub(crate) fn value(&self, name: &str) -> Option<String> {
        self.0.iter().find_map(|f| Some(f.strip_prefix(name)?.strip_prefix('=')?.trim().to_string()))
    }

    /// A `name=N` flag's number.
    pub(crate) fn number(&self, name: &str) -> Option<usize> {
        self.0.iter().find_map(|f| f.strip_prefix(name)?.strip_prefix('=')?.trim().parse().ok())
    }
}

pub(crate) struct DevFlagsPlugin(pub DevFlags);

impl Plugin for DevFlagsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.0.clone()).add_observer(cluster_single);
        if !self.0 .0.is_empty() {
            info!("DEV FLAGS {:?}", self.0 .0);
        }
    }

    fn finish(&self, app: &mut App) {
        let single = |schedules: &mut Schedules| {
            for (_, schedule) in schedules.iter_mut() {
                schedule.set_executor_kind(ExecutorKind::SingleThreaded);
            }
        };
        if self.0.has("single_main") {
            single(&mut app.world_mut().resource_mut::<Schedules>());
        }
        if self.0.has("single_render")
            && let Some(render) = app.get_sub_app_mut(RenderApp)
        {
            single(&mut render.world_mut().resource_mut::<Schedules>());
        }
    }
}

fn cluster_single(add: On<Add, bevy_mod_xr::camera::XrCamera>, flags: Res<DevFlags>, mut commands: Commands) {
    if flags.has("cluster_single") {
        commands.entity(add.entity).insert(bevy::light::cluster::ClusterConfig::Single);
    }
}
