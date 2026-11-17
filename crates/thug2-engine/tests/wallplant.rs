//! Wall plants against the room's walls, on the real engine (`THUG2_GAME_DIR`: the folder holding the player's
//! `Data`; skipped without it): the skater jumps into a wall again and again, from a little to the side each time,
//! and after each plant is still on the room's side of it. A wall across the room at world size 1, and one turned
//! 25° at world size 4.
use engine_core::Stage;
use engine_core::glam::Vec3;
use engine_core::skate::SkatePark;
use std::path::Path;
use thug2_engine::stage;
use thug2_sys::{self as t2, PAD_AT_REST, Pad, State, buttons};

/// How far the wall is from the room's middle, metres: the room is on its near side.
const WALL_AT: f32 = 3.0;

#[test]
fn a_wall_plant_leaves_the_skater_on_the_rooms_side() {
    let Some(dir) = std::env::var_os("THUG2_GAME_DIR") else {
        eprintln!("THUG2_GAME_DIR not set: skipped");
        return;
    };
    let cache = std::env::temp_dir().join("thug2-engine-test-cache");
    let mut engine = t2::boot(Path::new(&dir), Some(&cache), &["-level", "Default"]).expect("boot");
    for _ in 0..120 {
        engine.step(&PAD_AT_REST).expect("step");
    }
    for (scale, degrees) in [(1.0f32, 0.0f32), (4.0, 25.0)] {
        // Toward the wall, and along it.
        let toward = Vec3::new(degrees.to_radians().sin(), 0.0, degrees.to_radians().cos());
        let along = Vec3::Y.cross(toward);
        // A floor, and the wall: as the room's Scene planes come (the wall in both windings).
        let floor = [Vec3::new(-8.0, 0.0, -8.0), Vec3::new(-8.0, 0.0, 8.0), Vec3::new(8.0, 0.0, 8.0), Vec3::new(8.0, 0.0, -8.0)];
        let base = toward * WALL_AT;
        let wall = [base - along * 6.0, base + along * 6.0, base + along * 6.0 + Vec3::Y * 3.0, base - along * 6.0 + Vec3::Y * 3.0];
        let back = [wall[0], wall[3], wall[2], wall[1]];
        let stage = Stage { revision: 1, room_quads: vec![floor, wall, back], ..Default::default() };
        let faces = stage::triangles(&stage, &SkatePark::default(), scale);
        // How far past the wall's plane (negative: in the room).
        let past = |s: &t2::Skater| stage::from_game(s.position, scale).dot(toward) - WALL_AT;

        let push = Pad { left_y: 0, ..PAD_AT_REST };
        let (mut plants, mut behind) = (0, Vec::new());
        for trial in 0..40 {
            // From a little to the side each time (where the plant meets the wall), the jump a little nearer or
            // further (the skater covers 1/scale the room at world size `scale`).
            let side = -2.0 + 0.0731 * trial as f32;
            let ollie_at = (1.6 + 0.083 * (trial % 11) as f32) / scale;
            let start = along * side - toward * (4.0 / scale.sqrt());
            engine.set_world(&faces, Some(stage::spawn(start, toward, scale)));
            let mut last = engine.step(&PAD_AT_REST).unwrap().unwrap();
            let (mut crouch, mut jumped, mut planted_at) = (0, false, None);
            let mut furthest = f32::MIN;
            for frame in 0..600 {
                let mut pad = push;
                if !jumped && last.state == State::Ground && -past(&last) < ollie_at {
                    // Crouch, then let go: the ollie.
                    crouch += 1;
                    if crouch < 3 {
                        pad.buttons = buttons::CROSS;
                    } else {
                        jumped = true;
                    }
                } else if jumped && planted_at.is_none() && frame % 2 == 0 {
                    pad.buttons = buttons::CROSS; // (tapped in the air: the plant)
                }
                last = engine.step(&pad).unwrap().unwrap();
                if last.state == State::Wallplant && planted_at.is_none() {
                    planted_at = Some(frame);
                }
                if let Some(at) = planted_at {
                    furthest = furthest.max(past(&last));
                    if frame > at + 90 {
                        break;
                    }
                }
            }
            if planted_at.is_some() {
                plants += 1;
                if furthest > 0.0 {
                    behind.push((trial, furthest));
                }
            }
        }
        eprintln!("world size {scale}, wall turned {degrees}°: {plants} plants, {} behind the wall", behind.len());
        assert!(plants >= 5, "the skater planted on the wall ({plants} times at world size {scale})");
        assert!(behind.is_empty(), "behind the wall after a plant (metres past it), world size {scale}: {behind:?}");
    }
}
