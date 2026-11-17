//! What each engine's drawn model is given, kept exactly (`f32` bits): recorded before its model and pose went behind
//! the contract (step 6, docs/game-code-out-plan.md), so the move can't change what is drawn.
//!
//! - **Skate 3:** each bound joint's bone-local transform and the root's, from fixed ticks (a cut, a cadence change,
//!   a bone count change, an unchanged generation) at fixed alphas (outside 0..1 too), and the fallback before there
//!   is a pair. Synthetic always; on the real game too with `SKATE3_ASSET_ROOT` (its skeleton bound to its GLB, its
//!   own ticks).
//! - **THUG2:** each joint's matrix and the root between two steps' palettes and draw matrices (a short palette, junk
//!   w columns, a cut and not, the clock at fixed points of a step: re-recorded with generic pose interpolation, and
//!   again with THUG2's world size gone) and the model's parts as meshes and materials. Synthetic always; on the real game in
//!   [`thug2::real`] (ignored: the game boots once a process).
use bevy::prelude::*;

/// The bytes written, and their FNV-1a 64 digest.
#[derive(Default)]
pub(crate) struct Bytes(pub(crate) Vec<u8>);

impl Bytes {
    pub(crate) fn f(&mut self, v: f32) {
        self.0.extend(v.to_bits().to_le_bytes());
    }
    pub(crate) fn n(&mut self, n: usize) {
        self.0.extend((n as u64).to_le_bytes());
    }
    #[cfg_attr(not(feature = "thug2"), allow(dead_code))]
    pub(crate) fn m(&mut self, m: Mat4) {
        m.to_cols_array().into_iter().for_each(|x| self.f(x));
    }
    pub(crate) fn t(&mut self, t: Transform) {
        t.translation.to_array().into_iter().for_each(|x| self.f(x));
        t.rotation.to_array().into_iter().for_each(|x| self.f(x));
        t.scale.to_array().into_iter().for_each(|x| self.f(x));
    }
    pub(crate) fn digest(&self) -> String {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in &self.0 {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        format!("{} bytes {h:016x}", self.0.len())
    }
}

/// A fixed matrix from a seed: turned, moved, (a little) scaled.
fn matrix(seed: f32) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        Vec3::splat(1.0 + 0.01 * (seed * 0.7).sin()),
        Quat::from_euler(EulerRot::YXZ, seed * 0.31, (seed * 0.53).sin(), (seed * 0.17).cos() - 0.5),
        Vec3::new((seed * 1.3).sin() * 0.4, seed * 0.05, (seed * 0.9).cos() * 0.3),
    )
}

#[cfg(feature = "retail")]
mod skate3 {
    use super::*;
    use crate::model::{JointBinding, blend};
    use crate::presentation::{Presentation, Shot};
    use engine_core::Pose;
    use std::time::Duration;

    /// A completed tick as the engine gives it, with the desktop camera's shot of it and the step's period.
    pub(crate) struct Tick {
        pub pose: Pose,
        pub shot: Shot,
        pub period: Duration,
    }

    const ALPHAS: [f32; 6] = [-0.25, 0.0, 0.3, 0.5, 1.0, 1.4];

    /// What's drawn for each tick (and before any): the fallback joints before the first, then at each alpha the
    /// root's transform and each binding's joint-local one.
    pub(crate) fn drawn(ticks: &[Tick], bindings: &[(usize, Option<usize>)], fallback: &[Mat4]) -> String {
        let basis = skate3_engine::model::BASIS;
        let mut b = Bytes::default();
        let mut world = World::new();
        let bindings: Vec<JointBinding> = bindings
            .iter()
            .map(|&(joint, parent)| JointBinding { entity: world.spawn_empty().id(), joint, parent })
            .collect();
        let replay = crate::replay::Replay::default();
        let mut history = Presentation::default();
        // (no pair yet: the live pose, unblended)
        assert!(history.view(&replay, 0.5).is_none());
        let joints = crate::model::joint_transforms(&bindings, fallback, basis);
        b.n(joints.len());
        joints.iter().for_each(|&(_, t)| b.t(t));
        for tick in ticks {
            let mut pose = tick.pose.clone();
            pose.step.period = tick.period;
            history.0.offer(&pose, tick.shot);
            for alpha in ALPHAS {
                let (previous, current, alpha) = history.view(&replay, alpha).expect("a pair");
                b.t(blend(previous.root, current.root, alpha));
                let locals = crate::model::joint_locals(&bindings, &previous.joints, &current.joints, basis, alpha);
                b.n(locals.len());
                locals.into_iter().for_each(|t| b.t(t));
            }
        }
        b.digest()
    }

    /// Ticks of `bones` bones: a cut at the fourth, a cadence change at the sixth, the same generation again at the
    /// seventh, a bone more from the ninth.
    fn ticks(bones: usize) -> Vec<Tick> {
        (1..=11u64)
            .map(|g| {
                let s = g as f32;
                let count = if g >= 9 { bones + 1 } else { bones };
                Tick {
                    pose: Pose {
                        generation: if g == 7 { 6 } else { g },
                        discontinuity: g == 4,
                        root: matrix(s * 0.37 + 2.0) * Mat4::from_translation(Vec3::new(s * 0.1, 0.0, 0.0)),
                        joints: (0..count).map(|i| matrix(s * 0.21 + i as f32 * 0.77)).collect(),
                        ..Default::default()
                    },
                    shot: Shot { camera: Transform::from_matrix(matrix(s + 9.0)), fov: 1.0 + s * 0.01 },
                    period: Duration::from_micros(if g >= 6 { 33_333 } else { 16_667 }),
                }
            })
            .collect()
    }

    #[test]
    fn skate3_draws_exactly_what_it_did() {
        // (a tree, one joint with no parent bone besides the first, one bound joint's parent past it)
        let bindings: Vec<(usize, Option<usize>)> =
            (0..12).map(|i| (i, if i == 0 || i == 5 { None } else { Some((i - 1) / 2) })).chain([(3, Some(9))]).collect();
        let fallback: Vec<Mat4> = (0..12).map(|i| matrix(i as f32 * 1.9)).collect();
        let got = drawn(&ticks(12), &bindings, &fallback);
        eprintln!("skate3 synthetic: {got}");
        assert_eq!(got, RECORDED_SKATE3);
    }

    /// The real game (`SKATE3_ASSET_ROOT`: the converted data's `assets`; skipped without it): its skeleton bound
    /// to its GLB by name as the app binds it, and 150 of its own ticks standing still.
    #[cfg(feature = "xr")]
    #[test]
    fn skate3_draws_the_real_game_exactly_as_it_did() {
        let Some(root) = std::env::var_os("SKATE3_ASSET_ROOT") else {
            eprintln!("SKATE3_ASSET_ROOT not set: skipped");
            return;
        };
        let root = std::path::Path::new(&root);
        let bundle = skate3_sim::Bundle::open(root, &mut |_| {}).expect("the bundle");
        let config =
            skate3_sim::SimConfig { difficulty: skate3_sim::difficulty::Difficulty::Easy, map: None, spawn_offset: 0.0 };
        let mut sim = skate3_sim::Sim::load(&bundle, &config, &mut |_| {}).expect("the sim");
        sim.load_camera(root).expect("its camera");
        let model = skate3_engine::model::model("private/skater.glb", &sim.skater);
        let engine_core::ModelSource::Gltf { path, joint_names, .. } = &model.source else { panic!("a GLB") };
        let bindings = glb_bindings(&root.join(path), joint_names);
        assert!(bindings.len() > 20, "{} bound", bindings.len());
        let mut ticks = Vec::new();
        for i in 0..150 {
            sim.tick(true).expect("a tick");
            let Some(frame) = sim.camera_frame().copied() else { continue };
            let mut pose = Pose::default();
            skate3_engine::model::pose(&sim.skater, &frame, &mut pose);
            ticks.push(Tick {
                pose,
                shot: Shot { camera: crate::presentation::camera_transform(frame), fov: frame.field_of_view_degrees.to_radians() },
                period: if i == 100 { Duration::from_micros(1) } else { sim.period() },
            });
        }
        let mut fallback = Vec::new();
        skate3_engine::model::joints(&sim.skater, &mut fallback);
        let got = drawn(&ticks[ticks.len() - 60..], &bindings, &fallback);
        eprintln!("skate3 real: {got} ({} ticks, {} bindings)", ticks.len(), bindings.len());
        assert_eq!(got, RECORDED_SKATE3_REAL);
    }

    /// The real game's bodies (`SKATE3_ASSET_ROOT`; skipped without it): the board's footprint where the sim's deck
    /// is and along it, and the rider's bones, each joined to its parent.
    #[cfg(feature = "xr")]
    #[test]
    fn skate3_bodies_are_where_the_sim_has_them() {
        let Some(root) = std::env::var_os("SKATE3_ASSET_ROOT") else {
            eprintln!("SKATE3_ASSET_ROOT not set: skipped");
            return;
        };
        let root = std::path::Path::new(&root);
        let bundle = skate3_sim::Bundle::open(root, &mut |_| {}).expect("the bundle");
        let config =
            skate3_sim::SimConfig { difficulty: skate3_sim::difficulty::Difficulty::Easy, map: None, spawn_offset: 0.0 };
        let mut sim = skate3_sim::Sim::load(&bundle, &config, &mut |_| {}).expect("the sim");
        sim.load_camera(root).expect("its camera");
        for _ in 0..60 {
            sim.tick(true).expect("a tick");
        }
        let model = skate3_engine::model::model("private/skater.glb", &sim.skater);
        let mut pose = Pose::default();
        skate3_engine::model::pose(&sim.skater, sim.camera_frame().expect("a camera frame"), &mut pose);
        let bodies = crate::model::bodies(&model.groups, &model.parents, |p| pose.point(p));
        let deck = sim.link();
        let mut segments = 0;
        for body in &bodies {
            match body {
                engine_api::BodyShape::Footprint { middle, along, .. } => {
                    assert!((*middle - deck.deck).length() < 0.1, "the board's middle {middle} at its deck {}", deck.deck);
                    assert!(along.dot(deck.deck_forward.with_y(0.0).normalize()).abs() > 0.95, "along it: {along}");
                }
                engine_api::BodyShape::Segments(s) => {
                    segments = s.len();
                    assert!(s.iter().all(|(a, b)| a.distance(*b) < 0.6 && (*a - deck.deck).length() < 2.0), "{s:?}");
                }
            }
        }
        eprintln!("skate3 bodies: {} ({segments} rider bones)", bodies.len());
        assert_eq!(bodies.len(), 2);
        assert!(segments >= 20, "{segments} rider bones");
    }

    /// The GLB's skins bound as `animation::bind` binds them (its nodes as entities under a model root).
    #[cfg(feature = "xr")]
    fn glb_bindings(path: &std::path::Path, names: &[String]) -> Vec<(usize, Option<usize>)> {
        let bytes = std::fs::read(path).expect("the GLB");
        let gltf = gltf::Gltf::from_slice_without_validation(&bytes).expect("a GLB");
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let nodes: Vec<Entity> = gltf
            .nodes()
            .map(|n| world.spawn((Name::new(n.name().unwrap_or("").to_owned()), Transform::from_matrix(Mat4::from_cols_array_2d(&n.transform().matrix())))).id())
            .collect();
        let mut has_parent = vec![false; nodes.len()];
        for n in gltf.nodes() {
            for c in n.children() {
                world.entity_mut(nodes[c.index()]).insert(ChildOf(nodes[n.index()]));
                has_parent[c.index()] = true;
            }
        }
        for (i, &e) in nodes.iter().enumerate() {
            if !has_parent[i] {
                world.entity_mut(e).insert(ChildOf(root));
            }
        }
        let skins: Vec<Vec<Entity>> = gltf.skins().map(|s| s.joints().map(|j| nodes[j.index()]).collect()).collect();
        let node = |e: Entity| {
            let r = world.entity(e);
            r.get::<Name>().map(|n| (n.as_str().to_owned(), r.get::<Transform>().unwrap().to_matrix()))
        };
        let ancestors = |e: Entity| {
            let mut out = Vec::new();
            let mut at = e;
            while let Some(p) = world.entity(at).get::<ChildOf>() {
                at = p.parent();
                out.push(at);
            }
            out
        };
        crate::model::bind_skins(names, skins.iter().map(Vec::as_slice), node, ancestors, |e| e == root)
            .expect("bound")
            .into_iter()
            .map(|b| (b.joint, b.parent))
            .collect()
    }

    /// Recorded at 8773c47 (the old path, its maths lifted out as functions unchanged), before the model and pose went
    /// behind the contract.
    const RECORDED_SKATE3: &str = "38016 bytes 445d5099ea36739a";
    #[cfg(feature = "xr")]
    const RECORDED_SKATE3_REAL: &str = "522688 bytes 60d160ab6667a97f";
}

#[cfg(feature = "thug2")]
pub(crate) mod thug2 {
    use super::*;
    use thug2_sys as t2;

    /// Each joint's matrix and the root's between two steps' poses as the model layer draws them, with the game's
    /// clock 0, 0.4, 0.5 and 1 of a step after the newer one (and the newer alone, a cut).
    pub(crate) fn posed(before: &t2::Pose, now: &t2::Pose) -> String {
        use engine_core::Step;
        use std::time::Duration;
        let period = thug2_engine::model::PERIOD;
        let mut b = Bytes::default();
        for cut in [false, true] {
            let mut history = crate::model::PoseHistories::<()>::default();
            for (generation, game) in [(1, before), (2, now)] {
                let mut contract = engine_core::Pose::default();
                thug2_engine::model::pose(game, &mut contract);
                contract.generation = generation;
                contract.discontinuity = cut && generation == 2;
                contract.step = Step { at: period * generation as u32, period };
                history.offer(&contract, ());
            }
            let history = history.get(thug2_engine::model::SKATER).expect("its history");
            for since in [0.0, 0.4, 0.5, 1.0] {
                let (previous, current, alpha) =
                    history.view_at(period * 2 + Duration::from_secs_f64(period.as_secs_f64() * since)).expect("a pair");
                let (root, joints) = crate::model::flat_blend(previous, current, alpha, 64);
                b.m(root);
                b.n(joints.len());
                joints.into_iter().for_each(|m| b.m(m));
            }
        }
        b.digest()
    }

    /// What the palette and draw matrix are made of: how many of their matrices are mirrored or won't come apart into
    /// a move, a turn and a scale (`model::separable`, w columns fixed), their scales' range, and the most shear (the
    /// largest cosine between two of a matrix's axes).
    pub(crate) fn makeup(pose: &t2::Pose) -> (usize, usize, f32, f32, f32) {
        let mut contract = engine_core::Pose::default();
        thug2_engine::model::pose(pose, &mut contract);
        let (mut mirrored, mut broken, mut least, mut most, mut shear) = (0, 0, f32::MAX, 0.0f32, 0.0f32);
        let draw = contract.root * Mat4::from_scale(Vec3::splat(t2::INCHES_PER_METRE));
        for m in contract.joints.iter().copied().chain([draw]) {
            match crate::model::separable(m) {
                Some(true) => mirrored += 1,
                Some(false) => {}
                None => broken += 1,
            }
            let axes = [m.x_axis.truncate(), m.y_axis.truncate(), m.z_axis.truncate()];
            for a in axes {
                (least, most) = (least.min(a.length()), most.max(a.length()));
            }
            for (i, j) in [(0, 1), (1, 2), (0, 2)] {
                shear = shear.max(axes[i].normalize_or_zero().dot(axes[j].normalize_or_zero()).abs());
            }
        }
        (mirrored, broken, least, most, shear)
    }

    /// The model's parts as meshes and materials: every attribute, the indices, and the material's fields.
    pub(crate) fn built(model: &t2::SkaterModel) -> String {
        use bevy::mesh::VertexAttributeValues as V;
        let mut b = Bytes::default();
        let mut assets = Assets::<Image>::default();
        let model = thug2_engine::model::source(model);
        let engine_core::ModelSource::Meshes { meshes, textures, .. } = &model.source else { panic!("meshes") };
        let images: Vec<Handle<Image>> = textures.iter().map(|_| assets.add(Image::default())).collect();
        for part in meshes {
            let Some(mesh) = crate::model::mesh(part) else {
                b.n(0);
                continue;
            };
            b.n(1);
            for attribute in [
                Mesh::ATTRIBUTE_POSITION,
                Mesh::ATTRIBUTE_NORMAL,
                Mesh::ATTRIBUTE_UV_0,
                Mesh::ATTRIBUTE_COLOR,
                Mesh::ATTRIBUTE_JOINT_INDEX,
                Mesh::ATTRIBUTE_JOINT_WEIGHT,
            ] {
                match mesh.attribute(attribute) {
                    Some(V::Float32x3(v)) => v.iter().flatten().for_each(|&x| b.f(x)),
                    Some(V::Float32x2(v)) => v.iter().flatten().for_each(|&x| b.f(x)),
                    Some(V::Float32x4(v)) => v.iter().flatten().for_each(|&x| b.f(x)),
                    Some(V::Uint16x4(v)) => v.iter().flatten().for_each(|&x| b.n(x as usize)),
                    other => panic!("{other:?}"),
                }
            }
            let Some(bevy::mesh::Indices::U16(indices)) = mesh.indices() else { panic!("u16 indices") };
            b.n(indices.len());
            indices.iter().for_each(|&i| b.n(i as usize));
            let m = crate::model::material(&part.material, &images);
            m.base_color.to_srgba().to_f32_array().into_iter().for_each(|x| b.f(x));
            b.n(m.base_color_texture.as_ref().and_then(|t| images.iter().position(|i| i == t)).map_or(usize::MAX, |i| i));
            match m.alpha_mode {
                AlphaMode::Opaque => b.n(0),
                AlphaMode::Mask(c) => {
                    b.n(1);
                    b.f(c)
                }
                AlphaMode::Blend => b.n(2),
                other => panic!("{other:?}"),
            }
            b.n(m.double_sided as usize);
            b.n(match m.cull_mode {
                None => 0,
                Some(bevy::render::render_resource::Face::Back) => 1,
                Some(bevy::render::render_resource::Face::Front) => 2,
            });
            b.f(m.perceptual_roughness);
            b.n(m.unlit as usize);
        }
        b.digest()
    }

    /// A row-major palette matrix with junk in its w column (the game leaves it so).
    fn palette(seed: f32) -> [f32; 16] {
        let mut m = matrix(seed).transpose().to_cols_array();
        (m[3], m[7], m[11], m[15]) = (seed, -seed, 0.5, 7.0);
        m
    }

    fn vertex(p: [f32; 3], n: [f32; 3], seed: u32) -> t2::ffi::Thug2Vertex {
        t2::ffi::Thug2Vertex {
            position: p,
            normal: n,
            color: 0x80ff_4020u32.wrapping_add(seed * 0x0103_0507),
            uv: [p[0] * 0.1, p[2] * 0.2],
            bones: [(seed % 30) as u8, ((seed + 7) % 30) as u8, 40, 3],
            weights: if seed % 5 == 0 { [0.0; 4] } else { [0.5, 0.3, 0.4, 0.9] },
        }
    }

    /// Parts: one wound as ours, one wound the other way, one empty; blended, alpha-tested, opaque; two-sided and not.
    fn model() -> t2::SkaterModel {
        let quad = |flip: bool, y: f32| {
            let up = if flip { [0.0, -1.0, 0.0] } else { [0.0, 1.0, 0.0] };
            let vertices = (0..4u32)
                .map(|i| vertex([(i % 2) as f32 * 10.0, y, (i / 2) as f32 * 10.0], up, i * 3 + y as u32))
                .collect();
            (vertices, vec![0u16, 2, 1, 1, 2, 3])
        };
        let (v0, i0) = quad(false, 0.0);
        let (v1, i1) = quad(true, 5.0);
        t2::SkaterModel {
            meshes: vec![
                t2::SkaterMesh { vertices: v0, indices: i0, texture: Some(1), two_sided: false, alpha_ref: 0, blended: true, color: [0.5, 0.25, 0.75, 0.5] },
                t2::SkaterMesh { vertices: v1, indices: i1, texture: None, two_sided: true, alpha_ref: 3, blended: false, color: [0.6, 0.5, 0.1, 1.0] },
                t2::SkaterMesh { texture: Some(9), alpha_ref: 128, ..default() },
            ],
            textures: vec![t2::Texture::default(), t2::Texture::default()],
        }
    }

    #[test]
    fn thug2_draws_exactly_what_it_did() {
        let pose = |seed: f32| {
            let mut draw = palette(3.3 + seed);
            // (in game units, moved a step: 12 inches along x)
            (draw[12], draw[13], draw[14]) = (100.0 + seed * 120.0, 2.0 + seed * 10.0, -40.0 + seed * 20.0);
            t2::Pose { palette: (0..40).map(|i| palette(i as f32 * 0.61 + seed)).collect(), draw }
        };
        let got = [posed(&pose(0.0), &pose(0.1)), built(&model())];
        eprintln!("thug2 synthetic: {got:?}");
        assert_eq!(got, RECORDED_THUG2);
    }

    /// The real game (`THUG2_GAME_DIR`; alone, as the game boots once a process: `cargo test ... model_parity::thug2::
    /// real -- --ignored`): its model, and its pose after a fixed run on a floor.
    #[test]
    #[ignore = "needs the player's THUG2 files; boots the game (once a process)"]
    fn real() {
        let Some(dir) = std::env::var_os("THUG2_GAME_DIR") else { return };
        let cache = std::env::temp_dir().join("thug2-model-parity-cache");
        let mut engine = t2::boot(std::path::Path::new(&dir), Some(&cache), &["-level", "Default"]).expect("boot");
        for _ in 0..120 {
            engine.step(&t2::PAD_AT_REST).expect("step");
        }
        let (cx, cz) = (50_000.0, 50_000.0);
        let floor = [
            t2::Triangle { v: [[cx - 400.0, 0.0, cz - 400.0], [cx - 400.0, 0.0, cz + 400.0], [cx + 400.0, 0.0, cz + 400.0]], flags: t2::face::SKATABLE, terrain: 0 },
            t2::Triangle { v: [[cx - 400.0, 0.0, cz - 400.0], [cx + 400.0, 0.0, cz + 400.0], [cx + 400.0, 0.0, cz - 400.0]], flags: t2::face::SKATABLE, terrain: 0 },
        ];
        engine.set_world(&floor, Some(t2::Spawn { point: [cx, 20.0, cz - 300.0], heading_degrees: 0.0 }));
        let (mut previous, mut current) = (t2::Pose::default(), t2::Pose::default());
        let (mut mirrored, mut broken, mut least, mut most, mut shear) = (0, 0, f32::MAX, 0.0f32, 0.0f32);
        for frame in 0..90 {
            let mut pad = t2::PAD_AT_REST;
            if frame >= 20 {
                pad.left_y = 0;
            }
            if (60..=64).contains(&frame) {
                pad.buttons |= t2::buttons::CROSS;
            }
            engine.step(&pad).expect("step");
            std::mem::swap(&mut previous, &mut current);
            assert!(engine.skater_pose(&mut current));
            let (m, b, l, h, s) = makeup(&current);
            (mirrored, broken, least, most, shear) = (mirrored + m, broken + b, least.min(l), most.max(h), shear.max(s));
        }
        let model = engine.skater_model().expect("its model");
        let got = [posed(&previous, &current), built(&model)];
        eprintln!("thug2 real: {got:?} ({} parts, {} joints)", model.meshes.len(), current.palette.len());
        eprintln!(
            "thug2 real palettes and draw matrices, 90 steps: {mirrored} mirrored, {broken} not separable, axis lengths \
             {least}..{most}, most shear (cosine) {shear}"
        );
        assert_eq!(got, RECORDED_THUG2_REAL);
    }

    /// Recorded at 8773c47, as Skate 3's (the poses: ["33344 bytes 9f9943545708ccca"] synthetic,
    /// ["33344 bytes f6763fb9dcb1e942"] real). Re-recorded with generic pose interpolation, on purpose: the poses are
    /// now drawn between the game's last two steps' poses (root and every joint blended by the model layer), not the
    /// newest pose slid back by the skater's move, so `posed` takes two poses and its digests changed; the parts
    /// (`built`) are unchanged. The real game's palettes and draw matrices over the run: none mirrored, none that won't
    /// come apart, axes 0.81..1.03 long, no shear (cosines under 1e-6).
    /// Re-recorded when THUG2's world size went (2026-11-17): `posed` ran world sizes 1 and 2.5 into one digest
    /// (["66688 bytes d9a041747b850d5b"] synthetic, ["66688 bytes de9d8b900e1813cc"] real); these are its size-1 pass
    /// alone, recorded with the world size still in the code (the first half of the same bytes), so the removal
    /// changes nothing at size 1.
    const RECORDED_THUG2: [&str; 2] = ["33344 bytes db16f02920af864c", "1028 bytes 5cf400bfb29176e8"];
    const RECORDED_THUG2_REAL: [&str; 2] = ["33344 bytes 2127efec3eab0b44", "568712 bytes 88922c8caceb6a32"];
}
