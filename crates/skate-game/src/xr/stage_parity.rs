//! What each engine is given for a few worlds, kept exactly (`f32` bits): recorded before the AR world was split into
//! the stage and the park (steps 4-5, docs/game-code-out-plan.md), so the split can't change what an engine gets.
//! The same worlds, built the same way, through the stage and park and each engine's own conversion now.
use bevy::prelude::*;
use engine_core::Stage;
use engine_core::skate::SkatePark;
use crate::xr::build::pieces::{Piece, PieceKind};

/// The bytes written, and their FNV-1a 64 digest.
#[derive(Default)]
struct Bytes(Vec<u8>);

impl Bytes {
    fn f(&mut self, v: f32) {
        self.0.extend(v.to_bits().to_le_bytes());
    }
    fn n(&mut self, n: usize) {
        self.0.extend((n as u64).to_le_bytes());
    }
    fn v(&mut self, p: Vec3) {
        p.to_array().into_iter().for_each(|x| self.f(x));
    }
    fn digest(&self) -> String {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in &self.0 {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        format!("{} bytes {h:016x}", self.0.len())
    }
}

/// The room: a floor, a wall and a table top as Scene faces, some mesh triangles (one degenerate), the table's edges
/// as ledges.
fn room(to_world: &Transform) -> (Vec<[Vec3; 4]>, Vec<[Vec3; 3]>, Vec<(Vec3, Vec3)>) {
    let w = |p: [f32; 3]| to_world.transform_point(Vec3::from_array(p));
    let quads = vec![
        [[-3.0, 0.0, -3.0], [3.0, 0.0, -3.0], [3.0, 0.0, 3.0], [-3.0, 0.0, 3.0]].map(w),
        [[-3.0, 0.0, -3.0], [-3.0, 2.5, -3.0], [3.0, 2.5, -3.0], [3.0, 0.0, -3.0]].map(w),
        [[1.0, 0.75, 1.0], [2.0, 0.75, 1.0], [2.0, 0.75, 1.6], [1.0, 0.75, 1.6]].map(w),
    ];
    let triangles = vec![
        [[0.1, 0.4, 0.2], [0.6, 0.45, 0.25], [0.3, 0.9, 0.7]].map(w),
        [[-1.0, 0.0, 2.0], [-0.5, 0.3, 2.2], [-0.8, 0.1, 2.6]].map(w),
        [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]].map(w),
    ];
    let edges = vec![(w([1.0, 0.75, 1.0]), w([2.0, 0.75, 1.0])), (w([2.0, 0.75, 1.0]), w([2.0, 0.75, 1.6]))];
    (quads, triangles, edges)
}

/// Three worlds: the room alone, the room with a piece of every kind, and the same through a turned, scaled rig.
fn worlds() -> Vec<(Stage, SkatePark)> {
    let mut out = Vec::new();
    for (case, root) in [
        Transform::IDENTITY,
        Transform::IDENTITY,
        Transform { translation: Vec3::new(0.4, -0.2, 1.3), rotation: Quat::from_rotation_y(0.7), scale: Vec3::splat(1.7) },
    ]
    .into_iter()
    .enumerate()
    {
        let (quads, triangles, edges) = room(&root);
        let kinds: &[PieceKind] = if case == 0 { &[] } else { &PieceKind::ALL };
        let pieces = kinds.iter().enumerate().map(|(i, &kind)| {
            let at = Transform {
                translation: root.transform_point(Vec3::new(i as f32 * 0.9 - 6.0, 0.0, (i % 3) as f32 * 1.5)),
                rotation: root.rotation * Quat::from_rotation_y(i as f32 * 0.37),
                scale: Vec3::splat(root.scale.x * (0.8 + 0.1 * (i % 4) as f32)),
            };
            (100 + i as u64, at, Piece::new(kind).geometry())
        });
        out.push(crate::xr::real_world::assemble(case as u64 + 1, quads, triangles, edges, pieces));
    }
    out
}

/// Skate 3: `set_real_surfaces`' four arguments and `set_grind_rails`'.
fn skate3((stage, park): &(Stage, SkatePark)) -> String {
    let mut b = Bytes::default();
    let v = |b: &mut Bytes, p: skate_core::math::Vector3| [p.x, p.y, p.z].into_iter().for_each(|x| b.f(x));
    let (quads, triangles, edges, pieces) = skate3_engine::stage::surfaces(stage, park);
    b.n(quads.len());
    quads.iter().flatten().for_each(|&p| v(&mut b, p));
    b.n(triangles.len());
    triangles.iter().flatten().for_each(|&p| v(&mut b, p));
    b.n(edges.len());
    edges.iter().for_each(|&(p, q)| [p, q].into_iter().for_each(|p| v(&mut b, p)));
    b.n(pieces.len());
    for piece in &pieces {
        b.n(piece.quads.len());
        piece.quads.iter().flatten().for_each(|&p| v(&mut b, p));
        b.n(piece.rail_quads.len());
        piece.rail_quads.iter().flatten().for_each(|&p| v(&mut b, p));
        b.n(piece.edges.len());
        piece.edges.iter().for_each(|&(p, q)| [p, q].into_iter().for_each(|p| v(&mut b, p)));
    }
    let rails = skate3_engine::stage::rails(park);
    b.n(rails.len());
    for rail in &rails {
        b.n(rail.len());
        rail.iter().for_each(|&p| v(&mut b, p));
    }
    b.digest()
}

/// THUG2: its faces (`set_world`) and rails (`set_rails`).
fn thug2((stage, park): &(Stage, SkatePark)) -> String {
    let mut b = Bytes::default();
    let faces = thug2_engine::stage::triangles(stage, park);
    b.n(faces.len());
    for t in &faces {
        t.v.iter().flatten().for_each(|&x| b.f(x));
        b.0.extend(t.flags.to_le_bytes());
        b.0.extend(t.terrain.to_le_bytes());
    }
    let rails = thug2_engine::stage::rails(park);
    b.n(rails.len());
    for rail in &rails {
        b.n(rail.len());
        rail.iter().flatten().for_each(|&x| b.f(x));
    }
    b.digest()
}

/// SK8: its collision's triangles and surfaces, in order, and its rails.
fn sk8((stage, park): &(Stage, SkatePark)) -> String {
    let mut b = Bytes::default();
    let faces = crate::clean::sim::collision_faces(stage, park);
    b.n(faces.len());
    for (t, surface) in &faces {
        t.iter().for_each(|&p| b.v(p));
        b.0.extend(surface.to_le_bytes());
    }
    b.n(park.rails.len());
    for rail in &park.rails {
        b.n(rail.len());
        rail.iter().for_each(|&p| b.v(p));
    }
    b.digest()
}

#[test]
fn each_engine_is_given_exactly_what_it_was() {
    let got: Vec<[String; 3]> = worlds().iter().map(|w| [skate3(w), thug2(w), sk8(w)]).collect();
    for (i, g) in got.iter().enumerate() {
        eprintln!("world {i}: {g:?}");
    }
    // (the worlds have what's worth keeping: vert faces, rail tubes, edges, metal)
    let pieces = &worlds()[1];
    assert!(thug2_engine::stage::triangles(&pieces.0, &pieces.1).iter().any(|t| t.flags & thug2_sys::face::VERT != 0));
    assert!(skate3_engine::stage::surfaces(&pieces.0, &pieces.1).3.iter().any(|p| !p.rail_quads.is_empty()));
    assert!(skate3_engine::stage::surfaces(&pieces.0, &pieces.1).3.iter().any(|p| !p.edges.is_empty()));
    for (i, (g, w)) in got.iter().zip(RECORDED).enumerate() {
        for (engine, (g, w)) in ["Skate 3", "THUG2", "SK8"].iter().zip(g.iter().zip(w)) {
            assert_eq!(g, w, "world {i}, {engine}");
        }
    }
}

/// Recorded at 5dc9513 (f37f837 with the engines' inputs taken out as pure functions, no behaviour change), before
/// the split. THUG2's re-recorded at cb8a75d: its upright faces a thin slab (`thug2_engine::stage::WALL_SKIN`), so a
/// wall plant can't put the skater behind a wall. THUG2's world size 2.5 column dropped with the world size
/// (2026-11-17); its size-1 digests are the ones recorded with it.
const RECORDED: [[&str; 3]; 3] = [
    ["404 bytes e94bef4d55aae7e3", "480 bytes 81d2d88f3a6315c7", "440 bytes d31134ab8c18d474"],
    ["74592 bytes 5748a3e11b86bc44", "148204 bytes d7d963805da12db0", "114444 bytes 52f682f160123635"],
    ["74592 bytes d6dfe23a7269df61", "148204 bytes f2af15ec9ea92d67", "114444 bytes 0a010c6448a0c1b1"],
];
