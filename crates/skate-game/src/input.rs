//! Platform input adapter; no animation or physics state mutation here.
use crate::app::SimulationSet;
use bevy::prelude::*;

// The pad history and gesture recognisers are the sim's (skate3-sim); this module is the Bevy side.
pub(crate) use skate3_sim::input::ControllerStatus;
#[allow(unused_imports)]
pub(crate) use skate3_sim::input::{gesture_catalog, gesture_input, gesture_mapping};
pub(crate) mod platform;
use skate_core::input::tick::TickInput;

/// The sim's [`skate3_sim::input::ControllerInput`] as a Bevy resource (a transitional shim: the
/// sim has no Bevy, so the `Resource` impl lives on this wrapper).
#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct ControllerInput(pub skate3_sim::input::ControllerInput);

#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct PublishedTickInput(pub TickInput);

impl Default for PublishedTickInput {
    fn default() -> Self {
        Self(skate3_sim::input::idle_tick_input())
    }
}

/// Controller slot 0: the Touch controllers (`skate3_engine::input`). Disconnected until it is given a state (only in
/// the headset); the other slots are always disconnected.
#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct ExternalPad(pub skate3_engine::input::ExternalPad);

pub(crate) struct InputPlugin;
impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ControllerInput>()
            .init_resource::<PublishedTickInput>()
            .init_resource::<ExternalPad>()
            .add_systems(
                PreUpdate,
                poll_controllers.in_set(crate::app::SKATE3),
            )
            .add_systems(FixedUpdate, publish_actions.in_set(SimulationSet::Input));
        #[cfg(feature = "xr")]
        app.add_systems(
            PreUpdate,
            pad_from_touch
                .after(engine_api::HostInput)
                .before(poll_controllers)
                .run_if(bevy_mod_openxr::openxr_session_running)
                .in_set(crate::app::SKATE3),
        );
    }
}

/// The game's pad from the Touch controllers (`skate3_engine::input`), neutral while the shared layer has them.
#[cfg(feature = "xr")]
fn pad_from_touch(touch: Res<engine_api::LatestTouch>, held: Res<engine_api::PadHeld>, mut pad: ResMut<ExternalPad>) {
    pad.set_input(&touch.0, **held);
}

pub(crate) fn poll_controllers(mut input: ResMut<ControllerInput>, external: Res<ExternalPad>) {
    let previous = input.status;
    input.collect(std::array::from_fn(|slot| {
        if slot == 0 {
            external.packet()
        } else {
            // (only the external pad: the desktop's XInput transport is gone)
            Err(platform::DeviceError::Disconnected)
        }
    }));
    for (index, (&before, &after)) in previous.iter().zip(&input.status).enumerate() {
        if before != after {
            match after {
                ControllerStatus::Ready if index == 0 => {
                    info!("Controller {index}: XR Touch controllers ready")
                }
                ControllerStatus::Ready => info!("Controller {index}: ready"),
                ControllerStatus::Unavailable(platform::DeviceError::Disconnected) => {
                    info!("Controller {index}: disconnected");
                }
                _ => warn!("Controller {index}: {after:?}"),
            }
        }
    }
}

fn publish_actions(
    mut input: ResMut<ControllerInput>,
    mut published: ResMut<PublishedTickInput>,
) {
    // (always gameplay: the flat pause menu is gone, and the headset's menus hold the fixed tick instead: `app::Hold`)
    published.0 = skate3_sim::input::publish(&mut input, true);
}
