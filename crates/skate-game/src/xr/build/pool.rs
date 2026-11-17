//! What makes a sunk piece (a bowl, a drawn pool) read as a pool in a real floor: drawn over and round its shape, none
//! of it ridden (the lip stays the grind line it was).
//!
//! - **Coping** along the lip: pool coping, a rounded block over the lip and its top flush with the floor, or for a
//!   wooden bowl a steel pipe ([`coping_mesh`]).
//! - **Tiles**: a band of small tiles on the vertical wall under the coping (the plaster finish's, [`tile_mesh`]).
//! - **A drain** at the deep end's floor ([`drain_mesh`]).
//! - **Its own shade**: how much of the sky each point inside sees through the mouth (under the lip and down in the
//!   deep end, less), as the mesh's vertex colours ([`sky_seen`]); and the rim's shadow from the room's light, worked
//!   out in a shader from the mouth's outline ([`mouth_mask`], `xr::build::pool_shadow`).
//! - **Grounding**: a soft dark band on the floor round the coping (`xr::contact_shadow`'s ring material).
use super::pieces::{Finish, PieceGeometry};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::f32::consts::{FRAC_PI_2, PI};

/// Pool coping: how far it reaches back over the floor from the lip, and its nose's radius (it overhangs the lip by
/// that much), metres.
pub(crate) const COPING_DEPTH: f32 = 0.3;
pub(crate) const COPING_NOSE: f32 = 0.04;
/// A steel coping pipe's radius.
const PIPE: f32 = 0.03;
/// The tile band's height under the coping (at most the vertical wall), and a tile's size.
const TILE_BAND: f32 = 0.16;
const TILE: f32 = 0.04;
/// The drain's radius.
const DRAIN: f32 = 0.18;
/// The floor round the coping: how far out it is darkened, and how dark at the coping.
const GROUND_RING: f32 = 0.18;
const GROUND_DARK: f32 = 0.35;
/// How far drawn things stand off what they lie on (so they're in front of it in depth), metres.
const OFF: f32 = 0.004;

/// The lip offset out from the pool (`out` metres; negative: in), each point along the outward miter of its two sides.
fn offset(lip: &[Vec3], out: f32) -> Vec<Vec3> {
    let n = lip.len();
    // (the lip's way round: outward is to the right of it going round one way, the left the other)
    let area: f32 = (0..n).map(|i| {
        let (a, b) = (lip[i], lip[(i + 1) % n]);
        a.x * b.z - b.x * a.z
    }).sum();
    let sign = if area > 0.0 { -1.0 } else { 1.0 };
    let normal = |a: Vec3, b: Vec3| {
        let d = (b - a).with_y(0.0).normalize_or_zero();
        Vec3::new(-d.z, 0.0, d.x) * sign
    };
    (0..n)
        .map(|i| {
            let (prev, here, next) = (lip[(i + n - 1) % n], lip[i], lip[(i + 1) % n]);
            let (n0, n1) = (normal(prev, here), normal(here, next));
            let dir = (n0 + n1).normalize_or(n1);
            // (the miter, as long as it needs to be to keep both sides `out` away, but not spiking at sharp corners)
            let stretch = 1.0 / dir.dot(n1).max(0.5);
            here + dir * out * stretch
        })
        .collect()
}

/// A strip swept round the closed `lip` through a profile: each profile point (out from the lip, up) with its normal
/// (out, up); its texture coordinates metres along the lip and along the profile.
fn sweep(lip: &[Vec3], profile: &[(Vec2, Vec2)]) -> Mesh {
    let n = lip.len();
    let outward: Vec<Vec3> = offset(lip, 1.0).iter().zip(lip).map(|(o, p)| *o - *p).collect();
    let mut along = vec![0.0f32; n + 1];
    for i in 0..n {
        along[i + 1] = along[i] + lip[i].distance(lip[(i + 1) % n]);
    }
    let mut across = vec![0.0f32; profile.len()];
    for k in 1..profile.len() {
        across[k] = across[k - 1] + profile[k].0.distance(profile[k - 1].0);
    }
    let (mut positions, mut normals, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in 0..=n {
        let (p, out) = (lip[i % n], outward[i % n]);
        for (k, (at, normal)) in profile.iter().enumerate() {
            positions.push((p + out * at.x + Vec3::Y * at.y).to_array());
            normals.push((out.normalize_or_zero() * normal.x + Vec3::Y * normal.y).normalize_or(Vec3::Y).to_array());
            uvs.push([along[i], across[k]]);
        }
    }
    // (each quad wound counter-clockwise seen from the side its normals face, Bevy's front: which way that is turns
    // on the lip's way round and the profile's, so it is taken from the quad itself. A double-sided material lights
    // a face seen from its back with its normal flipped, so one wound the other way is lit as if it faced away.)
    let m = profile.len() as u32;
    let at = |i: u32| Vec3::from_array(positions[i as usize]);
    let normal_at = |i: u32| Vec3::from_array(normals[i as usize]);
    for i in 0..n as u32 {
        for k in 0..m - 1 {
            let (a, b) = (i * m + k, (i + 1) * m + k);
            let face = (at(b) - at(a)).cross(at(b + 1) - at(a)) + (at(b + 1) - at(a)).cross(at(a + 1) - at(a));
            let normal = normal_at(a) + normal_at(b) + normal_at(b + 1) + normal_at(a + 1);
            if face.dot(normal) >= 0.0 {
                indices.extend([a, b, b + 1, a, b + 1, a + 1]);
            } else {
                indices.extend([a, b + 1, b, a, a + 1, b + 1]);
            }
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}

/// The coping along a pool's lip. Pool coping: its top flush with the floor from `COPING_DEPTH` back over it to the
/// lip, then a rounded nose over the pool, under it back to the wall. A wooden bowl's: a steel pipe on the lip.
pub(crate) fn coping_mesh(geometry: &PieceGeometry, finish: Finish) -> Option<Mesh> {
    let lip = &geometry.mouth;
    if lip.len() < 3 {
        return None;
    }
    let mut profile: Vec<(Vec2, Vec2)> = Vec::new();
    if finish == Finish::Wood {
        // (round the pipe, sitting on the lip)
        for k in 0..=12 {
            let a = 2.0 * PI * k as f32 / 12.0;
            let normal = Vec2::new(a.cos(), a.sin());
            profile.push((Vec2::new(0.0, PIPE) + normal * PIPE, normal));
        }
        return Some(sweep(lip, &profile));
    }
    let top = OFF;
    // (out at the back of its top, then to the nose)
    profile.push((Vec2::new(COPING_DEPTH, top), Vec2::Y));
    profile.push((Vec2::new(0.0, top), Vec2::Y));
    // (the nose: over the pool and back under to the wall)
    for k in 0..=8 {
        let a = FRAC_PI_2 - PI * k as f32 / 8.0;
        let normal = Vec2::new(-a.cos(), a.sin());
        profile.push((Vec2::new(0.0, top - COPING_NOSE) + normal * COPING_NOSE, normal));
    }
    Some(sweep(lip, &profile))
}

/// A band of tiles on the vertical wall under the coping, standing a little off it.
pub(crate) fn tile_mesh(geometry: &PieceGeometry) -> Option<Mesh> {
    let lip = &geometry.mouth;
    if lip.len() < 3 || geometry.vert <= 0.0 {
        return None;
    }
    let below = 2.0 * COPING_NOSE;
    let band = TILE_BAND.min(geometry.vert + 0.02 - below).max(0.02);
    // (facing into the pool: out is negative)
    let profile = [(Vec2::new(-OFF, -below), Vec2::NEG_X), (Vec2::new(-OFF, -below - band), Vec2::NEG_X)];
    Some(sweep(lip, &profile))
}

/// The floor round the coping darkened: dark at its back edge, clear `GROUND_RING` further out (vertex alpha, for
/// `xr::contact_shadow`'s ring material).
pub(crate) fn ground_mesh(geometry: &PieceGeometry, finish: Finish) -> Option<Mesh> {
    let lip = &geometry.mouth;
    if lip.len() < 3 {
        return None;
    }
    let from = if finish == Finish::Wood { 0.0 } else { COPING_DEPTH };
    let profile = [(Vec2::new(from, OFF * 0.5), Vec2::Y), (Vec2::new(from + GROUND_RING, OFF * 0.5), Vec2::Y)];
    let mut mesh = sweep(lip, &profile);
    let count = mesh.count_vertices();
    let colors: Vec<[f32; 4]> = (0..count).map(|i| [0.0, 0.0, 0.0, if i % 2 == 0 { GROUND_DARK } else { 0.0 }]).collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    Some(mesh)
}

/// The drain: a disc on the floor at its deepest, lying along the floor's slope there.
pub(crate) fn drain_mesh(geometry: &PieceGeometry) -> Option<Mesh> {
    let centre = geometry.drain?;
    // (the floor's slope under it: the floor faces' normal nearest it)
    let up = geometry
        .quads
        .iter()
        .filter(|q| q.iter().any(|p| p.distance(centre) < 2.0))
        .map(super::pieces::quad_normal)
        .filter(|n| n.y > 0.7)
        .fold(Vec3::ZERO, |a, n| a + n)
        .normalize_or(Vec3::Y);
    let across = up.cross(Vec3::Z).normalize_or(Vec3::X);
    let other = up.cross(across);
    const SIDES: u32 = 24;
    let mut positions = vec![(centre + up * OFF).to_array()];
    let mut uvs = vec![[0.5, 0.5]];
    for k in 0..=SIDES {
        let a = 2.0 * PI * k as f32 / SIDES as f32;
        positions.push((centre + up * OFF + (across * a.cos() + other * a.sin()) * DRAIN).to_array());
        uvs.push([0.5 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin()]);
    }
    // (counter-clockwise seen from `up`: across, other, up is a right-handed basis)
    let indices: Vec<u32> = (1..=SIDES).flat_map(|k| [0, k, k + 1]).collect();
    let normals = vec![up.to_array(); positions.len()];
    Some(
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
            .with_inserted_indices(Indices::U32(indices)),
    )
}

/// Whether `p` (x, z) is inside the closed outline `mouth`.
pub(crate) fn inside(mouth: &[Vec2], p: Vec2) -> bool {
    let mut odd = false;
    let n = mouth.len();
    for i in 0..n {
        let (a, b) = (mouth[i], mouth[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            odd = !odd;
        }
    }
    odd
}

/// How much of the sky a point inside the pool sees through its mouth (0 to 1, cosine weighted about its `normal`): a
/// way out of it upward that leaves through the mouth (drawn in by the coping's overhang) sees the sky; any other
/// meets the pool's own walls or the floor round it.
pub(crate) fn sky_seen(mouth: &[Vec2], p: Vec3, normal: Vec3) -> f32 {
    // (a fixed spread of ways round the normal: golden-angle spiral, cosine weighted)
    const SAMPLES: usize = 32;
    if p.y > -1e-3 {
        return 1.0;
    }
    let tangent = normal.cross(if normal.y.abs() < 0.9 { Vec3::Y } else { Vec3::X }).normalize();
    let bitangent = normal.cross(tangent);
    let mut seen = 0.0;
    for i in 0..SAMPLES {
        let u = (i as f32 + 0.5) / SAMPLES as f32;
        let r = u.sqrt();
        let a = i as f32 * 2.399_963;
        let d = (tangent * (r * a.cos()) + bitangent * (r * a.sin()) + normal * (1.0 - u).sqrt()).normalize();
        if d.y <= 1e-4 {
            continue;
        }
        let q = p + d * (-p.y / d.y);
        if inside(mouth, Vec2::new(q.x, q.z)) {
            seen += 1.0;
        }
    }
    seen / SAMPLES as f32
}

/// The mouth drawn in by the coping's nose (what the sky is seen through), x, z.
pub(crate) fn sky_mouth(geometry: &PieceGeometry) -> Vec<Vec2> {
    offset(&geometry.mouth, -COPING_NOSE).iter().map(|p| Vec2::new(p.x, p.z)).collect()
}

/// A shade (vertex colour) for each of `positions` with `normals` inside a pool: full light where it sees all the
/// sky, down to about a third in its darkest corners.
pub(crate) fn sky_shades(geometry: &PieceGeometry, positions: &[[f32; 3]], normals: &[[f32; 3]]) -> Vec<[f32; 4]> {
    let mouth = sky_mouth(geometry);
    positions
        .iter()
        .zip(normals)
        .map(|(p, n)| {
            let seen = sky_seen(&mouth, Vec3::from_array(*p), Vec3::from_array(*n));
            let shade = 0.35 + 0.65 * seen.powf(0.6);
            [shade, shade, shade, 1.0]
        })
        .collect()
}

/// The mouth as a picture for the rim's shadow: inside it white, out of it black, over the rectangle `min..max`
/// (its x, z: the mouth with a margin). Its edge softened a little.
pub(crate) fn mouth_mask(geometry: &PieceGeometry) -> Option<(Image, Vec2, Vec2)> {
    const SIZE: u32 = 128;
    let mouth = sky_mouth(geometry);
    if mouth.len() < 3 {
        return None;
    }
    let (lo, hi) = mouth.iter().fold((Vec2::MAX, Vec2::MIN), |(a, b), p| (a.min(*p), b.max(*p)));
    let margin = (hi - lo) * 0.1 + Vec2::splat(0.1);
    let (min, max) = (lo - margin, hi + margin);
    let at = |x: u32, y: u32| min + (max - min) * Vec2::new((x as f32 + 0.5) / SIZE as f32, (y as f32 + 0.5) / SIZE as f32);
    let hard: Vec<f32> = (0..SIZE * SIZE).map(|i| if inside(&mouth, at(i % SIZE, i / SIZE)) { 1.0 } else { 0.0 }).collect();
    // (a 3x3 blur)
    let mut bytes = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let mut sum = 0.0;
            for (dx, dy) in (-1..=1).flat_map(|a| (-1..=1).map(move |b| (a, b))) {
                let (sx, sy) = ((x + dx).clamp(0, SIZE as i32 - 1), (y + dy).clamp(0, SIZE as i32 - 1));
                sum += hard[(sy as u32 * SIZE + sx as u32) as usize];
            }
            let v = (sum / 9.0 * 255.0) as u8;
            bytes.extend([v, v, v, 255]);
        }
    }
    let mut image = Image::new(
        Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        TextureDimension::D2,
        bytes,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = bevy::image::ImageSampler::linear();
    Some((image, min, max))
}

/// Small square tiles with grout between them, `TILE` a tile: blue mosaic, one texel row a tile's grout. The texture
/// covers 4 x 4 tiles.
pub(crate) fn tile_texture() -> Image {
    const PER: u32 = 16; // texels a tile
    const SIZE: u32 = PER * 4;
    let mut bytes = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    let mut seed = 0x2545_f491_u32;
    let mut shades = [[0u8; 3]; 16];
    for shade in &mut shades {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let jitter = (seed % 40) as i32 - 20;
        *shade = [(40 + jitter / 2).clamp(0, 255) as u8, (120 + jitter).clamp(0, 255) as u8, (170 + jitter).clamp(0, 255) as u8];
    }
    for y in 0..SIZE {
        for x in 0..SIZE {
            let grout = x % PER < 1 || y % PER < 1;
            let c = if grout { [205, 210, 205] } else { shades[((y / PER) * 4 + x / PER) as usize] };
            bytes.extend([c[0], c[1], c[2], 255]);
        }
    }
    repeating(bytes, SIZE)
}

/// Pool coping's surface: pale concrete blocks, a joint every `0.6` m along it.
pub(crate) fn coping_texture() -> Image {
    const SIZE: u32 = 64;
    let mut bytes = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    let mut seed = 0x9e37_79b9_u32;
    for _y in 0..SIZE {
        for x in 0..SIZE {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let speckle = (seed % 24) as i32 - 12;
            let joint = x < 1;
            let base = if joint { 150 } else { 222 + speckle };
            let v = base.clamp(0, 255) as u8;
            bytes.extend([v, v, (v as i32 - 6).clamp(0, 255) as u8, 255]);
        }
    }
    repeating(bytes, SIZE)
}

/// The drain: a dark grate of bars in a steel ring.
pub(crate) fn drain_texture() -> Image {
    const SIZE: u32 = 64;
    let mut bytes = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) / SIZE as f32 * 2.0 - Vec2::ONE;
            let r = p.length();
            let c = if r > 0.82 {
                [150, 152, 156]
            } else if (p.x * 6.0).fract().abs() < 0.3 {
                [120, 122, 126]
            } else {
                [28, 30, 32]
            };
            bytes.extend([c[0], c[1], c[2], 255]);
        }
    }
    let mut image = Image::new(
        Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        TextureDimension::D2,
        bytes,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = bevy::image::ImageSampler::linear();
    image
}

fn repeating(bytes: Vec<u8>, size: u32) -> Image {
    use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
    let mut image = Image::new(
        Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        TextureDimension::D2,
        bytes,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// The materials a pool's dressing is drawn with, made the first time they're needed.
#[derive(Resource, Default)]
pub(crate) struct PoolAssets {
    coping: Option<Handle<StandardMaterial>>,
    pipe: Option<Handle<StandardMaterial>>,
    tiles: Option<Handle<StandardMaterial>>,
    drain: Option<Handle<StandardMaterial>>,
}

/// A marker on a pool's floor ring (the contact shadows' toggle shows and hides it).
#[derive(Component)]
pub(crate) struct PoolGround;

impl PoolAssets {
    /// A pool's dressing, children of its visual `parent` (in its space): its coping, its tiles (plaster), its drain,
    /// the rim's shadow over its own `mesh`, and (with the contact shadows' `ring` material) the floor round it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dress(
        &mut self,
        commands: &mut Commands,
        parent: Entity,
        geometry: &PieceGeometry,
        finish: Finish,
        mesh: Handle<Mesh>,
        ring: Option<Handle<crate::xr::contact_shadow::ShadowMaterial>>,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
        shadows: &mut Assets<super::pool_shadow::PoolShadow>,
    ) {
        let child = |commands: &mut Commands, bundle: (Mesh3d, MeshMaterial3d<StandardMaterial>)| {
            commands.spawn((bundle, Transform::default(), ChildOf(parent)));
        };
        let textured = |images: &mut Assets<Image>, image: Image, metres: f32, color: Color, roughness: f32| StandardMaterial {
            base_color: color,
            base_color_texture: Some(images.add(image)),
            uv_transform: bevy::math::Affine2::from_scale(Vec2::splat(1.0 / metres)),
            perceptual_roughness: roughness,
            double_sided: true,
            cull_mode: None,
            ..default()
        };
        if let Some(coping) = coping_mesh(geometry, finish) {
            let material = if finish == Finish::Wood {
                self.pipe
                    .get_or_insert_with(|| {
                        materials.add(StandardMaterial {
                            base_color: Color::srgb(0.62, 0.64, 0.67),
                            metallic: 0.3,
                            perceptual_roughness: 0.35,
                            double_sided: true,
                            cull_mode: None,
                            ..default()
                        })
                    })
                    .clone()
            } else {
                self.coping
                    .get_or_insert_with(|| materials.add(textured(images, coping_texture(), COPING_TEXTURE_M, Color::WHITE, 0.85)))
                    .clone()
            };
            child(commands, (Mesh3d(meshes.add(coping)), MeshMaterial3d(material)));
        }
        if finish == Finish::Plaster
            && let Some(tiles) = tile_mesh(geometry)
        {
            let material = self
                .tiles
                .get_or_insert_with(|| materials.add(textured(images, tile_texture(), TILE_TEXTURE_M, Color::WHITE, 0.3)))
                .clone();
            child(commands, (Mesh3d(meshes.add(tiles)), MeshMaterial3d(material)));
        }
        if let Some(drain) = drain_mesh(geometry) {
            let material = self
                .drain
                .get_or_insert_with(|| {
                    materials.add(StandardMaterial {
                        base_color_texture: Some(images.add(drain_texture())),
                        perceptual_roughness: 0.5,
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    })
                })
                .clone();
            child(commands, (Mesh3d(meshes.add(drain)), MeshMaterial3d(material)));
        }
        if let Some((mask, min, max)) = mouth_mask(geometry) {
            let material = shadows.add(super::pool_shadow::PoolShadow::new(images.add(mask), min, max));
            commands.spawn((super::pool_shadow::PoolShadowOf, Mesh3d(mesh), MeshMaterial3d(material), Transform::default(), ChildOf(parent)));
        }
        if let (Some(ring), Some(ground)) = (ring, ground_mesh(geometry, finish)) {
            commands.spawn((PoolGround, Mesh3d(meshes.add(ground)), MeshMaterial3d(ring), Transform::default(), ChildOf(parent)));
        }
    }
}

/// How many metres one copy of the tile texture covers, and of the coping's (its joints 0.6 m apart).
pub(crate) const TILE_TEXTURE_M: f32 = TILE * 4.0;
pub(crate) const COPING_TEXTURE_M: f32 = 0.6;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xr::build::pieces::{Piece, PieceKind};

    /// A bowl's mouth goes round its lip at y 0; its walls have a vertical band; its floor is deeper at one end; its
    /// drain is on the floor.
    #[test]
    fn a_bowl_has_a_mouth_a_vertical_wall_and_a_deep_end() {
        let g = Piece::new(PieceKind::Bowl).geometry();
        assert!(g.mouth.len() > 8 && g.mouth.iter().all(|p| p.y.abs() < 1e-5));
        assert!(g.vert > 0.05);
        let floor: Vec<&[Vec3; 4]> = g.quads.iter().filter(|q| q.iter().all(|p| p.y < -1.0)).collect();
        let (lowest, highest) = floor.iter().flat_map(|q| q.iter()).fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
        assert!(highest - lowest > 0.3, "deeper at one end: {lowest}..{highest}");
        let drain = g.drain.expect("a drain");
        assert!(drain.y < -1.5 && drain.x > 0.0, "{drain}");
        // (straight up from just under the lip is the vertical wall: the point a little in is still at the lip's x, z)
        let wall = g.quads.iter().any(|q| q.iter().filter(|p| p.y.abs() < 1e-5).count() == 2 && q.iter().any(|p| (p.y + g.vert).abs() < 1e-4));
        assert!(wall, "a vertical wall's faces from the lip down");
    }

    /// The sky seen inside: all of it in the middle of the floor looking up, less under the lip, none facing down.
    #[test]
    fn under_the_lip_sees_less_sky_than_the_middle() {
        let g = Piece::new(PieceKind::Bowl).geometry();
        let mouth = sky_mouth(&g);
        let middle = sky_seen(&mouth, Vec3::new(0.0, -1.6, 0.0), Vec3::Y);
        // (on the vertical wall just under the lip at +z, facing in)
        let lip_z = g.mouth.iter().map(|p| p.z).fold(f32::MIN, f32::max);
        let under = sky_seen(&mouth, Vec3::new(0.0, -0.1, lip_z - 0.001), Vec3::NEG_Z);
        assert!(middle > 0.6, "{middle}");
        assert!(under < middle * 0.7, "{under} against {middle}");
    }

    /// Coping and tiles sweep round the lip: the coping's back edge `COPING_DEPTH` out of the pool, the tiles just
    /// inside it.
    #[test]
    fn coping_and_tiles_follow_the_lip() {
        let g = Piece::new(PieceKind::Bowl).geometry();
        let coping = coping_mesh(&g, Finish::Concrete).expect("coping");
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) = coping.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!() };
        let reach = g.mouth.iter().map(|p| p.x).fold(f32::MIN, f32::max);
        let out = p.iter().map(|p| p[0]).fold(f32::MIN, f32::max);
        assert!((out - (reach + COPING_DEPTH)).abs() < 0.02, "{out} against {reach}");
        assert!(tile_mesh(&g).is_some() && ground_mesh(&g, Finish::Concrete).is_some() && drain_mesh(&g).is_some());
        let (image, min, max) = mouth_mask(&g).expect("a mask");
        assert!(min.x < -reach && max.x > reach && image.width() == 128);
    }

    /// Every dressing triangle is wound counter-clockwise seen from the side its normals face (Bevy's front face):
    /// a double-sided `StandardMaterial` flips the normal of a fragment seen from its back, so one wound the other
    /// way is lit as if it faced away from the light.
    #[test]
    fn dressing_faces_the_way_its_normals_do() {
        let g = Piece::new(PieceKind::Bowl).geometry();
        let meshes = [
            ("coping", coping_mesh(&g, Finish::Concrete)),
            ("pipe", coping_mesh(&g, Finish::Wood)),
            ("tiles", tile_mesh(&g)),
            ("ground", ground_mesh(&g, Finish::Concrete)),
            ("drain", drain_mesh(&g)),
        ];
        for (name, mesh) in meshes {
            let mesh = mesh.expect(name);
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!() };
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(n)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else { panic!() };
            let Some(Indices::U32(indices)) = mesh.indices() else { panic!() };
            let (mut facing, mut away) = (0, 0);
            for t in indices.chunks_exact(3) {
                let [a, b, c] = [0, 1, 2].map(|k| Vec3::from_array(p[t[k] as usize]));
                let face = (b - a).cross(c - a);
                if face.length() < 1e-9 {
                    continue;
                }
                let normal: Vec3 = t.iter().map(|&i| Vec3::from_array(n[i as usize])).sum();
                if face.dot(normal) > 0.0 { facing += 1 } else { away += 1 }
            }
            assert!(facing > 0 && away == 0, "{name}: {away} of {} triangles wound against their normals", facing + away);
        }
    }
}
