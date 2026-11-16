//! Contact shadows: the board's on whatever is under it (the real floor and furniture, a park piece), and each park
//! piece's on the floor it stands on. Cheap, analytic ones - no shadow map (directional shadows cost the headset 13 fps):
//!
//! - **The board's.** A patch of the surfaces under the skater (the room's and the pieces' level faces, cut to a
//!   square around it, `ArWorld`), drawn with [`ShadowMaterial`]: soft ellipses under the deck and its two trucks,
//!   fainter and wider the higher it is over the surface. The patch is made again only when the skater leaves it or
//!   the world changes; each frame only the casters move.
//! - **The pieces'.** A soft ring round each piece's footprint where it meets the floor ([`piece_ring`], spawned with
//!   the piece's visual), hidden while the piece isn't standing on the room.
//!
//! Over passthrough, black at a coverage darkens the real room behind it (the compositor takes the eyes' picture as
//! premultiplied), and a virtual piece under it the same. The shadows are written into depth where they show, so
//! occlusion hides them behind real things as it hides the skater; where they don't show they're discarded and
//! written nowhere.
//!
//! Off with the AR settings' "Contact shadows".
use crate::xr::ar_settings::ArSettings;
use crate::xr::build::pieces::{PieceGeometry, PieceKind};
use crate::xr::link::{ArWorld, SkaterLink};
use crate::xr::rig::Rig;
use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;

/// How far round the skater the patch reaches, in skater sizes (a board is about 0.8).
const PATCH_REACH: f32 = 1.0;
/// The skater moved this far from the patch's middle (skater sizes): made again.
const PATCH_MOVE: f32 = 0.35;
/// Faces steeper than this aren't under anything (cosine of their slope).
const LEVEL_ENOUGH: f32 = 0.3;
/// Shadows lifted off what they lie on, real metres (so they're in front of a piece's face in depth).
const LIFT_M: f32 = 0.003;
/// A piece's ring: how far out from its footprint it fades, real metres, and how dark it is at the piece.
const RING_M: f32 = 0.12;
const RING_DARK: f32 = 0.5;
/// A piece whose base is this far over the room is in the air: no ring, real metres.
const STANDING_M: f32 = 0.05;

#[derive(Clone, Copy, Debug, Default, ShaderType)]
struct Caster {
    centre: Vec4,
    nose: Vec4,
    side: Vec4,
    shape: Vec4,
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
struct Params {
    casters: [Caster; 4],
    count: u32,
    mode: u32,
    opacity: f32,
    pad: f32,
}

/// Black at a coverage, written into depth where it shows (see the module notes).
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub(crate) struct ShadowMaterial {
    #[uniform(0)]
    params: Params,
}

impl Material for ShadowMaterial {
    fn fragment_shader() -> ShaderRef {
        concat!("embedded://", env!("CARGO_CRATE_NAME"), "/xr/contact_shadow.wgsl").into()
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
        // (in depth where it shows, for occlusion: see-through things aren't, and the occlusion pass leaves pixels
        // with no depth alone)
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_write_enabled = true;
        }
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The materials: the board's (its casters each frame) and the rings'.
#[derive(Resource)]
pub(crate) struct ShadowAssets {
    board: Handle<ShadowMaterial>,
    pub(crate) ring: Handle<ShadowMaterial>,
}

/// The patch under the skater, and what it was made from.
#[derive(Component)]
struct BoardShadow;

#[derive(Resource, Default)]
struct Patch {
    /// Its middle, the world's revision and the skater's size it was made for.
    made: Option<(Vec3, u64, f32)>,
    /// Its faces (lifted), for finding the surface under the casters.
    triangles: Vec<[Vec3; 3]>,
}

/// A piece's ring: where its middle is, in the piece's own space (where it stands).
#[derive(Component)]
pub(crate) struct PieceRing(pub(crate) Vec3);

pub(crate) struct ContactShadowPlugin;

impl Plugin for ContactShadowPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "contact_shadow.wgsl");
        app.add_plugins(MaterialPlugin::<ShadowMaterial>::default())
            .init_resource::<Patch>()
            .add_systems(Startup, setup)
            .add_systems(PostUpdate, (board_shadow, ring_standing, pool_grounds).before(bevy::transform::TransformSystems::Propagate));
    }
}

fn setup(mut commands: Commands, mut materials: ResMut<Assets<ShadowMaterial>>, mut meshes: ResMut<Assets<Mesh>>) {
    let board = materials.add(ShadowMaterial { params: Params { opacity: 1.0, ..default() } });
    let ring = materials.add(ShadowMaterial { params: Params { mode: 1, opacity: 1.0, ..default() } });
    commands.spawn((
        BoardShadow,
        Mesh3d(meshes.add(Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default()))),
        MeshMaterial3d(board.clone()),
        Transform::default(),
        Visibility::Hidden,
        Name::new("Board shadow"),
    ));
    commands.insert_resource(ShadowAssets { board, ring });
}

/// The board's shadow: its casters where the skater is, and the patch under it made again when it has moved off it.
#[allow(clippy::too_many_arguments)]
fn board_shadow(
    settings: Res<ArSettings>,
    world: Res<ArWorld>,
    link: Res<SkaterLink>,
    rig: Res<Rig>,
    assets: Option<Res<ShadowAssets>>,
    mut patch: ResMut<Patch>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ShadowMaterial>>,
    mut shadow: Query<(&Mesh3d, &mut Visibility), With<BoardShadow>>,
) {
    let (Some(assets), Ok((mesh, mut visibility))) = (assets, shadow.single_mut()) else { return };
    let Some((deck, nose)) = link.deck.filter(|_| settings.contact_shadows) else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    let size = link.size();
    let stale = patch.made.is_none_or(|(at, revision, made_size)| {
        revision != world.revision || made_size != size || (at - deck).with_y(0.0).length() > PATCH_MOVE * size
    });
    if stale {
        let lift = LIFT_M * rig.scale;
        let triangles = level_faces_near(&world, deck, PATCH_REACH * size, lift);
        if let Some(m) = meshes.get_mut(&mesh.0) {
            *m = patch_mesh(&triangles);
        }
        patch.triangles = triangles;
        patch.made = Some((deck, world.revision, size));
    }
    let casters = casters(deck, nose, size, |p| surface_under(&patch.triangles, p, size));
    visibility.set_if_neq(if casters.is_empty() { Visibility::Hidden } else { Visibility::Inherited });
    if let Some(material) = materials.get_mut(&assets.board) {
        let mut params = Params { mode: 0, opacity: 1.0, count: casters.len() as u32, ..default() };
        params.casters[..casters.len()].copy_from_slice(&casters);
        material.params = params;
    }
}

/// The deck's shadow and its trucks', over the surface under each (`under`: its height there); none over nothing.
fn casters(deck: Vec3, nose: Vec3, size: f32, under: impl Fn(Vec3) -> Option<f32>) -> Vec<Caster> {
    let along = nose.with_y(0.0).normalize_or(Vec3::Z);
    let across = Vec3::Y.cross(along);
    let mut out = Vec::new();
    // (the deck, then the trucks: smaller and darker, where the wheels meet the ground)
    for (offset, half_length, half_width, dark) in [(0.0, 0.42, 0.14, 0.65), (0.25, 0.1, 0.12, 0.55), (-0.25, 0.1, 0.12, 0.55)] {
        let at = deck + along * (offset * size);
        let Some(ground) = under(at) else { continue };
        let height = (at.y - ground).max(0.0);
        out.push(Caster {
            centre: at.with_y(ground).extend(height),
            nose: along.extend(half_length * size),
            side: across.extend(half_width * size),
            shape: Vec4::new(dark, 1.2 * size, 0.8, size),
        });
    }
    out
}

/// The highest of `triangles` under `p` (at most a skater's height under it, and not over it by more than a little).
fn surface_under(triangles: &[[Vec3; 3]], p: Vec3, size: f32) -> Option<f32> {
    let from = p + Vec3::Y * (0.2 * size);
    triangles
        .iter()
        .filter_map(|t| crate::xr::spawn::ray_triangle(from, Vec3::NEG_Y, t[0], t[1], t[2]))
        .filter(|&d| d < 2.2 * size)
        .min_by(f32::total_cmp)
        .map(|d| from.y - d)
}

/// The world's level faces (the room's and the pieces') near `centre`, cut to the square `reach` round it and lifted
/// `lift` off their face.
fn level_faces_near(world: &ArWorld, centre: Vec3, reach: f32, lift: f32) -> Vec<[Vec3; 3]> {
    let (lo, hi) = (Vec2::new(centre.x - reach, centre.z - reach), Vec2::new(centre.x + reach, centre.z + reach));
    let mut out = Vec::new();
    for t in world.all_triangles() {
        let normal = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
        if normal.y.abs() < LEVEL_ENOUGH {
            continue;
        }
        let (min, max) = t.iter().fold((Vec2::MAX, Vec2::MIN), |(a, b), p| (a.min(p.xz()), b.max(p.xz())));
        if max.x < lo.x || max.y < lo.y || min.x > hi.x || min.y > hi.y {
            continue;
        }
        let up = if normal.y < 0.0 { -normal } else { normal };
        let polygon = clip_square(t.to_vec(), lo, hi);
        for i in 1..polygon.len().saturating_sub(1) {
            out.push([polygon[0], polygon[i], polygon[i + 1]].map(|p| p + up * lift));
        }
    }
    out
}

/// A flat polygon cut to the square `lo..hi` (x, z), its height kept.
fn clip_square(mut polygon: Vec<Vec3>, lo: Vec2, hi: Vec2) -> Vec<Vec3> {
    // (each side of the square: a point is in when `inside` is 0 or more)
    let sides: [&dyn Fn(Vec3) -> f32; 4] =
        [&|p: Vec3| p.x - lo.x, &|p: Vec3| hi.x - p.x, &|p: Vec3| p.z - lo.y, &|p: Vec3| hi.y - p.z];
    for inside in sides {
        if polygon.is_empty() {
            break;
        }
        let mut out = Vec::with_capacity(polygon.len() + 2);
        for i in 0..polygon.len() {
            let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            let (da, db) = (inside(a), inside(b));
            if da >= 0.0 {
                out.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                out.push(a + (b - a) * (da / (da - db)));
            }
        }
        polygon = out;
    }
    polygon
}

fn patch_mesh(triangles: &[[Vec3; 3]]) -> Mesh {
    let positions: Vec<[f32; 3]> = triangles.iter().flatten().map(|p| p.to_array()).collect();
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
}

/// A piece's contact shadow, in its own space: the floor round where it stands darkened, dark at the piece and clear
/// `RING_M` out, following its outline as seen from above (a long, turned or curved drawn piece's own shape, not a box
/// round it). Made on a grid over its footprint: each point's distance to the piece's faces seen from above, as the
/// vertices' alpha. Laid at the piece's lowest point (where it meets what it stands on), with the middle of its base.
/// A rail's are its posts', one each (each on what it stands on); none for what is sunk into the floor.
pub(crate) fn piece_ring(geometry: &PieceGeometry, kind: PieceKind) -> Vec<(Mesh, Vec3)> {
    use PieceKind::*;
    if kind.sunk() {
        return Vec::new();
    }
    if matches!(kind, Rail | CurvedRail | RailPath) {
        return geometry
            .posts
            .iter()
            .filter_map(|&(first, count)| geometry.quads.get(first..first + count))
            .filter_map(outline_shadow)
            .collect();
    }
    outline_shadow(&geometry.quads).into_iter().collect()
}

/// The contact shadow of the faces `quads` (see [`piece_ring`]).
fn outline_shadow(quads: &[[Vec3; 4]]) -> Option<(Mesh, Vec3)> {
    // (its faces as seen from above: polygons in x, z)
    let shapes: Vec<[Vec2; 4]> = quads.iter().map(|q| q.map(|p| p.xz())).collect();
    let base = quads.iter().flatten().map(|p| p.y).fold(f32::MAX, f32::min);
    if shapes.is_empty() || !base.is_finite() {
        return None;
    }
    let (lo, hi) = shapes.iter().flatten().fold((Vec2::MAX, Vec2::MIN), |(a, b), p| (a.min(*p), b.max(*p)));
    let (lo, hi) = (lo - Vec2::splat(RING_M * 1.2), hi + Vec2::splat(RING_M * 1.2));
    // (cells half the ring's width, no more than RING_CELLS of them)
    let mut cell = RING_M * 0.5;
    while ((hi.x - lo.x) / cell).ceil() * ((hi.y - lo.y) / cell).ceil() > RING_CELLS as f32 {
        cell *= 1.25;
    }
    let (nx, nz) = (((hi.x - lo.x) / cell).ceil() as usize + 1, ((hi.y - lo.y) / cell).ceil() as usize + 1);
    // (each face's own box, to pass it by quickly)
    let boxes: Vec<(Vec2, Vec2)> = shapes.iter().map(|q| q.iter().fold((Vec2::MAX, Vec2::MIN), |(a, b), p| (a.min(*p), b.max(*p)))).collect();
    let alpha: Vec<f32> = (0..nz)
        .flat_map(|k| (0..nx).map(move |i| (i, k)))
        .map(|(i, k)| {
            let p = lo + Vec2::new(i as f32, k as f32) * cell;
            let mut nearest = RING_M;
            for (shape, (bl, bh)) in shapes.iter().zip(&boxes) {
                if p.x < bl.x - nearest || p.y < bl.y - nearest || p.x > bh.x + nearest || p.y > bh.y + nearest {
                    continue;
                }
                nearest = nearest.min(distance_to_quad(p, shape));
                if nearest <= 0.0 {
                    break;
                }
            }
            let t = (nearest / RING_M).clamp(0.0, 1.0);
            RING_DARK * (1.0 - t * t * (3.0 - 2.0 * t))
        })
        .collect();
    let y = base + LIFT_M;
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut index = vec![u32::MAX; nx * nz];
    let mut indices = Vec::new();
    for k in 0..nz - 1 {
        for i in 0..nx - 1 {
            let corners = [(i, k), (i + 1, k), (i + 1, k + 1), (i, k + 1)];
            // (only cells with some shadow in them)
            if corners.iter().all(|&(a, b)| alpha[b * nx + a] <= 0.0) {
                continue;
            }
            let [a, b, c, d] = corners.map(|(ci, ck)| {
                let at = ck * nx + ci;
                if index[at] == u32::MAX {
                    index[at] = positions.len() as u32;
                    let p = lo + Vec2::new(ci as f32, ck as f32) * cell;
                    positions.push([p.x, y, p.y]);
                    colors.push([0.0, 0.0, 0.0, alpha[at]]);
                }
                index[at]
            });
            indices.extend([a, b, c, a, c, d]);
        }
    }
    if indices.is_empty() {
        return None;
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
        .with_inserted_indices(Indices::U32(indices));
    let middle = (lo + hi) / 2.0;
    Some((mesh, Vec3::new(middle.x, base, middle.y)))
}

/// How many grid cells a piece's contact shadow is made on, at most.
const RING_CELLS: usize = 12_000;

/// How far `p` is from the face `q` (x, z; any order round it): 0 inside, else to its nearest side. A face seen
/// edge on (a wall's side) is its sides: a line.
fn distance_to_quad(p: Vec2, q: &[Vec2; 4]) -> f32 {
    let mut inside = false;
    let mut nearest = f32::MAX;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
        let ab = b - a;
        let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
        nearest = nearest.min(p.distance(a + ab * t));
    }
    if inside { 0.0 } else { nearest }
}

/// A piece's contact shadow shows only while it stands on the room (not up in the air), with contact shadows on, and
/// lies on what it stands on (a piece set a little into the floor, or just over it, is still standing: its shadow is on
/// the floor, not at its own base). Looked at again whenever the world changes and whenever the piece moves.
#[allow(clippy::type_complexity)]
fn ring_standing(
    settings: Res<ArSettings>,
    world: Res<ArWorld>,
    rig: Res<Rig>,
    parents: Query<&GlobalTransform, Without<PieceRing>>,
    mut rings: Query<(&PieceRing, &ChildOf, &mut Visibility, &mut Transform, Ref<GlobalTransform>)>,
    mut seen: Local<Option<(u64, bool)>>,
) {
    let changed = *seen != Some((world.revision, settings.contact_shadows));
    *seen = Some((world.revision, settings.contact_shadows));
    for (ring, parent, mut visibility, mut transform, global) in &mut rings {
        if !changed && !global.is_changed() {
            continue;
        }
        let Ok(parent) = parents.get(parent.parent()) else { continue };
        let base = parent.transform_point(ring.0);
        // (down from a little over its base: the room's surface nearest under it, or a little over it)
        let reach = STANDING_M * rig.scale;
        let from = base + Vec3::Y * reach;
        let surface = world
            .room()
            .filter_map(|t| crate::xr::spawn::ray_triangle(from, Vec3::NEG_Y, t[0], t[1], t[2]))
            .min_by(f32::total_cmp)
            .map(|d| from.y - d)
            .filter(|y| (base.y - y).abs() < reach);
        let standing = settings.contact_shadows && surface.is_some();
        visibility.set_if_neq(if standing { Visibility::Inherited } else { Visibility::Hidden });
        if let Some(y) = surface {
            // (onto the surface: up or down in the piece's own space, by its scale)
            let lift = (y - base.y) / parent.scale().y.max(1e-6);
            if (transform.translation.y - lift).abs() > 1e-5 {
                transform.translation.y = lift;
            }
        }
    }
}

/// A pool's floor ring (`xr::build::pool`) shows with contact shadows on.
fn pool_grounds(settings: Res<ArSettings>, mut grounds: Query<&mut Visibility, With<crate::xr::build::pool::PoolGround>>) {
    let shown = if settings.contact_shadows { Visibility::Inherited } else { Visibility::Hidden };
    for mut visibility in &mut grounds {
        visibility.set_if_neq(shown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The board's shadow half over a virtual piece (grey) and half over the room (a light clear colour standing in
    /// for passthrough: blended the same way), and a box's ring, from above at an angle, saved to look at:
    /// `SK8_SHADOW_SHOT=/path/shadow.png cargo test ... a_picture_of_contact_shadows -- --ignored`.
    #[test]
    #[ignore = "needs a GPU; writes a picture"]
    fn a_picture_of_contact_shadows() {
        use bevy::camera::RenderTarget;
        use bevy::render::render_resource::TextureFormat;
        use bevy::render::view::screenshot::{Screenshot, save_to_disk};
        use crate::xr::build::pieces::Piece;
        let Some(path) = std::env::var_os("SK8_SHADOW_SHOT") else { return };
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .build()
                .disable::<bevy::winit::WinitPlugin>()
                .set(WindowPlugin { primary_window: None, exit_condition: bevy::window::ExitCondition::DontExit, ..default() }),
        );
        app.add_plugins(ContactShadowPlugin);
        // (the floor, and a virtual piece's top over its left half, 5 cm up)
        let floor = [Vec3::new(-3.0, 0.0, -3.0), Vec3::new(-3.0, 0.0, 3.0), Vec3::new(3.0, 0.0, 3.0), Vec3::new(3.0, 0.0, -3.0)];
        let top = [Vec3::new(-1.5, 0.05, -1.0), Vec3::new(-1.5, 0.05, 1.0), Vec3::new(0.0, 0.05, 1.0), Vec3::new(0.0, 0.05, -1.0)];
        let world = ArWorld {
            revision: 1,
            room_quads: vec![floor],
            pieces: vec![crate::xr::link::PieceSurfaces { quads: vec![top], ..default() }],
            ..default()
        };
        app.insert_resource(world)
            .insert_resource(ArSettings::default())
            .insert_resource(Rig { scale: 1.0, world_origin: Vec3::ZERO, anchor: Some((Vec3::ZERO, 0.0)) })
            .insert_resource(SkaterLink { deck: Some((Vec3::new(0.0, 0.15, 0.0), Vec3::new(1.0, 0.0, 0.3))), ..default() });
        app.finish();
        app.cleanup();
        app.update();
        let image = Image::new_target_texture(800, 600, TextureFormat::Rgba8UnormSrgb, None);
        let image = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        app.world_mut().spawn((
            Camera3d::default(),
            // (a light room behind: the shadow over it is what passthrough gets, `src + (1 - a) * room`)
            Camera { clear_color: ClearColorConfig::Custom(Color::srgb(0.85, 0.82, 0.75)), ..default() },
            RenderTarget::Image(image.clone().into()),
            Transform::from_xyz(0.6, 1.6, 1.6).looking_at(Vec3::new(0.3, 0.0, 0.0), Vec3::Y),
        ));
        {
            let world = app.world_mut();
            let mesh = world.resource_mut::<Assets<Mesh>>().add(Rectangle::new(1.5, 2.0));
            let grey = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial { base_color: Color::srgb(0.55, 0.55, 0.6), unlit: true, ..default() });
            world.spawn((Mesh3d(mesh), MeshMaterial3d(grey.clone()), Transform::from_xyz(-0.75, 0.05, 0.0).with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))));
            // (a box standing on the room, with its ring)
            let piece = Piece::sized(PieceKind::Platform, 1.0);
            let (ring, middle) = piece_ring(&piece.geometry(), PieceKind::Platform).pop().expect("a ring");
            let body = world.resource_mut::<Assets<Mesh>>().add(Cuboid::new(0.6, 0.3, 0.6));
            let ring = world.resource_mut::<Assets<Mesh>>().add(ring);
            let (lo, hi) = piece.geometry().footprint();
            let box_at = Transform::from_xyz(1.2, 0.0, -0.6).with_scale(Vec3::new(0.6 / (hi.x - lo.x), 1.0, 0.6 / (hi.y - lo.y)));
            world.spawn((Mesh3d(body), MeshMaterial3d(grey), Transform::from_xyz(1.2, 0.15, -0.6)));
            let holder = world.spawn((box_at, Visibility::Inherited)).id();
            world.spawn((PieceRing(middle), Mesh3d(ring), MeshMaterial3d(world.resource::<ShadowAssets>().ring.clone()), Transform::default(), Visibility::Hidden, ChildOf(holder)));
        }
        for _ in 0..20 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        app.world_mut().spawn(Screenshot::image(image)).observe(save_to_disk(std::path::PathBuf::from(path)));
        for _ in 0..20 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// A floor quad cut to the square round the skater: just that square, lifted off it.
    #[test]
    fn the_patch_is_the_level_faces_round_the_skater() {
        let world = ArWorld {
            room_quads: vec![
                // (the floor, 10 m across)
                [Vec3::new(-5.0, 0.0, -5.0), Vec3::new(-5.0, 0.0, 5.0), Vec3::new(5.0, 0.0, 5.0), Vec3::new(5.0, 0.0, -5.0)],
                // (a wall: not under anything)
                [Vec3::new(-5.0, 0.0, 1.0), Vec3::new(5.0, 0.0, 1.0), Vec3::new(5.0, 2.0, 1.0), Vec3::new(-5.0, 2.0, 1.0)],
            ],
            ..default()
        };
        let faces = level_faces_near(&world, Vec3::new(1.0, 0.1, 2.0), 0.5, 0.01);
        assert!(!faces.is_empty());
        for p in faces.iter().flatten() {
            assert!((0.49..=1.51).contains(&p.x) && (1.49..=2.51).contains(&p.z), "{p}");
            assert!((p.y - 0.01).abs() < 1e-5, "lifted: {p}");
        }
        let area: f32 = faces.iter().map(|t| (t[1] - t[0]).cross(t[2] - t[0]).length() / 2.0).sum();
        assert!((area - 1.0).abs() < 1e-3, "the square's area: {area}");
    }

    /// The deck and its trucks cast onto the surface under them; higher, they're fainter.
    #[test]
    fn the_board_casts_onto_what_is_under_it() {
        let floor = |_: Vec3| Some(0.0);
        let low = casters(Vec3::new(0.0, 0.1, 0.0), Vec3::X, 1.0, floor);
        assert_eq!(low.len(), 3);
        assert!(low.iter().all(|c| c.centre.y == 0.0 && (c.centre.w - 0.1).abs() < 1e-6));
        // (the trucks along the nose)
        assert!((low[1].centre.x - 0.25).abs() < 1e-6 && (low[2].centre.x + 0.25).abs() < 1e-6);
        let high = casters(Vec3::new(0.0, 1.5, 0.0), Vec3::X, 1.0, floor);
        assert!(high[0].centre.w > low[0].centre.w);
        // (over nothing: no shadow)
        assert!(casters(Vec3::ZERO, Vec3::X, 1.0, |_| None).is_empty());
    }

    /// A box's contact shadow: dark at its sides, clear `RING_M` out; a wall has one; none for a rail; a slab turned
    /// across the axes is followed along its own outline, not its box.
    #[test]
    fn a_piece_has_a_contact_shadow_round_its_own_outline() {
        use crate::xr::build::pieces::Piece;
        let alpha_at = |mesh: &Mesh, p: Vec2| -> f32 {
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!() };
            let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!() };
            let near = |q: &[f32; 3]| Vec2::new(q[0], q[2]).distance(p);
            let (i, q) = positions.iter().enumerate().min_by(|a, b| near(a.1).total_cmp(&near(b.1))).unwrap();
            if near(q) > RING_M { 0.0 } else { colors[i][3] }
        };
        let piece = Piece::sized(PieceKind::Platform, 1.0);
        let (mesh, middle) = piece_ring(&piece.geometry(), PieceKind::Platform).pop().expect("a shadow");
        let (lo, hi) = piece.geometry().footprint();
        assert!((middle.x - (lo.x + hi.x) / 2.0).abs() < 0.05);
        assert!(alpha_at(&mesh, Vec2::new(hi.x + 0.01, middle.z)) > RING_DARK * 0.7, "dark at its side");
        assert!(alpha_at(&mesh, Vec2::new(hi.x + RING_M * 1.1, middle.z)) < 0.01, "clear past the ring");
        assert_eq!(piece_ring(&Piece::new(PieceKind::Wall).geometry(), PieceKind::Wall).len(), 1, "a wall has one");
        // (a rail: one a post, round it)
        let rail = Piece::sized(PieceKind::Rail, 1.0).geometry();
        let posts = piece_ring(&rail, PieceKind::Rail);
        assert!(posts.len() == rail.posts.len() && posts.len() >= 2, "{} for {} posts", posts.len(), rail.posts.len());
        assert!(piece_ring(&Piece::new(PieceKind::Bowl).geometry(), PieceKind::Bowl).is_empty());
        let slab = PieceGeometry {
            quads: vec![[Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 4.0), Vec3::new(3.6, 0.0, 4.4), Vec3::new(-0.4, 0.0, 0.4)]],
            ..default()
        };
        let (mesh, _) = piece_ring(&slab, PieceKind::Deck).pop().expect("a shadow");
        assert!(alpha_at(&mesh, Vec2::new(4.0, 0.0)) < 0.01, "not at its box's empty corner");
        assert!(alpha_at(&mesh, Vec2::new(2.02, 1.98)) > RING_DARK * 0.3, "along its long side");
    }
}
