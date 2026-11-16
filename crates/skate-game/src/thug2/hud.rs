//! THUG2's own HUD (the score and special meter, the trick string, its balance meter, goal messages): the game's 2D as
//! the harness takes it down each frame (`thug2_sys::Engine::hud`), drawn as the game draws it, and shown by a
//! controller (`xr::hand_hud`) rearranged: the game scatters it over its 4:3 screen, so each part ([`Part`]) is drawn
//! into a picture of its own and has a panel of its own. The balance meter is at the skater, as in the game, turned
//! to the eyes: a grind's or lip's arc over its head, a manual's upright meter beside it - as big as the skater is
//! drawn, and never smaller than a share of the view, so it reads from across the room and with the skater small
//! ([`place_meter`]). The score panel and the trick string are by the settings' controller: the score just over it (or above the
//! meter, when that's the left one too), the trick string (which grows long) to its left, its end by the controller.
//!
//! The game draws its 2D in Direct3D 9: each draw a triangle list of screen vertices with one texture, its colour the
//! texture's times the vertices' doubled, blended over what's under it (or added). Here each draw is a 2D mesh with a
//! material of that ([`HudPaint`], [`HudGlow`] for the added ones), laid over the last in the game's order.
use super::{Phase, Thug2};
use crate::xr::ar_settings::{ArSettings, HudHand};
use crate::xr::panel::XrPanel;
use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{RenderTarget, ScalingMode};
use bevy::mesh::{MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, Extent3d, RenderPipelineDescriptor,
    SpecializedMeshPipelineError, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPlugin, MeshMaterial2d};
use std::collections::HashMap;
use thug2_sys as t2;

/// A part of the HUD, with a panel of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Part {
    /// The score and the special meter: the screen's top left.
    Score,
    /// The balance meter (grind, lip: an arc over the middle; manual: one upright, left of it).
    Meter,
    /// The trick string, its points and multiplier (the bottom), and whatever else the game shows (messages, the
    /// freak-out meter after a bail).
    Trick,
}

impl Part {
    const ALL: [Part; 3] = [Part::Score, Part::Meter, Part::Trick];

    /// The layer its draws and its camera are on (nothing else's).
    fn layer(self) -> usize {
        match self {
            Part::Score => 28,
            Part::Meter => 29,
            Part::Trick => 30,
        }
    }

    /// The part of the screen its picture shows, pixels.
    fn size(self) -> Vec2 {
        match self {
            Part::Score => Vec2::new(220.0, 110.0),
            Part::Meter => Vec2::new(200.0, 160.0),
            Part::Trick => Vec2::new(640.0, 260.0),
        }
    }

    /// Its panel: real metres a screen pixel, and where it is from the grip (`xr::hand_hud::HandHud`'s offset and
    /// anchor); the meter's for an arc ([`Part::upright_meter`] for a manual's).
    fn panel(self) -> (f32, Vec2, Vec2) {
        match self {
            // (its bottom middle just over the grip)
            Part::Score => (0.00055, Vec2::new(0.0, 0.07), Vec2::new(0.0, -0.5)),
            // (the meter is at the skater: `place_meter`)
            Part::Meter => (0.0, Vec2::ZERO, Vec2::ZERO),
            // (its right edge, the string's end, left of the controller)
            Part::Trick => (0.0006, Vec2::new(-0.07, 0.03), Vec2::new(0.5, 0.0)),
        }
    }

    /// The panel's hand of its own (the meter: the left, the steering hand), and where it is (offset, anchor) when it
    /// shares the left controller with the meter (the score: its bottom middle above it).
    fn hand(self) -> (Option<HudHand>, Option<(Vec2, Vec2)>) {
        match self {
            Part::Meter => (None, None),
            Part::Score => (None, Some((Vec2::new(0.0, 0.135), Vec2::new(0.0, -0.5)))),
            Part::Trick => (None, None),
        }
    }

    /// What's drawn is an upright meter (taller than wide), not an arc.
    fn upright(drawn: Option<(Vec2, Vec2)>) -> bool {
        drawn.is_some_and(|(min, max)| max.y - min.y > max.x - min.x)
    }

    /// Which part a draw is, by where it is on the screen (`bounds`: its left top and right bottom, pixels), and
    /// whether the skater is balancing (only then is a small piece over the middle the balance meter's: otherwise it is
    /// the freak-out meter's, or a message's, and goes with the rest).
    fn of(bounds: (Vec2, Vec2), balancing: bool) -> Part {
        let (min, max) = bounds;
        let middle = (min + max) / 2.0;
        let size = max - min;
        if middle.y < 90.0 && middle.x < 330.0 {
            Part::Score
        } else if balancing && middle.y < 330.0 && (160.0..480.0).contains(&middle.x) && size.max_element() < 180.0 {
            Part::Meter
        } else {
            Part::Trick
        }
    }

    /// Where its camera looks (the screen's pixels): the score where the game puts it; the meter centred on what's
    /// drawn; the trick string from its left, centred up and down.
    fn view(self, drawn: Option<(Vec2, Vec2)>) -> Vec2 {
        let size = self.size();
        match (self, drawn) {
            (Part::Score, _) | (_, None) => size / 2.0,
            (Part::Meter, Some((min, max))) => (min + max) / 2.0,
            (Part::Trick, Some((min, max))) => Vec2::new(max.x - size.x / 2.0, (min.y + max.y) / 2.0),
        }
    }
}

/// A draw blended over what's under it (or opaque).
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub(super) struct HudPaint {
    /// x: the factor (2: doubled), y: textured, z: opaque.
    #[uniform(0)]
    mode: Vec4,
    #[texture(1)]
    #[sampler(2)]
    picture: Handle<Image>,
}

/// A draw added to what's under it.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub(super) struct HudGlow {
    #[uniform(0)]
    mode: Vec4,
    #[texture(1)]
    #[sampler(2)]
    picture: Handle<Image>,
}

fn shader() -> ShaderRef {
    concat!("embedded://", env!("CARGO_CRATE_NAME"), "/thug2/hud.wgsl").into()
}

impl Material2d for HudPaint {
    fn fragment_shader() -> ShaderRef {
        shader()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

impl Material2d for HudGlow {
    fn fragment_shader() -> ShaderRef {
        shader()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
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

/// The HUD's draws as drawn, and the game's textures as images.
#[derive(Resource)]
pub(super) struct Thug2Hud {
    white: Handle<Image>,
    textures: HashMap<i32, Handle<Image>>,
    /// Each draw's mesh entity, reused frame to frame with its mesh and materials.
    slots: Vec<Slot>,
    /// Each part's camera and panel.
    parts: Vec<(Part, Entity, Entity)>,
    drawn: t2::Hud,
}

struct Slot {
    entity: Entity,
    mesh: Handle<Mesh>,
    paint: Option<Handle<HudPaint>>,
    glow: Option<Handle<HudGlow>>,
}

/// THUG2's HUD: its materials (for every engine: the plugins aren't an engine's), then its systems in THUG2's.
pub(super) fn install(app: &mut App) {
    if !app.is_plugin_added::<bevy::render::RenderPlugin>() {
        return;
    }
    embedded_asset!(app, "hud.wgsl");
    app.add_plugins((Material2dPlugin::<HudPaint>::default(), Material2dPlugin::<HudGlow>::default()));
}

/// Whether the app draws THUG2's HUD (not without a renderer: tests).
pub(super) fn drawn(app: &App) -> bool {
    app.is_plugin_added::<bevy::render::RenderPlugin>()
}

pub(super) fn spawn(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let white = images.add(Image::new(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![255; 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ));
    let mut parts = Vec::new();
    for (order, part) in Part::ALL.into_iter().enumerate() {
        let size = part.size();
        let target = images.add(Image::new_target_texture(size.x as u32, size.y as u32, TextureFormat::Rgba8UnormSrgb, None));
        let camera = commands
            .spawn((
                engine_api::EngineHudCamera,
                Camera2d,
                Projection::Orthographic(OrthographicProjection {
                    scaling_mode: ScalingMode::Fixed { width: size.x, height: size.y },
                    ..OrthographicProjection::default_2d()
                }),
                Camera {
                    order: -13 - order as isize,
                    clear_color: ClearColorConfig::Custom(Color::NONE),
                    is_active: false,
                    ..default()
                },
                view_at(part.view(None)),
                RenderTarget::Image(target.clone().into()),
                RenderLayers::layer(part.layer()),
                Msaa::Off,
                Name::new(format!("THUG2 HUD camera ({part:?})")),
            ))
            .id();
        let (metres_per_pixel, offset, anchor) = part.panel();
        let mut panel = commands.spawn_empty();
        if part == Part::Meter {
            panel.insert(SkaterMeter { camera, empty: true, upright: false });
        } else {
            panel.insert(crate::xr::hand_hud::HandHud {
                    width_m: size.x * metres_per_pixel,
                    aspect: size.y / size.x,
                    offset,
                    anchor,
                    camera: Some(camera),
                    empty: true,
                    hand: part.hand().0,
                    on_left: part.hand().1,
            });
        }
        let panel = panel
            .insert((
                Mesh3d(meshes.add(Rectangle::new(1.0, size.y / size.x))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color_texture: Some(target),
                    unlit: true,
                    // (drawn over a clear picture: its colour is already its coverage's)
                    alpha_mode: AlphaMode::Premultiplied,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                })),
                Transform::default(),
                Visibility::Hidden,
                Name::new(format!("THUG2 HUD ({part:?})")),
            ))
            .id();
        parts.push((part, camera, panel));
    }
    commands.insert_resource(Thug2Hud { white, textures: HashMap::new(), slots: Vec::new(), drawn: t2::Hud::default(), parts });
}

/// The balance meter's panel, at the skater (not a controller's): its camera, and what it shows now (nothing; an upright meter, a manual's, or an arc).
#[derive(Component)]
pub(super) struct SkaterMeter {
    camera: Entity,
    empty: bool,
    upright: bool,
}

/// The meter panel's width against the skater's height (as it is drawn), and the least share of its distance from
/// the eyes it may be (about 9 degrees across: the meter itself some 6, legible from across a room and with the
/// skater small).
const METER_PER_SKATER: f32 = 1.2;
const METER_LEAST_SHARE: f32 = 0.16;
/// The skater's height, the game's metres.
const SKATER_HEIGHT: f32 = 1.8;

/// Where the meter panel goes and how wide: turned to the eye, an arc's over the head (clear of it, however big the
/// panel), an upright one beside it to the player's right. `feet` and `up` the skater's; `height` how tall it's drawn.
pub(super) fn meter_place(feet: Vec3, up: Vec3, height: f32, eye: Vec3, upright: bool) -> Transform {
    let middle = feet + up * (height * 0.55);
    let distance = eye.distance(middle);
    let width = (height * METER_PER_SKATER).max(distance * METER_LEAST_SHARE);
    let to_eye = (eye - middle).normalize_or(Vec3::Z);
    let right = Vec3::Y.cross(to_eye).normalize_or(Vec3::X);
    let at = if upright {
        // (the upright meter is about a fifth of the panel across: just clear of the skater's side)
        middle + right * (height * 0.3 + width * 0.15)
    } else {
        // (the arc is about a third of the panel high, its middle the panel's)
        feet + up * height + Vec3::Y * (width * 0.2)
    };
    // (facing the eyes from where it is)
    let facing = (eye - at).normalize_or(to_eye);
    Transform::from_translation(at).looking_to(-facing, Vec3::Y).with_scale(Vec3::splat(width))
}

/// The balance meter at the skater while it balances (the HUD on, no menu open); its camera on only then.
#[allow(clippy::type_complexity)]
pub(super) fn place_meter(
    thug2: Res<Thug2>,
    settings: Option<Res<ArSettings>>,
    panel: Option<Res<XrPanel>>,
    eyes: Query<&GlobalTransform, With<bevy_mod_xr::camera::XrCamera>>,
    mut meters: Query<(&SkaterMeter, &mut Transform, &mut Visibility)>,
    mut cameras: Query<&mut Camera>,
) {
    let eye = eyes.iter().map(|g| g.translation()).reduce(|a, b| (a + b) * 0.5);
    let skater = thug2.current.filter(|_| thug2.placed);
    for (meter, mut transform, mut visibility) in &mut meters {
        let seen = settings.as_ref().is_some_and(|s| s.hud) && !panel.as_ref().is_some_and(|p| p.visible);
        let place = match (skater, eye) {
            (Some(s), Some(eye)) if seen && !meter.empty => {
                let scale = thug2.scale_given.max(1e-3);
                Some(meter_place(super::from_game(s.position, scale), Vec3::from_array(s.up).normalize_or(Vec3::Y), SKATER_HEIGHT / scale, eye, meter.upright))
            }
            _ => None,
        };
        if let Some(place) = place {
            *transform = place;
        }
        visibility.set_if_neq(if place.is_some() { Visibility::Inherited } else { Visibility::Hidden });
        if let Ok(mut camera) = cameras.get_mut(meter.camera) {
            if camera.is_active != place.is_some() {
                camera.is_active = place.is_some();
            }
        }
    }
}

/// A part's camera looking at `at` on the screen (its pixels, from its top left).
fn view_at(at: Vec2) -> Transform {
    let [width, height] = t2::HUD_SIZE;
    Transform::from_xyz(at.x - width / 2.0, height / 2.0 - at.y, 100.0)
}

/// The game takes its 2D down only while it's seen: the HUD on and no menu over it.
pub(super) fn want(settings: Option<Res<ArSettings>>, panel: Option<Res<XrPanel>>, hud: Option<Res<Thug2Hud>>, mut thug2: ResMut<Thug2>) {
    let wanted = hud.is_some() && settings.is_some_and(|s| s.hud) && !panel.is_some_and(|p| p.visible);
    let thug2 = &mut *thug2;
    let Phase::Running(engine) = &mut thug2.phase else { return };
    if thug2.hud_on != wanted {
        engine.hud_enable(wanted);
        thug2.hud_on = wanted;
        thug2.hud = t2::Hud::default();
    }
}

/// The draws as meshes: one a draw, in its order (later ones over earlier), with the game's textures.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    mut commands: Commands,
    hud: Option<ResMut<Thug2Hud>>,
    mut thug2: ResMut<Thug2>,
    mut panels: Query<&mut crate::xr::hand_hud::HandHud>,
    mut meters: Query<&mut SkaterMeter>,
    mut views: Query<&mut Transform, With<Camera2d>>,
    images: Option<ResMut<Assets<Image>>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    paints: Option<ResMut<Assets<HudPaint>>>,
    glows: Option<ResMut<Assets<HudGlow>>>,
) {
    let (Some(mut hud), Some(mut images), Some(mut meshes), Some(mut paints), Some(mut glows)) = (hud, images, meshes, paints, glows)
    else {
        return;
    };
    let thug2 = &mut *thug2;
    if hud.drawn == thug2.hud {
        return;
    }
    let Phase::Running(engine) = &mut thug2.phase else { return };
    let frame = &thug2.hud;
    let balancing = thug2.current.is_some_and(|s| s.balance.is_some());
    for draw in &frame.draws {
        let Some(id) = draw.texture else { continue };
        if hud.textures.contains_key(&id) {
            continue;
        }
        let Some(texture) = engine.hud_texture(id) else { continue };
        let image = Image::new(
            Extent3d { width: texture.width.max(1), height: texture.height.max(1), depth_or_array_layers: 1 },
            TextureDimension::D2,
            texture.rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        let handle = images.add(image);
        hud.textures.insert(id, handle);
    }
    let [width, height] = t2::HUD_SIZE;
    // (what each part has drawn: its bounds on the screen)
    let mut drawn: [Option<(Vec2, Vec2)>; 3] = [None; 3];
    for (index, draw) in frame.draws.iter().enumerate() {
        let vertices = frame.vertices.get(draw.first_vertex as usize..(draw.first_vertex + draw.vertex_count) as usize).unwrap_or(&[]);
        let bounds = vertices.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(min, max), v| {
            let p = Vec2::from_array(v.position);
            (min.min(p), max.max(p))
        });
        let part = Part::of(bounds, balancing);
        let k = Part::ALL.iter().position(|p| *p == part).unwrap_or(0);
        if !vertices.is_empty() {
            drawn[k] = Some(drawn[k].map_or(bounds, |(min, max)| (min.min(bounds.0), max.max(bounds.1))));
        }
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD);
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vertices.iter().map(|v| [v.position[0] - width / 2.0, height / 2.0 - v.position[1], 0.0]).collect::<Vec<_>>(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vertices.iter().map(|v| v.uv).collect::<Vec<_>>());
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            vertices.iter().map(|v| v.color.map(|c| c as f32 / 255.0)).collect::<Vec<_>>(),
        );
        let picture = draw.texture.and_then(|id| hud.textures.get(&id).cloned());
        let mode = Vec4::new(
            if draw.doubled { 2.0 } else { 1.0 },
            if picture.is_some() { 1.0 } else { 0.0 },
            if draw.blend == t2::HudBlend::Opaque { 1.0 } else { 0.0 },
            0.0,
        );
        let picture = picture.unwrap_or_else(|| hud.white.clone());
        if index == hud.slots.len() {
            let handle = meshes.add(mesh);
            let entity = commands.spawn((Mesh2d(handle.clone()), RenderLayers::layer(Part::Trick.layer()), Name::new("THUG2 HUD draw"))).id();
            hud.slots.push(Slot { entity, mesh: handle, paint: None, glow: None });
        } else if let Some(old) = meshes.get_mut(&hud.slots[index].mesh) {
            *old = mesh;
        }
        let slot = &mut hud.slots[index];
        let mut entity = commands.entity(slot.entity);
        // (in the game's order: each over the last)
        entity.insert((Transform::from_xyz(0.0, 0.0, index as f32 * 0.01), Visibility::Inherited, RenderLayers::layer(part.layer())));
        if draw.blend == t2::HudBlend::Add {
            match &slot.glow {
                Some(h) => {
                    if let Some(m) = glows.get_mut(h) {
                        *m = HudGlow { mode, picture };
                    }
                }
                None => slot.glow = Some(glows.add(HudGlow { mode, picture })),
            }
            entity.remove::<MeshMaterial2d<HudPaint>>().insert(MeshMaterial2d(slot.glow.clone().unwrap()));
        } else {
            match &slot.paint {
                Some(h) => {
                    if let Some(m) = paints.get_mut(h) {
                        *m = HudPaint { mode, picture };
                    }
                }
                None => slot.paint = Some(paints.add(HudPaint { mode, picture })),
            }
            entity.remove::<MeshMaterial2d<HudGlow>>().insert(MeshMaterial2d(slot.paint.clone().unwrap()));
        }
    }
    for slot in &hud.slots[frame.draws.len().min(hud.slots.len())..] {
        commands.entity(slot.entity).insert(Visibility::Hidden);
    }
    for (k, (part, camera, panel)) in hud.parts.iter().enumerate() {
        if let Ok(mut view) = views.get_mut(*camera) {
            *view = view_at(part.view(drawn[k]));
        }
        if let Ok(mut panel) = panels.get_mut(*panel) {
            panel.empty = drawn[k].is_none();
        }
        if let Ok(mut meter) = meters.get_mut(*panel) {
            meter.empty = drawn[k].is_none();
            meter.upright = Part::upright(drawn[k]);
        }
    }
    hud.drawn = frame.clone();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The game's 2D parts, where it draws them (thug2-drive's captures): the score panel's pieces, the grind and
    /// manual meters' arcs and markers, the trick string.
    #[test]
    fn draws_go_to_their_parts_by_where_the_game_puts_them() {
        let at = |x0: f32, y0: f32, x1: f32, y1: f32| Part::of((Vec2::new(x0, y0), Vec2::new(x1, y1)), true);
        assert_eq!(at(9.0, 27.0, 195.0, 90.0), Part::Score);
        assert_eq!(at(136.0, 34.0, 195.0, 60.0), Part::Score);
        // (the grind arc and its marker)
        assert_eq!(at(256.0, 145.0, 384.0, 197.0), Part::Meter);
        assert_eq!(at(313.0, 144.0, 329.0, 164.0), Part::Meter);
        // (the manual's upright arc and marker, left of the middle)
        assert_eq!(at(233.0, 175.0, 266.0, 303.0), Part::Meter);
        assert_eq!(at(215.0, 233.0, 240.0, 250.0), Part::Meter);
        // (the trick string and its points)
        assert_eq!(at(250.0, 392.0, 376.0, 408.0), Part::Trick);
        assert_eq!(at(282.0, 372.0, 358.0, 388.0), Part::Trick);
        // (a message across the middle of the screen)
        assert_eq!(at(60.0, 200.0, 580.0, 260.0), Part::Trick);
        // (after a bail, not balancing: the freak-out meter's fist, ticks and bar stay together with the rest)
        let bailed = |x0: f32, y0: f32, x1: f32, y1: f32| Part::of((Vec2::new(x0, y0), Vec2::new(x1, y1)), false);
        assert_eq!(bailed(205.0, 100.0, 245.0, 160.0), Part::Trick);
        assert_eq!(bailed(324.0, 136.0, 330.0, 156.0), Part::Trick);
        assert_eq!(bailed(224.0, 136.0, 432.0, 158.0), Part::Trick);
        assert_eq!(bailed(9.0, 27.0, 195.0, 90.0), Part::Score);
    }

    /// The meter is over the skater's head (an arc) or beside it (upright), facing the eyes; close up it is as big as
    /// the skater is drawn, far off it keeps a share of the view.
    #[test]
    fn the_meter_is_at_the_skater_and_never_too_small_to_read() {
        let feet = Vec3::ZERO;
        let near = meter_place(feet, Vec3::Y, 1.8, Vec3::new(0.0, 1.6, 3.0), false);
        assert!(near.translation.y > 1.8, "over the head: {near:?}");
        assert!((near.scale.x - 1.8 * METER_PER_SKATER).abs() < 1e-4);
        let front = near.rotation * Vec3::Z;
        assert!(front.dot((Vec3::new(0.0, 1.6, 3.0) - near.translation).normalize()) > 0.95);
        // (a small skater far off: the share of the view wins)
        let far_eye = Vec3::new(0.0, 1.0, 40.0);
        let far = meter_place(feet, Vec3::Y, 0.2, far_eye, false);
        assert!(far.scale.x >= 40.0 * METER_LEAST_SHARE * 0.99, "{far:?}");
        assert!(far.translation.y > 0.2, "still over the head");
        // (upright: beside, to the player's right as they look at it - here +x)
        let beside = meter_place(feet, Vec3::Y, 1.8, Vec3::new(0.0, 1.6, 3.0), true);
        assert!(beside.translation.x > 0.5 && beside.translation.y < 1.8, "{beside:?}");
    }

    /// The trick string is seen from its end; the meter from its middle; the score where the game puts it.
    #[test]
    fn each_part_is_looked_at_where_it_is() {
        let string = (Vec2::new(250.0, 370.0), Vec2::new(376.0, 410.0));
        assert_eq!(Part::Trick.view(Some(string)), Vec2::new(376.0 - 320.0, 390.0));
        let arc = (Vec2::new(256.0, 145.0), Vec2::new(384.0, 197.0));
        assert_eq!(Part::Meter.view(Some(arc)), Vec2::new(320.0, 171.0));
        assert!(!Part::upright(Some(arc)));
        // (a manual's: its left side at the panel's left, by the controller)
        let manual = (Vec2::new(215.0, 175.0), Vec2::new(266.0, 303.0));
        assert!(Part::upright(Some(manual)));
        assert_eq!(Part::Meter.view(Some(manual)), Vec2::new(240.5, 239.0));
        assert_eq!(Part::Score.view(Some(arc)), Vec2::new(110.0, 55.0));
    }

    /// The HUD mid-grind as the app draws it, saved as a picture to look at:
    /// `THUG2_GAME_DIR=<folder holding Data> SK8_THUG2_HUD_SHOT=/path/hud.png cargo test ... thug2::hud::tests --
    /// --ignored` (alone: the game boots once a process).
    #[test]
    #[ignore = "needs a GPU and the player's THUG2 files; writes a picture"]
    fn a_picture_of_the_hud() {
        use bevy::render::view::screenshot::{Screenshot, save_to_disk};
        let (Some(path), Some(dir)) = (std::env::var_os("SK8_THUG2_HUD_SHOT"), std::env::var_os("THUG2_GAME_DIR")) else {
            return;
        };
        let cache = std::env::temp_dir().join("thug2-hud-shot-cache");
        let mut engine = t2::boot(Path::new(&dir), Some(&cache), &["-level", "Default"]).expect("boot");
        for _ in 0..120 {
            engine.step(&t2::PAD_AT_REST).expect("step");
        }
        // (thug2-drive's test run: a floor and a rail, pushed along, an ollie onto the rail and a nosegrind)
        let (cx, cz) = (50_000.0, 50_000.0);
        let quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]| {
            [
                t2::Triangle { v: [a, b, c], flags: t2::face::SKATABLE, terrain: 0 },
                t2::Triangle { v: [a, c, d], flags: t2::face::SKATABLE, terrain: 0 },
            ]
        };
        let floor = quad([cx - 400.0, 0.0, cz - 400.0], [cx - 400.0, 0.0, cz + 400.0], [cx + 400.0, 0.0, cz + 400.0], [cx + 400.0, 0.0, cz - 400.0]);
        engine.set_world(&floor, Some(t2::Spawn { point: [cx, 20.0, cz - 300.0], heading_degrees: 0.0 }));
        engine.set_rails(&[vec![[cx, 30.0, cz - 100.0], [cx, 30.0, cz + 80.0]]]);
        engine.hud_enable(true);
        for frame in 120..195 {
            let mut pad = t2::PAD_AT_REST;
            if frame >= 140 {
                pad.left_y = 0;
            }
            if (160..=164).contains(&frame) {
                pad.buttons |= t2::buttons::CROSS;
            }
            if frame >= 166 {
                pad.buttons |= t2::buttons::TRIANGLE;
            }
            engine.step(&pad).expect("step");
        }
        let mut frame = t2::Hud::default();
        engine.hud(&mut frame);
        assert!(!frame.draws.is_empty());
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .build()
                .disable::<bevy::winit::WinitPlugin>()
                .set(WindowPlugin { primary_window: None, exit_condition: bevy::window::ExitCondition::DontExit, ..default() }),
        );
        install(&mut app);
        app.insert_resource(Thug2 { phase: Phase::Running(engine), hud_on: true, hud: frame, ..default() })
            .add_systems(Startup, spawn)
            .add_systems(Update, draw);
        app.finish();
        app.cleanup();
        app.update();
        let targets: Vec<(String, Handle<Image>)> = {
            let world = app.world_mut();
            let mut cameras = world.query::<(&mut Camera, &RenderTarget, &Name)>();
            let mut targets = Vec::new();
            for (mut camera, render_target, name) in cameras.iter_mut(world) {
                camera.is_active = true;
                // (over grey, to see it)
                camera.clear_color = ClearColorConfig::Custom(Color::srgb(0.3, 0.3, 0.3));
                if let RenderTarget::Image(image) = render_target {
                    let part = name.as_str().trim_start_matches("THUG2 HUD camera (").trim_end_matches(')').to_lowercase();
                    targets.push((part, image.handle.clone()));
                }
            }
            targets
        };
        assert_eq!(targets.len(), 3, "a picture a part");
        for _ in 0..10 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // (`<path>` with the part's name before its extension: hud-score.png, hud-meter.png, hud-trick.png)
        let path = std::path::PathBuf::from(path);
        for (part, target) in targets {
            let file = path.with_file_name(format!(
                "{}-{part}.png",
                path.file_stem().and_then(|s| s.to_str()).unwrap_or("hud")
            ));
            app.world_mut().spawn(Screenshot::image(target)).observe(save_to_disk(file));
        }
        for _ in 0..20 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
