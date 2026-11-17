//! THUG2's side of the engine contract ([`engine_core`]): what the app needs to know about THUG2 to host it, said
//! as the contract's plain data, over the engine itself (`thug2-sys`). No Bevy.
//!
//! A temporary home: this crate and `thug2-sys` move to the thug2-headless repo together
//! (`docs/game-code-out-plan.md`, step 10). The rest of THUG2's knowledge joins it step by step.
//!
//! - [`engine`]: the game as the runtime drives it ([`engine_core::EngineCore`]), made on a worker by its factory.
//! - [`hud`]: the game's 2D as a [`engine_core::hud::HudFrame`]: which draw is which part, and where each part goes.
//! - [`input`]: the Touch controllers as the game's pad, and its controls sheet.
//! - [`model`]: the skater's model as meshes and plain materials, and its pose (palette and draw matrix) in the
//!   contract's space.
//! - [`sound`]: the game's voices as the contract's, from the skater's board, and its sounds' samples.
//! - [`stage`]: the stage and park as the game's world (its units, face classes, rails; not mid-grind), putting the
//!   skater down, and where it is.
pub mod engine;
pub mod hud;
pub mod input;
pub mod model;
pub mod sound;
pub mod stage;
