//! What the headset app and an engine say to each other (docs/engine-packaging-proposal.md,
//! docs/buildar-proposal.md).
//!
//! The app (`buildar`) carries the engines and the shared XR layer: the room, the courses, the menus, the
//! controllers. An engine implements a [`Mode`]: it runs that mode's game in that world. The root of this crate is
//! what every mode shares:
//! - **To the engine:** the controllers ([`LatestTouch`]) and whether the shared layer has them ([`PadHeld`]); the
//!   stage ([`CurrentStage`]); where to put the player ([`PlacePlayer`]).
//! - **From the engine:** where its player is ([`PlayerFocus`]) and its bodies ([`PlayerBodies`]), its own HUD panels ([`HandHud`], drawn by an
//!   [`EngineHudCamera`]), the settings a course keeps ([`CourseSettings`]).
//! - **Ownership:** an engine's per-frame systems are those in its [`EngineSet`]; what they spawn is
//!   [`EngineOwned`], hidden and silenced while another engine runs. [`EngineSwitched`] and [`BeforeSwitch`] mark a
//!   switch.
//!
//! Each mode's own part is a module: [`skate`] (the park over the stage, the skater, its trick text).
//!
//! The registry that runs the engines and switches between them is the app's (`buildar`'s `engines`).
use bevy::prelude::*;
use std::collections::HashMap;

pub mod skate;

/// The Bevy-free contract the engines implement (its own repo): re-exported where the app has always named them.
pub use engine_core;
pub use engine_core::{Control, HudHand, Mode, TouchState};

// ---- Modes, engines and ownership ----

/// An engine: a game run in the room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Engine {
    /// The clean-room engine (no game data).
    Sk8,
    /// The faithful port of Skate 3, on the player's imported data.
    Skate3,
    /// THUG2's own engine (the decomp), on the player's game files.
    Thug2,
    /// Mario Kart Wii (WiiCompiled's translation of the player's disc), free driving.
    Mkwii,
}

impl Engine {
    pub const ALL: [Engine; 4] = [Engine::Sk8, Engine::Skate3, Engine::Thug2, Engine::Mkwii];

    pub fn name(self) -> &'static str {
        match self {
            Engine::Sk8 => "SK8",
            Engine::Skate3 => "Skate 3",
            Engine::Thug2 => "THUG2",
            Engine::Mkwii => "Mario Kart Wii",
        }
    }

    /// Its name in files (`engine.txt`, a park's engine tag).
    pub fn key(self) -> &'static str {
        match self {
            Engine::Sk8 => "sk8",
            Engine::Skate3 => "skate3",
            Engine::Thug2 => "thug2",
            Engine::Mkwii => "mkwii",
        }
    }

    pub fn from_key(key: &str) -> Option<Engine> {
        Engine::ALL.into_iter().find(|e| e.key() == key.trim())
    }

    /// The mode it implements.
    pub fn mode(self) -> Mode {
        match self {
            Engine::Sk8 | Engine::Skate3 | Engine::Thug2 => Mode::Skate,
            Engine::Mkwii => Mode::Race,
        }
    }

    /// The world scale a course for it starts at (game metres to a real one): its game at a size that suits a room -
    /// a skater about 1/7 life size (chosen on the headset: x1.7 of 1/12), a kart toy-sized (1/8: its game's karts
    /// are about 2.5 m long). A course's size is set as a multiple of this (x1.0 is this).
    pub fn default_scale(self) -> f32 {
        match self {
            Engine::Sk8 | Engine::Skate3 | Engine::Thug2 => 12.0 / 1.7,
            Engine::Mkwii => 8.0,
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

/// What of an engine's settings a course keeps (THUG2's game speed: what the park was built to), and
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

/// This frame's Touch state (a Bluetooth game pad, when one speaks, folded in).
#[derive(Resource, Default)]
pub struct LatestTouch(pub TouchState);

/// The shared layer's input for this frame (PreUpdate): the controllers read ([`LatestTouch`]), and whether the
/// shared layer has them ([`PadHeld`]). An engine reads its pad after this.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostInput;

/// Whether the controllers are the shared layer's this frame ([`engine_core::PadHeld`]: `held` while a menu is open,
/// in build mode, aiming a spawn, placing the skater or a park; `pause` while a menu is open or in build mode).
#[derive(Resource, Default, Clone, Copy, Debug, Deref, DerefMut)]
pub struct PadHeld(pub engine_core::PadHeld);

// ---- The stage, placing the player, where the player is ----

/// The stage every engine is offered ([`engine_core::Stage`]: the room and the placed pieces' solid faces, the
/// runtime's world space), rebuilt by the shared layer when the room or the pieces change. Engines take it when they
/// can ([`engine_core::Applied`]); the shared layer doesn't wait for them.
#[derive(Resource, Default, Deref, DerefMut)]
pub struct CurrentStage(pub engine_core::Stage);

/// Put the player down ([`engine_core::PlacePlayer`]); the running engine says what that means.
#[derive(Message, Clone, Copy, Debug, Deref)]
pub struct PlacePlayer(pub engine_core::PlacePlayer);

impl PlacePlayer {
    pub fn new(point: Vec3, forward: Vec3) -> Self {
        Self(engine_core::PlacePlayer { point, forward })
    }
}

/// Where each engine's player is now ([`engine_core::Focus`]; none: not in the room yet): what the shared layer follows
/// (what's placed at the player, the menus' distance checks, carrying the player over to the next engine).
#[derive(Resource, Default)]
pub struct PlayerFocus(HashMap<Engine, engine_core::Focus>);

impl PlayerFocus {
    pub fn set(&mut self, engine: Engine, focus: Option<engine_core::Focus>) {
        match focus {
            Some(focus) => {
                self.0.insert(engine, focus);
            }
            None => {
                self.0.remove(&engine);
            }
        }
    }

    pub fn of(&self, engine: Engine) -> Option<engine_core::Focus> {
        self.0.get(&engine).copied()
    }
}

/// Where each engine's player's bodies are now, in the world (from its model's body groups and pose: the app's model
/// layer fills it): what their contact shadows are cast from. None for an engine: one body, cast from its focus.
#[derive(Resource, Default)]
pub struct PlayerBodies(HashMap<Engine, Vec<BodyShape>>);

impl PlayerBodies {
    pub fn set(&mut self, engine: Engine, bodies: Vec<BodyShape>) {
        if bodies.is_empty() {
            self.0.remove(&engine);
        } else {
            self.0.insert(engine, bodies);
        }
    }

    pub fn of(&self, engine: Engine) -> &[BodyShape] {
        self.0.get(&engine).map_or(&[], Vec::as_slice)
    }
}

/// The systems that set [`PlayerBodies`] late in a frame (in `PostUpdate`) run in this set; what reads them then runs
/// after it.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BodiesSet;

/// A body as its contact shadow casts it, in the world ([`engine_core::model::Cast`] placed by a pose).
#[derive(Clone, Debug, PartialEq)]
pub enum BodyShape {
    /// Fixed ellipses laid level from `middle` along `along` (a unit vector), in units of the player's focus height.
    Footprint { middle: Vec3, along: Vec3, ellipses: Vec<engine_core::model::Ellipse> },
    /// Bones, each from a joint to its parent.
    Segments(Vec<(Vec3, Vec3)>),
}

// ---- HUDs ----

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
    }
}
