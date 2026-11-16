//! The app's fonts (OFL: `fonts/OFL.txt`, `fonts/Inter-OFL.txt`), shared by the headset menus and the engines' own
//! screens: embedded, so the app needs no font files of its own.
use bevy::prelude::*;

/// Headings and HUD figures: a heavy italic sans (Montserrat).
pub(crate) const HEADING: &str = "embedded://skate_game/fonts/Montserrat-Italic.ttf";
/// Running text (Inter, variable weight).
pub(crate) const BODY: &str = "embedded://skate_game/fonts/Inter.ttf";

/// Embeds them (every app that shows text calls it once; again is harmless).
pub(crate) fn embed(app: &mut App) {
    bevy::asset::embedded_asset!(app, "fonts/Montserrat-Italic.ttf");
    bevy::asset::embedded_asset!(app, "fonts/Inter.ttf");
}
