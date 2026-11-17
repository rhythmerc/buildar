//! Skate 3's skater drawn by the generic model layer ([`crate::model`]): its model (`skate3_engine::model::model`: the
//! GLB bound to the sim's rig) and each tick's pose, blended between ticks ([`crate::presentation`]). The GLB supplies
//! the mesh, skin weights and hierarchy; it does not play a separate idle clip or own the skater's pose.
use crate::model::{self, JointBinding};
use crate::{app::FrameSet, assets::AssetStatus, physics::SkaterRuntime, world::PlayerRoot};
use bevy::{camera::visibility::NoFrustumCulling, mesh::skinning::SkinnedMesh, prelude::*};
use engine_core::ModelSource;

/// Skate 3's skater model, as its engine describes it.
#[derive(Resource)]
pub(crate) struct Skate3Model(pub engine_core::Model);

impl Skate3Model {
    /// Its GLB, its joints' names and their basis.
    pub(crate) fn gltf(&self) -> (&str, &[String], Mat4) {
        match &self.0.source {
            ModelSource::Gltf { path, joint_names, basis } => (path, joint_names, *basis),
            ModelSource::Meshes { .. } => unreachable!("Skate 3's skater is a GLB"),
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct AnimationStatus {
    pub ready: bool,
    pub(crate) bindings: Vec<JointBinding>,
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

#[allow(clippy::too_many_arguments)]
fn bind(
    status: Res<AssetStatus>,
    skater: Res<Skate3Model>,
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
    let (_, names, _) = skater.gltf();
    // (each visible modular rig, keeping its authored inverse binds)
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
    let result = model::bind_skins(
        names,
        skins,
        |e| nodes.get(e).ok().map(|(name, transform)| (name.as_str().to_owned(), transform.to_matrix())),
        |e| parents.iter_ancestors(e).collect(),
        |e| roots.contains(e),
    );
    match result {
        Ok(bindings) if bindings.is_empty() => (),
        Ok(bindings) => {
            animation.bindings = bindings;
            animation.ready = true;
            info!(
                "GAME_CHARACTER_READY bones={} source=physical_stock_pose",
                animation.bindings.len()
            );
        }
        Err(message) => {
            error!("Skate 3's skater: {message}");
            exit.write(AppExit::error());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn present(
    history: Res<crate::presentation::Presentation>,
    skater: Res<SkaterRuntime>,
    model: Res<Skate3Model>,
    replay: Res<crate::replay::Replay>,
    time: Res<Time<Fixed>>,
    hold: Res<crate::app::Hold>,
    animation: Res<AnimationStatus>,
    mut nodes: Query<&mut Transform>,
    mut live: Local<Vec<Mat4>>,
) {
    if !animation.ready {
        return;
    }
    let (_, _, basis) = model.gltf();
    let Some((previous, current, alpha)) = history.view(&replay, hold.alpha(&time)) else {
        // (no tick taken yet: the sim's pose as it is)
        skate3_engine::model::joints(&skater, &mut live);
        for (entity, transform) in model::joint_transforms(&animation.bindings, &live, basis) {
            if let Ok(mut node) = nodes.get_mut(entity) {
                *node = transform;
            }
        }
        return;
    };
    let locals = model::joint_locals(&animation.bindings, &previous.joints, &current.joints, basis, alpha);
    for (binding, transform) in animation.bindings.iter().zip(locals) {
        if let Ok(mut node) = nodes.get_mut(binding.entity) {
            *node = transform;
        }
    }
}
