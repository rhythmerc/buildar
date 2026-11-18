//! The Touch controllers' state, and a Bluetooth game pad folded into it ([`pad_into_touch`]). Each engine maps the
//! state to its own game's pad (Skate 3: `skate3_engine::input`; THUG2: `thug2_engine::input`; SK8: `clean::input`).
pub(crate) use engine_api::TouchState;

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

    fn default() -> TouchState {
        TouchState::default()
    }
}
