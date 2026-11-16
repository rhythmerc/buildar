//! THUG2's side of the engine contract ([`engine_core`]): what the app needs to know about THUG2 to host it, said
//! as the contract's plain data, over the engine itself (`thug2-sys`). No Bevy.
//!
//! A temporary home: this crate and `thug2-sys` move to the thug2-headless repo together
//! (`docs/game-code-out-plan.md`, step 10). The rest of THUG2's knowledge (pad, units, faces) joins it step by step.
//!
//! - [`hud`]: the game's 2D as a [`engine_core::hud::HudFrame`]: which draw is which part, and where each part goes.
pub mod hud;
