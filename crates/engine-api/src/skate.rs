//! Skate mode's part of the contract ([`Mode::Skate`](crate::Mode::Skate)) as the app's Bevy resources, over
//! [`engine_core::skate`]: the park over the stage, where the skater is, and its trick text.
use bevy::prelude::*;
pub use engine_core::skate::{ParkPiece, SkatePark, TrickText};

/// The park the skate engines are offered with the stage ([`crate::CurrentStage`]): rebuilt with it, the same
/// revision.
#[derive(Resource, Default, Deref, DerefMut)]
pub struct CurrentPark(pub SkatePark);

/// Where the skater is, filled each frame by the running engine's adapter ([`engine_core::skate::SkaterLink`]).
#[derive(Resource, Default, Deref, DerefMut)]
pub struct SkaterLink(pub engine_core::skate::SkaterLink);

/// What the generic trick readout shows; an engine without a HUD of its own writes it every frame.
#[derive(Resource, Default, Clone, PartialEq, Deref, DerefMut)]
pub struct TrickHud(pub TrickText);
