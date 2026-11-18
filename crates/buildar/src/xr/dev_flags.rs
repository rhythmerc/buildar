//! Switches for measuring on the headset without a rebuild: `dev-flags.txt` in the app's folder (the external files
//! folder on the Quest, written with adb), one flag a line, read at launch. Unknown lines are ignored; no file, no
//! flags. Each flag in use is logged (`DEV FLAGS`).
//!
//! - `single_main`: the update world's schedules run on one thread (no task hand-offs to the compute worker).
//! - `single_render`: the render world's likewise.
//! - `cluster_single`: one light cluster for the eyes, not Bevy's grid of them (a handful of lights).
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
