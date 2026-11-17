//! Skate 3's ticks as drawn: each completed tick's pose (`skate3_engine::model::pose`, its cuts included) carried by
//! the generic [`crate::model::PoseHistories`] (its one model, id 0), with the desktop camera's frame kept in step with
//! it. Camera, root and skin use the same interval; interpolated values never feed back into simulation.
//!
//! Its clock is Bevy's fixed one: a tick's pose is stamped with the fixed clock's time and timestep
//! (`engine_core::model::Step`), and drawn with that clock's time now ([`crate::app::Hold::now`]: its time plus its
//! leftover, held while Skate 3 is paused). The time since the tick in periods is then exactly the fixed clock's
//! overstep fraction, as before.
use crate::model::PoseHistories;
use crate::{app::SimulationSet, camera::CameraRuntime, physics::SkaterRuntime};
use bevy::prelude::*;
use skate_core::camera::CameraFrame;
use std::time::Duration;

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

/// Skate 3's skater: its one model.
pub(crate) const SKATER: engine_core::ModelId = 0;

#[derive(Resource, Default)]
pub(crate) struct Presentation(pub(crate) PoseHistories<Shot>);

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
            self.0.get(SKATER)?.view(alpha)
        }
    }

    /// The pair to draw between with the fixed clock at `now` ([`crate::app::Hold::now`]) and how far.
    pub fn view_at<'a>(
        &'a self,
        replay: &'a crate::replay::Replay,
        now: Duration,
    ) -> Option<(&'a Snapshot, &'a Snapshot, f32)> {
        let alpha = self.0.get(SKATER).map_or(0.0, |h| h.alpha(now));
        self.view(replay, alpha)
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
    pose.model = SKATER;
    // (the tick just run ended at the fixed clock's time; the sim has set the timestep it takes next)
    pose.step = engine_core::Step { at: time.elapsed(), period: time.timestep() };
    let shot = Shot { camera: camera_transform(frame), fov: frame.field_of_view_degrees.to_radians() };
    // (the parked replay records nothing: `crate::replay`)
    history.0.offer(&pose, shot);
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
