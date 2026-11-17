//! Skate 3's ticks as drawn: each completed tick's pose (`skate3_engine::model::pose`, its cuts included) carried by
//! the generic [`crate::model::PoseHistory`], with the desktop camera's frame kept in step with it. Camera, root and
//! skin use the same interval; interpolated values never feed back into simulation.
use crate::model::PoseHistory;
use crate::{app::SimulationSet, camera::CameraRuntime, physics::SkaterRuntime};
use bevy::prelude::*;
use skate_core::camera::CameraFrame;

pub(crate) use crate::model::blend;

pub(crate) struct PresentationPlugin;
impl Plugin for PresentationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Presentation>()
            .add_systems(
                FixedUpdate,
                capture
                    .after(SimulationSet::Physics)
                    .in_set(crate::app::SKATE3),
            );
    }
}

/// The desktop camera's frame of a tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shot {
    pub camera: Transform,
    pub fov: f32,
}

/// A tick as drawn: the skater's pose and the camera's shot.
pub(crate) type Snapshot = crate::model::Snapshot<Shot>;

#[derive(Resource, Default)]
pub(crate) struct Presentation(pub(crate) PoseHistory<Shot>);

impl Presentation {
    /// The pair to draw between (the replay's, while it plays) and how far.
    pub fn view<'a>(
        &'a self,
        replay: &'a crate::replay::Replay,
        alpha: f32,
    ) -> Option<(&'a Snapshot, &'a Snapshot, f32)> {
        if replay.active {
            replay.sample()
        } else {
            self.0.view(alpha)
        }
    }
}

fn capture(
    skater: Res<SkaterRuntime>,
    camera: Res<CameraRuntime>,
    time: Res<Time<Fixed>>,
    mut history: ResMut<Presentation>,
    mut pose: Local<engine_core::Pose>,
) {
    let Some(frame) = camera.frame else {
        return;
    };
    skate3_engine::model::pose(&skater, &frame, &mut pose);
    let shot = Shot { camera: camera_transform(frame), fov: frame.field_of_view_degrees.to_radians() };
    // (the parked replay records nothing: `crate::replay`)
    history.0.offer(&pose, shot, time.timestep());
}

pub(crate) fn camera_transform(frame: CameraFrame) -> Transform {
    let [right, up, at] = frame.basis.columns.map(Vec3::from_array);
    Transform {
        translation: Vec3::new(frame.position[0], frame.position[1], frame.position[2]),
        // Native At is forward; Bevy cameras look along local -Z.
        rotation: Quat::from_mat3(&Mat3::from_cols(-right, up, -at)).normalize(),
        ..default()
    }
}
