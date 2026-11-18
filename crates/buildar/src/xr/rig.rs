//! The tabletop rig: where the miniature sits in the room, and how big it is.
//!
//! The XR tracking root is the only thing that changes. For a tracking-space
//! point `p` the world point is `root.translation + root.rotation * (k * p)`,
//! so choosing
//!
//! ```text
//! rotation    = yaw(-anchor_yaw)
//! translation = W - rotation * (k * anchor_position),   W = 0
//! ```
//!
//! puts the world point `W` (the world's origin: `W` = 0) exactly at the anchor
//! in the room, with world -Z along the anchor's forward direction, and the
//! whole world `k` times smaller than life.
//!
//! The anchor is the STAGE origin (the Guardian's floor centre) facing STAGE
//! -Z, so the world is pinned to the room the same way every launch: a saved
//! layout (`xr::build`) comes back on the same tables.
use super::XrOptions;
use bevy::prelude::*;
use bevy_mod_xr::session::XrTrackingRoot;

#[derive(Resource, Debug)]
pub(crate) struct Rig {
    pub scale: f32,
    /// Tracking-space anchor position and yaw.
    pub anchor: Option<(Vec3, f32)>,
}

impl Rig {
    pub(crate) fn root_transform(&self) -> Option<Transform> {
        let (position, yaw) = self.anchor?;
        Some(root_transform(self.scale, position, yaw))
    }
}

/// The world scale at which what the shared layer draws in the game's space for the player (pointer rays, the rings
/// they end in) has its size: at any world scale it is drawn that size in the room, not the game's.
pub(crate) const MARKER_AT_SCALE: f32 = 12.0;

/// How much bigger, in game metres, those are drawn at world scale `scale`, to stay the same real size.
pub(crate) fn marker_size(scale: f32) -> f32 {
    scale / MARKER_AT_SCALE
}

/// The tracking root that puts the world's origin at `anchor` (each engine puts the room where its own game needs it,
/// inside itself).
pub(crate) fn root_transform(scale: f32, anchor: Vec3, yaw: f32) -> Transform {
    let rotation = Quat::from_rotation_y(-yaw);
    Transform {
        translation: -(rotation * (scale * anchor)),
        rotation,
        scale: Vec3::splat(scale),
    }
}

/// Yaw about +Y that turns -Z onto the horizontal part of `rotation`'s forward.
pub(crate) fn heading(rotation: Quat) -> f32 {
    let forward = rotation * Vec3::NEG_Z;
    (-forward.x).atan2(-forward.z)
}

pub(crate) struct RigPlugin(pub XrOptions);

impl Plugin for RigPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Rig {
            scale: self.0.scale,
            anchor: Some((Vec3::ZERO, 0.0)),
        })
        .add_systems(
            PostUpdate,
            apply_root.before(bevy::transform::TransformSystems::Propagate),
        )
        .init_resource::<RigAtStart>()
        .add_systems(PreUpdate, note_start)
        .add_systems(PostUpdate, warn_mid_frame.before(apply_root));
    }
}

/// The rig as the frame started (after `First`, where resizes are made).
#[derive(Resource, Default)]
struct RigAtStart(Option<(f32, Option<(Vec3, f32)>)>);

fn note_start(rig: Res<Rig>, mut at: ResMut<RigAtStart>) {
    at.0 = Some((rig.scale, rig.anchor));
}

/// Something changed the rig during the frame (not in `First`): pieces placed earlier in it were drawn at the old
/// scale while the headset's view had the new one - the world jumps for a frame. Said, to find the writers left.
fn warn_mid_frame(rig: Res<Rig>, at: Res<RigAtStart>, mut times: Local<u64>) {
    if let Some((scale, anchor)) = at.0
        && (scale != rig.scale || anchor != rig.anchor)
    {
        // (the first time, then every 90th: a stick held changes it every frame)
        *times += 1;
        if *times % 90 != 1 {
            return;
        }
        warn!("RIG changed mid-frame: world scale {scale:.3} -> {:.3}, anchor {anchor:?} -> {:?}", rig.scale, rig.anchor);
    }
}

fn apply_root(rig: Res<Rig>, mut root: Query<&mut Transform, With<XrTrackingRoot>>) {
    let (Some(target), Ok(mut transform)) = (rig.root_transform(), root.single_mut()) else {
        return;
    };
    if *transform != target {
        *transform = target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_world(root: &Transform, tracking: Vec3) -> Vec3 {
        root.transform_point(tracking)
    }

    #[test]
    fn a_marker_is_the_same_real_size_at_any_world_scale() {
        // (a 4 cm ray, in game metres, drawn under the tracking root: its real thickness)
        let real = |scale: f32| 0.04 * marker_size(scale) / root_transform(scale, Vec3::ZERO, 0.0).scale.x;
        let at_twelfth = real(MARKER_AT_SCALE);
        for scale in [1.0, 8.0, 12.0, 120.0] {
            assert!((real(scale) - at_twelfth).abs() < 1e-7, "at world scale {scale}: {} m", real(scale));
        }
    }

    #[test]
    fn anchor_shows_the_world_origin() {
        let anchor = Vec3::new(0.3, 0.75, -0.6);
        for yaw in [0.0, 1.0, -2.5] {
            let root = root_transform(12.0, anchor, yaw);
            assert!(to_world(&root, anchor).length() < 1e-4);
        }
    }

    #[test]
    fn room_distances_are_scaled_up_in_the_world() {
        let root = root_transform(12.0, Vec3::ZERO, 0.7);
        let a = to_world(&root, Vec3::new(0.0, 0.0, 0.0));
        let b = to_world(&root, Vec3::new(0.1, 0.0, 0.0));
        assert!((a.distance(b) - 1.2).abs() < 1e-5, "10 cm of table is 1.2 m of world");
    }

    #[test]
    fn anchor_forward_becomes_world_negative_z() {
        let yaw = 0.9;
        let controller = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(-0.4);
        assert!((heading(controller) - yaw).abs() < 1e-5, "pitch does not change heading");
        let root = root_transform(5.0, Vec3::ZERO, heading(controller));
        let forward = (root.rotation * (controller * Vec3::NEG_Z)).with_y(0.0).normalize();
        assert!(forward.distance(Vec3::NEG_Z) < 1e-4);
    }
}
