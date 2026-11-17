//! Wall plants against the room's walls, on the real engine (`THUG2_GAME_DIR`: the folder holding the player's
//! `Data`; skipped without it): the skater jumps into a wall again and again, from a little to the side each time,
//! and after each plant is still on the room's side of it.
use engine_core::Stage;
use engine_core::glam::Vec3;
use engine_core::skate::SkatePark;
use std::path::Path;
use thug2_engine::stage;
use thug2_sys::{self as t2, PAD_AT_REST, Pad, State, buttons};

/// The wall's plane (z, metres): the room is on its -z side.
const WALL_Z: f32 = 3.0;

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
    // A floor, and a wall across +z: as the room's Scene planes come (the wall in both windings).
    let floor = [Vec3::new(-6.0, 0.0, -6.0), Vec3::new(-6.0, 0.0, 6.0), Vec3::new(6.0, 0.0, 6.0), Vec3::new(6.0, 0.0, -6.0)];
    let wall = [Vec3::new(-6.0, 0.0, WALL_Z), Vec3::new(6.0, 0.0, WALL_Z), Vec3::new(6.0, 3.0, WALL_Z), Vec3::new(-6.0, 3.0, WALL_Z)];
    let back = [wall[0], wall[3], wall[2], wall[1]];
    let stage = Stage { revision: 1, room_quads: vec![floor, wall, back], ..Default::default() };
    let faces = stage::triangles(&stage, &SkatePark::default(), 1.0);

    let push = Pad { left_y: 0, ..PAD_AT_REST };
    let (mut plants, mut behind) = (0, Vec::new());
    for trial in 0..40 {
        // From a little to the side each time (where the plant meets the wall), the jump a little nearer or further.
        let x = -2.0 + 0.0731 * trial as f32;
        let ollie_at = 1.6 + 0.083 * (trial % 11) as f32;
        engine.set_world(&faces, Some(stage::spawn(Vec3::new(x, 0.0, -4.0), Vec3::Z, 1.0)));
        let mut last = engine.step(&PAD_AT_REST).unwrap().unwrap();
        let (mut crouch, mut jumped, mut planted_at) = (0, false, None);
        let mut furthest = f32::MIN;
        for frame in 0..600 {
            let here = stage::from_game(last.position, 1.0);
            let mut pad = push;
            if !jumped && last.state == State::Ground && WALL_Z - here.z < ollie_at {
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
                furthest = furthest.max(stage::from_game(last.position, 1.0).z);
                if frame > at + 90 {
                    break;
                }
            }
        }
        if planted_at.is_some() {
            plants += 1;
            eprintln!("trial {trial}: planted; furthest z {furthest:.4} (wall {WALL_Z})");
            if furthest > WALL_Z {
                behind.push((trial, furthest));
            }
        } else {
            eprintln!("trial {trial}: no plant ({:?} at {:?})", last.state, stage::from_game(last.position, 1.0));
        }
    }
    eprintln!("{plants} plants, {} behind the wall", behind.len());
    assert!(plants >= 10, "the skater planted on the wall ({plants} times)");
    assert!(behind.is_empty(), "behind the wall after a plant: {behind:?}");
}
