//! One renderer for every engine's HUD: an engine hands its HUD over as draw lists cut into parts
//! ([`engine_core::hud::HudFrame`]), and this draws each part into a picture of its own and puts it where its
//! [`Placement`] says - by a controller ([`HandHud`], placed by `xr::hand_hud`) or at the player in the room
//! ([`AtPlayerHud`], turned to the eyes: [`at_player`]).
//!
//! - **Pictures:** each part has a 2D camera ([`EngineHudCamera`]) looking at its area of the engine's canvas, on a
//!   render layer of its own from a small pool, drawing into an image of that area at the part's resolution.
//! - **Draws:** each draw is a 2D mesh with a [`HudPaint`] (one material and shader for every [`Shading`] and
//!   [`HudBlend`]), laid over the last in the engine's order. Meshes and materials are kept from frame to frame and
//!   changed in place; an unchanged frame is skipped.
//! - **Textures** are fetched from the engine once each, by id.
//! - **Ownership:** what it spawns is the engine's ([`EngineOwned`]): hidden while another runs, and the engine's own
//!   panels stand in for the corner readout (`xr::ui::hud`) while it runs.
use crate::engines::{Engine, EngineOwned};
use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{RenderTarget, ScalingMode};
use bevy::ecs::system::SystemParam;
use bevy::mesh::{MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, Extent3d, RenderPipelineDescriptor,
    ShaderType, SpecializedMeshPipelineError, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPlugin, MeshMaterial2d};
use engine_api::{EngineHudCamera, HandHud};
#[cfg(feature = "xr")]
use engine_core::Focus;
#[cfg(feature = "xr")]
use engine_core::hud::Side;
use engine_core::hud::{HudBlend, HudFrame, HudPart, Placement, Rgba8, Shading, TextureId};
use std::collections::HashMap;

/// The render layers the parts' pictures are drawn on, one a part (nothing else uses them: the controllers' shapes
/// have 20 and 21).
const LAYERS: std::ops::RangeInclusive<usize> = 22..=31;
/// The parts' cameras' order: before the views showing their pictures, each its own.
const FIRST_ORDER: isize = -20;

/// The engines' HUDs drawn: its materials, and placing what is at the player.
pub(crate) struct EngineHudPlugin;

impl Plugin for EngineHudPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(feature = "xr")]
        app.init_resource::<engine_api::PlayerFocus>().add_systems(
            PostUpdate,
            place_at_player.after(crate::xr::link::FocusSet).before(bevy::transform::TransformSystems::Propagate),
        );
        // (not without a renderer: tests)
        if app.is_plugin_added::<bevy::render::RenderPlugin>() {
            embedded_asset!(app, "engine_hud.wgsl");
            embedded_asset!(app, "engine_hud_panel.wgsl");
            app.add_plugins((Material2dPlugin::<HudPaint>::default(), bevy::pbr::MaterialPlugin::<PanelMaterial>::default()))
                .init_resource::<EngineHuds>();
            #[cfg(feature = "xr")]
            app.add_systems(Update, panel_depth);
        }
    }
}

/// What a part's panel is drawn with in the room: its picture, unlit, over everything (a HUD isn't hidden by what's in
/// front of it, virtual or real). No depth test; where it has paint it writes the near plane's depth
/// (`engine_hud_panel.wgsl`), which the room's occlusion pass never hides: that pass hides pixels by the scene's
/// depth, and with nothing written, a panel over a virtual piece that a real thing hid was hidden with it.
pub(crate) type PanelMaterial = bevy::pbr::ExtendedMaterial<StandardMaterial, OnTop>;

/// [`PanelMaterial`]'s part: drawn without a depth test, at the near plane's depth where painted.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
#[bind_group_data(OnTopKey)]
pub(crate) struct OnTop {
    /// Its own depth written instead, where it is (by the controller): real things nearer than it hide it (the AR
    /// setting `hud_depth_at_controller`, to compare the two).
    pub(crate) at_panel: bool,
}

/// What of an [`OnTop`] makes its pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct OnTopKey {
    at_panel: bool,
}

impl From<&OnTop> for OnTopKey {
    fn from(on_top: &OnTop) -> Self {
        Self { at_panel: on_top.at_panel }
    }
}

impl bevy::pbr::MaterialExtension for OnTop {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "embedded://buildar/engine_hud_panel.wgsl".into()
    }

    fn specialize(
        _: &bevy::pbr::MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if key.bind_group_data.at_panel
            && let Some(fragment) = &mut descriptor.fragment
        {
            fragment.shader_defs.push("DEPTH_AT_PANEL".into());
        }
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_compare = bevy::render::render_resource::CompareFunction::Always;
            depth.depth_write_enabled = true;
        }
        Ok(())
    }
}

/// The panels' depth as the AR settings have it (`hud_depth_at_controller`).
#[cfg(feature = "xr")]
fn panel_depth(settings: Option<Res<crate::xr::ar_settings::ArSettings>>, mut materials: ResMut<Assets<PanelMaterial>>) {
    let Some(settings) = settings else { return };
    let at_panel = settings.hud_depth_at_controller;
    let stale: Vec<_> = materials.iter().filter(|(_, m)| m.extension.at_panel != at_panel).map(|(id, _)| id).collect();
    for id in stale {
        if let Some(material) = materials.get_mut(id) {
            material.extension.at_panel = at_panel;
        }
    }
}

/// Adds the renderer once (each engine with a HUD asks for it).
pub(crate) fn install(app: &mut App) {
    if !app.is_plugin_added::<EngineHudPlugin>() {
        app.add_plugins(EngineHudPlugin);
    }
}

/// How a draw's colour is made ([`Shading`]) and laid over what's under it ([`HudBlend`]).
#[derive(Clone, Copy, Debug, PartialEq, ShaderType)]
struct HudParams {
    /// x: `Modulate`'s scale, y: textured, z: opaque, w: the shading (0: `Modulate` in gamma values, 1: in linear
    /// ones, 2: `Transform`).
    mode: Vec4,
    multiply: Vec4,
    add: Vec4,
}

/// A HUD draw's material (`engine_hud.wgsl`).
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, PartialEq)]
#[bind_group_data(HudPaintKey)]
pub(crate) struct HudPaint {
    #[uniform(0)]
    params: HudParams,
    #[texture(1)]
    #[sampler(2)]
    picture: Handle<Image>,
    /// Added to what's under it ([`HudBlend::Add`]), not over it.
    add: bool,
}

/// What of a [`HudPaint`] makes its pipeline: its blend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct HudPaintKey {
    add: bool,
}

impl From<&HudPaint> for HudPaintKey {
    fn from(paint: &HudPaint) -> Self {
        Self { add: paint.add }
    }
}

impl HudPaint {
    fn of(shading: Shading, blend: HudBlend, picture: Option<Handle<Image>>, white: &Handle<Image>) -> Self {
        let textured = if picture.is_some() { 1.0 } else { 0.0 };
        let opaque = if blend == HudBlend::Opaque { 1.0 } else { 0.0 };
        let (mode, multiply, add) = match shading {
            Shading::Modulate { scale, gamma } => {
                (Vec4::new(scale, textured, opaque, if gamma { 0.0 } else { 1.0 }), Vec4::ONE, Vec4::ZERO)
            }
            Shading::Transform { multiply, add } => {
                (Vec4::new(1.0, textured, opaque, 2.0), Vec4::from_array(multiply), Vec4::from_array(add))
            }
        };
        Self {
            params: HudParams { mode, multiply, add },
            picture: picture.unwrap_or_else(|| white.clone()),
            add: blend == HudBlend::Add,
        }
    }
}

impl Material2d for HudPaint {
    fn fragment_shader() -> ShaderRef {
        concat!("embedded://", env!("CARGO_CRATE_NAME"), "/engine_hud.wgsl").into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        // (every blend in the transparent phase, in the engine's order; an opaque draw is its alpha made 1)
        AlphaMode2d::Blend
    }

    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if !key.bind_group_data.add {
            return Ok(());
        }
        // (Direct3D's source alpha, one: the colour by its alpha onto what's there; the coverage left as it was)
        if let Some(fragment) = &mut descriptor.fragment {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState {
                    color: BlendComponent {
                        src_factor: BlendFactor::SrcAlpha,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Add,
                    },
                    alpha: BlendComponent {
                        src_factor: BlendFactor::Zero,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Add,
                    },
                });
            }
        }
        Ok(())
    }
}

/// A part's panel at the player in the room ([`Placement::AtPlayer`]): its camera, whether it has anything to show,
/// and where and how big it goes ([`at_player`]).
#[derive(Component, Clone, Copy, Debug)]
#[cfg_attr(not(feature = "xr"), allow(dead_code))]
pub(crate) struct AtPlayerHud {
    pub camera: Entity,
    pub empty: bool,
    /// Its [`Placement::AtPlayer`].
    pub placement: Placement,
}

/// The engines' HUDs as drawn (there only with a renderer).
#[derive(Resource)]
pub(crate) struct EngineHuds {
    /// For an untextured draw.
    white: Option<Handle<Image>>,
    /// The render layers no part has yet.
    free: Vec<usize>,
    engines: HashMap<Engine, Drawn>,
}

impl Default for EngineHuds {
    fn default() -> Self {
        Self { white: None, free: LAYERS.rev().collect(), engines: HashMap::new() }
    }
}

/// An engine's HUD as drawn.
#[derive(Default)]
struct Drawn {
    textures: HashMap<TextureId, Handle<Image>>,
    /// Each draw's mesh entity, by its index, reused frame to frame with its mesh and material.
    slots: Vec<Slot>,
    parts: Vec<PartSlot>,
    /// The last frame drawn (none yet: `shown` false).
    frame: HudFrame,
    shown: bool,
}

struct Slot {
    entity: Entity,
    mesh: Handle<Mesh>,
    paint: Handle<HudPaint>,
    layer: usize,
    visible: bool,
}

/// A part's picture: its camera and image, its panel, its layer.
struct PartSlot {
    camera: Entity,
    panel: Entity,
    layer: usize,
    target: Handle<Image>,
    size: Vec2,
    resolution: f32,
    panel_mesh: Handle<Mesh>,
    panel_material: Handle<PanelMaterial>,
    placement: Placement,
}

/// What draws an engine's HUD ([`HudRenderer::show`]).
#[derive(SystemParam)]
pub(crate) struct HudRenderer<'w, 's> {
    commands: Commands<'w, 's>,
    huds: Option<ResMut<'w, EngineHuds>>,
    images: Option<ResMut<'w, Assets<Image>>>,
    meshes: Option<ResMut<'w, Assets<Mesh>>>,
    paints: Option<ResMut<'w, Assets<HudPaint>>>,
    materials: Option<ResMut<'w, Assets<PanelMaterial>>>,
}

impl HudRenderer<'_, '_> {
    /// `engine`'s HUD as `frame` has it (an unchanged frame is skipped), its textures fetched as they're first
    /// drawn (`fetch`: none, not now - asked again with the next frame that changes). Whether it was drawn.
    pub(crate) fn show(&mut self, engine: Engine, frame: &HudFrame, mut fetch: impl FnMut(TextureId) -> Option<Rgba8>) -> bool {
        let (Some(huds), Some(images), Some(meshes), Some(paints), Some(materials)) = (
            self.huds.as_deref_mut(),
            self.images.as_deref_mut(),
            self.meshes.as_deref_mut(),
            self.paints.as_deref_mut(),
            self.materials.as_deref_mut(),
        ) else {
            return false;
        };
        let commands = &mut self.commands;
        let EngineHuds { white, free, engines } = huds;
        let drawn = engines.entry(engine).or_default();
        if drawn.shown && drawn.frame == *frame {
            return false;
        }
        let white = white.get_or_insert_with(|| images.add(image_of(Rgba8 { width: 1, height: 1, pixels: vec![255; 4] }))).clone();
        for draw in &frame.draws {
            let Some(id) = draw.texture else { continue };
            if drawn.textures.contains_key(&id) {
                continue;
            }
            if let Some(texture) = fetch(id) {
                drawn.textures.insert(id, images.add(image_of(texture)));
            }
        }
        // (the parts: a picture and a panel each, as the frame says)
        for (k, part) in frame.parts.iter().enumerate() {
            if k == drawn.parts.len() {
                let Some(layer) = free.pop() else {
                    warn!("{} HUD: no render layer left for its part {:?}", engine.name(), part.name);
                    break;
                };
                drawn.parts.push(PartSlot::spawn(commands, images, meshes, materials, engine, layer, part));
            }
            drawn.parts[k].update(commands, images, meshes, materials, frame.canvas, part);
        }
        for slot in drawn.parts.iter_mut().skip(frame.parts.len()) {
            slot.set_empty(commands);
        }
        // (the draws: a mesh each, in the engine's order - each over the last - on its part's layer)
        for (index, draw) in frame.draws.iter().enumerate() {
            let layer = drawn.parts.get(draw.part as usize).map(|p| p.layer);
            let picture = draw.texture.and_then(|id| drawn.textures.get(&id).cloned());
            let paint = HudPaint::of(draw.shading, draw.blend, picture, &white);
            let mesh = mesh_of(frame, index);
            if index == drawn.slots.len() {
                let mesh = meshes.add(mesh);
                let paint = paints.add(paint);
                let entity = commands
                    .spawn((
                        Mesh2d(mesh.clone()),
                        MeshMaterial2d(paint.clone()),
                        Transform::from_xyz(0.0, 0.0, index as f32 * 0.01),
                        Visibility::Hidden,
                        RenderLayers::layer(layer.unwrap_or(*LAYERS.end())),
                        Name::new(format!("{} HUD draw", engine.name())),
                        EngineOwned(engine),
                    ))
                    .id();
                drawn.slots.push(Slot { entity, mesh, paint, layer: layer.unwrap_or(*LAYERS.end()), visible: false });
            } else {
                let slot = &drawn.slots[index];
                if let Some(old) = meshes.get_mut(&slot.mesh) {
                    *old = mesh;
                }
                if paints.get(&slot.paint).is_some_and(|old| *old != paint)
                    && let Some(old) = paints.get_mut(&slot.paint)
                {
                    *old = paint;
                }
            }
            let slot = &mut drawn.slots[index];
            let mut entity = commands.entity(slot.entity);
            if let Some(layer) = layer.filter(|l| *l != slot.layer) {
                entity.insert(RenderLayers::layer(layer));
                slot.layer = layer;
            }
            if slot.visible != layer.is_some() {
                slot.visible = layer.is_some();
                entity.insert(if slot.visible { Visibility::Inherited } else { Visibility::Hidden });
            }
        }
        for slot in drawn.slots.iter_mut().skip(frame.draws.len()) {
            if std::mem::replace(&mut slot.visible, false) {
                commands.entity(slot.entity).insert(Visibility::Hidden);
            }
        }
        drawn.frame.clone_from(frame);
        drawn.shown = true;
        true
    }
}

impl PartSlot {
    fn spawn(
        commands: &mut Commands,
        images: &mut Assets<Image>,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<PanelMaterial>,
        engine: Engine,
        layer: usize,
        part: &HudPart,
    ) -> Self {
        let (size, resolution) = (part.size, part.resolution);
        let target = images.add(target_of(size, resolution));
        let camera = commands
            .spawn((
                EngineHudCamera,
                Camera2d,
                projection_of(size),
                Camera {
                    order: FIRST_ORDER - layer as isize,
                    clear_color: ClearColorConfig::Custom(Color::NONE),
                    is_active: false,
                    ..default()
                },
                Transform::default(),
                RenderTarget::Image(target.clone().into()),
                RenderLayers::layer(layer),
                Msaa::Off,
                Name::new(format!("{} HUD camera ({})", engine.name(), part.name)),
                EngineOwned(engine),
            ))
            .id();
        let panel_mesh = meshes.add(Rectangle::new(1.0, size.y / size.x));
        let panel_material = materials.add(PanelMaterial {
            base: StandardMaterial {
                base_color_texture: Some(target.clone()),
                unlit: true,
                // (drawn over a clear picture: its colour is already its coverage's)
                alpha_mode: AlphaMode::Premultiplied,
                double_sided: true,
                cull_mode: None,
                ..default()
            },
            extension: OnTop::default(),
        });
        let panel = commands
            .spawn((
                Mesh3d(panel_mesh.clone()),
                MeshMaterial3d(panel_material.clone()),
                Transform::default(),
                Visibility::Hidden,
                Name::new(format!("{} HUD ({})", engine.name(), part.name)),
                EngineOwned(engine),
            ))
            .id();
        let slot = Self { camera, panel, layer, target, size, resolution, panel_mesh, panel_material, placement: part.placement };
        slot.place(commands, part.empty, None);
        slot
    }

    /// The part as `part` is now: its picture's size, where its camera looks, its panel.
    fn update(
        &mut self,
        commands: &mut Commands,
        images: &mut Assets<Image>,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<PanelMaterial>,
        canvas: Vec2,
        part: &HudPart,
    ) {
        if (part.size, part.resolution) != (self.size, self.resolution) {
            (self.size, self.resolution) = (part.size, part.resolution);
            self.target = images.add(target_of(part.size, part.resolution));
            commands.entity(self.camera).insert((projection_of(part.size), RenderTarget::Image(self.target.clone().into())));
            if let Some(material) = materials.get_mut(&self.panel_material) {
                material.base.base_color_texture = Some(self.target.clone());
            }
            if let Some(mesh) = meshes.get_mut(&self.panel_mesh) {
                *mesh = Rectangle::new(1.0, part.size.y / part.size.x).into();
            }
        }
        commands.entity(self.camera).insert(view_at(part.view, canvas));
        let before = std::mem::replace(&mut self.placement, part.placement);
        self.place(commands, part.empty, Some(before));
    }

    /// Its panel's placement component, as its placement is (`before`: what it was).
    fn place(&self, commands: &mut Commands, empty: bool, before: Option<Placement>) {
        let mut panel = commands.entity(self.panel);
        let size = self.size;
        match self.placement {
            Placement::Hand { metres_per_px, offset, anchor, hand, on_left } => {
                if matches!(before, Some(Placement::AtPlayer { .. })) {
                    panel.remove::<AtPlayerHud>();
                }
                panel.insert(HandHud {
                    width_m: size.x * metres_per_px,
                    aspect: size.y / size.x,
                    offset,
                    anchor,
                    camera: Some(self.camera),
                    empty,
                    hand,
                    on_left,
                });
            }
            Placement::AtPlayer { .. } => {
                if matches!(before, Some(Placement::Hand { .. })) {
                    panel.remove::<HandHud>();
                }
                panel.insert(AtPlayerHud { camera: self.camera, empty, placement: self.placement });
            }
        }
    }

    fn set_empty(&self, commands: &mut Commands) {
        self.place(commands, true, Some(self.placement));
    }
}

/// A texture as an image.
fn image_of(texture: Rgba8) -> Image {
    Image::new(
        Extent3d { width: texture.width.max(1), height: texture.height.max(1), depth_or_array_layers: 1 },
        TextureDimension::D2,
        texture.pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// A part's picture: `size` canvas pixels at `resolution` pixels each.
fn target_of(size: Vec2, resolution: f32) -> Image {
    let [width, height] = (size * resolution).to_array().map(|v| (v as u32).max(1));
    Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None)
}

fn projection_of(size: Vec2) -> Projection {
    Projection::Orthographic(OrthographicProjection {
        scaling_mode: ScalingMode::Fixed { width: size.x, height: size.y },
        ..OrthographicProjection::default_2d()
    })
}

/// A part's camera looking at `at` on the canvas (its pixels, from its top left).
fn view_at(at: Vec2, canvas: Vec2) -> Transform {
    Transform::from_xyz(at.x - canvas.x / 2.0, canvas.y / 2.0 - at.y, 100.0)
}

/// A draw's mesh: its vertices on the canvas (its middle at the origin, y up), their texture coordinates and colours.
fn mesh_of(frame: &HudFrame, index: usize) -> Mesh {
    let vertices = frame.vertices_of(&frame.draws[index]);
    let [width, height] = frame.canvas.to_array();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vertices.iter().map(|v| [v.position.x - width / 2.0, height / 2.0 - v.position.y, 0.0]).collect::<Vec<_>>(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vertices.iter().map(|v| v.uv.to_array()).collect::<Vec<_>>());
    // (always: the shader reads them)
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vertices.iter().map(|v| v.color.map(|c| c as f32 / 255.0)).collect::<Vec<_>>());
    mesh
}

/// Where a part at the player goes and how wide (`placement`: its [`Placement::AtPlayer`]), turned to the eye: above
/// it, or beside it to the viewer's right, as clear of it as the engine says. As wide as `width_per_height` of its
/// drawn height, and never narrower than `least_view_share` of its distance from `eye`.
#[cfg(feature = "xr")]
pub(crate) fn at_player(focus: Focus, eye: Vec3, placement: Placement) -> Transform {
    let Placement::AtPlayer { side, width_per_height, least_view_share, middle, clear } = placement else {
        return Transform::from_translation(focus.feet);
    };
    let Focus { feet, up, height, .. } = focus;
    let middle = feet + up * (height * middle);
    let distance = eye.distance(middle);
    let width = (height * width_per_height).max(distance * least_view_share);
    let to_eye = (eye - middle).normalize_or(Vec3::Z);
    let right = Vec3::Y.cross(to_eye).normalize_or(Vec3::X);
    let at = match side {
        Side::Beside => middle + right * (height * clear.0 + width * clear.1),
        Side::Above => feet + up * height + Vec3::Y * (height * clear.0 + width * clear.1),
    };
    // (facing the eyes from where it is)
    let facing = (eye - at).normalize_or(to_eye);
    Transform::from_translation(at).looking_to(-facing, Vec3::Y).with_scale(Vec3::splat(width))
}

/// The running engine's parts at its player, while there's something to show (the HUD on, no menu open, the player
/// in the room); their cameras on only then.
#[cfg(feature = "xr")]
#[allow(clippy::type_complexity)]
fn place_at_player(
    settings: Option<Res<crate::xr::ar_settings::ArSettings>>,
    panel: Option<Res<crate::xr::panel::XrPanel>>,
    engines: Option<Res<crate::engines::Engines>>,
    focus: Res<engine_api::PlayerFocus>,
    eyes: Query<&GlobalTransform, With<bevy_mod_xr::camera::XrCamera>>,
    mut huds: Query<(&AtPlayerHud, &EngineOwned, &mut Transform, &mut Visibility)>,
    mut cameras: Query<&mut Camera>,
) {
    let running = engines.and_then(|e| e.current());
    let eye = eyes.iter().map(|g| g.translation()).reduce(|a, b| (a + b) * 0.5);
    let seen = settings.as_ref().is_some_and(|s| s.hud) && !panel.as_ref().is_some_and(|p| p.visible);
    for (hud, owner, mut transform, mut visibility) in &mut huds {
        // (another engine's is that engine's to hide: hidden while it isn't running; its camera stopped)
        let place = if Some(owner.0) != running {
            None
        } else {
            match (focus.of(owner.0), eye) {
                (Some(focus), Some(eye)) if seen && !hud.empty => Some(at_player(focus, eye, hud.placement)),
                _ => None,
            }
        };
        if Some(owner.0) == running {
            if let Some(place) = place {
                *transform = place;
            }
            visibility.set_if_neq(if place.is_some() { Visibility::Inherited } else { Visibility::Hidden });
        }
        if let Ok(mut camera) = cameras.get_mut(hud.camera)
            && camera.is_active != place.is_some()
        {
            camera.is_active = place.is_some();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A part at the player is over its head (above) or beside it, facing the eyes; close up it is as big as the
    /// player is drawn, far off it keeps a share of the view.
    #[cfg(feature = "xr")]
    #[test]
    fn a_part_at_the_player_is_there_and_never_too_small_to_read() {
        let (wide, share) = (1.2, 0.16);
        let place = |side: Side| Placement::AtPlayer {
            side,
            width_per_height: wide,
            least_view_share: share,
            middle: 0.55,
            clear: if side == Side::Beside { (0.3, 0.15) } else { (0.0, 0.2) },
        };
        let focus = |height: f32| Focus { feet: Vec3::ZERO, up: Vec3::Y, forward: Vec3::Z, height };
        let near = at_player(focus(1.8), Vec3::new(0.0, 1.6, 3.0), place(Side::Above));
        assert!(near.translation.y > 1.8, "over the head: {near:?}");
        assert!((near.scale.x - 1.8 * wide).abs() < 1e-4);
        let front = near.rotation * Vec3::Z;
        assert!(front.dot((Vec3::new(0.0, 1.6, 3.0) - near.translation).normalize()) > 0.95);
        // (a small player far off: the share of the view wins)
        let far_eye = Vec3::new(0.0, 1.0, 40.0);
        let far = at_player(focus(0.2), far_eye, place(Side::Above));
        assert!(far.scale.x >= 40.0 * share * 0.99, "{far:?}");
        assert!(far.translation.y > 0.2, "still over the head");
        // (beside: to the player's right as they look at it - here +x)
        let beside = at_player(focus(1.8), Vec3::new(0.0, 1.6, 3.0), place(Side::Beside));
        assert!(beside.translation.x > 0.5 && beside.translation.y < 1.8, "{beside:?}");
        // (exactly where the factors that were the renderer's - THUG2's meter: 0.55, 0.3, 0.15, 0.2 - put it)
        for (eye, height) in [(Vec3::new(0.0, 1.6, 3.0), 1.8), (far_eye, 0.2), (Vec3::new(2.3, 0.4, -1.7), 0.73)] {
            let f = Focus { feet: Vec3::new(0.3, 0.1, -0.2), up: Vec3::new(0.1, 1.0, 0.0).normalize(), forward: Vec3::Z, height };
            let middle = f.feet + f.up * (f.height * 0.55);
            let width = (f.height * wide).max(eye.distance(middle) * share);
            let right = Vec3::Y.cross((eye - middle).normalize_or(Vec3::Z)).normalize_or(Vec3::X);
            let was_beside = middle + right * (f.height * 0.3 + width * 0.15);
            let was_above = f.feet + f.up * f.height + Vec3::Y * (width * 0.2);
            assert_eq!(at_player(f, eye, place(Side::Beside)).translation, was_beside);
            assert_eq!(at_player(f, eye, place(Side::Above)).translation, was_above);
        }
    }

    /// Each shading and blend as the shader is told it.
    #[test]
    fn a_draw_tells_the_shader_its_shading_and_blend() {
        let white = Handle::<Image>::default();
        let gamma = HudPaint::of(Shading::Modulate { scale: 2.0, gamma: true }, HudBlend::Add, Some(white.clone()), &white);
        assert_eq!(gamma.params.mode, Vec4::new(2.0, 1.0, 0.0, 0.0));
        assert!(gamma.add);
        let plain = HudPaint::of(Shading::Modulate { scale: 1.0, gamma: false }, HudBlend::Opaque, None, &white);
        assert_eq!(plain.params.mode, Vec4::new(1.0, 0.0, 1.0, 1.0));
        assert!(!plain.add);
        let transform = HudPaint::of(
            Shading::Transform { multiply: [0.5, 1.0, 1.0, 0.25], add: [0.1, 0.0, 0.0, 0.0] },
            HudBlend::Alpha,
            Some(white.clone()),
            &white,
        );
        assert_eq!(transform.params.mode, Vec4::new(1.0, 1.0, 0.0, 2.0));
        assert_eq!(transform.params.multiply, Vec4::new(0.5, 1.0, 1.0, 0.25));
        assert_eq!(transform.params.add, Vec4::new(0.1, 0.0, 0.0, 0.0));
    }

    /// A draw's mesh is on the canvas with its middle at the origin, y up, its colours as given.
    #[test]
    fn a_draw_mesh_is_centred_on_the_canvas() {
        use engine_core::hud::{HudDraw, HudVertex};
        let frame = HudFrame {
            canvas: Vec2::new(640.0, 480.0),
            vertices: vec![
                HudVertex { position: Vec2::new(0.0, 0.0), uv: Vec2::ZERO, color: [255, 0, 51, 255] },
                HudVertex { position: Vec2::new(640.0, 0.0), uv: Vec2::X, color: [0; 4] },
                HudVertex { position: Vec2::new(640.0, 480.0), uv: Vec2::ONE, color: [0; 4] },
            ],
            draws: vec![HudDraw { first: 0, count: 3, ..default() }],
            parts: Vec::new(),
        };
        let mesh = mesh_of(&frame, 0);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!()
        };
        assert_eq!(positions[0], [-320.0, 240.0, 0.0]);
        assert_eq!(positions[2], [320.0, -240.0, 0.0]);
        let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else {
            panic!()
        };
        assert_eq!(colors[0], [1.0, 0.0, 0.2, 1.0]);
    }

    /// A picture of a part: `<frame>-<part>`, its size and its RGBA8 (sRGB) pixels.
    pub(crate) type Picture = (String, u32, u32, Vec<u8>);

    #[derive(Resource, Default)]
    struct TestFrame {
        engine: Option<Engine>,
        frame: HudFrame,
        textures: HashMap<TextureId, Rgba8>,
    }

    /// Draws `frames` (an engine's, each named) through the renderer as the app does, off screen, and takes a picture
    /// of each part of each: over nothing (clear), as the panels show them.
    pub(crate) fn pictures(engine: Engine, frames: &[(String, HudFrame)], textures: HashMap<TextureId, Rgba8>) -> Vec<Picture> {
        use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
        use std::sync::{Arc, Mutex};
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .build()
                .disable::<bevy::winit::WinitPlugin>()
                .set(WindowPlugin { primary_window: None, exit_condition: bevy::window::ExitCondition::DontExit, ..default() }),
        );
        install(&mut app);
        app.insert_resource(TestFrame { engine: Some(engine), frame: HudFrame::default(), textures })
            .add_systems(Update, |mut renderer: HudRenderer, test: Res<TestFrame>| {
                if let Some(engine) = test.engine {
                    renderer.show(engine, &test.frame, |id| test.textures.get(&id).cloned());
                }
            })
            // (every part's camera on: nothing places them here, and no engine runs)
            .add_systems(PostUpdate, on.after(bevy::transform::TransformSystems::Propagate));
        #[cfg(feature = "xr")]
        app.add_systems(PostUpdate, on.after(place_at_player));
        fn on(mut cameras: Query<&mut Camera, With<EngineHudCamera>>) {
            for mut camera in &mut cameras {
                camera.is_active = true;
            }
        }
        app.finish();
        app.cleanup();
        let taken: Arc<Mutex<Vec<Picture>>> = Arc::default();
        for (name, frame) in frames {
            app.world_mut().resource_mut::<TestFrame>().frame = frame.clone();
            for _ in 0..12 {
                app.update();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let targets: Vec<(String, Handle<Image>)> = {
                let world = app.world_mut();
                let mut cameras = world.query_filtered::<(&RenderTarget, &Name), With<EngineHudCamera>>();
                cameras
                    .iter(world)
                    .filter_map(|(target, camera)| {
                        let part = camera.as_str().rsplit_once('(')?.1.trim_end_matches(')').to_string();
                        match target {
                            RenderTarget::Image(image) => Some((part, image.handle.clone())),
                            _ => None,
                        }
                    })
                    .collect()
            };
            for (part, target) in targets {
                let (taken, name) = (taken.clone(), format!("{name}-{part}"));
                app.world_mut().spawn(Screenshot::image(target)).observe(move |shot: On<ScreenshotCaptured>| {
                    let image = &shot.image;
                    taken.lock().unwrap().push((name.clone(), image.width(), image.height(), image.data.clone().unwrap_or_default()));
                });
            }
            for _ in 0..12 {
                app.update();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        std::mem::take(&mut *taken.lock().unwrap())
    }

    /// Writes pictures as `<dir>/<name>.rgba` (its width and height, little-endian u32s, then its pixels) and a PNG
    /// to look at.
    pub(crate) fn save(dir: &std::path::Path, pictures: &[Picture]) {
        std::fs::create_dir_all(dir).unwrap();
        for (name, width, height, pixels) in pictures {
            let mut bytes = Vec::new();
            bytes.extend(width.to_le_bytes());
            bytes.extend(height.to_le_bytes());
            bytes.extend(pixels);
            std::fs::write(dir.join(format!("{name}.rgba")), bytes).unwrap();
            let image = Image::new(
                Extent3d { width: *width, height: *height, depth_or_array_layers: 1 },
                TextureDimension::D2,
                pixels.clone(),
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::MAIN_WORLD,
            );
            if let Ok(picture) = image.try_into_dynamic() {
                let _ = picture.to_rgba8().save(dir.join(format!("{name}.png")));
            }
        }
    }

    fn srgb_decode(c: f64) -> f64 {
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    }

    fn srgb_encode(c: f64) -> f64 {
        if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
    }

    /// Each shading and blend drawn on the GPU, against its maths worked here: a texel and a vertex colour of known
    /// values, over nothing and over an opaque background (a 16 px part: its middle pixel read).
    #[test]
    #[ignore = "needs a GPU"]
    fn each_shading_and_blend_draws_as_its_maths_say() {
        use engine_core::hud::{HudDraw, HudVertex};
        let texel = [200u8, 100, 50, 180];
        let vertex = [230u8, 128, 64, 200];
        let background = [20u8, 60, 140, 255];
        let quad = |frame: &mut HudFrame, color: [u8; 4], texture: Option<TextureId>, blend: HudBlend, shading: Shading| {
            let first = frame.vertices.len() as u32;
            let corners = [Vec2::new(0.0, 0.0), Vec2::new(16.0, 0.0), Vec2::new(16.0, 16.0), Vec2::new(0.0, 16.0)];
            for i in [0, 1, 2, 0, 2, 3] {
                frame.vertices.push(HudVertex { position: corners[i], uv: corners[i] / 16.0, color });
            }
            frame.draws.push(HudDraw { first, count: 6, texture, part: 0, blend, shading });
        };
        let part = HudPart {
            name: "case".into(),
            size: Vec2::splat(16.0),
            resolution: 1.0,
            view: Vec2::splat(8.0),
            empty: false,
            placement: Placement::Hand { metres_per_px: 0.001, offset: Vec2::ZERO, anchor: Vec2::ZERO, hand: None, on_left: None },
        };
        let gamma2 = Shading::Modulate { scale: 2.0, gamma: true };
        let linear = Shading::Modulate { scale: 1.0, gamma: false };
        let transform = Shading::Transform { multiply: [0.8, 1.2, 0.5, 0.9], add: [0.05, -0.1, 0.2, 0.0] };
        // (name, over the background, textured, blend, shading)
        let cases = [
            ("gamma-alpha", false, true, HudBlend::Alpha, gamma2),
            ("gamma-alpha-untextured", false, false, HudBlend::Alpha, gamma2),
            ("gamma-add-over", true, true, HudBlend::Add, gamma2),
            ("gamma-add-nothing", false, true, HudBlend::Add, gamma2),
            ("gamma-opaque", false, true, HudBlend::Opaque, Shading::Modulate { scale: 1.0, gamma: true }),
            ("linear-alpha", false, true, HudBlend::Alpha, linear),
            ("linear-alpha-over", true, true, HudBlend::Alpha, linear),
            ("transform-alpha", false, true, HudBlend::Alpha, transform),
            ("transform-add-over", true, true, HudBlend::Add, transform),
            ("transform-opaque", false, true, HudBlend::Opaque, transform),
        ];
        let frames: Vec<(String, HudFrame)> = cases
            .iter()
            .map(|&(name, over, textured, blend, shading)| {
                let mut frame = HudFrame { canvas: Vec2::splat(16.0), parts: vec![part.clone()], ..default() };
                if over {
                    quad(&mut frame, background, None, HudBlend::Opaque, Shading::Modulate { scale: 1.0, gamma: false });
                }
                quad(&mut frame, vertex, textured.then_some(7), blend, shading);
                (name.to_string(), frame)
            })
            .collect();
        let textures = HashMap::from([(7, Rgba8 { width: 1, height: 1, pixels: texel.to_vec() })]);
        let pictures = pictures(Engine::Thug2, &frames, textures);
        assert_eq!(pictures.len(), cases.len());
        let unit = |c: u8| c as f64 / 255.0;
        let t_lin = [0, 1, 2].map(|i| srgb_decode(unit(texel[i])));
        let mut worst = 0.0f64;
        for (name, over, textured, blend, shading) in cases {
            // (the draw's colour, linear, and its alpha)
            let (rgb, alpha): ([f64; 3], f64) = match shading {
                Shading::Modulate { scale, gamma: true } => {
                    let (mut c, mut a) = ([0, 1, 2].map(|i| unit(vertex[i])), unit(vertex[3]));
                    if textured {
                        c = [0, 1, 2].map(|i| c[i] * t_lin[i].powf(1.0 / 2.2) * scale as f64);
                        a = a * unit(texel[3]) * scale as f64;
                    }
                    (c.map(|c| c.clamp(0.0, 1.0).powf(2.2)), a.clamp(0.0, 1.0))
                }
                Shading::Modulate { scale, gamma: false } => {
                    let c = [0, 1, 2].map(|i| srgb_decode(unit(vertex[i])) * t_lin[i] * scale as f64);
                    (c.map(|c| c.clamp(0.0, 1.0)), (unit(vertex[3]) * unit(texel[3]) * scale as f64).clamp(0.0, 1.0))
                }
                Shading::Transform { multiply, add } => {
                    let c = [0, 1, 2].map(|i| (t_lin[i] * multiply[i] as f64 + add[i] as f64).clamp(0.0, 1.0));
                    (c, (unit(texel[3]) * multiply[3] as f64 + add[3] as f64).clamp(0.0, 1.0))
                }
            };
            let alpha = if blend == HudBlend::Opaque { 1.0 } else { alpha };
            let (under, under_alpha) = if over { ([0, 1, 2].map(|i| srgb_decode(unit(background[i]))), 1.0) } else { ([0.0; 3], 0.0) };
            let (out, out_alpha) = match blend {
                HudBlend::Add => ([0, 1, 2].map(|i| under[i] + rgb[i] * alpha), under_alpha),
                _ => ([0, 1, 2].map(|i| rgb[i] * alpha + under[i] * (1.0 - alpha)), alpha + under_alpha * (1.0 - alpha)),
            };
            let want = [srgb_encode(out[0].min(1.0)), srgb_encode(out[1].min(1.0)), srgb_encode(out[2].min(1.0)), out_alpha].map(|c| c * 255.0);
            let (_, width, _, pixels) = pictures.iter().find(|p| p.0 == format!("{name}-case")).expect(name);
            let at = ((8 * width + 8) * 4) as usize;
            let got = &pixels[at..at + 4];
            for i in 0..4 {
                let off = (got[i] as f64 - want[i]).abs();
                worst = worst.max(off);
                assert!(off <= 1.5, "{name}: got {got:?}, the maths {want:?}");
            }
        }
        eprintln!("each shading and blend: at most {worst:.2} of 255 off its maths");
    }

    /// THUG2's HUD frames recorded from the game (`<dir>/rec`: game data, kept out of the repo) drawn through this
    /// renderer into `<dir>/new`, to compare with the old path's pictures: `SK8_THUG2_HUD_REPLAY=<dir>`.
    #[cfg(feature = "thug2")]
    #[test]
    #[ignore = "needs a GPU and recorded THUG2 HUD frames"]
    fn thug2_recorded_frames_drawn() {
        use thug2_sys as t2;
        let Some(dir) = std::env::var_os("SK8_THUG2_HUD_REPLAY").map(std::path::PathBuf::from) else { return };
        let mut frames = Vec::new();
        let mut textures = HashMap::new();
        let mut names: Vec<_> = std::fs::read_dir(dir.join("rec")).unwrap().flatten().map(|e| e.path()).collect();
        names.sort();
        for path in names {
            let file = path.file_name().unwrap().to_string_lossy().into_owned();
            let bytes = std::fs::read(&path).unwrap();
            if let Some(id) = file.strip_prefix("tex-").and_then(|f| f.strip_suffix(".bin")) {
                let width = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
                let height = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
                let texture = t2::Texture { width, height, rgba: bytes[8..].to_vec() };
                textures.insert(thug2_engine::hud::texture_id(id.parse().unwrap()), thug2_engine::hud::texture(texture));
                continue;
            }
            let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let f = |v: &serde_json::Value| v.as_f64().unwrap() as f32;
            let hud = t2::Hud {
                draws: json["draws"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|d| t2::HudDraw {
                        first_vertex: d[0].as_u64().unwrap() as u32,
                        vertex_count: d[1].as_u64().unwrap() as u32,
                        texture: d[2].as_i64().map(|t| t as i32),
                        blend: [t2::HudBlend::Alpha, t2::HudBlend::Add, t2::HudBlend::Opaque][d[3].as_u64().unwrap() as usize],
                        doubled: d[4].as_bool().unwrap(),
                    })
                    .collect(),
                vertices: json["vertices"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| t2::HudVertex {
                        position: [f(&v[0][0]), f(&v[0][1])],
                        uv: [f(&v[1][0]), f(&v[1][1])],
                        color: [0, 1, 2, 3].map(|i| v[2][i].as_u64().unwrap() as u8),
                    })
                    .collect(),
            };
            let mut frame = HudFrame::default();
            thug2_engine::hud::frame(&hud, json["balancing"].as_bool().unwrap(), &mut frame);
            frames.push((file.trim_end_matches(".json").to_string(), frame));
        }
        let pictures = pictures(Engine::Thug2, &frames, textures);
        save(&dir.join("new"), &pictures);
    }
}
