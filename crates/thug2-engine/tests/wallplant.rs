//! The skater against the room's walls, on the real engine (`THUG2_GAME_DIR`: the folder holding the player's
//! `Data`; skipped without it). One test (the engine boots once a process):
//! - wall plants: the skater jumps into a wall again and again, from a little to the side each time, and after each
//!   plant is still on the room's side of it. A wall across the room at world size 1, and one turned 25° at world
//!   size 4;
//! - wall hits: rolling and jumping into a wall, straight on and glancing, slow to fast (down a ramp), each from a
//!   few places along it, at world size 1 and (a turned wall) 4: the skater never ends up behind the wall's plane.
//!   (With the wall's sides in one plane, `stage::WALL_SKIN` 0, a straight-on jump planted off the ollie's press and
//!   rode up the back of the wall every time at 1x, often at 4x; rolling and glancing hits never went in.)
use engine_core::Stage;
use engine_core::glam::Vec3;
use engine_core::skate::SkatePark;
use std::path::Path;
use thug2_engine::stage;
use thug2_sys::{self as t2, PAD_AT_REST, Pad, State, buttons};

/// How far the wall is from the room's middle, metres: the room is on its near side.
const WALL_AT: f32 = 3.0;
const PUSH: Pad = Pad { left_y: 0, ..PAD_AT_REST };

#[test]
fn the_skater_stays_on_the_rooms_side_of_a_wall() {
    let Some(dir) = std::env::var_os("THUG2_GAME_DIR") else {
        eprintln!("THUG2_GAME_DIR not set: skipped");
        return;
    };
    let cache = std::env::temp_dir().join("thug2-engine-test-cache");
    let mut engine = t2::boot(Path::new(&dir), Some(&cache), &["-level", "Default"]).expect("boot");
    for _ in 0..120 {
        engine.step(&PAD_AT_REST).expect("step");
    }
    wall_plants(&mut engine);
    wall_hits(&mut engine);
}

/// A wall across `toward` at [`WALL_AT`], 60 m long and `height` tall, as the room's Scene planes come (in both
/// windings), on a floor; and `more` (ramps).
fn room(toward: Vec3, height: f32, more: &[[Vec3; 4]]) -> Stage {
    let along = Vec3::Y.cross(toward);
    let floor = [Vec3::new(-40.0, 0.0, -40.0), Vec3::new(-40.0, 0.0, 40.0), Vec3::new(40.0, 0.0, 40.0), Vec3::new(40.0, 0.0, -40.0)];
    let base = toward * WALL_AT;
    let up = Vec3::Y * height;
    let wall = [base - along * 30.0, base + along * 30.0, base + along * 30.0 + up, base - along * 30.0 + up];
    let back = [wall[0], wall[3], wall[2], wall[1]];
    let mut room_quads = vec![floor, wall, back];
    room_quads.extend_from_slice(more);
    Stage { revision: 1, room_quads, ..Default::default() }
}

fn wall_plants(engine: &mut t2::Engine) {
    for (scale, degrees) in [(1.0f32, 0.0f32), (4.0, 25.0)] {
        // Toward the wall, and along it.
        let toward = Vec3::new(degrees.to_radians().sin(), 0.0, degrees.to_radians().cos());
        let along = Vec3::Y.cross(toward);
        let faces = stage::triangles(&room(toward, 3.0, &[]), &SkatePark::default(), scale);
        // How far past the wall's plane (negative: in the room).
        let past = |s: &t2::Skater| stage::from_game(s.position, scale).dot(toward) - WALL_AT;

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
                let mut pad = PUSH;
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

#[derive(Clone, Copy, Debug, PartialEq)]
enum Speed {
    /// Braked down to about 150 inches a second (the game kicks along by itself).
    Slow,
    /// Pushed all the way.
    Pushed,
    /// Pushed all the way down a ramp this high (metres), 2 m short of the wall.
    Ramp(f32),
}

/// One run into the wall: how fast it met it (inches a second, across the floor), how far past the wall's plane it
/// got (metres; negative: in the room), the frames it spent within 1 cm of the plane or past it, of them the frames
/// it rode the wall as its floor (its up across, not up), and the states it was in there.
struct Hit {
    speed: f32,
    furthest: f32,
    at_plane: u32,
    on_wall: u32,
    states: Vec<State>,
    /// It wall planted (the ollie's press can plant it straight into the wall).
    planted: bool,
}

fn wall_hits(engine: &mut t2::Engine) {
    let mut problems = Vec::new();
    // A wall across the room at world size 1, and one turned 25° at world size 4 (lengths in the room a quarter as
    // long there: the same in the game's units).
    for (scale, wall_degrees) in [(1.0f32, 0.0f32), (4.0, 25.0)] {
        let k = 1.0 / scale;
        let toward = Vec3::new(wall_degrees.to_radians().sin(), 0.0, wall_degrees.to_radians().cos());
        let along = Vec3::Y.cross(toward);
        let past = |s: &t2::Skater| stage::from_game(s.position, scale).dot(toward) - WALL_AT;
        // (the skin, metres: the room's side of the wall is this far in front of its plane)
        let skin = stage::WALL_SKIN / t2::INCHES_PER_METRE * k;
        for jump in [false, true] {
            for degrees in [0.0f32, 45.0, 75.0] {
                // Its heading: `degrees` off straight at the wall.
                let heading = toward * degrees.to_radians().cos() + along * degrees.to_radians().sin();
                let side = Vec3::Y.cross(heading);
                for speed in [Speed::Slow, Speed::Pushed, Speed::Ramp(1.5), Speed::Ramp(3.0), Speed::Ramp(6.0)] {
                    let mut hits = Vec::new();
                    for place in 0..5 {
                        // Where it meets the wall, a little along it each time.
                        let meet = toward * WALL_AT + along * (0.137 * place as f32 - 0.3) * k;
                        let (ramps, start) = match speed {
                            Speed::Ramp(height) => {
                                // A ramp down to 2 m short of the wall (25°), a 1.5 m deck on top.
                                let foot = meet - heading * (2.0 * k / degrees.to_radians().cos().max(0.25));
                                let top = foot - heading * (height * k / 25f32.to_radians().tan()) + Vec3::Y * height * k;
                                let back = top - heading * 1.5 * k;
                                let w = side * 1.5 * k;
                                let ramp = [foot - w, foot + w, top + w, top - w];
                                let deck = [top - w, top + w, back + w, back - w];
                                (vec![ramp, deck], back + heading * 0.5 * k)
                            }
                            _ => (Vec::new(), meet - heading * (8.0 / degrees.to_radians().cos().max(0.25)).min(12.0) * k),
                        };
                        let faces = stage::triangles(&room(toward, 12.0, &ramps), &SkatePark::default(), scale);
                        engine.set_world(&faces, Some(stage::spawn(start, heading, scale)));
                        let mut last = engine.step(&PAD_AT_REST).unwrap().unwrap();
                        let mut hit = Hit { speed: 0.0, furthest: f32::MIN, at_plane: 0, on_wall: 0, states: Vec::new(), planted: false };
                        let (mut crouch, mut jumped, mut met) = (0, false, None);
                        for frame in 0..900 {
                            let [vx, _, vz] = last.velocity;
                            let across = (vx * vx + vz * vz).sqrt();
                            let mut pad = match speed {
                                Speed::Slow if across > 150.0 => Pad { left_y: 0xff, ..PAD_AT_REST },
                                Speed::Slow => PAD_AT_REST,
                                _ => PUSH,
                            };
                            // The ollie: when it would be about a quarter second short of the wall.
                            let to_wall = -past(&last) / degrees.to_radians().cos();
                            if jump && !jumped && last.state == State::Ground
                                && to_wall < (across / t2::INCHES_PER_METRE * 0.25 + 0.3) * k
                            {
                                crouch += 1;
                                if crouch < 3 {
                                    pad.buttons = buttons::CROSS;
                                } else {
                                    jumped = true;
                                }
                            }
                            last = engine.step(&pad).unwrap().unwrap();
                            let p = past(&last);
                            hit.planted |= last.state == State::Wallplant;
                            if met.is_none() && p > -1.0 * k {
                                met = Some(frame);
                                hit.speed = across;
                            }
                            if met.is_some() {
                                hit.furthest = hit.furthest.max(p);
                                if p > -0.01 * k {
                                    hit.at_plane += 1;
                                    if last.up[1].abs() < 0.5 {
                                        hit.on_wall += 1;
                                    }
                                    if !hit.states.contains(&last.state) {
                                        hit.states.push(last.state);
                                    }
                                }
                            }
                            if met.is_some_and(|m| frame > m + 180) {
                                break;
                            }
                        }
                        if met.is_none() && std::env::var_os("WALLPLANT_DEBUG").is_some() {
                            eprintln!("  unmet {jump} {degrees} {speed:?} {place}: start {start} {:?} at {} v {:?} up {:?}", last.state, stage::from_game(last.position, scale), last.velocity, last.up);
                        }
                        hits.push(hit);
                    }
                    let how = if jump { "jumped" } else { "rolled" };
                    let speeds: Vec<i32> = hits.iter().map(|h| h.speed as i32).collect();
                    let planted = hits.iter().filter(|h| h.planted).count();
                    let behind = hits.iter().filter(|h| h.furthest > 0.0005 * k).count();
                    let inside = hits.iter().filter(|h| h.furthest > -skin - 0.0005 * k && h.furthest <= 0.0005 * k).count();
                    let furthest: Vec<String> = hits.iter().map(|h| format!("{:+.4}", h.furthest)).collect();
                    let at_plane: Vec<u32> = hits.iter().map(|h| h.at_plane).collect();
                    let on_wall: Vec<u32> = hits.iter().map(|h| h.on_wall).collect();
                    let mut states: Vec<State> = hits.iter().flat_map(|h| h.states.clone()).collect();
                    states.dedup();
                    eprintln!(
                        "HIT {scale}x wall {wall_degrees}° {how} {degrees:>2}° {speed:?}: in/s {speeds:?}; planted {planted}/5, behind {behind}/5, at the face {inside}/5; furthest m {furthest:?}; frames at the plane {at_plane:?}, riding it {on_wall:?}; {states:?}"
                    );
                    if behind > 0 {
                        problems.push(format!("{scale}x {how} {degrees}° {speed:?}: {behind}/5 behind"));
                    }
                }
            }
        }
    }
    eprintln!("wall hits (skin {} in): {}", stage::WALL_SKIN, if problems.is_empty() { "none behind".into() } else { problems.join("; ") });
    assert!(problems.is_empty(), "behind the wall after hitting it: {problems:?}");
}
