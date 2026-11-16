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

/// The pad: the XR Touch controllers (`xr::actions`). When this resource exists it owns
/// controller slot 0; the other slots are always disconnected.
#[derive(Resource, Default)]
pub(crate) struct ExternalPad {
    state: Option<(u16, [u8; 2], [i16; 2], [i16; 2])>,
    packet: u32,
}

impl ExternalPad {
    #[cfg(feature = "xr")]
    pub(crate) fn set(&mut self, state: skate_core::input::xbox::XboxState) {
        let next = Some((state.buttons, state.triggers, state.left, state.right));
        if next != self.state {
            self.packet = self.packet.wrapping_add(1);
            self.state = next;
        }
    }

    fn packet(&self) -> Result<platform::DevicePacket, platform::DeviceError> {
        let (buttons, triggers, left, right) =
            self.state.ok_or(platform::DeviceError::Disconnected)?;
        Ok(platform::DevicePacket {
            number: self.packet,
            state: skate_core::input::xbox::XboxState {
                buttons,
                triggers,
                left,
                right,
            },
            // XINPUT_DEVSUBTYPE_GAMEPAD.
            subtype: 1,
        })
    }
}

pub(crate) struct InputPlugin;
impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ControllerInput>()
            .init_resource::<PublishedTickInput>()
            .add_systems(
                PreUpdate,
                poll_controllers.in_set(crate::app::SKATE3),
            )
            .add_systems(FixedUpdate, publish_actions.in_set(SimulationSet::Input));
    }
}

pub(crate) fn poll_controllers(
    mut input: ResMut<ControllerInput>,
    external: Option<Res<ExternalPad>>,
) {
    let previous = input.status;
    input.collect(std::array::from_fn(|slot| {
        if slot == 0
            && let Some(external) = &external
        {
            external.packet()
        } else {
            // (only the external pad: the desktop's XInput transport is gone)
            Err(platform::DeviceError::Disconnected)
        }
    }));
    for (index, (&before, &after)) in previous.iter().zip(&input.status).enumerate() {
        if before != after {
            match after {
                ControllerStatus::Ready if index == 0 && external.is_some() => {
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
