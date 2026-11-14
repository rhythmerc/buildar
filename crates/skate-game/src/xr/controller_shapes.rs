//! The controllers hide what is behind them, by their own shapes
//! (`XR_FB_render_model`): the headset's depth image often misses a
//! controller (its dark top above all), so the park showed through it.
//!
//! The runtime has a model of each controller the player holds. Each is
//! loaded once (its shape only), and drawn where the controller is as
//! nothing but depth (`build/occluder.wgsl`: colour and alpha zero, so
//! passthrough shows there): the real controller is seen in front of
//! whatever is behind it. A setting under occlusion, off by default.
//!
//! The shape is not drawn quite where the controller is, but where the
//! passthrough shows it. What the player sees of the room is the camera's
//! picture, and the camera is some 6 cm ahead of the eye: the headset makes
//! the eye's view from it by what it takes the room's distance to be, and
//! it takes a controller (which its depth sensing misses) to be as far as
//! what is behind it. So the controller is seen where the camera's line to
//! it meets the room behind, which is a few degrees further from the middle
//! of the view than it is. Each eye has its own shape, moved to there.
use super::actions::{Grip, Hand};
use super::ar_settings::ArSettings;
use super::rig::Rig;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy_mod_xr::camera::XrCamera;
use bevy_mod_openxr::session::OxrSession;
use openxr::sys;

pub(crate) struct ControllerShapesPlugin;

impl Plugin for ControllerShapesPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<Unseen>::default())
            .init_resource::<Shapes>()
            .add_observer(eye_layers)
            .add_systems(Update, (load.run_if(resource_exists::<OxrSession>), place).chain());
    }
}

/// The extension this needs, asked for when the session is made.
pub(crate) fn enable(exts: &mut openxr::ExtensionSet) {
    exts.fb_render_model = true;
}

/// Drawn as nothing but depth.
#[derive(Asset, TypePath, bevy::render::render_resource::AsBindGroup, Clone, Default)]
struct Unseen {
    #[uniform(0)]
    _unused: Vec4,
}

impl Material for Unseen {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "embedded://skate_game/xr/build/occluder.wgsl".into()
    }
}

/// Each hand's controller's shape, once loaded, and the tries so far.
#[derive(Resource, Default)]
struct Shapes {
    made: [Option<[Entity; 2]>; 2],
    tries: u32,
    wait: u32,
}

/// A controller's shape in the room, as one eye (0 left, 1 right) is to see it.
#[derive(Component)]
struct Shape(Hand, u32);

/// The layers the eyes' shapes are drawn on: each eye's camera draws its own.
const EYE_LAYERS: [usize; 2] = [20, 21];
/// The passthrough camera from its eye, in the head's axes, m (the left
/// one's lens pose as the headset gives it, less the eye's place).
const CAMERA_FROM_EYE: Vec3 = Vec3::new(0.0, -0.017, -0.063);
/// How far the room is taken to be behind a controller when nothing of it
/// is known there, m.
const ROOM_M: f32 = 2.0;

fn eye_layers(add: On<Add, XrCamera>, eyes: Query<&XrCamera>, mut commands: Commands) {
    if let Ok(eye) = eyes.get(add.entity) {
        commands.entity(add.entity).insert(RenderLayers::from_layers(&[0, EYE_LAYERS[(eye.0 as usize).min(1)]]));
    }
}

/// Where the passthrough shows a point: on the eye's line to where the
/// camera's line through the point meets the room (`room_m` from the
/// camera), as far from the eye as the point is.
fn seen_at(point: Vec3, eye: Vec3, camera: Vec3, room_m: f32) -> Vec3 {
    let to_point = point - camera;
    let reach = to_point.length();
    if reach < 1e-4 || room_m <= reach {
        return point;
    }
    let behind = camera + to_point / reach * room_m;
    eye + (behind - eye).normalize() * (point - eye).length()
}

/// How far along a ray a triangle is (none: missed).
fn hit(origin: Vec3, dir: Vec3, [a, b, c]: [Vec3; 3]) -> Option<f32> {
    let (ab, ac) = (b - a, c - a);
    let p = dir.cross(ac);
    let det = ab.dot(p);
    if det.abs() < 1e-9 {
        return None;
    }
    let t = origin - a;
    let u = t.dot(p) / det;
    let q = t.cross(ab);
    let v = dir.dot(q) / det;
    let along = ac.dot(q) / det;
    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && along > 0.0).then_some(along)
}

/// How many times the models are asked for (a second apart: the runtime
/// may not have them when the session starts).
const TRIES: u32 = 20;

fn load(
    session: Res<OxrSession>,
    mut shapes: ResMut<Shapes>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<Unseen>>,
) {
    if shapes.made.iter().all(Option::is_some) || shapes.tries >= TRIES {
        return;
    }
    shapes.wait += 1;
    if shapes.wait % 72 != 1 {
        return;
    }
    shapes.tries += 1;
    for (slot, hand, path) in [(0, Hand::Left, "/model_fb/controller/left"), (1, Hand::Right, "/model_fb/controller/right")] {
        if shapes.made[slot].is_some() {
            continue;
        }
        match model(&session, path).and_then(|bytes| shape(&bytes)) {
            Ok(mesh) => {
                info!("Controller shapes: {path}: {} triangles", mesh.indices().map_or(0, |i| i.len() / 3));
                let (mesh, material) = (meshes.add(mesh), materials.add(Unseen::default()));
                shapes.made[slot] = Some([0, 1].map(|eye| {
                    commands
                        .spawn((
                            Shape(hand, eye),
                            Mesh3d(mesh.clone()),
                            MeshMaterial3d(material.clone()),
                            RenderLayers::layer(EYE_LAYERS[eye as usize]),
                            Visibility::Hidden,
                        ))
                        .id()
                }));
            }
            Err(why) if shapes.tries == TRIES => warn!("Controller shapes: {path}: {why}"),
            Err(_) => {}
        }
    }
}

/// A controller's model as the runtime has it (a binary glTF).
fn model(session: &OxrSession, path: &str) -> Result<Vec<u8>, String> {
    let instance = session.instance();
    let fns = instance.exts().fb_render_model.ok_or("the headset gives no controller models")?;
    let path = instance.string_to_path(path).map_err(|e| e.to_string())?;
    let ok = |result: sys::Result, what: &str| if result.into_raw() >= 0 { Ok(()) } else { Err(format!("{what}: {result:?}")) };
    unsafe {
        // (the paths must have been listed before a model's properties are asked for)
        let mut count = 0;
        ok((fns.enumerate_render_model_paths)(session.as_raw(), 0, &mut count, std::ptr::null_mut()), "listing the models")?;
        let mut paths =
            vec![sys::RenderModelPathInfoFB { ty: sys::RenderModelPathInfoFB::TYPE, next: std::ptr::null_mut(), path: sys::Path::NULL }; count as usize];
        ok((fns.enumerate_render_model_paths)(session.as_raw(), count, &mut count, paths.as_mut_ptr()), "listing the models")?;
        let mut wants = sys::RenderModelCapabilitiesRequestFB {
            ty: sys::RenderModelCapabilitiesRequestFB::TYPE,
            next: std::ptr::null_mut(),
            flags: sys::RenderModelFlagsFB::SUPPORTS_GLTF_2_0_SUBSET_1 | sys::RenderModelFlagsFB::SUPPORTS_GLTF_2_0_SUBSET_2,
        };
        let mut properties = sys::RenderModelPropertiesFB {
            ty: sys::RenderModelPropertiesFB::TYPE,
            next: (&raw mut wants).cast(),
            vendor_id: 0,
            model_name: [0; sys::MAX_RENDER_MODEL_NAME_SIZE_FB],
            model_key: sys::RenderModelKeyFB::NULL,
            model_version: 0,
            flags: sys::RenderModelFlagsFB::EMPTY,
        };
        ok((fns.get_render_model_properties)(session.as_raw(), path, &mut properties), "the model's properties")?;
        if properties.model_key == sys::RenderModelKeyFB::NULL {
            return Err("no model yet".into());
        }
        let info = sys::RenderModelLoadInfoFB { ty: sys::RenderModelLoadInfoFB::TYPE, next: std::ptr::null_mut(), model_key: properties.model_key };
        let mut buffer = sys::RenderModelBufferFB {
            ty: sys::RenderModelBufferFB::TYPE,
            next: std::ptr::null_mut(),
            buffer_capacity_input: 0,
            buffer_count_output: 0,
            buffer: std::ptr::null_mut(),
        };
        ok((fns.load_render_model)(session.as_raw(), &info, &mut buffer), "the model's size")?;
        let mut bytes = vec![0u8; buffer.buffer_count_output as usize];
        buffer.buffer_capacity_input = bytes.len() as u32;
        buffer.buffer = bytes.as_mut_ptr();
        ok((fns.load_render_model)(session.as_raw(), &info, &mut buffer), "the model")?;
        bytes.truncate(buffer.buffer_count_output as usize);
        Ok(bytes)
    }
}

/// A binary glTF's triangles as one mesh, each node's where its transforms
/// put it. (Only the shape: its textures, which Bevy may not read, are
/// never looked at.)
fn shape(bytes: &[u8]) -> Result<Mesh, String> {
    let gltf = gltf::Gltf::from_slice_without_validation(bytes).map_err(|e| e.to_string())?;
    let blob = gltf.blob.as_deref().ok_or("the model has no data")?;
    let (mut positions, mut indices) = (Vec::<[f32; 3]>::new(), Vec::<u32>::new());
    fn walk(node: gltf::Node, parent: Mat4, blob: &[u8], positions: &mut Vec<[f32; 3]>, indices: &mut Vec<u32>) {
        let here = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            for primitive in mesh.primitives().filter(|p| p.mode() == gltf::mesh::Mode::Triangles) {
                let reader = primitive.reader(|buffer| matches!(buffer.source(), gltf::buffer::Source::Bin).then_some(blob));
                let Some(points) = reader.read_positions() else { continue };
                let base = positions.len() as u32;
                positions.extend(points.map(|p| here.transform_point3(Vec3::from(p)).to_array()));
                let count = positions.len() as u32 - base;
                match reader.read_indices() {
                    Some(read) => indices.extend(read.into_u32().map(|i| base + i)),
                    None => indices.extend(base..base + count),
                }
            }
        }
        for child in node.children() {
            walk(child, here, blob, positions, indices);
        }
    }
    let scene = gltf.document.default_scene().or_else(|| gltf.document.scenes().next()).ok_or("the model has no scene")?;
    for node in scene.nodes() {
        walk(node, Mat4::IDENTITY, blob, &mut positions, &mut indices);
    }
    if indices.len() < 3 || indices.iter().any(|&i| i as usize >= positions.len()) {
        return Err("the model has no shape".into());
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let uvs = vec![[0.0, 0.0]; positions.len()];
    Ok(Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(bevy::mesh::Indices::U32(indices)))
}

/// Each shape goes where the passthrough shows its controller to its eye
/// (the models are made about the grip pose), in the game's world; shown
/// while the setting is on.
fn place(
    settings: Res<ArSettings>,
    rig: Res<Rig>,
    world: Res<super::link::ArWorld>,
    grips: Query<(&Transform, &Grip), Without<Shape>>,
    // (The eye cameras' own transforms are tracking space, as the grips' are.)
    eyes: Query<(&Transform, &XrCamera), Without<Shape>>,
    mut shapes: Query<(&mut Transform, &mut Visibility, &Shape)>,
) {
    let root = rig.root_transform();
    for (mut at, mut visibility, shape) in &mut shapes {
        let grip = grips.iter().find(|(_, grip)| grip.0 == shape.0).map(|(t, _)| *t);
        let eye = eyes.iter().find(|(_, eye)| eye.0 == shape.1).map(|(t, _)| *t);
        let (Some(root), Some(grip), Some(eye), true) = (root, grip, eye, settings.occlusion.on() && settings.occlusion_controllers) else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        // The room behind the controller, along the camera's line to it (the room is kept in the game's world).
        let camera = eye.translation + eye.rotation * CAMERA_FROM_EYE;
        let (from, to) = (root.transform_point(camera), root.transform_point(grip.translation));
        let reach = (to - from).length();
        let dir = (to - from) / reach.max(1e-6);
        let room = world.room().filter_map(|triangle| hit(from, dir, triangle)).filter(|&along| along > reach).fold(f32::INFINITY, f32::min);
        let scale = root.scale.x.max(1e-6);
        let room_m = if room.is_finite() { room / scale } else { reach / scale + ROOM_M };
        let moved = seen_at(grip.translation, eye.translation, camera, room_m) - grip.translation;
        *at = root * Transform { translation: grip.translation + moved, ..grip };
        visibility.set_if_neq(Visibility::Visible);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A binary glTF of one triangle under a node moved 1 along x.
    fn triangle() -> Vec<u8> {
        let mut bin = Vec::new();
        for p in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for v in p {
                bin.extend(v.to_le_bytes());
            }
        }
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0,"translation":[1,0,0]}}],"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}}}]}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}}],"bufferViews":[{{"buffer":0,"byteLength":{}}}],"buffers":[{{"byteLength":{}}}]}}"#,
            bin.len(),
            bin.len()
        );
        let mut json = json.into_bytes();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        let mut out = Vec::new();
        out.extend(b"glTF");
        out.extend(2u32.to_le_bytes());
        out.extend(((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(b"JSON");
        out.extend(&json);
        out.extend((bin.len() as u32).to_le_bytes());
        out.extend(b"BIN\0");
        out.extend(&bin);
        out
    }

    /// The case measured from a screenshot: a controller 45 cm off, 19 degrees
    /// to the right, with the floor 1.3 m behind, was seen about 2 degrees
    /// further right than it was.
    #[test]
    fn a_controller_is_seen_further_from_the_middle_than_it_is() {
        let (eye, camera) = (Vec3::ZERO, CAMERA_FROM_EYE);
        let point = Vec3::new(0.145, -0.093, -0.42);
        let seen = seen_at(point, eye, camera, 1.3);
        let degrees = |p: Vec3| (p.x / -p.z).atan().to_degrees();
        assert!((degrees(point) - 19.0).abs() < 0.2);
        assert!((degrees(seen) - degrees(point) - 2.0).abs() < 0.5, "{}", degrees(seen) - degrees(point));
        assert!((seen.length() - point.length()).abs() < 1e-5, "as far from the eye as it was");
        // Nothing behind it nearer than itself: where it is.
        assert_eq!(seen_at(point, eye, camera, 0.2), point);
    }

    #[test]
    fn a_ray_finds_how_far_a_triangle_is() {
        let triangle = [Vec3::new(-1.0, -1.0, -2.0), Vec3::new(1.0, -1.0, -2.0), Vec3::new(0.0, 1.0, -2.0)];
        assert!((hit(Vec3::ZERO, Vec3::NEG_Z, triangle).unwrap() - 2.0).abs() < 1e-5);
        assert!(hit(Vec3::ZERO, Vec3::Z, triangle).is_none(), "behind the ray");
        assert!(hit(Vec3::X * 5.0, Vec3::NEG_Z, triangle).is_none(), "beside it");
    }

    #[test]
    fn a_models_shape_is_its_triangles_where_its_nodes_put_them() {
        let mesh = shape(&triangle()).unwrap();
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().as_float3().unwrap();
        assert_eq!(positions, [[1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [1.0, 1.0, 0.0]]);
        assert_eq!(mesh.indices().unwrap().len(), 3);
        assert!(shape(b"not a model").is_err());
    }
}
