//! Quest Touch controllers presented as the Xbox pad the imported-disc tier's input pipeline was
//! built for. Everything downstream (the TU3 conversion in `skate_core::input::xbox`, gesture
//! mapping, flick-it) sees an ordinary `XboxState`, so skating on Touch uses the exact same code as
//! on a pad.
//!
//! Face buttons, sticks, triggers and stick clicks map one-to-one (the Touch labels A/B/X/Y match
//! the Xbox ones); the grips are the bumpers. The left menu button is the shared layer's (a tap is
//! the headset menu, a hold build mode, menu + both grips the SK8 sheet), so while it is held the
//! left stick and X do not reach the game. Touch has no D-pad, Back or Start, and nothing on the
//! headset reads them.
#[cfg(feature = "retail")]
pub(crate) use skate_core::input::xbox::XboxState;
pub(crate) use engine_api::TouchState;

#[cfg(feature = "retail")]
mod bits {
    pub(crate) const LEFT_THUMB: u16 = 0x0040;
    pub(crate) const RIGHT_THUMB: u16 = 0x0080;
    pub(crate) const LEFT_SHOULDER: u16 = 0x0100;
    pub(crate) const RIGHT_SHOULDER: u16 = 0x0200;
    pub(crate) const A: u16 = 0x1000;
    pub(crate) const B: u16 = 0x2000;
    pub(crate) const X: u16 = 0x4000;
    pub(crate) const Y: u16 = 0x8000;
}
#[cfg(feature = "retail")]
use bits::*;

/// A grip counts as a held bumper past this pull.
#[cfg(feature = "retail")]
const GRIP_PRESSED: f32 = 0.5;

/// The Xbox pad state for the Touch controllers' state.
#[cfg(feature = "retail")]
pub(crate) fn touch_to_pad(touch: &TouchState) -> XboxState {
    let mut buttons = 0u16;
    // The menu button's chords are the shared layer's: neither the stick nor X leaks into the game.
    let (left, x) = if touch.menu { ([0.0, 0.0], false) } else { (touch.left_stick, touch.x) };
    for (pressed, bit) in [
        (touch.a, A),
        (touch.b, B),
        (x, X),
        (touch.y, Y),
        (touch.left_stick_click, LEFT_THUMB),
        (touch.right_stick_click, RIGHT_THUMB),
        (touch.left_grip >= GRIP_PRESSED, LEFT_SHOULDER),
        (touch.right_grip >= GRIP_PRESSED, RIGHT_SHOULDER),
    ] {
        if pressed {
            buttons |= bit;
        }
    }
    XboxState {
        buttons,
        triggers: [trigger(touch.left_trigger), trigger(touch.right_trigger)],
        left: stick(left),
        right: stick(touch.right_stick),
    }
}

/// A game pad's buttons, as `pad_into_touch` takes them (the winit Android backend's bits).
#[cfg(any(target_os = "android", test))]
pub(crate) mod pad {
    pub const A: u32 = 1 << 0;
    pub const B: u32 = 1 << 1;
    pub const X: u32 = 1 << 2;
    pub const Y: u32 = 1 << 3;
    pub const LB: u32 = 1 << 4;
    pub const RB: u32 = 1 << 5;
    pub const LS: u32 = 1 << 6;
    pub const RS: u32 = 1 << 7;
    pub const START: u32 = 1 << 8;
    pub const SELECT: u32 = 1 << 9;
}

/// A game pad (on Quest, a Bluetooth Xbox controller) folded into the Touch state, so it drives the
/// skater, the menus and build mode exactly as the controllers do: the same buttons, the bumpers as
/// the grips, Start (or View) as the menu button. Axes are the pad's as Android gives them (left x,
/// left y, right x, right y with +y down, left / right trigger 0..1); each stick or trigger is
/// whichever of the two is pushed further, buttons either.
#[cfg(any(target_os = "android", test))]
pub(crate) fn pad_into_touch(t: TouchState, axes: [f32; 8], buttons: u32) -> TouchState {
    let wider = |a: f32, b: f32| if b.abs() > a.abs() { b } else { a };
    let held = |bit: u32| buttons & bit != 0;
    TouchState {
        left_stick: [wider(t.left_stick[0], axes[0]), wider(t.left_stick[1], -axes[1])],
        right_stick: [wider(t.right_stick[0], axes[2]), wider(t.right_stick[1], -axes[3])],
        left_trigger: t.left_trigger.max(axes[4]),
        right_trigger: t.right_trigger.max(axes[5]),
        left_grip: if held(pad::LB) { 1.0 } else { t.left_grip },
        right_grip: if held(pad::RB) { 1.0 } else { t.right_grip },
        a: t.a || held(pad::A),
        b: t.b || held(pad::B),
        x: t.x || held(pad::X),
        y: t.y || held(pad::Y),
        left_stick_click: t.left_stick_click || held(pad::LS),
        right_stick_click: t.right_stick_click || held(pad::RS),
        menu: t.menu || held(pad::START) || held(pad::SELECT),
    }
}

#[cfg(feature = "retail")]
fn trigger(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(feature = "retail")]
fn stick([x, y]: [f32; 2]) -> [i16; 2] {
    // XInput's range is asymmetric (-32768..=32767); full deflection either
    // way lands on the ends the TU3 conversion normalises by 1/32768.
    let axis = |v: f32| {
        let v = v.clamp(-1.0, 1.0);
        if v < 0.0 {
            (v * 32768.0).round() as i16
        } else {
            (v * 32767.0).round() as i16
        }
    };
    [axis(x), axis(y)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pad_drives_the_touch_state() {
        let idle = TouchState::default();
        let t = pad_into_touch(idle, [0.5, -1.0, 0.0, 0.8, 1.0, 0.25, 0.0, 0.0], pad::A | pad::RB | pad::START);
        assert_eq!(t.left_stick, [0.5, 1.0], "pad +y is down, Touch +y is up");
        assert_eq!(t.right_stick, [0.0, -0.8]);
        assert_eq!((t.left_trigger, t.right_trigger), (1.0, 0.25));
        assert_eq!((t.left_grip, t.right_grip), (0.0, 1.0));
        assert!(t.a && !t.b && t.menu);
        // the Touch controllers still count: the stronger of the two
        let touch = TouchState { left_stick: [-0.9, 0.0], b: true, ..default() };
        let t = pad_into_touch(touch, [0.3; 8], 0);
        assert_eq!(t.left_stick[0], -0.9);
        assert!(t.b);
    }

    #[cfg(feature = "retail")]
    #[test]
    fn buttons_sticks_and_triggers_map_one_to_one() {
        let state = touch_to_pad(&TouchState {
            left_stick: [-1.0, 1.0],
            right_stick: [0.5, -0.25],
            left_trigger: 1.0,
            right_trigger: 0.5,
            left_grip: 0.9,
            right_grip: 0.2,
            a: true,
            y: true,
            right_stick_click: true,
            ..default()
        });
        assert_eq!(state.buttons, A | Y | RIGHT_THUMB | LEFT_SHOULDER);
        assert_eq!(state.left, [-32768, 32767]);
        assert_eq!(state.right, [16384, -8192]);
        assert_eq!(state.triggers, [255, 128]);
    }

    #[cfg(feature = "retail")]
    #[test]
    fn out_of_range_runtime_values_are_clamped() {
        let state = touch_to_pad(&TouchState {
            left_stick: [1.2, -3.0],
            left_trigger: -0.1,
            right_trigger: 1.5,
            ..default()
        });
        assert_eq!(state.left, [32767, -32768]);
        assert_eq!(state.triggers, [0, 255]);
    }

    #[cfg(feature = "retail")]
    #[test]
    fn menu_held_keeps_the_left_stick_and_x_from_the_game() {
        let held = touch_to_pad(&TouchState { menu: true, left_stick: [0.9, 0.0], x: true, a: true, ..default() });
        assert_eq!(held.buttons, A, "only the menu's chord inputs are held back");
        assert_eq!(held.left, [0, 0], "stick does not also steer");
        let released = touch_to_pad(&TouchState { left_stick: [0.9, 0.0], x: true, ..default() });
        assert_eq!(released.buttons, X);
        assert_ne!(released.left, [0, 0]);
    }

    fn default() -> TouchState {
        TouchState::default()
    }
}
