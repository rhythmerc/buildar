//! Skate 3's sound (`player`: its own sound graph, or its ticks as the skating sounds the app carries) and the clean
//! driver every skating engine without sound of its own shares (`clean`).

use bevy::prelude::*;

#[path = "player_audio.rs"]
pub mod player;
#[path = "player_audio_clean.rs"]
pub(crate) mod clean;

/// A front-end one-shot to play, by the 64-bit AttribSys id retail passes to `GlobalFEPlaySound`
/// (`sub_825DFAF0`).
/// (Written by the scoring HUD.)
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrontEndSoundRequest(pub u64);

/// One completed local-player physics tick ([`skate3_sim::skate_audio::PlayerAudioObservation`])
/// as a Bevy message (transitional: the sim has no Bevy to derive `Message` with).
#[derive(Message, Clone, Debug, PartialEq, Deref, DerefMut)]
pub struct PlayerAudioObservation(pub skate3_sim::skate_audio::PlayerAudioObservation);

pub struct SkateAudioPlugin;

impl Plugin for SkateAudioPlugin {
    fn build(&self, app: &mut App) {
        player::install(app);
        app.add_message::<PlayerAudioObservation>();
    }
}
