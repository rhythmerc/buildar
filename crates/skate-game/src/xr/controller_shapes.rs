//! The controllers hide what is behind them, by their own shapes
//! (`XR_FB_render_model`): the headset's depth image often misses a
//! controller (its dark top above all), so the park showed through it.
//!
//! The runtime has a model of each controller the player holds. Each is
//! loaded once (its shape only), and drawn where the controller is as
//! nothing but depth (`build/occluder.wgsl`: colour and alpha zero, so
//! passthrough shows there): the real controller is seen in front of
//! whatever is behind it. A setting under occlusion, off by default.
use super::actions::{Grip, Hand};
use super::ar_settings::ArSettings;
use super::rig::Rig;
use bevy::prelude::*;
use bevy_mod_openxr::session::OxrSession;
use openxr::sys;

pub(crate) struct ControllerShapesPlugin;

impl Plugin for ControllerShapesPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<Unseen>::default())
            .init_resource::<Shapes>()
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
    made: [Option<Entity>; 2],
    tries: u32,
    wait: u32,
}

/// A controller's shape in the room.
#[derive(Component)]
struct Shape(Hand);

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
                let entity = commands
                    .spawn((Shape(hand), Mesh3d(meshes.add(mesh)), MeshMaterial3d(materials.add(Unseen::default())), Visibility::Hidden))
                    .id();
                shapes.made[slot] = Some(entity);
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

/// Each shape goes where its controller's grip is (the models are made
/// about the grip pose), in the game's world; shown while the setting is on.
fn place(
    settings: Res<ArSettings>,
    rig: Res<Rig>,
    grips: Query<(&Transform, &Grip), Without<Shape>>,
    mut shapes: Query<(&mut Transform, &mut Visibility, &Shape)>,
) {
    let root = rig.root_transform();
    for (mut at, mut visibility, shape) in &mut shapes {
        let grip = grips.iter().find(|(_, grip)| grip.0 == shape.0).map(|(t, _)| *t);
        let (Some(root), Some(grip), true) = (root, grip, settings.occlusion.on() && settings.occlusion_controllers) else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        *at = root * grip;
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

    #[test]
    fn a_models_shape_is_its_triangles_where_its_nodes_put_them() {
        let mesh = shape(&triangle()).unwrap();
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap().as_float3().unwrap();
        assert_eq!(positions, [[1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [1.0, 1.0, 0.0]]);
        assert_eq!(mesh.indices().unwrap().len(), 3);
        assert!(shape(b"not a model").is_err());
    }
}
