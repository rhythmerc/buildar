//! A course's size: its world scale (game metres to a real one) as the player sets it, a multiple of its engine's own
//! ([`Engine::default_scale`]: x1.0 is a skater 1/12 life size, a kart toy-sized), in tenths from x0.1 to x15.
//!
//! Resizing a course changes only how big its game is in the room: its pieces keep their real size and place (the
//! park scale and the pieces' game positions move the other way), in every room it was put down in.
use super::parks::{self, ParkFile, Parks};
use super::{MAX_PARK_SCALE, MIN_PARK_SCALE, PlacedPieces};
use crate::engines::{Engine, Engines};
use crate::xr::rig::Rig;
use bevy::prelude::*;

/// The size's range and its step (x0.1 to x15, in tenths).
pub(crate) const MIN_TIMES: f32 = 0.1;
pub(crate) const MAX_TIMES: f32 = 15.0;
const STEPS_PER_TIMES: f32 = 10.0;

/// The size of a course at world scale `scale` for an engine whose own is `default`.
pub(crate) fn times(scale: f32, default: f32) -> f32 {
    default / scale.max(1e-6)
}

/// The world scale of size `times`.
pub(crate) fn scale_of(times: f32, default: f32) -> f32 {
    default / times.clamp(MIN_TIMES, MAX_TIMES)
}

/// A world scale kept in the sizes' range for an engine whose own is `default`.
pub(crate) fn clamped(scale: f32, default: f32) -> f32 {
    scale.clamp(default / MAX_TIMES, default / MIN_TIMES)
}

/// `times` moved `steps` tenths (on the tenths: a size between them goes to the next one that way).
pub(crate) fn stepped(times: f32, steps: i32) -> f32 {
    let tenths = (times * STEPS_PER_TIMES).round() as i32 + steps;
    (tenths as f32 / STEPS_PER_TIMES).clamp(MIN_TIMES, MAX_TIMES)
}

/// A size as shown: "x1.0".
pub(crate) fn label(times: f32) -> String {
    format!("x{:.1}", (times * STEPS_PER_TIMES).round() / STEPS_PER_TIMES)
}

/// The engine a course is played with: the one it was made for, else its mode's (the one running, if it's of that
/// mode), else the running one.
pub(crate) fn engine_of(world: &World, file: &ParkFile) -> Option<Engine> {
    file.engine
        .as_deref()
        .and_then(Engine::from_key)
        .or_else(|| file.mode().and_then(|m| parks::engine_of_mode(world, m)))
        .or_else(|| world.get_resource::<Engines>().and_then(|e| e.current()))
}

/// The world scale a course's x1.0 is.
pub(crate) fn default_of(world: &World, file: &ParkFile) -> f32 {
    engine_of(world, file).map_or(Engine::Skate3.default_scale(), Engine::default_scale)
}

/// The open course's x1.0 (none open: the running engine's).
pub(crate) fn current_default(world: &World) -> f32 {
    let parks = world.resource::<Parks>();
    match parks.current.as_ref().and_then(|c| parks.get(&c.id)) {
        Some(entry) => default_of(world, &entry.file),
        None => world.get_resource::<Engines>().and_then(|e| e.current()).map_or(Engine::Skate3.default_scale(), Engine::default_scale),
    }
}

/// The open course's size now.
pub(crate) fn current_times(world: &World) -> f32 {
    times(world.resource::<Rig>().scale, current_default(world))
}

/// The open course (or, none open, the room) resized to `times`: its pieces keep their real size and place, and its
/// other rooms' records of it follow.
pub(crate) fn set_current_times(world: &mut World, times: f32) {
    let default = current_default(world);
    let old = world.resource::<Rig>().scale;
    let scale = scale_of(times, default);
    let factor = scale / old;
    {
        let mut placed = world.resource_mut::<PlacedPieces>();
        placed.park_scale = (placed.park_scale * factor).clamp(MIN_PARK_SCALE, MAX_PARK_SCALE);
    }
    {
        // (the open room's record and the file are written by the next save; the others' records here)
        let mut parks = world.resource_mut::<Parks>();
        if let Some(current) = parks.current.clone() {
            if let Some(entry) = parks.get_mut(&current.id) {
                for record in entry.file.anchors.iter_mut().filter(|r| Some(&r.uuid) != current.anchor.as_ref()) {
                    record.scale *= factor;
                }
            }
        }
    }
    crate::xr::ui::pages::set_world_scale(world, scale);
}

/// A course's file resized to `times` (a course not open): its pieces' game positions, its park scale, its scale and
/// each room's record of it, all by the same factor - in every room it is the same real size as before.
pub(crate) fn resize_file(file: &mut ParkFile, times: f32, default: f32) {
    let scale = scale_of(times, default);
    let factor = scale / file.scale.max(1e-6);
    for piece in &mut file.pieces {
        piece.position = piece.position.map(|v| v * factor);
    }
    file.park_scale = (file.park_scale * factor).clamp(MIN_PARK_SCALE, MAX_PARK_SCALE);
    file.scale = scale;
    for record in &mut file.anchors {
        record.scale *= factor;
    }
}

/// Course `id` resized to `times`: live when it is open, else its file.
pub(crate) fn set_course_times(world: &mut World, id: &str, times: f32) {
    if world.resource::<Parks>().is_current(id) {
        set_current_times(world, times);
        return;
    }
    let Some(file) = world.resource::<Parks>().get(id).map(|p| p.file.clone()) else { return };
    let default = default_of(world, &file);
    let mut parks = world.resource_mut::<Parks>();
    if let Some(entry) = parks.get_mut(id) {
        resize_file(&mut entry.file, times, default);
    }
    parks.save(id);
}

/// Course `id`'s size.
pub(crate) fn course_times(world: &World, id: &str) -> f32 {
    if world.resource::<Parks>().is_current(id) {
        return current_times(world);
    }
    world.resource::<Parks>().get(id).map_or(1.0, |p| times(p.file.scale, default_of(world, &p.file)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_step_in_tenths_between_their_caps() {
        assert_eq!(label(times(12.0, 12.0)), "x1.0");
        assert_eq!(stepped(1.0, 1), 1.1);
        assert_eq!(stepped(1.0, -3), 0.7);
        assert_eq!(stepped(0.1, -1), MIN_TIMES);
        assert_eq!(stepped(15.0, 1), MAX_TIMES);
        // (a size off the tenths goes onto them)
        assert_eq!(label(stepped(times(10.32, 12.0), 0)), "x1.2");
        // (life size: a skater at x12, a kart at x8)
        assert_eq!(scale_of(12.0, 12.0), 1.0);
        assert_eq!(scale_of(8.0, 8.0), 1.0);
        assert_eq!(clamped(0.1, 8.0), 8.0 / MAX_TIMES);
    }

}
