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

/// Menu navigation from the keyboard's arrows, as pad-style bits (1 up, 2 down, 4 left, 8 right),
/// with auto-repeat while held. (On the desktop it also read XInput pads; that transport is gone.)
#[derive(Resource, Default)]
pub(crate) struct Navigation {
    pub pressed: u16,
    previous: u16,
    held_for: f32,
    repeat_at: f32,
}

pub(crate) fn navigation(
    mut nav: ResMut<Navigation>,
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let mut current = 0;
    for (key, bit) in [
        (KeyCode::ArrowUp, 1),
        (KeyCode::ArrowDown, 2),
        (KeyCode::ArrowLeft, 4),
        (KeyCode::ArrowRight, 8),
    ] {
        if keys.pressed(key) {
            current |= bit;
        }
    }
    nav.pressed = current & !nav.previous;
    if current & 15 != 0 && current & 15 == nav.previous & 15 {
        nav.held_for += time.delta_secs();
        if nav.held_for >= nav.repeat_at {
            nav.pressed |= current & 15;
            nav.repeat_at += 0.09;
        }
    } else {
        nav.held_for = 0.;
        nav.repeat_at = 0.35;
    }
    nav.previous = current;
}

pub(crate) struct InputPlugin;
impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ControllerInput>()
            .init_resource::<PublishedTickInput>()
            .init_resource::<Navigation>()
            .add_systems(
                PreUpdate,
                navigation
                    .before(crate::graphics_menu::MenuInput)
                    .in_set(crate::app::SKATE3),
            )
            .add_systems(
                PreUpdate,
                poll_controllers
                    .run_if(crate::graphics_menu::gameplay_active)
                    .in_set(crate::app::SKATE3),
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
    menu: Option<Res<crate::graphics_menu::Menu>>,
) {
    let gameplay = crate::graphics_menu::gameplay_active(menu);
    published.0 = skate3_sim::input::publish(&mut input, gameplay);
}
