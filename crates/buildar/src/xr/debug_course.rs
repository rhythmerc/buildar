//! A course opened for measuring, without the room (the dev flag `course=<id, name or "busiest">`): once the head
//! is tracked, the course is opened on its own engine with its middle a couple of metres in front of the head, on
//! the floor, turned to face the player; and the room's surfaces are replaced by a debug box about it - a floor and
//! four walls, a margin out from the course - so the engine collides with that and nothing of the real room. For
//! profiling courses on an unattended headset.
//!
//! Nothing is written: the course is kept by no anchor and never saved (`build::parks::DEBUG_ANCHOR`). The menu
//! doesn't open at launch.
use super::build::{PlacedPieces, parks};
use super::dev_flags::DevFlags;
use super::planes::Shape;
use bevy::prelude::*;
use bevy_mod_xr::camera::XrCamera;

pub(crate) struct DebugCoursePlugin;

impl Plugin for DebugCoursePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugRoom>().add_systems(Update, open);
    }
}

/// The debug box the room's surfaces are replaced by (tracking space), once a course is opened for measuring.
#[derive(Resource, Default)]
pub(crate) struct DebugRoom(pub Option<Vec<Shape>>);

/// How far in front of the head the course's middle goes, m; the box's margin about the course and its walls'
/// height, m.
const AHEAD_M: f32 = 2.0;
const MARGIN_M: f32 = 1.5;
const WALL_M: f32 = 2.5;
/// Frames waited after the head is first tracked (the parks loaded, the engine started).
const SETTLE_FRAMES: u32 = 90;

/// The course the flag names: an id, a name (any case), or "busiest" (the most pieces).
fn pick(wanted: &str, parks: &[(String, String, usize)]) -> Option<String> {
    let wanted = wanted.trim();
    if wanted.eq_ignore_ascii_case("busiest") {
        return parks.iter().max_by_key(|p| p.2).map(|p| p.0.clone());
    }
    parks.iter().find(|p| p.0 == wanted || p.1.eq_ignore_ascii_case(wanted)).map(|p| p.0.clone())
}

fn open(world: &mut World, mut state: Local<(u32, bool)>) {
    if state.1 {
        return;
    }
    let Some(wanted) = world.get_resource::<DevFlags>().and_then(|f| f.value("course")) else {
        state.1 = true;
        return;
    };
    // (the head: between the eyes, once they're tracked)
    let mut eyes = world.query_filtered::<&Transform, With<XrCamera>>();
    let eyes: Vec<Transform> = eyes.iter(world).copied().collect();
    let tracked: Vec<&Transform> = eyes.iter().filter(|e| e.translation != Vec3::ZERO).collect();
    if tracked.is_empty() {
        return;
    }
    state.0 += 1;
    if state.0 < SETTLE_FRAMES {
        return;
    }
    state.1 = true;
    let listed = parks::listed(world);
    let Some(id) = pick(&wanted, &listed) else {
        warn!("DEBUG COURSE no course {wanted:?} among {:?}", listed.iter().map(|p| &p.1).collect::<Vec<_>>());
        return;
    };
    let head = tracked.iter().map(|e| e.translation).sum::<Vec3>() / tracked.len() as f32;
    let forward = (tracked[0].rotation * Vec3::NEG_Z).with_y(0.0).normalize_or(Vec3::NEG_Z);
    let floor = world.resource::<super::ar_settings::ArSettings>().floor_offset;
    let target = (head + forward * AHEAD_M).with_y(floor);
    // (turned so its -Z runs away from the player, as the player faces)
    let yaw = (-forward.x).atan2(-forward.z);
    parks::use_its_engine(world, &id);
    if !parks::open_for_debug(world, &id, target, yaw) {
        warn!("DEBUG COURSE {id} couldn't be opened");
        return;
    }
    let placed: Vec<Vec3> = world.resource::<PlacedPieces>().items.iter().map(|i| Vec3::from_array(i.position)).collect();
    let shapes = room_box(&placed, target, floor);
    info!("DEBUG COURSE {id} open at {target:.2?}; the room is a debug box of {} surfaces", shapes.len());
    world.resource_mut::<DebugRoom>().0 = Some(shapes);
}

/// A floor and four inward-facing walls about `points` (and `middle`), `MARGIN_M` out, the floor at `floor`.
pub(crate) fn room_box(points: &[Vec3], middle: Vec3, floor: f32) -> Vec<Shape> {
    let (mut min, mut max) = (middle.xz(), middle.xz());
    for p in points {
        min = min.min(p.xz());
        max = max.max(p.xz());
    }
    let (min, max) = (min - Vec2::splat(MARGIN_M), max + Vec2::splat(MARGIN_M));
    let centre = (min + max) * 0.5;
    let half = (max - min) * 0.5;
    let mut shapes = vec![Shape::Plane { pose: Transform::from_xyz(centre.x, floor, centre.y), half, floor: true }];
    // Each wall: its normal (inward), where it stands, and how long it runs.
    let walls = [
        (Vec3::X, Vec2::new(min.x, centre.y), half.y),
        (Vec3::NEG_X, Vec2::new(max.x, centre.y), half.y),
        (Vec3::Z, Vec2::new(centre.x, min.y), half.x),
        (Vec3::NEG_Z, Vec2::new(centre.x, max.y), half.x),
    ];
    for (normal, at, along) in walls {
        let rotation = Quat::from_rotation_arc(Vec3::Y, normal);
        // (local X and Z: whichever is upright spans the height, the other the wall's length)
        let x_upright = (rotation * Vec3::X).y.abs() > 0.5;
        let half = if x_upright { Vec2::new(WALL_M * 0.5, along) } else { Vec2::new(along, WALL_M * 0.5) };
        shapes.push(Shape::Plane {
            pose: Transform::from_xyz(at.x, floor + WALL_M * 0.5, at.y).with_rotation(rotation),
            half,
            floor: false,
        });
    }
    shapes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_course_is_picked_by_id_name_or_size() {
        let parks = vec![("a1".to_string(), "Park 1".to_string(), 3), ("b2".to_string(), "Big One".to_string(), 9)];
        assert_eq!(pick("a1", &parks).as_deref(), Some("a1"));
        assert_eq!(pick("big one", &parks).as_deref(), Some("b2"));
        assert_eq!(pick("busiest", &parks).as_deref(), Some("b2"));
        assert_eq!(pick("nope", &parks), None);
    }

    /// The box encloses the course with its margin; each wall faces in and spans the wall's height upright.
    #[test]
    fn the_debug_box_walls_the_course_in() {
        let shapes = room_box(&[Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 0.3, 2.0)], Vec3::ZERO, 0.0);
        assert_eq!(shapes.len(), 5);
        let Shape::Plane { half, floor: true, .. } = shapes[0] else { panic!("a floor first") };
        assert!((half - Vec2::new(1.0 + MARGIN_M, 1.5 + MARGIN_M)).length() < 1e-5);
        for shape in &shapes[1..] {
            let Shape::Plane { pose, half, floor: false } = shape else { panic!("walls") };
            let normal = pose.rotation * Vec3::Y;
            // (facing the middle of the box)
            let inward = (Vec3::new(0.0, pose.translation.y, 0.5) - pose.translation).normalize();
            assert!(normal.dot(inward) > 0.0, "{normal} at {}", pose.translation);
            let upright = if (pose.rotation * Vec3::X).y.abs() > 0.5 { half.x } else { half.y };
            assert!((upright - WALL_M * 0.5).abs() < 1e-5);
        }
    }
}
