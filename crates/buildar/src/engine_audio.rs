//! An engine's sound in the room, whichever engine it is (`engine_core::audio`):
//!
//! - [`EngineStream`]: an engine's own mix, made and rendered on a thread of its own and paced to the output; the mixer
//!   pulls it from a [`LivePcm`] queue. Played from where the engine says (in AR, a spatial emitter there).
//! - [`Voices`]: the clips an engine plays, each voice an emitter at its place at the engine's level and speed. A voice
//!   played again from its start is a new emitter; a looping one goes when the engine stops it, one played once when
//!   it has run out. While the engine is held its loops wait too.
//!
//! Each engine's adapter feeds them (Skate 3's stream: `skate_audio::player`; THUG2's voices: `thug2`).
use crate::live_pcm::LivePcm;
use crate::stream_pcm::StreamPcm;
use bevy::audio::{AudioPlayer, AudioSinkPlayback, PlaybackSettings, SpatialAudioSink, Volume};
use bevy::prelude::*;
use engine_core::audio::{Clip, StreamFactory, Voice};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The streams' audio source, once for every engine (a second copy would start each sound twice).
pub(crate) fn add_stream_source(app: &mut App) {
    use bevy::audio::AddAudioSource;
    if !app.world().contains_resource::<Assets<LivePcm>>() {
        app.add_audio_source::<LivePcm>();
    }
}

/// Rendered frames the output queue holds at most: what a stalled output may lag by.
const QUEUE_FRAMES: usize = 4096;

enum Status {
    Ready(LivePcm),
    Failed(String),
}

/// An engine's own mix playing (`engine_core::audio::Stream`).
pub(crate) struct EngineStream {
    name: &'static str,
    paused: Arc<AtomicBool>,
    status: Mutex<Receiver<Status>>,
    live: Option<LivePcm>,
    output: Option<Entity>,
    last_report: Instant,
}

impl EngineStream {
    /// Makes the stream on a thread of its own (named `name`), which then renders it a block's time at a time.
    pub(crate) fn start(name: &'static str, factory: StreamFactory) -> Self {
        let paused = Arc::new(AtomicBool::new(false));
        let (status_tx, status) = mpsc::channel();
        let held = paused.clone();
        let started = std::thread::Builder::new().name(name.into()).spawn(move || {
            if let Err(error) = run(factory, &held, &status_tx) {
                eprintln!("{name}: stream stopped: {error}");
                let _ = status_tx.send(Status::Failed(error));
            }
        });
        if let Err(error) = started {
            error!("{name}: could not start its stream: {error}");
        }
        Self { name, paused, status: Mutex::new(status), live: None, output: None, last_report: Instant::now() }
    }

    /// Each frame: whether the engine is held, and where the sound is from (in AR the output is a spatial emitter
    /// there, panned and faded by where it is from the head; its channels folded into one). The output appears once
    /// the stream is made.
    pub(crate) fn update(&mut self, commands: &mut Commands, assets: &mut Assets<LivePcm>, paused: bool, at: Option<Vec3>, spatial: bool, transforms: &mut Query<&mut Transform>) {
        self.paused.store(paused, Ordering::Relaxed);
        let statuses: Vec<_> = self.status.lock().unwrap_or_else(|p| p.into_inner()).try_iter().collect();
        for status in statuses {
            match status {
                Status::Ready(live) => {
                    info!("{}: stream ready", self.name);
                    let handle = assets.add(live.clone());
                    self.output = Some(
                        commands
                            .spawn((
                                AudioPlayer(handle),
                                // LivePcm never ends. Loop mode would wrap it in Rodio's buffering repeater, which
                                // eagerly captures startup silence and retains every streamed sample.
                                PlaybackSettings::ONCE.with_volume(Volume::Linear(1.0)).with_spatial(spatial),
                                Transform::from_translation(at.unwrap_or_default()),
                                Name::new(self.name),
                            ))
                            .id(),
                    );
                    self.live = Some(live);
                }
                Status::Failed(error) => error!("{}: stream stopped: {error}", self.name),
            }
        }
        if let (Some(at), Some(output)) = (at, self.output)
            && let Ok(mut transform) = transforms.get_mut(output)
            && transform.translation != at
        {
            transform.translation = at;
        }
        if let Some(live) = &self.live
            && self.last_report.elapsed() >= Duration::from_secs(2)
        {
            let stats = live.snapshot();
            eprintln!(
                "{} stream pushed={} consumed={} nonzero={} underflow={} queued_frames={}",
                self.name,
                stats.pushed,
                stats.consumed,
                stats.consumed_nonzero,
                stats.underflow,
                live.queued_frames(),
            );
            self.last_report = Instant::now();
        }
    }
}

fn run(factory: StreamFactory, paused: &AtomicBool, status: &mpsc::Sender<Status>) -> Result<(), String> {
    let mut stream = factory()?;
    let live = LivePcm::new(stream.channels(), stream.rate(), QUEUE_FRAMES)?;
    let _ = status.send(Status::Ready(live.clone()));
    let block = stream.block();
    let mut deadline = Instant::now();
    loop {
        let samples = stream.render(paused.load(Ordering::Relaxed))?;
        live.push(&samples)?;
        deadline += block;
        let now = Instant::now();
        if deadline > now {
            std::thread::sleep(deadline - now);
        } else if now.duration_since(deadline) > block * 4 {
            deadline = now;
        }
    }
}

/// The voices as an engine's steps leave them, and the clips new to the app.
#[derive(Default)]
pub(crate) struct Heard {
    /// The voices playing after the last step.
    playing: Vec<Voice>,
    /// The voices played from their start in the steps since the app last played them (a short one may have run out
    /// already).
    started: Vec<Voice>,
    new_clips: Vec<(u32, Clip)>,
    fetched: HashSet<u32>,
    starts: HashMap<u32, u32>,
}

impl Heard {
    /// After a step: the voices playing; `clip` fetches a clip new to the app (once each).
    pub(crate) fn after_step(&mut self, playing: Vec<Voice>, mut clip: impl FnMut(u32) -> Option<Clip>) {
        for v in &playing {
            if self.starts.insert(v.id, v.starts) == Some(v.starts) {
                continue;
            }
            self.started.retain(|s| s.id != v.id);
            self.started.push(*v);
            if self.fetched.insert(v.clip) {
                match clip(v.clip) {
                    Some(c) => self.new_clips.push((v.clip, c)),
                    None => warn!("sound {} has no samples", v.clip),
                }
            }
        }
        self.playing = playing;
    }

    /// The voices started since the last play are dropped (they can't be heard yet: the engine isn't in the room).
    pub(crate) fn forget_started(&mut self) {
        self.started.clear();
    }
}

/// Clips as an audio source of their own: the voices' (`StreamPcm` plays them).
#[derive(Asset, TypePath, Clone)]
pub(crate) struct VoicePcm(StreamPcm);

impl bevy::audio::Decodable for VoicePcm {
    type DecoderItem = f32;
    type Decoder = crate::stream_pcm::PcmPlayback;

    fn decoder(&self) -> Self::Decoder {
        bevy::audio::Decodable::decoder(&self.0)
    }
}

/// The voices' audio source, once for every engine (a second copy would start each sound twice).
pub(crate) fn add_voice_source(app: &mut App) {
    use bevy::audio::AddAudioSource;
    if !app.world().contains_resource::<Assets<VoicePcm>>() {
        app.add_audio_source::<VoicePcm>();
    }
}

/// An engine's clips, and each as a source once and looping.
#[derive(Default)]
pub(crate) struct Voices {
    clips: HashMap<u32, (Arc<Vec<i16>>, u16, u32)>,
    sources: HashMap<(u32, bool), Handle<VoicePcm>>,
}

/// An emitter playing one of an engine's voices.
#[derive(Component)]
pub(crate) struct VoiceEmitter {
    pub(crate) voice: u32,
    pub(crate) looping: bool,
}

impl Voices {
    /// The clips the app has.
    #[cfg(test)]
    pub(crate) fn clips(&self) -> usize {
        self.clips.len()
    }

    /// The voices heard since the last frame, played: `held` the engine waits (a menu), `level` the player's volume.
    #[allow(clippy::type_complexity)]
    pub(crate) fn play(
        &mut self,
        heard: &mut Heard,
        held: bool,
        level: f32,
        commands: &mut Commands,
        assets: &mut Assets<VoicePcm>,
        emitters: &mut Query<(Entity, &VoiceEmitter, &mut Transform, Option<&mut SpatialAudioSink>)>,
    ) {
        for (id, clip) in heard.new_clips.drain(..) {
            self.clips.insert(id, (Arc::new(clip.samples), clip.channels, clip.rate));
        }
        let started = std::mem::take(&mut heard.started);
        let playing = &heard.playing;
        for (entity, emitter, mut transform, sink) in emitters.iter_mut() {
            let now = playing.iter().find(|v| v.id == emitter.voice);
            let replaced = started.iter().any(|v| v.id == emitter.voice);
            if replaced || (emitter.looping && now.is_none()) {
                commands.entity(entity).despawn();
                continue;
            }
            let Some(v) = now else { continue };
            transform.translation = v.at;
            if let Some(mut sink) = sink {
                sink.set_volume(Volume::Linear(v.gain * level));
                sink.set_speed(v.speed);
                match (held, sink.is_paused()) {
                    (true, false) => sink.pause(),
                    (false, true) => sink.play(),
                    _ => {}
                }
            }
        }
        if held {
            return;
        }
        for v in started {
            let Some((samples, channels, rate)) = self.clips.get(&v.clip).cloned() else { continue };
            let source = self
                .sources
                .entry((v.clip, v.looping))
                .or_insert_with(|| assets.add(VoicePcm(StreamPcm::from_i16(samples, channels, rate, v.looping))))
                .clone();
            // (where it is now, if it still plays)
            let at = playing.iter().find(|p| p.id == v.id).map_or(v.at, |p| p.at);
            let settings = if v.looping { PlaybackSettings::ONCE } else { PlaybackSettings::DESPAWN };
            commands.spawn((
                VoiceEmitter { voice: v.id, looping: v.looping },
                AudioPlayer(source),
                settings.with_spatial(true).with_volume(Volume::Linear(v.gain * level)).with_speed(v.speed),
                Transform::from_translation(at),
                Name::new("engine voice"),
            ));
        }
    }
}
