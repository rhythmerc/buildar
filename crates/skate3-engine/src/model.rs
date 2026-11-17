//! The skater's model and pose as the contract's ([`engine_core::model`]).
//!
//! - **Model** ([`model`]): the manifest's GLB, its skin's joints named as the stock rig's bones, and the rig's
//!   parents. The GLB was exported through Blender: its bone-local axes are turned -90 degrees about x from the rig's
//!   own ([`BASIS`]; both are y-up in the world). That is a skin basis change, not a physics turn.
//! - **Pose** ([`pose`]): the sim's render pose (the rig's bones in the animation root's space) and that root in the
//!   world, published once a tick. It is a cut (not blended from the one before) when the gameplay camera cuts or the
//!   sim's teleport flag is up.
use engine_core::glam::{Mat4, Vec4};
use engine_core::model::{Model, ModelSource, Pose};
use skate_core::camera::CameraFrame;
use skate3_sim::physics::SkaterRuntime;

/// A bone's matrix in the GLB's axes: the rig's times this (`x, -z, y`).
// (-Vec4::Z, as the app always built it: its zeros negative, so products round exactly as they did)
pub const BASIS: Mat4 = Mat4::from_cols(Vec4::X, Vec4::new(-0.0, -0.0, -1.0, -0.0), Vec4::Y, Vec4::W);

/// The skater's model: the GLB at `character_scene` (the manifest's), bound to the sim's rig.
pub fn model(character_scene: &str, skater: &SkaterRuntime) -> Model {
    let frames = &skater.animation.evaluator.frames;
    Model {
        source: ModelSource::Gltf { path: character_scene.to_owned(), joint_names: frames.bone_names.clone(), basis: BASIS },
        parents: frames.parents.iter().map(|&p| u16::try_from(p).ok()).collect(),
        groups: Vec::new(),
    }
}

/// The skater's pose after the last tick, with the camera's frame of it (`camera`).
pub fn pose(skater: &SkaterRuntime, camera: &CameraFrame, out: &mut Pose) {
    let pose = skate3_sim::SkaterPose::of(skater);
    out.generation = pose.generation;
    out.discontinuity = camera.discontinuity || skater.player_input.processed.flags_2472 & (1 << 10) != 0;
    out.root = pose.root;
    out.joints.clear();
    out.joints.extend(pose.bones.iter().copied().map(skate3_sim::animation::native_matrix));
}

/// The sim's render pose now, unblended (before a tick has been published).
pub fn joints(skater: &SkaterRuntime, out: &mut Vec<Mat4>) {
    out.clear();
    out.extend(skater.render_pose.iter().copied().map(skate3_sim::animation::native_matrix));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The basis is the app's old `render_basis` exactly, to the zeros' signs.
    #[test]
    fn the_basis_is_x_minus_z_y() {
        let old = Mat4::from_cols(Vec4::X, -Vec4::Z, Vec4::Y, Vec4::W);
        assert_eq!(BASIS.to_cols_array().map(f32::to_bits), old.to_cols_array().map(f32::to_bits));
    }
}
