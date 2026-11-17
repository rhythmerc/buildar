//! The engines' models drawn, from the contract's plain data ([`engine_core::model`]): one renderer for every engine's
//! model, knowing nothing of which engine gave it.
//!
//! - **A glTF source** (Skate 3's): its scene drawn with its own materials, each skin joint bound to the pose's joint
//!   of its name ([`bind_skins`]), its parent the nearest ancestor that is a joint too. Each joint's local transform
//!   is relative to that parent, and between two steps the bone-local transforms are blended ([`joint_locals`]): turned,
//!   not their matrices' entries nor their world positions, so joints keep their hierarchy while limbs turn.
//! - **Meshes as data** (THUG2's): Bevy meshes ([`mesh`]) with the app's materials ([`material`]: the standard lit one or
//!   unlit), skinned with identity inverse binds to joints whose transforms are the pose's root times each joint
//!   ([`flat_joints`]); not blended.
//! - **Carrying a pose between steps** ([`PoseHistory`]): the last two steps' poses, taken once each; a cut, a change
//!   of the engine's step or of its joint count starts the pair again, so nothing is blended across them.
//!
//! The per-engine systems that feed these (when a pose is taken, which entities are whose) are still each engine's
//! adapter's until one generic driver replaces them (`docs/game-code-out-plan.md`, step 8).
use bevy::prelude::*;
use engine_core::model::{AlphaMode, Lighting, MaterialDesc, MeshData, Pose};
use std::time::Duration;

// ---- Carrying a pose between steps ----

/// A step's pose as drawn: its root's transform and its joints' matrices, with what an engine's adapter keeps in step
/// with them (`extra`: Skate 3's desktop camera).
#[derive(Clone)]
pub(crate) struct Snapshot<E> {
    pub root: Transform,
    pub joints: Vec<Mat4>,
    pub extra: E,
}

/// The last two steps' poses, to draw between.
pub(crate) struct PoseHistory<E> {
    pair: Option<(Snapshot<E>, Snapshot<E>)>,
    generation: u64,
    period: Duration,
}

impl<E> Default for PoseHistory<E> {
    fn default() -> Self {
        Self { pair: None, generation: 0, period: Duration::ZERO }
    }
}

impl<E: Clone> PoseHistory<E> {
    /// `pose` as the newest of the pair, made when the engine steps every `period` (a pose already taken: false).
    pub(crate) fn offer(&mut self, pose: &Pose, extra: E, period: Duration) -> bool {
        if pose.generation == self.generation {
            return false;
        }
        let next = Snapshot { root: Transform::from_matrix(pose.root), joints: pose.joints.clone(), extra };
        // Snap cuts and cadence changes instead of sweeping through the world or blending poses across different
        // step periods.
        let reset = pose.discontinuity || self.period != period;
        if let Some((previous, current)) = self.pair.as_mut() {
            if reset || current.joints.len() != next.joints.len() {
                *previous = next.clone();
            } else {
                std::mem::swap(previous, current);
            }
            *current = next;
        } else {
            self.pair = Some((next.clone(), next));
        }
        self.generation = pose.generation;
        self.period = period;
        true
    }

    pub(crate) fn pair(&self) -> Option<(&Snapshot<E>, &Snapshot<E>)> {
        self.pair.as_ref().map(|(a, b)| (a, b))
    }

    /// The pair and how far from the older to the newer to draw (`alpha`, kept within 0..1).
    pub(crate) fn view(&self, alpha: f32) -> Option<(&Snapshot<E>, &Snapshot<E>, f32)> {
        self.pair().map(|(a, b)| (a, b, alpha.clamp(0.0, 1.0)))
    }
}

/// Between two transforms: positions and scales along a line, the turn the short way round.
pub(crate) fn blend(a: Transform, b: Transform, alpha: f32) -> Transform {
    Transform {
        translation: a.translation.lerp(b.translation, alpha),
        rotation: a.rotation.normalize().slerp(b.rotation.normalize(), alpha),
        scale: a.scale.lerp(b.scale, alpha),
    }
}

// ---- A glTF source ----

/// A skin joint of a glTF model bound to the pose: its node, its joint, and its parent joint.
#[cfg_attr(not(feature = "retail"), allow(dead_code))] // (only Skate 3 gives a glTF so far)
pub(crate) struct JointBinding {
    pub(crate) entity: Entity,
    pub(crate) joint: usize,
    pub(crate) parent: Option<usize>,
}

/// Each skin's joints bound to the pose's joints by name (`names`, any case), with the nearest named ancestor as the
/// parent joint. `node`: a node's name and local matrix; `ancestors`: a node's ancestors, nearest first; `is_root`:
/// the model's root (the search stops there). A node between two joints must be the identity (another import
/// convention would double a pose).
#[cfg_attr(not(feature = "retail"), allow(dead_code))] // (only Skate 3 gives a glTF so far)
pub(crate) fn bind_skins<'a>(
    names: &[String],
    skins: impl IntoIterator<Item = &'a [Entity]>,
    node: impl Fn(Entity) -> Option<(String, Mat4)>,
    ancestors: impl Fn(Entity) -> Vec<Entity>,
    is_root: impl Fn(Entity) -> bool,
) -> Result<Vec<JointBinding>, String> {
    let mut bindings = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for skin in skins {
        for &entity in skin {
            if !seen.insert(entity) {
                continue;
            }
            let (name, _) = node(entity).ok_or("A model's skin joint is missing its name or transform")?;
            let joint = names
                .iter()
                .position(|n| n.eq_ignore_ascii_case(name.as_str()))
                .ok_or_else(|| format!("The model's skin joint {name} is not one of the pose's"))?;
            let mut parent = None;
            for ancestor in ancestors(entity) {
                if is_root(ancestor) {
                    break;
                }
                if let Some((name, transform)) = node(ancestor) {
                    if let Some(index) = names.iter().position(|n| n.eq_ignore_ascii_case(name.as_str())) {
                        parent = Some(index);
                        break;
                    }
                    if !transform.abs_diff_eq(Mat4::IDENTITY, 0.00001) {
                        return Err(format!("The model's node {name}, between two joints, has an unsupported transform"));
                    }
                }
            }
            bindings.push(JointBinding { entity, joint, parent });
        }
    }
    Ok(bindings)
}

/// A bound joint's local transform in a pose's `joints` (`basis`: the source's).
#[cfg_attr(not(feature = "retail"), allow(dead_code))] // (only Skate 3 gives a glTF so far)
fn local(binding: &JointBinding, joints: &[Mat4], basis: Mat4) -> Option<Transform> {
    let global = *joints.get(binding.joint)? * basis;
    Some(Transform::from_matrix(if let Some(parent) = binding.parent {
        (*joints.get(parent)? * basis).inverse() * global
    } else {
        global
    }))
}

/// Each bound joint's local transform in `joints`, unblended (a joint the pose hasn't: left as it is).
#[cfg_attr(not(feature = "retail"), allow(dead_code))] // (only Skate 3 gives a glTF so far)
pub(crate) fn joint_transforms(bindings: &[JointBinding], joints: &[Mat4], basis: Mat4) -> Vec<(Entity, Transform)> {
    bindings.iter().filter_map(|b| Some((b.entity, local(b, joints, basis)?))).collect()
}

/// Each bound joint's local transform `alpha` of the way from `previous` to `current` (poses with the same joints).
#[cfg_attr(not(feature = "retail"), allow(dead_code))] // (only Skate 3 gives a glTF so far)
pub(crate) fn joint_locals(bindings: &[JointBinding], previous: &[Mat4], current: &[Mat4], basis: Mat4, alpha: f32) -> Vec<Transform> {
    bindings
        .iter()
        .map(|b| {
            let at = |joints: &[Mat4]| local(b, joints, basis).expect("a pose with the bound joints");
            blend(at(previous), at(current), alpha)
        })
        .collect()
}

// ---- Meshes as data ----

/// A model's part, as spawned by [`spawn_meshes`].
#[cfg_attr(not(feature = "thug2"), allow(dead_code))] // (only THUG2 gives meshes as data so far)
#[derive(Component)]
pub(crate) struct ModelPart;

/// A model given as meshes, spawned: its root (where the model is, for culling: skinning ignores it) and its joints.
#[cfg_attr(not(feature = "thug2"), allow(dead_code))] // (only THUG2 gives meshes as data so far)
pub(crate) struct FlatModel {
    pub(crate) root: Entity,
    pub(crate) joints: Vec<Entity>,
}

/// Meshes and textures given as data spawned under a new root named `name` (hidden), skinned to `joint_count` joints
/// with identity inverse binds.
#[cfg_attr(not(feature = "thug2"), allow(dead_code))] // (only THUG2 gives meshes as data so far)
pub(crate) fn spawn_meshes(world: &mut World, name: &str, meshes: &[MeshData], textures: &[engine_core::hud::Rgba8], joint_count: u16) -> FlatModel {
    use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
    let images: Vec<Handle<Image>> = {
        let mut images = world.resource_mut::<Assets<Image>>();
        textures.iter().map(|t| images.add(image(t))).collect()
    };
    let joint_count = usize::from(joint_count);
    let bindposes = world
        .resource_mut::<Assets<SkinnedMeshInverseBindposes>>()
        .add(SkinnedMeshInverseBindposes::from(vec![Mat4::IDENTITY; joint_count]));
    let root = world.spawn((Name::new(name.to_owned()), Transform::default(), Visibility::Hidden)).id();
    let joints: Vec<Entity> = (0..joint_count).map(|_| world.spawn((Transform::default(), Visibility::default())).id()).collect();
    for part in meshes {
        let Some(m) = mesh(part) else { continue };
        let material = material(&part.material, &images);
        let m = world.resource_mut::<Assets<Mesh>>().add(m);
        let material = world.resource_mut::<Assets<StandardMaterial>>().add(material);
        let part = world
            .spawn((
                ModelPart,
                Mesh3d(m),
                MeshMaterial3d(material),
                SkinnedMesh { inverse_bindposes: bindposes.clone(), joints: joints.clone() },
                // (bounds of the bind pose: a pose reaches past them)
                bevy::camera::visibility::NoFrustumCulling,
                Transform::default(),
            ))
            .id();
        world.entity_mut(root).add_child(part);
    }
    FlatModel { root, joints }
}

/// A texture given as data (sRGB, straight alpha), repeated, filtered.
#[cfg_attr(not(feature = "thug2"), allow(dead_code))] // (only THUG2 gives meshes as data so far)
pub(crate) fn image(t: &engine_core::hud::Rgba8) -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let mut image = Image::new(
        Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        t.pixels.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

/// A mesh given as data as a Bevy mesh (none with no triangles).
#[cfg_attr(not(feature = "thug2"), allow(dead_code))] // (only THUG2 gives meshes as data so far)
pub(crate) fn mesh(data: &MeshData) -> Option<Mesh> {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
    if data.indices.is_empty() {
        return None;
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, data.positions.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, data.normals.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, data.uvs.clone());
    if data.material.vertex_color && !data.colors.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, data.colors.clone());
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(data.joints.clone()));
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, data.weights.clone());
    mesh.insert_indices(Indices::U16(data.indices.clone()));
    Some(mesh)
}

/// A material given as data drawn by the app's lighting models (`images`: its model's textures).
#[cfg_attr(not(feature = "thug2"), allow(dead_code))] // (only THUG2 gives meshes as data so far)
pub(crate) fn material(desc: &MaterialDesc, images: &[Handle<Image>]) -> StandardMaterial {
    let [r, g, b, a] = desc.base_color;
    StandardMaterial {
        base_color: Color::srgba(r, g, b, a),
        base_color_texture: desc.texture.and_then(|t| images.get(t as usize).cloned()),
        alpha_mode: match desc.alpha {
            AlphaMode::Opaque => bevy::prelude::AlphaMode::Opaque,
            AlphaMode::Mask(cutoff) => bevy::prelude::AlphaMode::Mask(cutoff),
            AlphaMode::Blend => bevy::prelude::AlphaMode::Blend,
        },
        double_sided: desc.two_sided,
        cull_mode: if desc.two_sided { None } else { Some(bevy::render::render_resource::Face::Back) },
        perceptual_roughness: 0.85,
        unlit: desc.lighting == Lighting::Unlit,
        ..default()
    }
}

/// A flat model's root and each of its `count` joints' transforms in the world for `pose` (a joint past the pose's:
/// the root's).
#[cfg_attr(not(feature = "thug2"), allow(dead_code))] // (only THUG2 gives meshes as data so far)
pub(crate) fn flat_joints(pose: &Pose, count: usize) -> (Mat4, Vec<Mat4>) {
    let joints = (0..count).map(|i| pose.root * pose.joints.get(i).copied().unwrap_or(Mat4::IDENTITY)).collect();
    (pose.root, joints)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(generation: u64, x: f32, joints: usize, discontinuity: bool) -> Pose {
        Pose {
            generation,
            discontinuity,
            root: Mat4::from_translation(Vec3::X * x),
            joints: vec![Mat4::from_translation(Vec3::Y * x); joints],
        }
    }

    /// A pose is taken once; a cut, a new step period or a new joint count isn't blended from the one before.
    #[test]
    fn a_cut_or_a_new_period_starts_the_pair_again() {
        let step = Duration::from_millis(16);
        let mut h = PoseHistory::<()>::default();
        assert!(h.view(0.5).is_none());
        assert!(h.offer(&pose(1, 0.0, 2, false), (), step));
        assert!(!h.offer(&pose(1, 5.0, 2, false), (), step), "taken once");
        h.offer(&pose(2, 1.0, 2, false), (), step);
        let (a, b, t) = h.view(2.0).unwrap();
        assert_eq!((a.root.translation.x, b.root.translation.x, t), (0.0, 1.0, 1.0));
        for (next, joints, cut, period) in [(3, 2, true, step), (4, 2, false, step * 2), (5, 3, false, step * 2)] {
            h.offer(&pose(next, next as f32, joints, cut), (), period);
            let (a, b) = h.pair().unwrap();
            assert_eq!(a.root.translation.x, b.root.translation.x, "pose {next}: not blended");
        }
    }

    /// A glTF joint is placed against its parent joint; a parentless one in the model's space.
    #[test]
    fn a_joint_is_local_to_its_parent() {
        let mut world = World::new();
        let (e0, e1) = (world.spawn_empty().id(), world.spawn_empty().id());
        let bindings = [JointBinding { entity: e0, joint: 0, parent: None }, JointBinding { entity: e1, joint: 1, parent: Some(0) }];
        let joints = [Mat4::from_translation(Vec3::X), Mat4::from_translation(Vec3::new(1.0, 2.0, 0.0))];
        let t = joint_transforms(&bindings, &joints, Mat4::IDENTITY);
        assert_eq!(t[0].1.translation, Vec3::X);
        assert_eq!(t[1].1.translation, Vec3::Y * 2.0);
        let half = joint_locals(&bindings, &joints, &[Mat4::IDENTITY, Mat4::IDENTITY], Mat4::IDENTITY, 0.5);
        assert_eq!(half[1].translation, Vec3::Y);
    }
}
