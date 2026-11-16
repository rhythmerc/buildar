//! The Skate 3 sim's snapshots (`physics::network`: plain data, no transport) as the network's messages
//! (`skate_net`), and back. The sim doesn't know `skate_net`, so it can stand alone; this is where they meet.
use crate::physics::network as sim;
use bevy::math::Mat4;
use skate_net::{
    Body, Bone, Pose,
    packed::{BodyState, PoseState},
};

pub(crate) fn pose(p: sim::Pose) -> Pose {
    Pose { p: p.p, q: p.q }
}

pub(crate) fn sim_pose(p: Pose) -> sim::Pose {
    sim::Pose { p: p.p, q: p.q }
}

/// A pose's matrix.
pub(crate) fn matrix(p: Pose) -> Mat4 {
    sim::matrix(sim_pose(p))
}

/// A matrix as a pose.
pub(crate) fn from_matrix(m: Mat4) -> Pose {
    pose(sim::pose(m))
}

pub(crate) fn body_state(s: sim::BodyState) -> BodyState {
    BodyState {
        root: pose(s.root),
        enabled: s.enabled,
        bodies: s
            .bodies
            .into_iter()
            .map(|b| Body { pose: pose(b.pose), velocity: b.velocity, angular: b.angular })
            .collect(),
    }
}

pub(crate) fn sim_body_state(s: &BodyState) -> sim::BodyState {
    sim::BodyState {
        root: sim_pose(s.root),
        enabled: s.enabled,
        bodies: s
            .bodies
            .iter()
            .map(|b| sim::Body { pose: sim_pose(b.pose), velocity: b.velocity, angular: b.angular })
            .collect(),
    }
}

pub(crate) fn pose_state(s: sim::PoseState) -> PoseState {
    PoseState {
        root: pose(s.root),
        bones: s.bones.into_iter().map(|b| Bone { index: b.index, pose: pose(b.pose) }).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_state_goes_over_and_back_unchanged() {
        let root = sim::Pose { p: [1.0, 2.0, 3.0], q: [0.0, 0.0, 0.0, 1.0] };
        let state = sim::BodyState {
            root,
            enabled: 1 << 63 | 5,
            bodies: vec![sim::Body { pose: root, velocity: [0.5, 0.0, 0.0], angular: [0.0, 1.0, 0.0] }],
        };
        assert_eq!(sim_body_state(&body_state(state.clone())), state);
        assert_eq!(skate_net::hash(b"a"), 0xaf63dc4c8601ec8c, "the sim's fingerprint hash matches");
    }
}
