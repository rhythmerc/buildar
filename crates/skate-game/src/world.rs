use crate::assets::AssetManifest;
use bevy::prelude::*;

/// World transform owner. Animation only writes descendant bone transforms.
#[derive(Component)]
pub(crate) struct PlayerRoot;

/// Skate 3's light. The room light (`xr::room_light`) fades the sun and the ambient, so they stay as they are.
const CLEAR: Color = Color::srgb(0.065, 0.08, 0.10);
const AMBIENT: f32 = 350.;
const SUN_LUX: f32 = 11000.;

fn ambient() -> GlobalAmbientLight {
    GlobalAmbientLight {
        color: Color::WHITE,
        brightness: AMBIENT,
        ..default()
    }
}

pub(crate) struct WorldPlugin;
impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(CLEAR))
            .insert_resource(ambient())
            .add_systems(Startup, spawn);
    }
}

fn spawn(world: &mut World) {
    let scene = {
        let server = world.resource::<AssetServer>();
        let manifest = world.resource::<AssetManifest>();
        server.load(GltfAssetLabel::Scene(0).from_asset(manifest.0.character_scene.clone()))
    };
    world
        .spawn((PlayerRoot, Transform::default(), Visibility::default()))
        .with_children(|parent| {
            parent.spawn(SceneRoot(scene));
        });
    // AR passthrough: the real room is the floor and the player builds the course (`xr::build`), so none of the
    // test course is drawn there.
    let hide_floor = world
        .get_resource::<crate::config::Config>()
        .is_some_and(|config| config.xr.is_some());
    spawn_course(world, hide_floor);
}

/// The test course (there are no maps): its light, and on the desktop its surfaces.
fn spawn_course(world: &mut World, hide_floor: bool) {
    world.insert_resource(ClearColor(CLEAR));
    world.insert_resource(ambient());
    let colors = [
        Color::srgb(0.16, 0.19, 0.21),
        Color::srgb(0.48, 0.35, 0.22),
        Color::srgb(0.30, 0.43, 0.48),
        Color::srgb(0.24, 0.48, 0.31),
    ];
    let groups = crate::physics::ground::surfaces().into_iter().zip(colors);
    for (quads, color) in groups.take(if hide_floor { 0 } else { 4 }) {
        let positions: Vec<[f32; 3]> = quads
            .into_iter()
            .flat_map(|vertices| {
                [0, 2, 1, 0, 3, 2].map(|i| {
                    let v = vertices[i];
                    [v.x, v.y, v.z]
                })
            })
            .collect();
        let mut mesh = Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.compute_flat_normals();
        let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
        let material = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
            base_color: color,
            perceptual_roughness: 0.9,
            double_sided: true,
            cull_mode: None,
            ..default()
        });
        world.spawn((Mesh3d(mesh), MeshMaterial3d(material), Transform::default()));
    }
    world.spawn((
        DirectionalLight {
            illuminance: SUN_LUX,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(4., 7., 4.).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn course(hide_floor: bool) -> World {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        spawn_course(&mut world, hide_floor);
        world
    }

    /// The headset's course is one light, as it was with the map code: 11,000 lux with shadows from (4, 7, 4) at the
    /// origin, ambient 350 and the same clear colour; and no floor. The desktop also draws the four surfaces.
    #[test]
    fn the_ar_course_keeps_its_light_and_draws_no_floor() {
        let mut world = course(true);
        let lights: Vec<_> = world
            .query::<(&DirectionalLight, &Transform)>()
            .iter(&world)
            .map(|(l, t)| (l.illuminance, l.shadows_enabled, l.color, *t))
            .collect();
        assert_eq!(
            lights,
            [(11000., true, Color::WHITE, Transform::from_xyz(4., 7., 4.).looking_at(Vec3::ZERO, Vec3::Y))]
        );
        assert_eq!(world.resource::<GlobalAmbientLight>().brightness, 350.);
        assert_eq!(world.resource::<GlobalAmbientLight>().color, Color::WHITE);
        assert_eq!(world.resource::<ClearColor>().0, Color::srgb(0.065, 0.08, 0.10));
        assert_eq!(world.query::<&Mesh3d>().iter(&world).count(), 0);

        let mut world = course(false);
        assert_eq!(world.query::<&Mesh3d>().iter(&world).count(), 4);
        assert_eq!(world.query::<&DirectionalLight>().iter(&world).count(), 1);
    }
}
