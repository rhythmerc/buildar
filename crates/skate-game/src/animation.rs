//! Render the same stock skeleton that animation and physics update.
//! The GLB supplies the mesh, skin weights and hierarchy; it does not play a
//! separate idle clip or own the skater's pose.
use crate::{app::FrameSet, assets::AssetStatus, physics::SkaterRuntime, world::PlayerRoot};
use bevy::{camera::visibility::NoFrustumCulling, mesh::skinning::SkinnedMesh, prelude::*};

#[derive(Resource, Default)]
pub(crate) struct AnimationStatus {
    pub ready: bool,
    pub(crate) bindings: Vec<BoneBinding>,
}
pub(crate) struct BoneBinding {
    pub(crate) entity: Entity,
    pub(crate) bone: usize,
    pub(crate) parent_bone: Option<usize>,
}
impl AnimationStatus {
    pub(crate) fn pose_transforms(&self, pose: &[Mat4]) -> Vec<(Entity, Transform)> {
        self.bindings
            .iter()
            .filter_map(|b| {
                let global = *pose.get(b.bone)? * render_basis();
                let local = if let Some(parent) = b.parent_bone {
                    (*pose.get(parent)? * render_basis()).inverse() * global
                } else {
                    global
                };
                Some((b.entity, Transform::from_matrix(local)))
            })
            .collect()
    }
}
pub(crate) struct AnimationPlugin;
impl Plugin for AnimationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AnimationStatus>().add_systems(
            Update,
            (disable_skater_culling, bind, present)
                .chain()
                .in_set(FrameSet::Animation),
        );
    }
}
fn disable_skater_culling(
    mut commands: Commands,
    skins: Query<Entity, (With<SkinnedMesh>, Without<NoFrustumCulling>)>,
    parents: Query<&ChildOf>,
    roots: Query<Entity, With<PlayerRoot>>,
) {
    // GLB primitive bounds describe the bind pose, not the animated skin.
    // Apply to every skater mesh (not just the first skin used to bind bones).
    // World meshes retain their normal culling behavior.
    for entity in &skins {
        if parents.iter_ancestors(entity).any(|e| roots.contains(e)) {
            commands.entity(entity).insert(NoFrustumCulling);
        }
    }
}

fn bind(
    status: Res<AssetStatus>,
    skater: Res<SkaterRuntime>,
    mut animation: ResMut<AnimationStatus>,
    skins: Query<(Entity, &SkinnedMesh)>,
    nodes: Query<(&Name, &Transform)>,
    parents: Query<&ChildOf>,
    visibility: Query<&Visibility>,
    roots: Query<Entity, With<PlayerRoot>>,
    mut exit: MessageWriter<AppExit>,
) {
    if *status != AssetStatus::Ready || animation.ready {
        return;
    }
    let names = &skater.animation.evaluator.frames.bone_names;
    let skins = skins.iter().filter_map(|(entity, skin)| {
        if !parents.iter_ancestors(entity).any(|e| roots.contains(e)) {
            return None;
        }
        if parents
            .iter_ancestors(entity)
            .any(|e| visibility.get(e).is_ok_and(|v| *v == Visibility::Hidden))
        {
            return None;
        }
        Some(skin.joints.as_slice())
    });
    let result = bind_skins(
        names,
        skins,
        |e| nodes.get(e).ok().map(|(name, transform)| (name.as_str().to_owned(), transform.to_matrix())),
        |e| parents.iter_ancestors(e).collect(),
        |e| roots.contains(e),
    )
    .map(|bindings| if bindings.is_empty() { None } else { Some(bindings) });
    match result {
        Ok(Some(binding)) => {
            animation.bindings = binding;
            animation.ready = true;
            info!(
                "GAME_CHARACTER_READY bones={} source=physical_stock_pose",
                animation.bindings.len()
            );
        }
        Ok(None) => (),
        Err(message) => {
            error!("{message}");
            exit.write(AppExit::error());
        }
    }
}
/// Each visible skin's joints bound to the stock rig's bones by name (`names`), with the nearest named ancestor as
/// the parent bone. `node`: a node's name and local matrix; `ancestors`: a node's ancestors, nearest first;
/// `is_root`: the model's root (the search stops there).
pub(crate) fn bind_skins<'a>(
    names: &[String],
    skins: impl IntoIterator<Item = &'a [Entity]>,
    node: impl Fn(Entity) -> Option<(String, Mat4)>,
    ancestors: impl Fn(Entity) -> Vec<Entity>,
    is_root: impl Fn(Entity) -> bool,
) -> Result<Vec<BoneBinding>, String> {
    let mut bindings = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for skin in skins {
        let mut binding = Vec::with_capacity(skin.len());
        for &joint in skin {
            if !seen.insert(joint) {
                continue;
            }
            let (name, _) = node(joint).ok_or("Skater skin joint is missing its name or transform")?;
            let bone = names
                .iter()
                .position(|n| n.eq_ignore_ascii_case(name.as_str()))
                .ok_or_else(|| format!("Skater skin bone {name} is absent from the stock rig"))?;
            let mut parent_bone = None;
            for parent in ancestors(joint) {
                if is_root(parent) {
                    break;
                }
                if let Some((name, transform)) = node(parent) {
                    if let Some(index) = names.iter().position(|n| n.eq_ignore_ascii_case(name.as_str())) {
                        parent_bone = Some(index);
                        break;
                    }
                    // The supplied GLB's armature node is identity. Reject
                    // another import convention rather than double a pose.
                    if !transform.abs_diff_eq(Mat4::IDENTITY, 0.00001) {
                        return Err(format!("Skater armature ancestor {name} has an unsupported transform"));
                    }
                }
            }
            binding.push(BoneBinding { entity: joint, bone, parent_bone });
        }
        // Bind each visible modular rig, retaining its authored inverse binds.
        bindings.extend(binding);
    }
    Ok(bindings)
}

/// Each binding's bone-local transform between two snapshots: bone-local rotations blended, not matrix entries or
/// independent world positions, so joints retain their hierarchy while limbs turn.
pub(crate) fn blended_locals(
    bindings: &[BoneBinding],
    previous: &crate::presentation::Snapshot,
    current: &crate::presentation::Snapshot,
    alpha: f32,
) -> Vec<Transform> {
    let basis = render_basis();
    bindings
        .iter()
        .map(|binding| {
            let local = |snapshot: &crate::presentation::Snapshot| {
                let global = snapshot.bones[binding.bone] * basis;
                Transform::from_matrix(if let Some(parent) = binding.parent_bone {
                    (snapshot.bones[parent] * basis).inverse() * global
                } else {
                    global
                })
            };
            crate::presentation::blend(local(previous), local(current), alpha)
        })
        .collect()
}

fn present(
    history: Res<crate::presentation::Presentation>,
    skater: Res<SkaterRuntime>,
    replay: Res<crate::replay::Replay>,
    time: Res<Time<Fixed>>,
    hold: Res<crate::app::Hold>,
    animation: Res<AnimationStatus>,
    mut nodes: Query<&mut Transform>,
) {
    if !animation.ready {
        return;
    }
    let Some((previous, current, alpha)) = history.view(&replay, hold.alpha(&time)) else {
        let pose: Vec<_> = skater
            .render_pose
            .iter()
            .copied()
            .map(native_matrix)
            .collect();
        for (entity, transform) in animation.pose_transforms(&pose) {
            if let Ok(mut node) = nodes.get_mut(entity) {
                *node = transform;
            }
        }
        return;
    };
    for (binding, transform) in animation.bindings.iter().zip(blended_locals(&animation.bindings, previous, current, alpha)) {
        if let Ok(mut node) = nodes.get_mut(binding.entity) {
            *node = transform;
        }
    }
}
/// Existing GLB was exported through Blender: its bone-local axes are
/// rotated -90 degrees about X relative to the native frames. Both files'
/// world positions are Y-up. This is a skin basis change, not a physics turn.
pub(crate) fn render_basis() -> Mat4 {
    Mat4::from_cols(Vec4::X, -Vec4::Z, Vec4::Y, Vec4::W)
}
pub(crate) use skate3_sim::animation::native_matrix;
