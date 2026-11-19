//! Courses played as the player plays them from the menu, unattended (the dev flag `play=<name>;<name>;...`): for
//! reproducing on the simulator what the player does on the headset. Once the head is tracked, each course in turn
//! is played by the parks screen's own Play (`ui::parks::play`), its engine switched to as the button does; one not
//! kept in this room (none is, on the simulator) is put down a couple of metres ahead, as the player's trigger
//! would. Between courses the menu is opened again, as the player pauses to pick the next. `play_every=N`: seconds
//! between them (12 by default).
use super::dev_flags::DevFlags;
use bevy::prelude::*;
use bevy_mod_xr::camera::XrCamera;

pub(crate) struct DevPlayPlugin;

impl Plugin for DevPlayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, play);
    }
}

#[derive(Default)]
struct State {
    /// The courses still to play; read once.
    left: Option<Vec<String>>,
    /// Seconds since the head was first tracked; when the next is played.
    clock: f32,
    next: f32,
    /// The menu opened ahead of the next (a frame before it is played).
    menu: bool,
}

/// How long after the head is tracked the first is played, s.
const FIRST_S: f32 = 4.0;
/// How far ahead a course not kept here is put down, m.
const AHEAD_M: f32 = 2.0;

fn play(world: &mut World, mut state: Local<State>) {
    let left = state.left.get_or_insert_with(|| {
        let names = world.get_resource::<DevFlags>().and_then(|f| f.value("play")).unwrap_or_default();
        names.split(';').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
    });
    if left.is_empty() {
        return;
    }
    let mut eyes = world.query_filtered::<&Transform, With<XrCamera>>();
    let Some(eye) = eyes.iter(world).find(|e| e.translation != Vec3::ZERO).copied() else { return };
    let every = world.get_resource::<DevFlags>().and_then(|f| f.number("play_every")).map_or(12.0, |n| n as f32);
    if state.next == 0.0 {
        state.next = FIRST_S;
    }
    state.clock += world.resource::<Time<Real>>().delta_secs();
    if state.clock < state.next {
        return;
    }
    if !state.menu {
        // (the menu up, as the player opens it to pick)
        super::ui::open_menu_for_dev(world);
        state.menu = true;
        return;
    }
    state.menu = false;
    let name = state.left.as_mut().unwrap().remove(0);
    state.next = state.clock + every;
    let found = {
        let parks = world.resource::<super::build::parks::Parks>();
        parks.list.iter().find(|p| p.id == name || p.file.name.eq_ignore_ascii_case(&name)).map(|p| p.id.clone())
    };
    let Some(id) = found else {
        warn!("DEV PLAY no course {name:?}");
        return;
    };
    let forward = (eye.rotation * Vec3::NEG_Z).with_y(0.0).normalize_or(Vec3::NEG_Z);
    let floor = world.resource::<super::ar_settings::ArSettings>().floor_offset;
    let ahead = (eye.translation + forward * AHEAD_M).with_y(floor);
    info!("DEV PLAY {name} ({id})");
    super::ui::play_for_dev(world, &id, ahead);
}
