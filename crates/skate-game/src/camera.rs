//! Normal gameplay camera rendering endpoint. Simulation owns the cadence of
//! CameraRuntime::advance after its complete physical/animation publication.
use crate::{app::FrameSet, config::Config};
use bevy::prelude::*;
// The camera graph and runtime are the sim's (skate3-sim); this module draws with its frame.
#[allow(unused_imports)]
pub(crate) use skate3_sim::camera::*;

/// The sim's [`skate3_sim::camera::CameraRuntime`] as a Bevy resource (transitional shim).
#[derive(Resource, Deref, DerefMut)]
pub(crate) struct CameraRuntime(pub skate3_sim::camera::CameraRuntime);

impl CameraRuntime {
    pub(crate) fn load(root: &std::path::Path) -> Result<Self, String> {
        skate3_sim::camera::CameraRuntime::load(root).map(Self)
    }
}

#[derive(Component)]
pub(crate) struct GameplayCamera;

pub(crate) struct CameraPlugin;
impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn).add_systems(
            Update,
            present
                .after(FrameSet::Animation)
                .in_set(crate::app::SKATE3),
        );
    }
}
fn spawn(mut commands: Commands, config: Res<Config>) {
    let runtime = CameraRuntime::load(&config.asset_root)
        .unwrap_or_else(|error| panic!("Cannot initialize normal gameplay camera: {error}"));
    commands.insert_resource(runtime);
    commands.spawn((
        GameplayCamera,
        Camera3d::default(),
        Camera {
            is_active: false,
            ..default()
        },
        Transform::default(),
    ));
}

pub(crate) fn present(
    mut runtime: ResMut<CameraRuntime>,
    windows: Query<&Window>,
    history: Res<crate::presentation::Presentation>,
    time: Res<Time<Fixed>>,
    hold: Res<crate::app::Hold>,
    replay: Res<crate::replay::Replay>,
    config: Res<Config>,
    mut cameras: Query<(&mut Camera, &mut Transform, &mut Projection), With<GameplayCamera>>,
) {
    if let Ok(window) = windows.single() {
        // A minimised window reports zero height, and 0/0 is NaN. That NaN
        // reaches the field of view, the frame fails the finite check in
        // `runtime`, and the game exits with "Normal gameplay camera produced
        // a non-finite frame" -- so minimising the window killed the running
        // game. Keep the last good ratio instead: there is nothing to draw at
        // zero area anyway, and it is restored the moment the window is.
        let (width, height) = (window.width(), window.height());
        if width > 0. && height > 0. {
            runtime.set_aspect_ratio(width / height);
        }
    }
    let Some((previous, current, alpha)) = history.view(&replay, hold.alpha(&time)) else {
        return;
    };
    for (mut camera, mut transform, mut projection) in &mut cameras {
        *transform = crate::presentation::blend(previous.camera, current.camera, alpha);
        if replay.active {
            if let Some(free) = replay.free_camera {
                *transform = free;
            }
        }
        if let Projection::Perspective(p) = &mut *projection {
            p.fov = previous.fov + (current.fov - previous.fov) * alpha;
        }
        // In XR the eye cameras are the only views (see `xr::headset_only_rendering`).
        camera.is_active = config.xr.is_none();
    }
}
