//! Meta's own keyboard in the room (`XR_META_virtual_keyboard`), for typing a name: where the headset has it, the
//! menus' renaming types on it (else on the panel's own keys).
//!
//! **Made** when a name is first to be typed ([`Typing::wanted`]), on the app's main thread: Meta's runtime lends that
//! thread's Java handle to a worker of its own while the call waits, so from any other thread the two used it at once
//! and Android aborted the app. (A debuggable APK's CheckJNI stalls the runtime's borrowing too: the app isn't one.)
//!
//! **Drawn by the app** from the model the runtime gives (`XR_FB_render_model`, "/model_meta/keyboard/virtual": a
//! glTF of 1900 nodes): each node an entity under the tracking root, each primitive a mesh (the keys' labels with 16
//! morph targets each), unlit. Each frame the runtime says which of its 8700 one-channel animations are where
//! (`Anim`: a node's scale - a key's state, a layout shown - or a label's morph weights, each from its first keyframe
//! to its second by a fraction), and which of its four textures (`metaVirtualKeyboard://texture/<id>`: the glyphs, the
//! typed-text strips) have changed: those are set and uploaded. It is put where the runtime locates it.
//!
//! **Worked** with the controllers' rays and triggers, sent to the runtime each frame; what's typed comes back as
//! events ([`Typed`]: text, erase, enter), taken by the menus.
use super::actions::Hand;
use super::actions::LatestTouch;
use super::panel::{XrPanel, HEIGHT_M};
use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::mesh::morph::{MeshMorphWeights, MorphAttributes, MorphTargetImage};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_mod_openxr::poll_events::{OxrEventHandlerExt, OxrEventIn};
use bevy_mod_openxr::resources::{OxrFrameState, OxrInstance, OxrSystemId};
use bevy_mod_openxr::session::OxrSession;
use bevy_mod_xr::session::XrTrackingRoot;
use bevy_mod_xr::spaces::XrPrimaryReferenceSpace;
use openxr::sys;
use openxr::sys::Handle as _;
use std::collections::HashMap;

pub(crate) struct VirtualKeyboardPlugin;

impl Plugin for VirtualKeyboardPlugin {
    fn build(&self, app: &mut App) {
        // (after the menus have asked for it, this frame: they ask each frame a name is typed, and the ask is let go
        // at its end)
        app.init_resource::<Typing>()
            .add_oxr_event_handler(on_event)
            .add_systems(PostUpdate, run.run_if(resource_exists::<OxrSession>).before(bevy::transform::TransformSystems::Propagate))
            .add_systems(Last, |mut typing: ResMut<Typing>| typing.wanted = None);
    }
}

/// The extension, asked for when the session is made (dropped where the headset doesn't offer it).
pub(crate) fn enable(exts: &mut openxr::ExtensionSet) {
    exts.meta_virtual_keyboard = true;
    exts.fb_render_model = true;
}

/// Typing a name: what the menus want, and what's typed.
#[derive(Resource, Default)]
pub(crate) struct Typing {
    /// A name is being typed, from this text (set each frame while it is; none: the keyboard goes).
    pub wanted: Option<String>,
    /// The keyboard is up: the panel's own keys aren't wanted.
    pub up: bool,
    /// What was typed since the menus last took it.
    pub typed: Vec<Typed>,
}

/// What was typed on the keyboard.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Typed {
    Text(String),
    Erase,
    Enter,
}

/// One of the model's animations: a node's scale, or its meshes' morph weights, from its first keyframe to its
/// second by the runtime's fraction.
enum Anim {
    Scale { node: usize, from: Vec3, to: Vec3 },
    Weights { node: usize, from: Vec<f32>, to: Vec<f32> },
    Other,
}

/// The keyboard drawn: its root (under the tracking root), each node's entity, each node's mesh entities with morph
/// weights, its animations, and its textures by the runtime's id.
struct Model {
    root: Entity,
    nodes: Vec<Entity>,
    morphs: Vec<Vec<Entity>>,
    anims: Vec<Anim>,
    textures: HashMap<u64, Handle<Image>>,
}

/// The keyboard: made (its handle and space), drawn, up or not, and how making it went.
#[derive(Default)]
struct State {
    keyboard: Option<(sys::VirtualKeyboardMETA, sys::Space)>,
    model: Option<Model>,
    up: bool,
    /// Tries at fetching the model (the runtime may want a frame or two), and given up (no keyboard here).
    tries: u32,
    failed: bool,
    /// The two-call buffers, kept (the animation states' are made each frame: they hold pointers).
    dirty: Vec<u64>,
    pixels: Vec<u8>,
}

/// Below the panel, toward the player, tilted up to face them (m, degrees).
const BELOW_PANEL_M: f32 = 0.1;
const TOWARD_PLAYER_M: f32 = 0.12;
const TILT_DEG: f32 = 35.0;

/// (On the main thread: see the module's notes.)
#[allow(clippy::too_many_arguments)]
fn run(
    _main: bevy::ecs::system::NonSendMarker,
    session: Res<OxrSession>,
    instance: Res<OxrInstance>,
    system: Option<Res<OxrSystemId>>,
    reference: Option<Res<XrPrimaryReferenceSpace>>,
    frame: Option<Res<OxrFrameState>>,
    (panel, touch): (Res<XrPanel>, Res<LatestTouch>),
    hands: Query<(&Hand, &Transform), Without<KeyboardNode>>,
    tracking_root: Query<Entity, With<XrTrackingRoot>>,
    mut nodes: Query<&mut Transform, With<KeyboardNode>>,
    mut weights: Query<&mut MeshMorphWeights>,
    mut typing: ResMut<Typing>,
    mut commands: Commands,
    mut assets: (ResMut<Assets<Mesh>>, ResMut<Assets<Image>>, ResMut<Assets<StandardMaterial>>),
    mut state: Local<State>,
) {
    let Some(fns) = instance.exts().meta_virtual_keyboard else {
        typing.up = false;
        return;
    };
    let (Some(reference), Some(frame)) = (reference, frame) else { return };
    let base = sys::Space::from_raw(reference.as_raw());
    let wanted = typing.wanted.clone();

    // Made the first time it's wanted (and drawn: the model may take a frame or two).
    if wanted.is_some() && !state.failed && state.model.is_none() {
        if state.keyboard.is_none() {
            match make(&session, &instance, system.as_deref(), fns, base) {
                Ok(made) => state.keyboard = Some(made),
                Err(why) => {
                    warn!("VKB: {why}: the panel's keys type");
                    state.failed = true;
                    return;
                }
            }
        }
        state.tries += 1;
        match super::controller_shapes::model(&session, "/model_meta/keyboard/virtual") {
            Ok(bytes) => {
                let Ok(root) = tracking_root.single() else { return };
                match build(&bytes, &mut commands, &mut assets.0, &mut assets.1, &mut assets.2, root) {
                    Ok(model) => {
                        info!("VKB: drawn ({} nodes, {} animations, {} textures)", model.nodes.len(), model.anims.len(), model.textures.len());
                        state.model = Some(model);
                    }
                    Err(why) => {
                        warn!("VKB: the model: {why}: the panel's keys type");
                        state.failed = true;
                    }
                }
            }
            Err(why) if state.tries >= 60 => {
                warn!("VKB: the model: {why}: the panel's keys type");
                state.failed = true;
            }
            Err(_) => {}
        }
        return;
    }
    let (Some((keyboard, space)), Some(root)) = (state.keyboard, state.model.as_ref().map(|m| m.root)) else {
        typing.up = false;
        return;
    };

    // Up when a name is wanted, gone when not.
    if wanted.is_some() != state.up {
        state.up = wanted.is_some();
        let info = sys::VirtualKeyboardModelVisibilitySetInfoMETA { ty: sys::VirtualKeyboardModelVisibilitySetInfoMETA::TYPE, next: std::ptr::null(), visible: state.up.into() };
        // SAFETY: the keyboard is live; the info is read during the call.
        let _ = unsafe { (fns.set_virtual_keyboard_model_visibility)(keyboard, &info) };
        commands.entity(root).insert(if state.up { Visibility::Inherited } else { Visibility::Hidden });
        if let Some(text) = &wanted {
            // (under the panel, facing the player; the name so far, for its suggestions)
            let panel = panel.pose();
            let up = panel.rotation * Vec3::Y;
            let toward = panel.rotation * Vec3::Z;
            let at = panel.translation - up * (HEIGHT_M * 0.5 + BELOW_PANEL_M) + toward * TOWARD_PLAYER_M;
            let rotation = panel.rotation * Quat::from_rotation_x(-TILT_DEG.to_radians());
            let location = sys::VirtualKeyboardLocationInfoMETA {
                ty: sys::VirtualKeyboardLocationInfoMETA::TYPE,
                next: std::ptr::null(),
                location_type: sys::VirtualKeyboardLocationTypeMETA::CUSTOM,
                space: base,
                pose_in_space: pose(at, rotation),
                scale: 1.0,
            };
            // SAFETY: as above.
            let placed = unsafe { (fns.suggest_virtual_keyboard_location)(keyboard, &location) };
            if placed.into_raw() < 0 {
                warn!("VKB: putting it under the panel: {placed:?}");
            }
            let context = std::ffi::CString::new(text.as_str()).unwrap_or_default();
            let change = sys::VirtualKeyboardTextContextChangeInfoMETA { ty: sys::VirtualKeyboardTextContextChangeInfoMETA::TYPE, next: std::ptr::null(), text_context: context.as_ptr() };
            // SAFETY: the text is read during the call.
            let _ = unsafe { (fns.change_virtual_keyboard_text_context)(keyboard, &change) };
        }
    }
    typing.up = state.up;
    if !state.up {
        return;
    }

    // The controllers' rays and triggers, to the runtime.
    for (hand, aim) in &hands {
        let (source, pressed) = match hand {
            Hand::Left => (sys::VirtualKeyboardInputSourceMETA::CONTROLLER_RAY_LEFT, touch.0.left_trigger > 0.5),
            Hand::Right => (sys::VirtualKeyboardInputSourceMETA::CONTROLLER_RAY_RIGHT, touch.0.right_trigger > 0.5),
        };
        let info = sys::VirtualKeyboardInputInfoMETA {
            ty: sys::VirtualKeyboardInputInfoMETA::TYPE,
            next: std::ptr::null(),
            input_source: source,
            input_space: base,
            input_pose_in_space: pose(aim.translation, aim.rotation),
            input_state: if pressed { sys::VirtualKeyboardInputStateFlagsMETA::PRESSED } else { sys::VirtualKeyboardInputStateFlagsMETA::EMPTY },
        };
        let mut root_pose = pose(aim.translation, aim.rotation);
        // SAFETY: the info is read and the pose written during the call.
        let _ = unsafe { (fns.send_virtual_keyboard_input)(keyboard, &info, &mut root_pose) };
    }

    // Where the runtime has it, and its size.
    let mut location = sys::SpaceLocation { ty: sys::SpaceLocation::TYPE, next: std::ptr::null_mut(), location_flags: sys::SpaceLocationFlags::EMPTY, pose: sys::Posef::IDENTITY };
    // SAFETY: both spaces are live; the location is written.
    let located = unsafe { (instance.fp().locate_space)(space, base, frame.predicted_display_time, &mut location) };
    let mut scale = 1.0f32;
    // SAFETY: the scale is written.
    let _ = unsafe { (fns.get_virtual_keyboard_scale)(keyboard, &mut scale) };
    if located.into_raw() >= 0
        && location.location_flags.contains(sys::SpaceLocationFlags::POSITION_VALID | sys::SpaceLocationFlags::ORIENTATION_VALID)
        && let Ok(mut tf) = nodes.get_mut(root)
    {
        let p = location.pose;
        *tf = Transform {
            translation: Vec3::new(p.position.x, p.position.y, p.position.z),
            rotation: Quat::from_xyzw(p.orientation.x, p.orientation.y, p.orientation.z, p.orientation.w),
            scale: Vec3::splat(scale),
        };
    }

    // The animations where the runtime has them.
    let mut query = sys::VirtualKeyboardModelAnimationStatesMETA {
        ty: sys::VirtualKeyboardModelAnimationStatesMETA::TYPE,
        next: std::ptr::null_mut(),
        state_capacity_input: 0,
        state_count_output: 0,
        states: std::ptr::null_mut(),
    };
    // SAFETY: a count asked for, then that many states written.
    let counted = unsafe { (fns.get_virtual_keyboard_model_animation_states)(keyboard, &mut query) };
    let count = if counted.into_raw() >= 0 { query.state_count_output } else { 0 };
    if count > 0 {
        let blank = sys::VirtualKeyboardAnimationStateMETA { ty: sys::VirtualKeyboardAnimationStateMETA::TYPE, next: std::ptr::null_mut(), animation_index: 0, fraction: 0.0 };
        let mut states = vec![blank; count as usize];
        let model = &state.model;
        query.state_capacity_input = count;
        query.states = states.as_mut_ptr();
        // SAFETY: as above.
        if unsafe { (fns.get_virtual_keyboard_model_animation_states)(keyboard, &mut query) }.into_raw() >= 0
            && let Some(model) = model.as_ref()
        {
            for s in states.iter().take(query.state_count_output as usize) {
                let t = s.fraction.clamp(0.0, 1.0);
                match model.anims.get(s.animation_index as usize) {
                    Some(Anim::Scale { node, from, to }) => {
                        if let Ok(mut tf) = nodes.get_mut(model.nodes[*node]) {
                            tf.scale = from.lerp(*to, t);
                        }
                    }
                    Some(Anim::Weights { node, from, to }) => {
                        let w: Vec<f32> = from.iter().zip(to).map(|(a, b)| a + (b - a) * t).collect();
                        for &mesh in &model.morphs[*node] {
                            if let Ok(mut weights) = weights.get_mut(mesh)
                                && let Ok(new) = MeshMorphWeights::new(w.clone())
                            {
                                *weights = new;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // The textures it has changed.
    let mut dirty = 0u32;
    // SAFETY: a count asked for, then that many ids written.
    let counted = unsafe { (fns.get_virtual_keyboard_dirty_textures)(keyboard, 0, &mut dirty, std::ptr::null_mut()) };
    if counted.into_raw() >= 0 && dirty > 0 {
        let State { dirty: ids, pixels, model, .. } = &mut *state;
        ids.resize(dirty as usize, 0);
        let mut got = 0u32;
        // SAFETY: as above.
        if unsafe { (fns.get_virtual_keyboard_dirty_textures)(keyboard, dirty, &mut got, ids.as_mut_ptr()) }.into_raw() >= 0 {
            for &id in ids.iter().take(got as usize) {
                let mut data = sys::VirtualKeyboardTextureDataMETA {
                    ty: sys::VirtualKeyboardTextureDataMETA::TYPE,
                    next: std::ptr::null_mut(),
                    texture_width: 0,
                    texture_height: 0,
                    buffer_capacity_input: 0,
                    buffer_count_output: 0,
                    buffer: std::ptr::null_mut(),
                };
                // SAFETY: the size asked for, then that many bytes written.
                if unsafe { (fns.get_virtual_keyboard_texture_data)(keyboard, id, &mut data) }.into_raw() < 0 || data.buffer_count_output == 0 {
                    continue;
                }
                pixels.resize(data.buffer_count_output as usize, 0);
                data.buffer_capacity_input = data.buffer_count_output;
                data.buffer = pixels.as_mut_ptr();
                // SAFETY: as above.
                if unsafe { (fns.get_virtual_keyboard_texture_data)(keyboard, id, &mut data) }.into_raw() < 0 {
                    continue;
                }
                if let Some(handle) = model.as_ref().and_then(|m| m.textures.get(&id))
                    && let Some(image) = assets.1.get_mut(handle)
                    && image.width() == data.texture_width
                    && image.height() == data.texture_height
                {
                    image.data = Some(pixels[..data.buffer_count_output as usize].to_vec());
                }
            }
        }
    }
}

/// The keyboard made, and its space (in `base`).
fn make(
    session: &OxrSession,
    instance: &OxrInstance,
    system: Option<&OxrSystemId>,
    fns: openxr::raw::VirtualKeyboardMETA,
    base: sys::Space,
) -> Result<(sys::VirtualKeyboardMETA, sys::Space), String> {
    let ok = |result: sys::Result, what: &str| if result.into_raw() >= 0 { Ok(()) } else { Err(format!("{what}: {result:?}")) };
    if let Some(system) = system {
        let mut vk = sys::SystemVirtualKeyboardPropertiesMETA { ty: sys::SystemVirtualKeyboardPropertiesMETA::TYPE, next: std::ptr::null_mut(), supports_virtual_keyboard: sys::FALSE };
        // SAFETY: the properties are written; the chain is one struct of ours.
        let mut props: sys::SystemProperties = unsafe { std::mem::zeroed() };
        props.ty = sys::SystemProperties::TYPE;
        props.next = (&raw mut vk).cast();
        // SAFETY: as above.
        ok(unsafe { (instance.fp().get_system_properties)(instance.as_raw(), system.0, &mut props) }, "the system's properties")?;
        if vk.supports_virtual_keyboard == sys::FALSE {
            return Err("this headset has no virtual keyboard".into());
        }
    }
    let started = std::time::Instant::now();
    let info = sys::VirtualKeyboardCreateInfoMETA { ty: sys::VirtualKeyboardCreateInfoMETA::TYPE, next: std::ptr::null() };
    let mut keyboard = sys::VirtualKeyboardMETA::NULL;
    // SAFETY: the session is live; the handle is written.
    ok(unsafe { (fns.create_virtual_keyboard)(session.as_raw(), &info, &mut keyboard) }, "making the keyboard")?;
    let space_info = sys::VirtualKeyboardSpaceCreateInfoMETA {
        ty: sys::VirtualKeyboardSpaceCreateInfoMETA::TYPE,
        next: std::ptr::null(),
        location_type: sys::VirtualKeyboardLocationTypeMETA::CUSTOM,
        space: base,
        pose_in_space: sys::Posef::IDENTITY,
    };
    let mut space = sys::Space::NULL;
    // SAFETY: as above.
    ok(unsafe { (fns.create_virtual_keyboard_space)(session.as_raw(), keyboard, &space_info, &mut space) }, "its space")?;
    info!("VKB: made ({:.0} ms)", started.elapsed().as_secs_f64() * 1000.0);
    Ok((keyboard, space))
}

/// What's typed, from the runtime's events.
fn on_event(event: OxrEventIn, mut typing: ResMut<Typing>) {
    match *event {
        openxr::Event::VirtualKeyboardCommitTextMETA(e) => {
            let raw = e.text();
            let bytes: Vec<u8> = raw.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
            let text = String::from_utf8_lossy(&bytes).into_owned();
            if !text.is_empty() {
                typing.typed.push(Typed::Text(text));
            }
        }
        openxr::Event::VirtualKeyboardBackspaceMETA(_) => typing.typed.push(Typed::Erase),
        openxr::Event::VirtualKeyboardEnterMETA(_) => typing.typed.push(Typed::Enter),
        _ => {}
    }
}

fn pose(at: Vec3, rotation: Quat) -> sys::Posef {
    sys::Posef {
        orientation: sys::Quaternionf { x: rotation.x, y: rotation.y, z: rotation.z, w: rotation.w },
        position: sys::Vector3f { x: at.x, y: at.y, z: at.z },
    }
}

/// One of the keyboard's node entities.
#[derive(Component)]
struct KeyboardNode;

/// The model drawn: its nodes as entities under `parent` (hidden until it's up), its meshes, materials and textures.
fn build(
    bytes: &[u8],
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    parent: Entity,
) -> Result<Model, String> {
    let gltf = gltf::Gltf::from_slice_without_validation(bytes).map_err(|e| e.to_string())?;
    let blob = gltf.blob.as_deref().unwrap_or_default();
    let doc = &gltf.document;
    let view = |v: &gltf::buffer::View| blob.get(v.offset()..v.offset() + v.length());

    // The images: the runtime's (by id, blank until it sends them), and the model's own (small PNGs).
    let mut textures = HashMap::new();
    let images_of: Vec<Handle<Image>> = doc
        .images()
        .map(|image| match image.source() {
            gltf::image::Source::Uri { uri, .. } => {
                let (id, w, h) = runtime_texture(uri).unwrap_or((0, 1, 1));
                let handle = images.add(blank(w, h));
                if id != 0 {
                    textures.insert(id, handle.clone());
                }
                handle
            }
            gltf::image::Source::View { view: v, mime_type } => {
                let image = view(&v)
                    .and_then(|data| {
                        Image::from_buffer(data, ImageType::MimeType(mime_type), CompressedImageFormats::NONE, true, ImageSampler::Default, RenderAssetUsages::default()).ok()
                    })
                    .unwrap_or_else(|| blank(1, 1));
                images.add(image)
            }
        })
        .collect();
    let materials_of: Vec<Handle<StandardMaterial>> = doc
        .materials()
        .map(|m| {
            let pbr = m.pbr_metallic_roughness();
            let [r, g, b, a] = pbr.base_color_factor();
            materials.add(StandardMaterial {
                base_color: Color::linear_rgba(r, g, b, a),
                base_color_texture: pbr.base_color_texture().map(|t| images_of[t.texture().source().index()].clone()),
                alpha_mode: match m.alpha_mode() {
                    gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                    gltf::material::AlphaMode::Mask => AlphaMode::Mask(m.alpha_cutoff().unwrap_or(0.5)),
                    gltf::material::AlphaMode::Blend => AlphaMode::Blend,
                },
                double_sided: m.double_sided(),
                cull_mode: if m.double_sided() { None } else { Some(bevy::render::render_resource::Face::Back) },
                unlit: true,
                ..default()
            })
        })
        .collect();

    // The nodes, each an entity in its parent's place; the root's parent is `parent`.
    let root = commands.spawn((KeyboardNode, Transform::default(), Visibility::Hidden, ChildOf(parent), Name::new("Meta keyboard"))).id();
    let mut nodes = vec![Entity::PLACEHOLDER; doc.nodes().count()];
    let mut morphs = vec![Vec::new(); nodes.len()];
    for node in doc.nodes() {
        let (t, r, s) = node.transform().decomposed();
        nodes[node.index()] = commands
            .spawn((KeyboardNode, Transform { translation: Vec3::from(t), rotation: Quat::from_array(r), scale: Vec3::from(s) }, Visibility::Inherited))
            .id();
    }
    let mut has_parent = vec![false; nodes.len()];
    for node in doc.nodes() {
        for child in node.children() {
            has_parent[child.index()] = true;
            commands.entity(nodes[child.index()]).insert(ChildOf(nodes[node.index()]));
        }
    }
    for (i, &e) in nodes.iter().enumerate() {
        if !has_parent[i] {
            commands.entity(e).insert(ChildOf(root));
        }
    }
    // The meshes: each primitive its own entity under its node.
    for node in doc.nodes() {
        let Some(mesh) = node.mesh() else { continue };
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|_| Some(blob));
            let Some(positions) = reader.read_positions().map(|p| p.collect::<Vec<_>>()) else { continue };
            let count = positions.len();
            let mut out = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
            out.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            if let Some(n) = reader.read_normals() {
                out.insert_attribute(Mesh::ATTRIBUTE_NORMAL, n.collect::<Vec<_>>());
            }
            if let Some(t) = reader.read_tangents() {
                out.insert_attribute(Mesh::ATTRIBUTE_TANGENT, t.collect::<Vec<_>>());
            }
            if let Some(uv) = reader.read_tex_coords(0) {
                out.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv.into_f32().collect::<Vec<_>>());
            }
            if let Some(indices) = reader.read_indices() {
                out.insert_indices(Indices::U32(indices.into_u32().collect()));
            }
            let targets: Vec<Vec<[f32; 3]>> =
                reader.read_morph_targets().map(|(p, _, _)| p.map(|p| p.collect()).unwrap_or_else(|| vec![[0.0; 3]; count])).collect();
            let morph = !targets.is_empty();
            if morph {
                let image = MorphTargetImage::new(
                    targets.iter().map(|t| t.iter().map(|&p| MorphAttributes::new(Vec3::from(p), Vec3::ZERO, Vec3::ZERO))),
                    count,
                    RenderAssetUsages::default(),
                )
                .map_err(|e| format!("{e:?}"))?;
                out.set_morph_targets(images.add(image.0));
            }
            let material =
                primitive.material().index().map_or_else(|| materials.add(StandardMaterial { unlit: true, ..default() }), |i| materials_of[i].clone());
            let mut entity =
                commands.spawn((Mesh3d(meshes.add(out)), MeshMaterial3d(material), Transform::default(), Visibility::Inherited, ChildOf(nodes[node.index()])));
            if morph {
                let weights = mesh.weights().map(<[f32]>::to_vec).unwrap_or_else(|| vec![0.0; targets.len()]);
                entity.insert(MeshMorphWeights::new(weights).map_err(|e| format!("{e:?}"))?);
                morphs[node.index()].push(entity.id());
            }
        }
    }
    let anims = animations(doc, blob);
    Ok(Model { root, nodes, morphs, anims, textures })
}

/// The model's animations: each one channel, its first and second keyframes.
fn animations(doc: &gltf::Document, blob: &[u8]) -> Vec<Anim> {
    doc.animations()
        .map(|a| {
            let Some(channel) = a.channels().next() else { return Anim::Other };
            let node = channel.target().node().index();
            let reader = channel.reader(|_| Some(blob));
            match reader.read_outputs() {
                Some(gltf::animation::util::ReadOutputs::Scales(s)) => {
                    let s: Vec<[f32; 3]> = s.collect();
                    match (s.first(), s.get(1)) {
                        (Some(a), Some(b)) => Anim::Scale { node, from: Vec3::from(*a), to: Vec3::from(*b) },
                        (Some(a), None) => Anim::Scale { node, from: Vec3::from(*a), to: Vec3::from(*a) },
                        _ => Anim::Other,
                    }
                }
                Some(gltf::animation::util::ReadOutputs::MorphTargetWeights(w)) => {
                    let w: Vec<f32> = w.into_f32().collect();
                    let half = w.len() / 2;
                    if half == 0 {
                        return Anim::Other;
                    }
                    Anim::Weights { node, from: w[..half].to_vec(), to: w[half..half * 2].to_vec() }
                }
                _ => Anim::Other,
            }
        })
        .collect()
}

/// A runtime texture's id and size, from its uri (`metaVirtualKeyboard://texture/<id>?w=<w>&h=<h>&fmt=RGBA32`).
fn runtime_texture(uri: &str) -> Option<(u64, u32, u32)> {
    let rest = uri.strip_prefix("metaVirtualKeyboard://texture/")?;
    let (id, query) = rest.split_once('?').unwrap_or((rest, ""));
    let get = |key: &str| query.split('&').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('=')?.parse::<u32>().ok());
    Some((id.parse().ok()?, get("w").unwrap_or(1), get("h").unwrap_or(1)))
}

/// A clear image, `w` by `h`.
fn blank(w: u32, h: u32) -> Image {
    Image::new_fill(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_runtime_texture_is_known_by_its_uri() {
        assert_eq!(runtime_texture("metaVirtualKeyboard://texture/711712087033584587?w=1430&h=90&fmt=RGBA32"), Some((711712087033584587, 1430, 90)));
        assert_eq!(runtime_texture("other.png"), None);
    }

    /// With the model the headset gave (`VKB_GLB`): every animation read, the labels' with their 16 weights.
    #[test]
    fn the_keyboards_animations_are_read() {
        let Some(path) = std::env::var_os("VKB_GLB") else { return };
        let bytes = std::fs::read(path).unwrap();
        let gltf = gltf::Gltf::from_slice_without_validation(&bytes).unwrap();
        let anims = animations(&gltf.document, gltf.blob.as_deref().unwrap());
        assert_eq!(anims.len(), 8703);
        assert!(anims.iter().all(|a| !matches!(a, Anim::Other)));
        assert!(anims.iter().any(|a| matches!(a, Anim::Weights { from, .. } if from.len() == 16)));
    }
}
