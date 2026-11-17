//! The skater's model and pose as the contract's ([`engine_core::model`]): its meshes and plain materials, and each
//! step's palette and draw matrix carried into the contract's space.
//!
//! - **Meshes** ([`source`]): bind-pose vertices in game units, skinned by up to three of the palette's joints a
//!   vertex. The game's vertex colours are BGRA bytes with 0x80 as unchanged (the game doubles them): here they are
//!   multipliers, `/128`. A part's colour is its first pass's, also doubled. Some parts are wound the other way to
//!   the contract's (counter-clockwise front): turned. Alpha: blended, alpha-tested at its reference, or opaque.
//! - **Pose** ([`pose`]): the game's matrices are row-major for row vectors, read here as column ones (transposed),
//!   and the game clears their w column (its skinning reads only the rest, as for a point): w is 1 again here, or a
//!   product would drop the translations of the matrices before it. The root is (game to contract) x (draw matrix);
//!   the joints are the palette (a joint's inverse bind times its posed matrix), so skinning needs no inverse binds.
//!
//! The C API gives no joint names, no parents and no joint positions (only the palette), so the model has no
//! skeleton shape: [`engine_core::model::Model::parents`] is empty.
use engine_core::glam::{Mat4, Vec3};
use engine_core::hud::Rgba8;
use engine_core::model::{AlphaMode, Lighting, MaterialDesc, MeshData, Model, ModelSource, Pose};
use thug2_sys as t2;

/// The joints the skater is posed with (the palette's most).
pub const JOINTS: u16 = 64;

/// The skater's model as the contract's.
pub fn source(model: &t2::SkaterModel) -> Model {
    Model {
        source: ModelSource::Meshes {
            meshes: model.meshes.iter().map(mesh).collect(),
            textures: model.textures.iter().map(|t| Rgba8 { width: t.width, height: t.height, pixels: t.rgba.clone() }).collect(),
            joint_count: JOINTS,
        },
        parents: Vec::new(),
        groups: Vec::new(),
    }
}

/// A part of the model (one for each of the game's, those with no triangles too: the runtime draws none of them).
fn mesh(part: &t2::SkaterMesh) -> MeshData {
    let v = &part.vertices;
    let mut indices = part.indices.clone();
    if wound_backwards(part) {
        for t in indices.chunks_exact_mut(3) {
            t.swap(1, 2);
        }
    }
    let colour = part.color.map(|c| (c * 2.0).min(1.0));
    MeshData {
        positions: v.iter().map(|v| v.position).collect(),
        normals: v.iter().map(|v| v.normal).collect(),
        uvs: v.iter().map(|v| v.uv).collect(),
        // (BGRA bytes, 0x80 unchanged: the game doubles them)
        colors: v
            .iter()
            .map(|v| {
                let [b, g, r, a] = v.color.to_le_bytes();
                [r, g, b, a].map(|c| (c as f32 / 128.0).min(1.0))
            })
            .collect(),
        joints: v.iter().map(|v| [v.bones[0], v.bones[1], v.bones[2], 0].map(u16::from)).collect(),
        weights: v
            .iter()
            .map(|v| {
                let sum: f32 = v.weights[..3].iter().sum();
                let w = if sum > 1e-6 { v.weights[..3].iter().map(|w| w / sum).collect::<Vec<_>>() } else { vec![1.0, 0.0, 0.0] };
                [w[0], w[1], w[2], 0.0]
            })
            .collect(),
        indices,
        material: MaterialDesc {
            base_color: [colour[0], colour[1], colour[2], 1.0],
            texture: part.texture.map(|t| t as u32),
            alpha: if part.blended {
                AlphaMode::Blend
            } else if part.alpha_ref > 0 {
                AlphaMode::Mask((part.alpha_ref as f32 / 255.0).max(0.02))
            } else {
                AlphaMode::Opaque
            },
            two_sided: part.two_sided,
            vertex_color: true,
            lighting: Lighting::Standard,
        },
    }
}

/// A mesh's triangles turned the other way to ours (front: counter-clockwise): most of them face away from their
/// vertices' normals.
fn wound_backwards(part: &t2::SkaterMesh) -> bool {
    let mut against = 0i64;
    for t in part.indices.chunks_exact(3) {
        let [a, b, c] = [0, 1, 2].map(|k| &part.vertices[t[k] as usize]);
        let (pa, pb, pc) = (Vec3::from_array(a.position), Vec3::from_array(b.position), Vec3::from_array(c.position));
        let geometric = (pb - pa).cross(pc - pa);
        let normal = Vec3::from_array(a.normal) + Vec3::from_array(b.normal) + Vec3::from_array(c.normal);
        against += if geometric.dot(normal) < 0.0 { 1 } else { -1 };
    }
    against > 0
}

/// The game's pose after a step as the contract's, at world size `scale`, the whole moved by `shift` (game units:
/// the host's, to draw it between the game's last two frames; zero for the step as it is).
pub fn pose(game: &t2::Pose, scale: f32, shift: Vec3, generation: u64, out: &mut Pose) {
    let to_ours =
        Mat4::from_scale(Vec3::splat(1.0 / (t2::INCHES_PER_METRE * scale))) * Mat4::from_translation(-crate::stage::OFFSET);
    let mut draw = game.draw;
    (draw[3], draw[7], draw[11], draw[15]) = (0.0, 0.0, 0.0, 1.0);
    out.generation = generation;
    out.discontinuity = false;
    out.root = to_ours * Mat4::from_translation(shift) * Mat4::from_cols_array(&draw);
    out.joints.clear();
    out.joints.extend(game.palette.iter().map(|m| {
        let mut m = *m;
        (m[3], m[7], m[11], m[15]) = (0.0, 0.0, 0.0, 1.0);
        Mat4::from_cols_array(&m)
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(p: [f32; 3], n: [f32; 3]) -> t2::ffi::Thug2Vertex {
        t2::ffi::Thug2Vertex { position: p, normal: n, color: 0x8040_2010, weights: [2.0, 1.0, 1.0, 5.0], ..Default::default() }
    }

    /// A part wound clockwise to its normals is turned; colours halved, weights of three made to sum to 1.
    #[test]
    fn a_part_is_turned_to_face_its_normals() {
        let down = [0.0, -1.0, 0.0];
        let part = t2::SkaterMesh {
            vertices: vec![vertex([0.0; 3], down), vertex([0.0, 0.0, 1.0], down), vertex([1.0, 0.0, 0.0], down)],
            indices: vec![0, 1, 2],
            color: [0.5, 0.25, 1.0, 0.5],
            alpha_ref: 64,
            ..Default::default()
        };
        // ((1-0) x (2-0) = +z x +x = +y: up, against the normals)
        let m = mesh(&part);
        assert_eq!(m.indices, [0, 2, 1]);
        assert_eq!(m.colors[0], [0.5, 0.25, 0.125, 1.0]);
        assert_eq!(m.weights[0], [0.5, 0.25, 0.25, 0.0]);
        assert_eq!(m.material.base_color, [1.0, 0.5, 1.0, 1.0]);
        assert_eq!(m.material.alpha, AlphaMode::Mask(64.0 / 255.0));
        assert!(mesh(&t2::SkaterMesh::default()).indices.is_empty());
    }

    /// The root carries the model from game units at the offset to metres; the joints are the palette, w fixed.
    #[test]
    fn a_pose_is_in_metres_with_its_w_column_fixed() {
        // (row-major for row vectors: the translation is the fourth row, the w column junk)
        let mut draw = Mat4::from_translation(crate::stage::OFFSET + Vec3::new(t2::INCHES_PER_METRE, 0.0, 0.0)).to_cols_array();
        (draw[3], draw[15]) = (9.0, 0.0);
        let game = t2::Pose { palette: vec![draw], draw };
        let mut out = Pose::default();
        pose(&game, 1.0, Vec3::ZERO, 7, &mut out);
        assert_eq!(out.generation, 7);
        let at = out.root.transform_point3(Vec3::ZERO);
        assert!((at - Vec3::X).length() < 1e-3, "{at}");
        assert_eq!(out.joints.len(), 1);
        assert_eq!(out.joints[0].w_axis.w, 1.0);
    }
}
