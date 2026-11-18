//! The engines' models drawn, from the contract's plain data ([`engine_core::model`]): one renderer for every engine's
//! model, knowing nothing of which engine gave it.
//!
//! - **A glTF source** (Skate 3's): its scene drawn with its own materials, each skin joint bound to the pose's joint
//!   of its name ([`bind_skins`]), its parent the nearest ancestor that is a joint too. Each joint's local transform
//!   is relative to that parent, and between two steps the bone-local transforms are blended ([`joint_locals`]): turned,
//!   not their matrices' entries nor their world positions, so joints keep their hierarchy while limbs turn.
//! - **Meshes as data** (THUG2's): Bevy meshes ([`mesh`]) with the app's materials ([`material`]: the standard lit one or
//!   unlit), skinned with identity inverse binds to joints whose transforms are the pose's root times each joint
//!   ([`flat_joints`]). With no skeleton's shape (no `parents`), each joint's matrix is blended on its own in the
//!   model's space, turned about the axis its step turns it about ([`flat_blend`], [`blend_joint`]).
//! - **Carrying a pose between steps** ([`PoseHistory`], one a model in [`PoseHistories`]): the last two steps' poses,
//!   taken once each; a cut (the engine's `discontinuity`: it put the model somewhere outside its motion), a change of
//!   the engine's step period or of its joint count, or a jump it didn't mark (a turn no body makes in one step, a
//!   mirror: [`JUMP_TURN_RATE`]) starts the pair again, so nothing is blended across them. How far between them is drawn is the same for every engine: the time since the
//!   newer one's step on the engine's clock, in periods ([`PoseHistory::alpha`], `engine_core::model::Step`). The root
//!   is blended the same way for every model ([`blend`]).
//! - **Its bodies** ([`bodies`]): each body group placed by the pose, for the contact shadows
//!   ([`engine_api::PlayerBodies`]).
//!
//! The per-engine systems that feed these (when a pose is taken, which entities are whose) are still each engine's
//! adapter's until one generic driver replaces them (`docs/game-code-out-plan.md`, step 8).
use bevy::prelude::*;
use engine_api::BodyShape;
use engine_core::model::{AlphaMode, BodyGroup, Cast, Lighting, MaterialDesc, MeshData, ModelId, Point, Pose, Step};
use std::time::Duration;

// ---- Its bodies ----

/// A model's body groups placed in the world (`point`: where a joint's point is now): a footprint between its two
/// points, or a segment for each of a group's joints whose parent (`parents`) is in the group too.
pub(crate) fn bodies(groups: &[BodyGroup], parents: &[Option<u16>], point: impl Fn(Point) -> Option<Vec3>) -> Vec<BodyShape> {
    let origin = |joint: u16| point(Point { joint, at: Vec3::ZERO });
    groups
        .iter()
        .filter_map(|group| match &group.cast {
            Cast::Footprint { back, front, ellipses } => {
                let (back, front) = (point(*back)?, point(*front)?);
                let along = (front - back).with_y(0.0).try_normalize()?;
                Some(BodyShape::Footprint { middle: (back + front) / 2.0, along, ellipses: ellipses.clone() })
            }
            Cast::Segments => {
                let segments: Vec<(Vec3, Vec3)> = group
                    .joints
                    .iter()
                    .filter_map(|&joint| {
                        let parent = parents.get(usize::from(joint)).copied().flatten()?;
                        group.joints.contains(&parent).then_some(())?;
                        Some((origin(parent)?, origin(joint)?))
                    })
                    .collect();
                (!segments.is_empty()).then_some(BodyShape::Segments(segments))
            }
        })
        .collect()
}

// ---- Carrying a pose between steps ----

/// A step's pose as drawn: its root's transform (and its matrix as given) and its joints' matrices.
#[derive(Clone)]
pub(crate) struct Snapshot {
    pub root: Transform,
    pub root_matrix: Mat4,
    pub joints: Vec<Mat4>,
}

/// A model's last two poses, to draw between, and the newer one's step (`engine_core::model::Step`: when it was on
/// the engine's clock, and its period).
pub(crate) struct PoseHistory {
    pair: Option<(Snapshot, Snapshot)>,
    generation: u64,
    step: Step,
}

impl Default for PoseHistory {
    fn default() -> Self {
        Self { pair: None, generation: 0, step: Step::default() }
    }
}

impl PoseHistory {
    /// `pose` as the newest of the pair (a pose already taken: false).
    pub(crate) fn offer(&mut self, pose: &Pose) -> bool {
        if pose.generation == self.generation {
            return false;
        }
        let next =
            Snapshot { root: Transform::from_matrix(pose.root), root_matrix: pose.root, joints: pose.joints.clone() };
        // Snap cuts and cadence changes instead of sweeping through the world or blending poses across different
        // step periods; and a jump the engine didn't mark ([`jumped`]).
        let reset = pose.discontinuity || self.step.period != pose.step.period;
        if let Some((previous, current)) = self.pair.as_mut() {
            if reset || current.joints.len() != next.joints.len() || jumped(current, &next, pose.step.period) {
                *previous = next.clone();
            } else {
                std::mem::swap(previous, current);
            }
            *current = next;
        } else {
            self.pair = Some((next.clone(), next));
        }
        self.generation = pose.generation;
        self.step = pose.step;
        true
    }

    pub(crate) fn pair(&self) -> Option<(&Snapshot, &Snapshot)> {
        self.pair.as_ref().map(|(a, b)| (a, b))
    }

    /// How far from the older pose to the newer to draw with the engine's clock at `now`: the time since the newer
    /// one's step, in its periods (`Step::alpha`; not kept within 0..1). The same for every engine.
    pub(crate) fn alpha(&self, now: Duration) -> f32 {
        self.step.alpha(now)
    }

    /// The pair and how far from the older to the newer to draw (`alpha`, kept within 0..1).
    pub(crate) fn view(&self, alpha: f32) -> Option<(&Snapshot, &Snapshot, f32)> {
        self.pair().map(|(a, b)| (a, b, alpha.clamp(0.0, 1.0)))
    }

    /// The pair and how far to draw between them with the engine's clock at `now`.
    #[cfg_attr(not(feature = "thug2"), allow(dead_code))] // (Skate 3 draws through its replay's view)
    pub(crate) fn view_at(&self, now: Duration) -> Option<(&Snapshot, &Snapshot, f32)> {
        self.view(self.alpha(now))
    }
}

/// How fast a model's root may turn before it is a jump, not a motion (radians a second): a quarter turn in a 60 Hz
/// step, 15 turns a second, as thug2-decomp's own between-ticks drawing (`transform_blend.h`: kObjectJump). Nothing a
/// body does turns that fast (a skater's spins are a few turns a second); an engine putting its model somewhere new
/// (a respawn, a wall plant turning it round at once) does. THUG2, measured (`thug2-engine/tests/placements.rs`): a
/// smooth step turns the root at most 60 degrees, a jump 100 to 180.
pub(crate) const JUMP_TURN_RATE: f32 = std::f32::consts::FRAC_PI_2 * 60.0;

/// The same for a joint: 150 degrees in a 60 Hz step. Animations snap joints further than bodies turn: THUG2's ollie
/// pops a joint by up to 116 degrees in a step and a bail's joints jump about 135, all blended ([`blend_joint`] turns
/// them about their pivots); its mirrored animations and turn-rounds jump joints 163 to 180.
pub(crate) const JOINT_JUMP_TURN_RATE: f32 = std::f32::consts::PI * 150.0 / 180.0 * 60.0;

/// Whether the step from `previous` to `next` (a step of `period`) is a jump the engine didn't mark as a cut: the
/// root mirrored or unmirrored, the root turned faster than [`JUMP_TURN_RATE`], or any joint faster than
/// [`JOINT_JUMP_TURN_RATE`]. A step too long to tell (a turn past half a turn reads as one the other way round) or of
/// no period is checked for the mirror only. The whole pose snaps: a joint snapped alone would tear the model from
/// its neighbours.
fn jumped(previous: &Snapshot, next: &Snapshot, period: Duration) -> bool {
    if separable(previous.root_matrix) != separable(next.root_matrix) {
        return true;
    }
    let turned = |a: Mat4, b: Mat4, rate: f32| {
        let limit = rate * period.as_secs_f32();
        limit > 0.0 && limit < std::f32::consts::PI && separable(a).is_some() && separable(b).is_some() && turn(a, b) > limit
    };
    turned(previous.root_matrix, next.root_matrix, JUMP_TURN_RATE)
        || previous.joints.iter().zip(&next.joints).any(|(&a, &b)| turned(a, b, JOINT_JUMP_TURN_RATE))
}

/// The angle of the turn from one matrix's rotation to another's (radians, 0..pi), each taken apart as [`blend`]
/// takes it.
fn turn(a: Mat4, b: Mat4) -> f32 {
    Transform::from_matrix(a).rotation.normalize().angle_between(Transform::from_matrix(b).rotation.normalize())
}

/// The running engine's models' histories, one a model (by [`ModelId`]: every engine has one so far, id 0). A model
/// left out of a step is forgotten when the next step's poses come (a step: the poses with one [`Step`] time).
pub(crate) struct PoseHistories {
    /// Each model's history, and its latest pose's step time.
    models: Vec<(ModelId, PoseHistory, Duration)>,
    /// The latest step's time.
    latest: Option<Duration>,
}

impl Default for PoseHistories {
    fn default() -> Self {
        Self { models: Vec::new(), latest: None }
    }
}

impl PoseHistories {
    /// `pose` into its model's history (a pose already taken: false).
    pub(crate) fn offer(&mut self, pose: &Pose) -> bool {
        let at = pose.step.at;
        match self.latest {
            // (a new step: the models the last one left out go; never the one posed now)
            Some(latest) if at > latest => {
                self.models.retain(|(id, _, seen)| *seen >= latest || *id == pose.model);
                self.latest = Some(at);
            }
            // (the clock started again: what came before it goes)
            Some(latest) if at < latest => {
                self.models.clear();
                self.latest = Some(at);
            }
            Some(_) => {}
            None => self.latest = Some(at),
        }
        let i = match self.models.iter().position(|(id, _, _)| *id == pose.model) {
            Some(i) => i,
            None => {
                self.models.push((pose.model, PoseHistory::default(), at));
                self.models.len() - 1
            }
        };
        let (_, history, seen) = &mut self.models[i];
        *seen = at;
        history.offer(pose)
    }

    /// A model's history (none: no pose of it yet, or it was forgotten).
    pub(crate) fn get(&self, model: ModelId) -> Option<&PoseHistory> {
        self.models.iter().find(|(id, _, _)| *id == model).map(|(_, h, _)| h)
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

/// (Before an engine's first pair of poses its live pose was drawn so; the driver now shows a model from its first
/// pair. Kept for the recorded drawings' digests, `model_parity`.)
#[cfg(test)]
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

/// A flat model's root and each of its `count` joints' matrices in the world for one pose (`root`, `joints`; a joint
/// past the pose's: the root's).
#[cfg_attr(not(feature = "thug2"), allow(dead_code))] // (only THUG2 gives meshes as data so far)
pub(crate) fn flat_joints(root: Mat4, joints: &[Mat4], count: usize) -> (Mat4, Vec<Mat4>) {
    (root, (0..count).map(|i| root * joints.get(i).copied().unwrap_or(Mat4::IDENTITY)).collect())
}

/// A model with no skeleton's shape (no `parents`), `alpha` of the way between two poses: its root blended as every
/// model's is ([`blend`]), and each joint's matrix in the model's space blended on its own, turned about the axis its
/// step turns it about ([`blend_joint`]); then,
/// as [`flat_joints`], the root and each of `count` joints in the world. At 0 and 1, the two poses exactly.
#[cfg_attr(not(feature = "thug2"), allow(dead_code))] // (only THUG2 gives meshes as data so far)
pub(crate) fn flat_blend(previous: &Snapshot, current: &Snapshot, alpha: f32, count: usize) -> (Mat4, Vec<Mat4>) {
    if alpha <= 0.0 {
        return flat_joints(previous.root_matrix, &previous.joints, count);
    }
    if alpha >= 1.0 {
        return flat_joints(current.root_matrix, &current.joints, count);
    }
    // (the root as every model's, `blend`, guarded as a joint is: a root that turns mirrored between two steps snaps)
    let root = blend_matrix(previous.root_matrix, current.root_matrix, alpha);
    let at = |joints: &[Mat4], i: usize| joints.get(i).copied().unwrap_or(Mat4::IDENTITY);
    let joints = (0..count).map(|i| root * blend_joint(at(&previous.joints, i), at(&current.joints, i), alpha)).collect();
    (root, joints)
}

/// A matrix `alpha` of the way from `a` to `b`, each taken apart into a move, a turn and a scale ([`blend`]: lerp,
/// slerp the short way, lerp). Two that won't come apart into the same kind (one mirrored and the other not, or either
/// flat or not finite: [`separable`]) aren't blended: the nearer one is drawn. At 0 and 1, `a` and `b` exactly.
pub(crate) fn blend_matrix(a: Mat4, b: Mat4, alpha: f32) -> Mat4 {
    if alpha <= 0.0 {
        return a;
    }
    if alpha >= 1.0 {
        return b;
    }
    match (separable(a), separable(b)) {
        (Some(sa), Some(sb)) if sa == sb => blend(Transform::from_matrix(a), Transform::from_matrix(b), alpha).to_matrix(),
        _ if alpha < 0.5 => a,
        _ => b,
    }
}

/// A joint's matrix in its model's space `alpha` of the way from `a` to `b`: as [`blend_matrix`], but moved along
/// the step's screw. The step from `a` to `b` is a turn about some axis and a move along it, and the joint goes that
/// part of the turn about that axis: a bone turning about its joint stays on its arc about the joint, however far it
/// turns. Taken apart about the model's origin instead (moved along a line), a bone away from the origin is pulled
/// off its arc towards the chord between its two places: by about its distance from the origin times turn² / 8,
/// and at a half turn by its distance times 1.4 (the limbs thrown about). Small turns come out as [`blend_matrix`]'s.
/// At 0 and 1, `a` and `b` exactly.
pub(crate) fn blend_joint(a: Mat4, b: Mat4, alpha: f32) -> Mat4 {
    if alpha <= 0.0 {
        return a;
    }
    if alpha >= 1.0 {
        return b;
    }
    match (separable(a), separable(b)) {
        (Some(sa), Some(sb)) if sa == sb => {
            let (ta, tb) = (Transform::from_matrix(a), Transform::from_matrix(b));
            let (rotation, translation) = screw(ta.rotation.normalize(), ta.translation, tb.rotation.normalize(), tb.translation, alpha);
            Mat4::from_scale_rotation_translation(ta.scale.lerp(tb.scale, alpha), rotation, translation)
        }
        _ if alpha < 0.5 => a,
        _ => b,
    }
}

/// `alpha` of the rigid step from (`qa`, `ta`) to (`qb`, `tb`) along its screw - its turn the short way, about its
/// axis, and its move along that axis - applied to (`qa`, `ta`).
fn screw(qa: Quat, ta: Vec3, qb: Quat, tb: Vec3, alpha: f32) -> (Quat, Vec3) {
    let mut d = (qb * qa.inverse()).normalize();
    if d.w < 0.0 {
        d = -d;
    }
    // (the sine of half the turn; next to none: a move along a line)
    let half_sin = d.xyz().length();
    if half_sin < 1e-6 {
        return (qa.slerp(qb, alpha), ta.lerp(tb, alpha));
    }
    let axis = d.xyz() / half_sin;
    let angle = 2.0 * half_sin.atan2(d.w);
    // The step's own move (b = d a + step): along the axis, and across it (the turn is about an axis off the origin).
    let step = tb - d * ta;
    let along = axis * axis.dot(step);
    let across = step - along;
    // (across the axis, the part turn about the axis' point: (1 - e^(i alpha angle)) / (1 - e^(i angle)) of the move
    // across - the chords' ratio, turned back by half the turn not yet made)
    let ratio = (alpha * angle / 2.0).sin() / half_sin;
    let moved = Quat::from_axis_angle(axis, (alpha - 1.0) * angle / 2.0) * (across * ratio) + along * alpha;
    let part = Quat::from_axis_angle(axis, alpha * angle);
    ((part * qa).normalize(), part * ta + moved)
}

/// Whether a matrix comes apart into a move, a turn and a scale, and if so whether it is mirrored (a negative
/// determinant: taken apart as a negative scale along x). Not: a w row other than (0, 0, 0, 1), an axis too short to
/// turn (under 1e-6 of the longest), or anything not finite.
pub(crate) fn separable(m: Mat4) -> Option<bool> {
    let lengths = [m.x_axis.truncate().length(), m.y_axis.truncate().length(), m.z_axis.truncate().length()];
    let longest = lengths.iter().copied().fold(0.0, f32::max);
    let det = m.determinant();
    let affine = m.row(3) == Vec4::W;
    (affine && det.is_finite() && m.is_finite() && longest > 0.0 && lengths.iter().all(|&l| l > longest * 1e-6) && det != 0.0)
        .then_some(det < 0.0)
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
            ..Default::default()
        }
    }

    fn timed(mut pose: Pose, at_ms: u64, period: Duration) -> Pose {
        pose.step = Step { at: Duration::from_millis(at_ms), period };
        pose
    }

    /// A pose is taken once; a cut, a new step period or a new joint count isn't blended from the one before.
    #[test]
    fn a_cut_or_a_new_period_starts_the_pair_again() {
        let step = Duration::from_millis(16);
        let mut h = PoseHistory::default();
        assert!(h.view(0.5).is_none());
        assert!(h.offer(&timed(pose(1, 0.0, 2, false), 0, step)));
        assert!(!h.offer(&timed(pose(1, 5.0, 2, false), 0, step)), "taken once");
        h.offer(&timed(pose(2, 1.0, 2, false), 0, step));
        let (a, b, t) = h.view(2.0).unwrap();
        assert_eq!((a.root.translation.x, b.root.translation.x, t), (0.0, 1.0, 1.0));
        for (next, joints, cut, period) in [(3, 2, true, step), (4, 2, false, step * 2), (5, 3, false, step * 2)] {
            h.offer(&timed(pose(next, next as f32, joints, cut), 0, period));
            let (a, b) = h.pair().unwrap();
            assert_eq!(a.root.translation.x, b.root.translation.x, "pose {next}: not blended");
        }
    }

    /// A turned, moved, scaled matrix (`mirror`: x flipped).
    fn trs(x: f32, turn: f32, scale: f32, mirror: bool) -> Mat4 {
        let s = Vec3::new(if mirror { -scale } else { scale }, scale, scale);
        Mat4::from_scale_rotation_translation(s, Quat::from_rotation_y(turn), Vec3::new(x, 0.0, 0.0))
    }

    fn close(a: Mat4, b: Mat4) -> bool {
        a.abs_diff_eq(b, 1e-5)
    }

    /// Two poses of a model with no skeleton's shape, a step apart: a flat model's history.
    fn flat_pair(a: &Pose, b: &Pose) -> PoseHistory {
        let mut h = PoseHistory::default();
        h.offer(a);
        h.offer(b);
        h
    }

    fn flat(generation: u64, at_ms: u64, root: Mat4, joints: Vec<Mat4>) -> Pose {
        Pose { generation, root, joints, step: Step { at: Duration::from_millis(at_ms), period: Duration::from_millis(10) }, ..Default::default() }
    }

    /// As [`flat`], a step of 40 ms: too long for a turn to tell a jump by ([`JUMP_TURN_RATE`]), so any turn blends.
    fn slow(generation: u64, at_ms: u64, root: Mat4, joints: Vec<Mat4>) -> Pose {
        let mut pose = flat(generation, at_ms, root, joints);
        pose.step.period = Duration::from_millis(40);
        pose
    }

    /// Halfway between two matrices: half the move, half the turn, half the scale; the root the same way.
    #[test]
    fn a_flat_joint_is_blended_by_its_move_turn_and_scale() {
        let a = slow(1, 10, Mat4::IDENTITY, vec![trs(0.0, 0.0, 1.0, false)]);
        let b = slow(2, 20, trs(4.0, 0.0, 1.0, false), vec![trs(2.0, std::f32::consts::FRAC_PI_2, 3.0, false)]);
        let h = flat_pair(&a, &b);
        // (20 ms after the newer pose's step, a 40 ms period: halfway)
        let (p, c, alpha) = h.view_at(Duration::from_millis(40)).unwrap();
        assert_eq!(alpha, 0.5);
        let (root, joints) = flat_blend(p, c, alpha, 2);
        assert!(close(root, trs(2.0, 0.0, 1.0, false)), "{root}");
        // (the joint: half its turn and its scale; its move along its turn's screw, `blend_joint`)
        let joint = Transform::from_matrix(root.inverse() * joints[0]);
        assert!(joint.rotation.abs_diff_eq(Quat::from_rotation_y(std::f32::consts::FRAC_PI_4), 1e-5), "{joint:?}");
        assert!(joint.scale.abs_diff_eq(Vec3::splat(2.0), 1e-5));
        assert!(close(joints[0], root * blend_joint(a.joints[0], b.joints[0], 0.5)));
        assert!(close(joints[1], root), "a joint past the pose's is the root's");
    }

    /// At 0 and 1, exactly the two poses as drawn unblended (`flat_joints`); the clock past a period stays at the newer.
    #[test]
    fn a_flat_blend_at_its_ends_is_each_pose_exactly() {
        let a = slow(1, 10, trs(0.3, 0.2, 0.9, false), vec![trs(1.0, 0.7, 1.1, false), trs(0.1, -2.0, 0.5, true)]);
        let b = slow(2, 20, trs(0.7, 1.2, 1.0, false), vec![trs(1.5, 2.9, 1.3, false), trs(0.2, 2.0, 0.6, true)]);
        let h = flat_pair(&a, &b);
        let (p, c) = h.pair().unwrap();
        let bits = |(r, j): (Mat4, Vec<Mat4>)| (r.to_cols_array().map(f32::to_bits), j.iter().map(|m| m.to_cols_array().map(f32::to_bits)).collect::<Vec<_>>());
        assert_eq!(bits(flat_blend(p, c, 0.0, 2)), bits(flat_joints(a.root, &a.joints, 2)));
        assert_eq!(bits(flat_blend(p, c, 1.0, 2)), bits(flat_joints(b.root, &b.joints, 2)));
        assert_eq!(h.view_at(Duration::from_millis(20)).unwrap().2, 0.0);
        assert_eq!(h.view_at(Duration::from_millis(75)).unwrap().2, 1.0);
        assert_eq!(h.view_at(Duration::from_millis(5)).unwrap().2, 0.0, "a clock behind the step: the older pose");
    }

    /// The turn goes the short way: from 170 degrees to -170, through 180, not through 0.
    #[test]
    fn a_flat_turn_goes_the_short_way() {
        let (a, b) = (trs(0.0, 170f32.to_radians(), 1.0, false), trs(0.0, -170f32.to_radians(), 1.0, false));
        assert!(close(blend_matrix(a, b, 0.5), trs(0.0, std::f32::consts::PI, 1.0, false)), "{}", blend_matrix(a, b, 0.5));
        // (and when the two quaternions come out on opposite sides)
        let q = Quat::from_rotation_y(0.1);
        let (a, b) = (Mat4::from_quat(q), Mat4::from_quat(-Quat::from_rotation_y(0.3)));
        assert!(close(blend_matrix(a, b, 0.5), Mat4::from_rotation_y(0.2)));
    }

    /// Mirrored matrices blend with mirrored ones; a mirrored and an unmirrored one, or a flat or broken one, aren't
    /// blended: the nearer is drawn (a root mirrored or unmirrored between two steps: the whole newer pose).
    #[test]
    fn a_mirror_or_a_flat_matrix_is_not_blended_through() {
        let (a, b) = (trs(0.0, 0.0, 1.0, true), trs(2.0, 1.0, 3.0, true));
        let half = blend_matrix(a, b, 0.5);
        assert!(close(half, trs(1.0, 0.5, 2.0, true)), "{half}");
        assert!(half.determinant() < 0.0);
        let plain = trs(2.0, 1.0, 1.0, false);
        assert_eq!(blend_matrix(a, plain, 0.4), a);
        assert_eq!(blend_matrix(a, plain, 0.6), plain);
        let squashed = Mat4::from_scale(Vec3::new(1.0, 0.0, 1.0));
        assert_eq!(separable(squashed), None);
        assert_eq!(blend_matrix(squashed, plain, 0.3), squashed);
        let mut broken = plain;
        broken.w_axis.x = f32::NAN;
        assert_eq!(separable(broken), None);
        assert_eq!(blend_matrix(plain, broken, 0.3), plain);
        let mut projective = plain;
        projective.x_axis.w = 0.5;
        assert_eq!(separable(projective), None);
        assert_eq!((separable(plain), separable(a)), (Some(false), Some(true)));
        // (a root turning mirrored between two steps: a jump - the whole newer pose, its joints carried by its root)
        let p = slow(1, 10, a, vec![Mat4::IDENTITY]);
        let q = slow(2, 20, plain, vec![Mat4::IDENTITY]);
        let h = flat_pair(&p, &q);
        let (prev, cur) = h.pair().unwrap();
        assert_eq!(flat_blend(prev, cur, 0.3, 1), (plain, vec![plain]));
        assert_eq!(flat_blend(prev, cur, 0.7, 1), (plain, vec![plain]));
        // (the same handedness: the root blended exactly as `blend` blends every model's root)
        let q = slow(2, 20, trs(2.0, 1.0, 3.0, true), vec![Mat4::IDENTITY]);
        let h = flat_pair(&p, &q);
        let (prev, cur) = h.pair().unwrap();
        assert_eq!(flat_blend(prev, cur, 0.3, 1).0, blend(prev.root, cur.root, 0.3).to_matrix());
    }

    /// A cut, a new period or a new joint count: drawn as the newer pose whatever the clock says.
    #[test]
    fn a_flat_model_snaps_on_a_cut_a_new_period_or_a_new_joint_count() {
        let a = flat(1, 10, Mat4::IDENTITY, vec![trs(0.0, 0.0, 1.0, false)]);
        let mut cut = flat(2, 20, trs(5.0, 1.0, 1.0, false), vec![trs(1.0, 1.0, 1.0, false)]);
        cut.discontinuity = true;
        let mut slower = cut.clone();
        (slower.discontinuity, slower.step.period) = (false, Duration::from_millis(20));
        let mut more = cut.clone();
        (more.discontinuity, more.joints) = (false, vec![trs(1.0, 1.0, 1.0, false); 2]);
        for b in [cut, slower, more] {
            let h = flat_pair(&a, &b);
            let (p, c, alpha) = h.view_at(Duration::from_millis(23)).unwrap();
            assert!(alpha > 0.0 && alpha < 1.0);
            let (root, joints) = flat_blend(p, c, alpha, 1);
            let (r, j) = flat_joints(b.root, &b.joints, 1);
            assert!(close(root, r) && close(joints[0], j[0]), "{root} {}", joints[0]);
        }
    }

    /// A bone turning about its joint, away from the model's origin, stays on its arc about the joint between steps
    /// (taken apart about the origin, half a turn would pull it onto the joint's far side's chord); small turns come
    /// out as `blend_matrix`'s; the move along the turn's axis goes along with it.
    #[test]
    fn a_flat_joint_turns_about_its_own_joint() {
        let joint = Vec3::new(0.4, 1.0, 0.0);
        let about = |angle: f32| Mat4::from_translation(joint) * Mat4::from_rotation_z(angle) * Mat4::from_translation(-joint);
        for angle in [0.3f32, 1.5, 3.0] {
            let (a, b) = (about(0.0), about(angle));
            for alpha in [0.25, 0.5, 0.75] {
                let m = blend_joint(a, b, alpha);
                assert!(close(m, about(angle * alpha)), "{angle} {alpha}: {m}");
                assert!(m.transform_point3(joint).distance(joint) < 1e-5);
            }
        }
        // (the old way: the joint itself swung off by most of its distance from the origin at a near half turn)
        assert!(blend_matrix(about(0.0), about(3.0), 0.5).transform_point3(joint).distance(joint) > 0.8);
        // (a small turn, 3 degrees with a 10 cm move: as taken apart, to under a millimetre - the arc's sagitta)
        let (a, b) = (trs(0.2, 0.1, 1.0, false), trs(0.3, 0.15, 1.02, false));
        assert!(blend_joint(a, b, 0.5).abs_diff_eq(blend_matrix(a, b, 0.5), 1e-3));
        // (a screw: half the turn about the axis and half the move along it; mirrored ones alike)
        let screwed = |t: f32| Mat4::from_translation(Vec3::Z * t) * about(t);
        assert!(close(blend_joint(screwed(0.0), screwed(2.0), 0.5), screwed(1.0)));
        let mirror = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
        assert!(close(blend_joint(about(0.0) * mirror, about(2.0) * mirror, 0.5), about(1.0) * mirror));
        // (the ends exactly; the short way round; a mirror coming or going: the nearer)
        assert_eq!(blend_joint(a, b, 0.0), a);
        assert_eq!(blend_joint(a, b, 1.0), b);
        assert!(close(blend_joint(about(-3.0), about(3.0), 0.5), about(std::f32::consts::PI)));
        assert_eq!(blend_joint(a, b * mirror, 0.4), a);
    }

    /// The root turning more than a quarter turn in a 60 Hz step, a joint more than 150 degrees, or the root mirrored,
    /// snaps the whole pose though the engine didn't mark a cut; spins and flips (30 degrees a step, 5 turns a second)
    /// and an ollie's joint pop blend; a step too long to tell blends whatever it turned.
    #[test]
    fn a_jump_the_engine_did_not_mark_snaps() {
        let period = Duration::from_micros(16_667);
        let turned = |root: f32, joint: f32| flat(2, 33, Mat4::from_rotation_y(root), vec![Mat4::IDENTITY, Mat4::from_rotation_x(joint)]);
        let snaps = |b: &Pose, period: Duration| {
            let mut a = turned(0.0, 0.0);
            let mut b = b.clone();
            (a.generation, a.step.at) = (1, Duration::from_millis(16));
            (a.step.period, b.step.period) = (period, period);
            let h = flat_pair(&a, &b);
            let (p, c) = h.pair().unwrap();
            p.root_matrix == c.root_matrix && p.joints == c.joints
        };
        let quarter = std::f32::consts::FRAC_PI_2;
        assert!(snaps(&turned(quarter * 1.1, 0.0), period), "the root turned round");
        assert!(snaps(&turned(0.0, 160f32.to_radians()), period), "a joint snapped (an animation mirrored)");
        assert!(!snaps(&turned(0.0, 120f32.to_radians()), period), "an ollie's pop: a joint's fast turn, blended");
        assert!(!snaps(&turned(30f32.to_radians(), 30f32.to_radians()), period), "a spin and a flip");
        assert!(!snaps(&turned(quarter * 0.95, 0.0), period));
        assert!(!snaps(&turned(quarter * 1.5, quarter * 1.5), period * 2), "too long a step to tell");
        assert!(!snaps(&turned(quarter * 1.5, 0.0), Duration::ZERO), "no period: can't tell");
        let mut mirrored = turned(0.1, 0.0);
        mirrored.root *= Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
        assert!(snaps(&mirrored, period * 2), "mirrored");
    }

    /// A pose given again (the same generation) isn't taken: the pair and the clock's start stay as they were.
    #[test]
    fn a_pose_given_again_does_not_move_the_pair_on() {
        let a = flat(1, 10, Mat4::IDENTITY, vec![trs(0.0, 0.0, 1.0, false)]);
        let b = flat(2, 20, trs(2.0, 0.0, 1.0, false), vec![trs(2.0, 0.0, 1.0, false)]);
        let mut h = flat_pair(&a, &b);
        let mut again = flat(2, 30, trs(9.0, 0.0, 1.0, false), vec![trs(9.0, 0.0, 1.0, false)]);
        again.discontinuity = true;
        assert!(!h.offer(&again));
        let (p, c, alpha) = h.view_at(Duration::from_millis(25)).unwrap();
        assert_eq!((p.root_matrix, c.root_matrix, alpha), (a.root, b.root, 0.5));
    }

    /// One history a model; a model a step leaves out is forgotten when the next step comes; the one posed is kept.
    #[test]
    fn each_model_has_its_own_history_and_a_model_left_out_is_dropped() {
        let mut h = PoseHistories::default();
        let of = |model: ModelId, generation: u64, at: u64| Pose { model, ..flat(generation, at, Mat4::IDENTITY, vec![]) };
        assert!(h.offer(&of(0, 1, 10)));
        assert!(h.offer(&of(1, 1, 10)), "model 1's own generation 1");
        assert!(h.offer(&of(0, 2, 20)));
        assert!(h.get(1).is_some(), "left out of step 20 only once step 30 comes");
        assert!(h.offer(&of(0, 3, 30)));
        assert!(h.get(1).is_none());
        assert!(h.get(0).is_some());
        // (a model posed alone, skipping steps, keeps its history)
        assert!(h.offer(&of(0, 4, 60)));
        assert_eq!(h.get(0).unwrap().pair().unwrap().0.joints.len(), 0);
        assert!(!h.offer(&of(0, 4, 70)), "taken once");
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
