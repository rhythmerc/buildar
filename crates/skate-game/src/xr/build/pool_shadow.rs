//! The rim's shadow inside a pool, from the light the pieces are lit by (the room's light, when it is on): a pool's
//! own mesh drawn again over it with [`PoolShadow`], which traces each point toward the light and darkens it where the
//! line misses the mouth (the mouth as a picture: `pool::mouth_mask`). Each frame only where the pool is and where the
//! light comes from change.
use bevy::asset::embedded_asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;

/// How dark the rim's shadow is.
const DARKNESS: f32 = 0.55;
/// Where the light comes from when no light in the scene says (toward it, world): high, a little to one side.
const DEFAULT_LIGHT: Vec3 = Vec3::new(0.35, 1.0, 0.25);

#[derive(Clone, Copy, Debug, Default, ShaderType)]
struct Params {
    local_from_world: Mat4,
    light: Vec4,
    light_world: Vec4,
    mask_rect: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub(crate) struct PoolShadow {
    #[uniform(0)]
    params: Params,
    #[texture(1)]
    #[sampler(2)]
    mask: Handle<Image>,
}

impl Material for PoolShadow {
    fn fragment_shader() -> ShaderRef {
        concat!("embedded://", env!("CARGO_CRATE_NAME"), "/xr/build/pool_shadow.wgsl").into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }
    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // (the pool's faces are in depth already: drawn over them, as they are)
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_write_enabled = false;
        }
        descriptor.primitive.cull_mode = Some(bevy::render::render_resource::Face::Back);
        Ok(())
    }
}

impl PoolShadow {
    /// A pool's shadow material over the mouth's picture `mask`, which covers `min..max` (x, z) of the pool's space.
    pub(crate) fn new(mask: Handle<Image>, min: Vec2, max: Vec2) -> Self {
        Self { params: Params { mask_rect: Vec4::new(min.x, min.y, max.x - min.x, max.y - min.y), ..default() }, mask }
    }
}

/// A pool's shadow (on the entity drawing it, a child of the pool).
#[derive(Component)]
pub(crate) struct PoolShadowOf;

pub(crate) struct PoolShadowPlugin;

impl Plugin for PoolShadowPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "pool_shadow.wgsl");
        app.add_plugins(MaterialPlugin::<PoolShadow>::default())
            .add_systems(PostUpdate, follow.after(bevy::transform::TransformSystems::Propagate));
    }
}

/// Toward the light the pieces are lit by: the brightest directional light shining, else a default.
fn toward_light<'a>(lights: impl Iterator<Item = (&'a DirectionalLight, &'a GlobalTransform, &'a InheritedVisibility)>) -> Vec3 {
    lights
        .filter(|(l, _, seen)| l.illuminance > 0.0 && seen.get())
        .max_by(|a, b| a.0.illuminance.total_cmp(&b.0.illuminance))
        // (a directional light shines along its -z: toward it is +z)
        .map(|(_, g, _)| g.back().as_vec3())
        .filter(|d| d.y > 0.05)
        .unwrap_or(DEFAULT_LIGHT.normalize())
}

fn follow(
    lights: Query<(&DirectionalLight, &GlobalTransform, &InheritedVisibility)>,
    shadows: Query<(&GlobalTransform, &MeshMaterial3d<PoolShadow>), With<PoolShadowOf>>,
    mut materials: ResMut<Assets<PoolShadow>>,
) {
    if shadows.is_empty() {
        return;
    }
    let toward = toward_light(lights.iter());
    for (global, handle) in &shadows {
        let local_from_world = global.to_matrix().inverse();
        // (in the pool's own space, its scale taken out: only the way matters)
        let light = local_from_world.transform_vector3(toward).normalize_or(Vec3::Y);
        let Some(material) = materials.get(&handle.0) else { continue };
        let p = material.params;
        if p.local_from_world == local_from_world && p.light.truncate() == light {
            continue;
        }
        if let Some(material) = materials.get_mut(&handle.0) {
            material.params.local_from_world = local_from_world;
            material.params.light = light.extend(DARKNESS);
            material.params.light_world = toward.extend(0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No light shining: the default, from high up; the brightest shining one otherwise (toward it).
    #[test]
    fn the_shadow_comes_from_the_brightest_light() {
        let seen = InheritedVisibility::HIDDEN;
        let visible = InheritedVisibility::VISIBLE;
        let dark = DirectionalLight { illuminance: 0.0, ..default() };
        let low = DirectionalLight { illuminance: 100.0, ..default() };
        let bright = DirectionalLight { illuminance: 5000.0, ..default() };
        let down_x = GlobalTransform::from(Transform::IDENTITY.looking_to(Vec3::new(-1.0, -1.0, 0.0), Vec3::Y));
        let down_z = GlobalTransform::from(Transform::IDENTITY.looking_to(Vec3::new(0.0, -1.0, -1.0), Vec3::Y));
        assert!((toward_light([(&dark, &down_x, &visible)].into_iter()) - DEFAULT_LIGHT.normalize()).length() < 1e-5);
        let way = toward_light([(&low, &down_x, &visible), (&bright, &down_z, &visible), (&bright, &down_x, &seen)].into_iter());
        assert!((way - Vec3::new(0.0, 1.0, 1.0).normalize()).length() < 1e-4, "{way}");
    }
}
