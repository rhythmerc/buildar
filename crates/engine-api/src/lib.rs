//! What the headset app and an engine say to each other (docs/engine-packaging-proposal.md,
//! docs/buildar-proposal.md).
//!
//! The app (`skate-game`) carries the engines and the shared XR layer: the room, the courses, the menus, the
//! controllers. An engine implements a [`Mode`]: it runs that mode's game in that world. The root of this crate is
//! what every mode shares:
//! - **To the engine:** the controllers ([`LatestTouch`]) and whether the shared layer has them ([`PadHeld`]).
//! - **From the engine:** its own HUD panels ([`HandHud`], drawn by an [`EngineHudCamera`]), the settings a course
//!   keeps ([`CourseSettings`]).
//! - **Ownership:** an engine's per-frame systems are those in its [`EngineSet`]; what they spawn is
//!   [`EngineOwned`], hidden and silenced while another engine runs. [`EngineSwitched`] and [`BeforeSwitch`] mark a
//!   switch.
//!
//! Each mode's own part is a module: [`skate`] (the park and room ridden, the skater, its trick text).
//!
//! The registry that runs the engines and switches between them is the app's (`skate-game`'s `engines`).
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod skate;

// ---- Modes, engines and ownership ----

/// What kind of game an engine plays, and so what kind of course it is played on and built with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Skating a park: pieces placed in the room, and the room itself ([`skate`]).
    Skate,
}

impl Mode {
    pub const ALL: [Mode; 1] = [Mode::Skate];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Skate => "Skate",
        }
    }

    /// Its name in files (a course's `mode` tag).
    pub fn key(self) -> &'static str {
        match self {
            Mode::Skate => "skate",
        }
    }

    pub fn from_key(key: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.key() == key.trim())
    }

    /// What its courses are called.
    pub fn course(self) -> &'static str {
        match self {
            Mode::Skate => "park",
        }
    }
}

/// An engine: a game run in the room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Engine {
    /// The clean-room engine (no game data).
    Sk8,
    /// The faithful port of Skate 3, on the player's imported data.
    Skate3,
    /// THUG2's own engine (the decomp), on the player's game files.
    Thug2,
}

impl Engine {
    pub const ALL: [Engine; 3] = [Engine::Sk8, Engine::Skate3, Engine::Thug2];

    pub fn name(self) -> &'static str {
        match self {
            Engine::Sk8 => "SK8",
            Engine::Skate3 => "Skate 3",
            Engine::Thug2 => "THUG2",
        }
    }

    /// Its name in files (`engine.txt`, a park's engine tag).
    pub fn key(self) -> &'static str {
        match self {
            Engine::Sk8 => "sk8",
            Engine::Skate3 => "skate3",
            Engine::Thug2 => "thug2",
        }
    }

    pub fn from_key(key: &str) -> Option<Engine> {
        Engine::ALL.into_iter().find(|e| e.key() == key.trim())
    }

    /// The mode it implements.
    pub fn mode(self) -> Mode {
        match self {
            Engine::Sk8 | Engine::Skate3 | Engine::Thug2 => Mode::Skate,
        }
    }
}

/// Whether an engine can run now; `Err` says why not (for the menu).
pub type Availability = fn(&World) -> Result<(), String>;

/// The set an engine's per-frame systems go in, in any schedule: what is in it (directly or through sets of its
/// own) is that engine's - run only while it runs, and what it spawns is its own. Nothing else is: a plugin of
/// Bevy's or of the shared layer's added while an engine registers stays shared.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EngineSet(pub Engine);

/// Spawned by an engine: hidden and silenced while another runs.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineOwned(pub Engine);

/// Sent after a switch (also the first engine's start, with `from` none), once the new engine is running.
#[derive(Message, Clone, Copy, Debug)]
pub struct EngineSwitched {
    pub from: Option<Engine>,
    pub to: Engine,
}

/// Run before a switch with the engine being left (the XR layer notes where the skater is).
#[derive(Resource, Default)]
pub struct BeforeSwitch(pub Vec<fn(&mut World, Engine)>);

/// A park setting's getter and setter.
pub type CourseSettingHooks = (fn(&World) -> serde_json::Value, fn(&mut World, &serde_json::Value));

/// What of an engine's settings a course keeps (THUG2's world size and game speed: what the park was built to), and
/// how to put them back: each engine that has any registers them. Saved with the park, applied when it's opened.
#[derive(Resource, Default)]
pub struct CourseSettings(pub HashMap<Engine, CourseSettingHooks>);

impl CourseSettings {
    /// `engine`'s settings now (none: it keeps none).
    pub fn of(world: &World, engine: Engine) -> Option<serde_json::Value> {
        let (get, _) = *world.get_resource::<CourseSettings>()?.0.get(&engine)?;
        Some(get(world))
    }

    /// `engine`'s settings put back as `settings` say.
    pub fn apply(world: &mut World, engine: Engine, settings: &serde_json::Value) {
        let Some(&(_, set)) = world.get_resource::<CourseSettings>().and_then(|p| p.0.get(&engine)) else { return };
        set(world, settings);
    }
}

// ---- Input ----

/// One frame of raw Touch state, OpenXR ranges: sticks -1..1 (+y forward),
/// triggers and grips 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TouchState {
    pub left_stick: [f32; 2],
    pub right_stick: [f32; 2],
    pub left_trigger: f32,
    pub right_trigger: f32,
    pub left_grip: f32,
    pub right_grip: f32,
    pub a: bool,
    pub b: bool,
    pub x: bool,
    pub y: bool,
    pub left_stick_click: bool,
    pub right_stick_click: bool,
    pub menu: bool,
}

/// This frame's Touch state (a Bluetooth game pad, when one speaks, folded in).
#[derive(Resource, Default)]
pub struct LatestTouch(pub TouchState);

/// The shared layer's input for this frame (PreUpdate): the controllers read ([`LatestTouch`]), and whether the
/// shared layer has them ([`PadHeld`]). An engine reads its pad after this.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostInput;

/// Whether the controllers are the shared layer's this frame.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct PadHeld {
    /// A menu open, build mode, aiming a spawn, placing the skater or a park: the skater gets a neutral pad.
    pub held: bool,
    /// A menu open or build mode (the player is busy elsewhere): an engine may pause its game.
    pub pause: bool,
}

// ---- HUDs ----

/// The controller a game's own HUD floats by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HudHand {
    Left,
    #[default]
    Right,
}

/// How far above the controller's grip a panel's middle is by default, real metres.
pub const ABOVE_HAND_M: f32 = 0.16;

/// The camera drawing an engine's HUD into its image: kept on in the headset (where other flat cameras are turned
/// off), run only while its panel shows ([`HandHud::camera`]).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct EngineHudCamera;

/// An engine's HUD panel, placed by a controller by the app: a quad one unit wide and `aspect` high, shown
/// `width_m` real metres wide. Its `anchor` (a point of it: -0.5..0.5 across and up from its middle) is put
/// `offset` from the grip (real metres: right and up, as the player sees it).
#[derive(Component, Clone, Copy, Debug)]
pub struct HandHud {
    pub width_m: f32,
    pub aspect: f32,
    pub offset: Vec2,
    pub anchor: Vec2,
    /// The camera drawing its image (on only while it is shown).
    pub camera: Option<Entity>,
    /// Nothing to show now (hidden).
    pub empty: bool,
    /// Always by this controller (none: the settings' one).
    pub hand: Option<HudHand>,
    /// By the settings' controller when that is `HudHand::Left` - shared with what keeps to the left one - its offset
    /// and anchor are these instead.
    pub on_left: Option<(Vec2, Vec2)>,
}

impl HandHud {
    /// A panel `width_m` wide with its middle `ABOVE_HAND_M` over the grip.
    pub fn above(width_m: f32, aspect: f32, camera: Option<Entity>) -> Self {
        Self {
            width_m,
            aspect,
            offset: Vec2::new(0.0, ABOVE_HAND_M),
            anchor: Vec2::ZERO,
            camera,
            empty: false,
            hand: None,
            on_left: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_keys_round_trip() {
        for engine in Engine::ALL {
            assert_eq!(Engine::from_key(engine.key()), Some(engine));
        }
        assert_eq!(Engine::from_key(" thug2\n"), Some(Engine::Thug2));
        assert_eq!(Engine::from_key("tony"), None);
        for mode in Mode::ALL {
            assert_eq!(Mode::from_key(mode.key()), Some(mode));
        }
        assert_eq!(Mode::from_key("race"), None);
    }
}
