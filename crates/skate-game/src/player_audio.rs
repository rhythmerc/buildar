//! Skate 3's sound in the app, one of two ways:
//!
//! - **Its own:** the sim's recovered rider and board sound graph (`skate3_engine::sound::stream`), on the player's
//!   decoded sound banks, played by the app's stream host ([`crate::engine_audio::EngineStream`]) from the skater.
//! - **The skating sounds the app carries** (the clean driver, `skate_audio::clean`), from its ticks
//!   (`skate3_engine::sound::Heard`): on an Android without the decoded banks (the headset has no XMA decoder: they
//!   come with the imported data, `audio-pcm-cache`). `SKATE_AUDIO=clean|retail` overrides that.
//!
//! Either way the ticks come from the physics as [`PlayerAudioObservation`]s, the score multiplier's front-end sounds
//! as [`FrontEndSoundRequest`]s, and while Skate 3 is held (the desktop's menu pauses virtual time; the headset's menus
//! and build mode hold it: `crate::app::Hold`) its sound is too.
use std::path::PathBuf;

use bevy::prelude::*;
use skate3_engine::sound::{Heard, StreamFeed};

use super::clean::{SkaterSoundSet, SkaterSounds, SpatialSkateAudio};
use super::{FrontEndSoundRequest, PlayerAudioObservation};
use crate::engine_audio::EngineStream;
use crate::engines::Engine;
use crate::live_pcm::LivePcm;

#[derive(Resource)]
pub struct PlayerAudioAssets(pub PathBuf);

/// Whether this build plays the clean sounds instead of Skate 3's own.
fn clean() -> bool {
    match std::env::var("SKATE_AUDIO").as_deref() {
        Ok("clean") => true,
        Ok("retail") => false,
        _ => cfg!(target_os = "android") && !skate3_engine::sound::banks_decoded(),
    }
}

pub(super) fn install(app: &mut App) {
    // The session marker registers this too, but its plugin returns early when its validation
    // data will not load, and the multiplier sounds must not depend on that.
    app.add_message::<FrontEndSoundRequest>();
    if clean() {
        info!("SKATE_PLAYER_AUDIO using the clean sounds (SKATE_AUDIO=retail for the retail runtime)");
        super::clean::install(app);
        app.init_resource::<Skate3Heard>()
            .add_systems(Update, feed_clean.before(SkaterSoundSet).in_set(crate::app::SKATE3));
        return;
    }
    crate::engine_audio::add_stream_source(app);
    app.add_systems(Startup, start).add_systems(Update, forward.in_set(crate::app::SKATE3));
}

fn paused(virtual_time: &Time<bevy::time::Virtual>, hold: Option<&crate::app::Hold>) -> bool {
    virtual_time.is_paused() || hold.is_some_and(|h| h.paused())
}

/// Skate 3's ticks as the clean driver's sounds.
#[derive(Resource, Default)]
pub(crate) struct Skate3Heard(Heard);

pub(crate) fn feed_clean(
    mut heard: ResMut<Skate3Heard>,
    mut observations: MessageReader<PlayerAudioObservation>,
    mut front_end: MessageReader<FrontEndSoundRequest>,
    virtual_time: Res<Time<bevy::time::Virtual>>,
    hold: Option<Res<crate::app::Hold>>,
    mut out: ResMut<SkaterSounds>,
) {
    for o in observations.read() {
        heard.0.observe(o);
    }
    if front_end.read().count() > 0 {
        heard.0.front_end();
    }
    *out = SkaterSounds { engine: Some(Engine::Skate3), paused: paused(&virtual_time, hold.as_deref()), sounds: heard.0.take() };
}

/// Skate 3's own sound: its ticks' feed and the stream playing.
#[derive(Resource)]
struct Skate3Sound {
    feed: StreamFeed,
    stream: EngineStream,
    dropped: u64,
}

fn start(mut commands: Commands, configured: Option<Res<PlayerAudioAssets>>) {
    let Some(configured) = configured else { return };
    let (feed, factory) = skate3_engine::sound::stream(configured.0.clone());
    commands.insert_resource(Skate3Sound { feed, stream: EngineStream::start("skate-player-audio", factory), dropped: 0 });
}

#[allow(clippy::too_many_arguments)]
fn forward(
    mut commands: Commands,
    sound: Option<ResMut<Skate3Sound>>,
    mut observations: MessageReader<PlayerAudioObservation>,
    mut front_end: MessageReader<FrontEndSoundRequest>,
    virtual_time: Res<Time<bevy::time::Virtual>>,
    hold: Option<Res<crate::app::Hold>>,
    spatial: Option<Res<SpatialSkateAudio>>,
    mut assets: ResMut<Assets<LivePcm>>,
    mut transforms: Query<&mut Transform>,
) {
    let Some(mut sound) = sound else { return };
    let sound = &mut *sound;
    // The multiplier's sounds come on their own schedule, not with the ticks: they ride out on the next tick
    // (`StreamFeed::front_end`).
    for request in front_end.read() {
        sound.feed.front_end(request.0);
    }
    let paused = paused(&virtual_time, hold.as_deref());
    // (held, the sim doesn't tick: the stream repeats the last tick as retail's frozen state)
    if !paused {
        for o in observations.read() {
            sound.feed.observe(o);
        }
    }
    // In AR the mix comes from the skater in the room: Bevy's spatial sink folds its two channels into one (retail's
    // own pan is for its camera, not the player's head) and pans and fades that by where the skater is from the head.
    let at = sound.feed.at();
    sound.stream.update(&mut commands, &mut assets, paused, at, spatial.is_some(), &mut transforms);
    sound.dropped += sound.feed.dropped();
    if sound.dropped >= 60 {
        warn!("skate-audio dropped {} stale physics observations", sound.dropped);
        sound.dropped = 0;
    }
}
