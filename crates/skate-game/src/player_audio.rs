//! Single-owner host for the recovered rider/board sound graph.
//!
//! The graph itself is the sim's (`skate3_sim::skate_audio::player`): asset IO and XMA decoding
//! happen once on this worker before the runtime is published. The Bevy mixer sees one permanent
//! stereo source; neither the mixer callback nor gameplay touches guest memory.

use std::path::PathBuf;
use std::sync::{
    Mutex,
    mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
};
use std::thread;
use std::time::{Duration, Instant};

use bevy::audio::{AudioPlayer, PlaybackSettings, Volume};
use bevy::prelude::*;
#[allow(unused_imports)]
pub(crate) use skate3_sim::skate_audio::player::{
    FRAME_SECONDS, OBSERVATION_QUEUE, OUTPUT_CHANNELS, PlayerAudio, QUEUE_FRAMES, SAMPLE_RATE,
    banks_decoded, multiplier_sound_id,
};

use super::{LivePcm, PlayerAudioObservation};

#[derive(Resource)]
struct PlayerAudioHost {
    input: SyncSender<skate3_sim::skate_audio::PlayerAudioObservation>,
    /// The last forwarded observation, repeated while paused (see [`forward`]).
    last_observation: Option<skate3_sim::skate_audio::PlayerAudioObservation>,
    paused_carry: f32,
    status: Mutex<Receiver<WorkerStatus>>,
    live: LivePcm,
    last_report: Instant,
    dropped_observations: u64,
    /// Front-end one-shot ids waiting for the next observation to carry them to the worker.
    pending_frontend: Vec<u64>,
}

#[derive(Resource)]
pub struct PlayerAudioAssets(pub PathBuf);

enum WorkerStatus {
    Ready { samples: usize, cache_hits: usize },
    Failed(String),
}

pub(super) fn install(app: &mut App) {
    // The session marker registers this too, but its plugin returns early when its validation
    // data will not load, and the multiplier sounds must not depend on that.
    app.add_message::<crate::skate_audio::FrontEndSoundRequest>();
    if super::clean::enabled() {
        info!("SKATE_PLAYER_AUDIO using the clean sounds (SKATE_AUDIO=retail for the retail runtime)");
        super::clean::install(app);
        return;
    }
    app.add_systems(Startup, start)
        .add_systems(Update, (forward, follow_skater, report).chain());
}


/// The mix's player, which in AR ([`super::clean::SpatialSkateAudio`]) is an emitter kept at the skater.
#[derive(Component)]
struct PlayerOutput;

fn start(
    mut commands: Commands,
    mut assets: ResMut<Assets<LivePcm>>,
    configured: Option<Res<PlayerAudioAssets>>,
    spatial: Option<Res<super::clean::SpatialSkateAudio>>,
) {
    let Some(configured) = configured else { return };
    let live = match LivePcm::new(OUTPUT_CHANNELS, SAMPLE_RATE, QUEUE_FRAMES) {
        Ok(live) => live,
        Err(error) => {
            error!("skate-audio player source: {error}");
            return;
        }
    };
    let handle = assets.add(live.clone());
    commands.spawn((
        PlayerOutput,
        AudioPlayer(handle),
        // LivePcm never ends. Loop mode would wrap it in Rodio's buffering repeater, which
        // eagerly captures startup silence and retains every streamed sample.
        // In AR the mix comes from the skater in the room: Bevy's spatial sink folds its two
        // channels into one (retail's own pan is for its camera, not the player's head) and pans
        // and fades that by where the skater is from the head.
        PlaybackSettings::ONCE
            .with_volume(Volume::Linear(1.0))
            .with_spatial(spatial.is_some()),
        Transform::default(),
    ));

    let (input, observations) = mpsc::sync_channel(OBSERVATION_QUEUE);
    let (status_tx, status) = mpsc::channel();
    let assets = configured.0.clone();
    let worker_live = live.clone();
    thread::Builder::new()
        .name("skate-player-audio".into())
        .spawn(move || {
            if let Err(error) = run(assets, worker_live, observations, &status_tx) {
                eprintln!("SKATE_PLAYER_AUDIO failed={error}");
                let _ = status_tx.send(WorkerStatus::Failed(error));
            }
        })
        .map_err(|error| error!("skate-audio could not start player worker: {error}"))
        .ok();
    commands.insert_resource(PlayerAudioHost {
        input,
        last_observation: None,
        paused_carry: 0.0,
        status: Mutex::new(status),
        live,
        last_report: Instant::now(),
        dropped_observations: 0,
        pending_frontend: Vec::new(),
    });
}

fn forward(
    mut host: Option<ResMut<PlayerAudioHost>>,
    mut observations: MessageReader<PlayerAudioObservation>,
    mut marker_sounds: MessageReader<crate::skate_audio::FrontEndSoundRequest>,
    virtual_time: Res<Time<bevy::time::Virtual>>,
    real_time: Res<Time<bevy::time::Real>>,
) {
    let Some(host) = host.as_deref_mut() else {
        return;
    };
    // The session marker fires on its own schedule, not with the simulation, so its sounds are
    // buffered and ride out on the next observation rather than being dropped if this frame
    // produced none. Retail calls `GlobalFEPlaySound` straight from the UI; this is the closest
    // equivalent that still reaches the one audio worker.
    host.pending_frontend
        .extend(marker_sounds.read().map(|sound| sound.0));
    // Retail keeps publishing an audio state every frame while the game is paused — its physics
    // values simply stay frozen — and silences the player path through SFXObj_Pause's input. The
    // simulation schedule does not run while `Time<Virtual>` is paused, so repeat the last
    // observation at the simulation rate with `paused` set, which is what retail's frozen state is.
    if virtual_time.is_paused() {
        if let Some(last) = host.last_observation.clone() {
            host.paused_carry += real_time.delta_secs();
            while host.paused_carry >= FRAME_SECONDS {
                host.paused_carry -= FRAME_SECONDS;
                let mut frozen = last.clone();
                frozen.retail.paused = true;
                let _ = host.input.try_send(frozen);
            }
        }
        return;
    }
    host.paused_carry = 0.0;
    for observation in observations.read() {
        host.last_observation = Some(observation.0.clone());
        let mut observation = observation.0.clone();
        observation
            .frontend_sounds
            .append(&mut std::mem::take(&mut host.pending_frontend));
        match host.input.try_send(observation) {
            Ok(()) => {}
            Err(TrySendError::Full(lost)) => {
                host.dropped_observations += 1;
                eprintln!(
                    "SKATE_PLAYER_AUDIO observation_overflow tick={} footstep={} push={} landed={} events={:?}",
                    lost.tick,
                    lost.footstep_strength,
                    lost.foot_push_speed,
                    lost.landed,
                    lost.events
                );
            }
            Err(TrySendError::Disconnected(_)) => break,
        }
    }
}

/// Keep the mix's emitter where the skater is (its last observation's place, in the game world).
fn follow_skater(host: Option<Res<PlayerAudioHost>>, mut output: Query<&mut Transform, With<PlayerOutput>>) {
    let Some(at) = host.and_then(|h| h.last_observation.as_ref().map(|o| Vec3::from_array(o.retail.position))) else {
        return;
    };
    for mut transform in &mut output {
        if transform.translation != at {
            transform.translation = at;
        }
    }
}

fn report(mut host: Option<ResMut<PlayerAudioHost>>) {
    let Some(host) = host.as_deref_mut() else {
        return;
    };
    let statuses: Vec<_> = host
        .status
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .try_iter()
        .collect();
    for status in statuses {
        match status {
            WorkerStatus::Ready {
                samples,
                cache_hits,
            } => {
                info!("skate-audio player graph ready: {samples} samples ({cache_hits} from cache)")
            }
            WorkerStatus::Failed(error) => error!("skate-audio player graph stopped: {error}"),
        }
    }
    if host.dropped_observations >= 60 {
        warn!(
            "skate-audio dropped {} stale physics observations",
            host.dropped_observations
        );
        host.dropped_observations = 0;
    }
    if host.last_report.elapsed() >= Duration::from_secs(2) {
        let stats = host.live.snapshot();
        eprintln!(
            "SKATE_PLAYER_AUDIO stream pushed={} consumed={} nonzero={} underflow={} queued_frames={}",
            stats.pushed,
            stats.consumed,
            stats.consumed_nonzero,
            stats.underflow,
            host.live.queued_frames(),
        );
        host.last_report = Instant::now();
    }
}

fn run(
    assets: PathBuf,
    live: LivePcm,
    observations: Receiver<skate3_sim::skate_audio::PlayerAudioObservation>,
    status: &mpsc::Sender<WorkerStatus>,
) -> Result<(), String> {
    let mut audio = PlayerAudio::prepare(&assets)?;
    let (sample_count, cache_hits) = (audio.samples, audio.cache_hits);
    let _ = status.send(WorkerStatus::Ready {
        samples: sample_count,
        cache_hits,
    });
    eprintln!("SKATE_PLAYER_AUDIO ready samples={sample_count} cache_hits={cache_hits}");

    let block = PlayerAudio::block_duration();
    let mut deadline = Instant::now();
    loop {
        let observation = next_observation(&observations)?;
        let stereo = audio.render_block(observation.as_ref())?;
        live.push(&stereo)?;
        deadline += block;
        let now = Instant::now();
        if deadline > now {
            thread::sleep(deadline - now);
        } else if now.duration_since(deadline) > block * 4 {
            deadline = now;
        }
    }
}

fn next_observation<T>(observations: &Receiver<T>) -> Result<Option<T>, String> {
    match observations.try_recv() {
        Ok(observation) => Ok(Some(observation)),
        Err(TryRecvError::Empty) => Ok(None),
        Err(TryRecvError::Disconnected) => {
            Err("player audio observation source disconnected".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_contact_and_landing_survive_a_following_idle_tick() {
        let (sender, receiver) = mpsc::sync_channel(16);
        // One presentation frame can forward multiple completed physics ticks together.
        sender.send((10, "foot contact")).unwrap();
        sender.send((11, "landing")).unwrap();
        sender.send((12, "idle")).unwrap();
        assert_eq!(
            next_observation(&receiver).unwrap(),
            Some((10, "foot contact"))
        );
        assert_eq!(next_observation(&receiver).unwrap(), Some((11, "landing")));
        assert_eq!(next_observation(&receiver).unwrap(), Some((12, "idle")));
        assert_eq!(next_observation(&receiver).unwrap(), None);
    }
}
