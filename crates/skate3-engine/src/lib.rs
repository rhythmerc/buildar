//! **Skate 3 only.** Skate 3's side of the engine contract ([`engine_core`]): what the app needs to know about Skate 3
//! to host it, said as the contract's plain data, over its simulation (`skate3-sim`). No Bevy.
//!
//! It lives in the app's workspace for now and is meant to move into the skate3-headless repo as it is (beside
//! `skate3-sim`, or into it as `skate3_sim::hud`: `docs/game-code-out-plan.md`, step 3). Nothing here may depend on
//! the app.
//!
//! - [`hud`]: the retail trick display (its movie, read from the player's prepared HUD files) as a
//!   [`engine_core::hud::HudFrame`].
//! - [`input`]: the Touch controllers as the Xbox pad the sim reads, and its controls sheet.
//! - [`model`]: the skater's model (its GLB bound to the sim's rig) and its pose after each tick, cuts included.
//! - [`stage`]: the stage and park as the sim's collision and rails (not mid-grind), putting the skater down, and
//!   where the rider and the board are.
//! - [`trick`]: the sim's scoring as the plain trick readout.
pub mod hud;
pub mod input;
pub mod model;
pub mod stage;
pub mod trick;
