//! Clean player sounds: a small sample driver for skating engines without sound of their own.
//!
//! The retail-exact player audio (`player_audio.rs`) needs the decoded retail banks, grains and
//! the recovered authored graph. On the Quest that chain is fragile, so the AR build plays the
//! skate3-cleanroom web build's 31 sounds instead: real CC0 skateboard recordings plus a few
//! resynthesised ones (`clean-audio/`, extracted by `tools/extract_clean_audio.py`, credits in
//! `clean-audio/CREDITS.json`). They are compiled into the binary, so nothing has to be pushed
//! to the headset.
//!
//! One driver for every skating engine without sound of its own: the running engine's adapter
//! fills [`SkaterSounds`] (the skate mode's `engine_core::skate::sound`: the cues since the last
//! frame and how the skater is), and the driver plays them:
//! - loops (rolling per surface, grinds, powerslide) run all the time at zero volume and are
//!   faded and pitched by speed each frame;
//! - one-shots (pop, flip, catch, landing, push, footstep, bail, the multiplier chime) fire on
//!   the cues, with a random variant and a little pitch spread.
//!
//! SK8 feeds it (`crate::clean`); Skate 3 only where the player's decoded sound banks aren't
//! (`skate_audio::player`: its own sound otherwise). The driver is shared, not an engine's: while
//! an engine that didn't fill it runs, it is silent.
//!
//! In AR ([`SpatialSkateAudio`] present) every sound is a spatial emitter at the skater, heard
//! through the headset's listener (`xr::spatial_audio`), so it pans and fades as the skater
//! moves around the room.
use std::sync::Arc;

use bevy::audio::{AudioSinkPlayback, SpatialAudioSink, Volume};
use bevy::prelude::*;
use engine_core::skate::sound::{Cue, Rolling, SkateSounds};

use crate::binaural::{CleanPcm, Emitter, Voice};
use crate::clean_mix::{Buffer, Strip, Tune, Tuning};
use crate::clean_synth::{KINDS, RATE};
use crate::reverb::{ReverbPcm, RoomFromWorld};

macro_rules! clean_wavs {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/clean-audio/", $name, ".wav")) as &[u8])),*]
    };
}

const WAVS: &[(&str, &[u8])] = clean_wavs!(
    "bail_1", "bail_2", "catch_1", "catch_2", "catch_3", "chime", "flip_1", "flip_2", "flip_3",
    "footstep_1", "footstep_2", "footstep_3", "footstep_4", "grind_concrete", "grind_metal",
    "land_impact_1", "land_impact_2", "land_impact_3", "land_impact_4", "pop_1", "pop_2", "pop_3",
    "pop_4", "powerslide", "push_1", "push_2", "push_3", "roll_metal", "roll_rough",
    "roll_smooth", "roll_wood",
);

/// Overall level; the CC0 files are normalised per category (hits peak near -4 dBFS, loops sit
/// around -24 dBFS RMS).
const MASTER: f32 = 1.5;
/// Loops are mastered quiet so they sit under the hits; this restores them.
const LOOP_GAIN: f32 = 2.6;
/// Volume fade time constant, seconds.
const FADE_S: f32 = 0.06;
/// Below this board speed the wheels are silent, m/s.
const ROLL_MIN_SPEED: f32 = 0.25;
/// Board speed at which rolling is at full volume, m/s.
const ROLL_FULL_SPEED: f32 = 7.0;
/// The powerslide's scrape at full speed, against a rolling loop's full level. The recording is
/// a harsh one, and both engines call a hard turn at walking pace a powerslide: at the level of
/// the wheels (and never under 40% of it, as it was) every slow turn screeched.
const SLIDE_GAIN: f32 = 0.35;
/// Board speed under which a powerslide is silent, and at which its scrape is full, m/s.
const SLIDE_MIN_SPEED: f32 = 1.5;
const SLIDE_FULL_SPEED: f32 = 7.0;
/// The scrape is played with what is above this taken out of it (its squeal), Hz.
const SLIDE_LOWPASS_HZ: f32 = 2500.0;
/// Rolling on a wooden ramp: the smooth loop at this pitch (a hollower, lower rumble).
const WOOD_PITCH: f32 = 0.82;
/// How close to a hinted rail the skater must be for a metal grind, metres.
const METAL_RAIL_DISTANCE_M: f32 = 0.6;
/// Delay from a flip's start to its catch, seconds.
const CATCH_DELAY_S: f32 = 0.32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Loop {
    RollSmooth,
    RollRough,
    RollWood,
    RollMetal,
    GrindMetal,
    GrindConcrete,
    Powerslide,
}

impl Loop {
    const ALL: [Loop; 7] = [
        Loop::RollSmooth,
        Loop::RollRough,
        Loop::RollWood,
        Loop::RollMetal,
        Loop::GrindMetal,
        Loop::GrindConcrete,
        Loop::Powerslide,
    ];

    fn file(self) -> &'static str {
        match self {
            Loop::RollSmooth => "roll_smooth",
            Loop::RollRough => "roll_rough",
            Loop::RollWood => "roll_wood",
            Loop::RollMetal => "roll_metal",
            Loop::GrindMetal => "grind_metal",
            Loop::GrindConcrete => "grind_concrete",
            Loop::Powerslide => "powerslide",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shot {
    Pop,
    Flip,
    Catch,
    Land,
    Push,
    Footstep,
    Bail,
    Chime,
}

impl Shot {
    const ALL: [Shot; 8] = [
        Shot::Pop,
        Shot::Flip,
        Shot::Catch,
        Shot::Land,
        Shot::Push,
        Shot::Footstep,
        Shot::Bail,
        Shot::Chime,
    ];

    fn prefix(self) -> &'static str {
        match self {
            Shot::Pop => "pop",
            Shot::Flip => "flip",
            Shot::Catch => "catch",
            Shot::Land => "land_impact",
            Shot::Push => "push",
            Shot::Footstep => "footstep",
            Shot::Bail => "bail",
            Shot::Chime => "chime",
        }
    }
}

/// Where in the world the clean driver should hear something other than the default surface,
/// filled by whoever builds the collision (the AR build's placed pieces). World space, the same
/// space as `RetailAudioInputs::position`.
#[derive(Resource, Default, Clone, Debug)]
pub(crate) struct SurfaceHints {
    /// Metal rails and copings: a grind near one is a metal grind.
    pub metal_rails: Vec<(Vec3, Vec3)>,
    /// Wooden ramps (min, max): rolling inside one is rolling on wood.
    pub wood_boxes: Vec<(Vec3, Vec3)>,
}

impl SurfaceHints {
    fn near_metal(&self, p: Vec3) -> bool {
        self.metal_rails
            .iter()
            .any(|&(a, b)| segment_distance(p, a, b) <= METAL_RAIL_DISTANCE_M)
    }

    fn on_wood(&self, p: Vec3) -> bool {
        self.wood_boxes.iter().any(|&(min, max)| {
            let lo = min - Vec3::new(0.15, 0.1, 0.15);
            let hi = max + Vec3::new(0.15, 1.2, 0.15);
            p.cmpge(lo).all() && p.cmple(hi).all()
        })
    }
}

fn segment_distance(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 1e-8 {
        ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

/// Present when sounds should come from the skater in 3D (the XR build inserts it).
#[derive(Resource, Default)]
pub(crate) struct SpatialSkateAudio;

#[derive(Resource)]
struct CleanAudio {
    spatial: bool,
    /// The skater's latest world position, where spatial sounds are emitted.
    source: Vec3,
    shots: Vec<(Shot, Vec<(Handle<CleanPcm>, Option<Arc<Voice>>)>)>,
    loops: Vec<(Loop, Entity, f32)>,
    /// Heard through Steam Audio's HRTF (`crate::binaural`): where the sounds are from the head,
    /// and each loop's voice (its pitch is the voice's, not the sink's).
    emitter: Option<Arc<Emitter>>,
    voices: Vec<Option<Arc<Voice>>>,
    /// Each kind's level and EQ ([`KINDS`]' order: the loops, then the shots), its sounds'
    /// samples as they play, and the recordings they started as.
    strips: Vec<Arc<Strip>>,
    buffers: Vec<Vec<Arc<Buffer>>>,
    recorded: Vec<Vec<Arc<Vec<i16>>>>,
    /// Wooden ramps have a loop of their own (the synthesised one; the recording creaks).
    wood_loop: bool,
    targets: [(f32, f32); 7],
    last_pick: [usize; 8],
    rng: u32,
    pending_catch: Option<f32>,
    last_land: f32,
    last_wipe: f32,
    paused: bool,
    /// What was played, for the tests.
    #[cfg(test)]
    log: Vec<String>,
}

/// The running skating engine's sounds, for the driver: its adapter fills them each frame
/// (before [`SkaterSoundSet`]), adding its cues; the driver takes the cues.
#[derive(Resource, Default, Debug)]
pub(crate) struct SkaterSounds {
    /// Whose they are: the driver is silent while another engine runs.
    pub engine: Option<crate::engines::Engine>,
    /// The engine is held (a menu, build mode): its loops fade out.
    pub paused: bool,
    pub sounds: SkateSounds,
}

/// Where the driver plays the frame's sounds (`Update`): the engines' adapters fill [`SkaterSounds`] before it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SkaterSoundSet;

/// The driver, once for every engine (and not an engine's own: added outside `engines::add_engine`, it starts at
/// launch and its sounds are shared); later calls do nothing.
pub(crate) fn install(app: &mut App) {
    if app.world().contains_resource::<SkaterSounds>() {
        return;
    }
    add_sources(app);
    app.init_resource::<RoomFromWorld>()
        .init_resource::<SurfaceHints>()
        .init_resource::<SkaterSounds>()
        .add_systems(Startup, setup)
        .add_systems(Update, (drive, lab, mix, follow_skater).chain().in_set(SkaterSoundSet));
}

/// The sounds' sources, once: Bevy's playback systems for them are shared - a second copy would start each sound
/// twice.
fn add_sources(app: &mut App) {
    use bevy::audio::AddAudioSource;
    if !app.world().contains_resource::<Assets<CleanPcm>>() {
        app.add_audio_source::<CleanPcm>().add_audio_source::<ReverbPcm>();
    }
}

/// The frame's sounds, played: the cues, then the loops by how the skater is.
fn drive(
    mut commands: Commands,
    audio: Option<ResMut<CleanAudio>>,
    hints: Res<SurfaceHints>,
    mut heard: ResMut<SkaterSounds>,
    engines: Option<Res<crate::engines::Engines>>,
    real_time: Res<Time<bevy::time::Real>>,
) {
    let cues = std::mem::take(&mut heard.sounds.cues);
    let Some(mut audio) = audio else { return };
    // (without engines to switch between, the one engine's)
    let live = engines.is_none_or(|e| e.current() == heard.engine);
    let sounds = &heard.sounds;
    let now = real_time.elapsed_secs();
    audio.paused = heard.paused || !live;
    audio.source = sounds.at;
    if let Some(at) = audio.pending_catch {
        if now >= at {
            audio.pending_catch = None;
            audio.play(&mut commands, Shot::Catch, 0.8);
        }
    }
    for cue in cues.into_iter().filter(|_| live) {
        match cue {
            Cue::Pop => audio.play(&mut commands, Shot::Pop, 0.9),
            // A flip: the board's whoosh now, the catch a moment later.
            Cue::Flip => {
                audio.play(&mut commands, Shot::Flip, 0.7);
                audio.pending_catch = Some(now + CATCH_DELAY_S);
            }
            // Landing: once per touchdown, louder the harder it hits.
            Cue::Land { impact } => {
                if now - audio.last_land > 0.15 {
                    audio.last_land = now;
                    let gain = (0.45 + impact.abs() / 6.0).clamp(0.45, 1.0);
                    audio.play(&mut commands, Shot::Land, gain);
                }
            }
            Cue::Push => audio.play(&mut commands, Shot::Push, 0.75),
            Cue::Footstep { strength } => audio.play(&mut commands, Shot::Footstep, 0.4 + 0.4 * strength.min(1.0)),
            Cue::Bail => {
                if now - audio.last_wipe > 0.8 {
                    audio.last_wipe = now;
                    audio.play(&mut commands, Shot::Bail, 1.0);
                }
            }
            Cue::Chime => audio.play(&mut commands, Shot::Chime, 0.5),
        }
    }
    // (a catch is only heard in the air)
    if !sounds.airborne {
        audio.pending_catch = None;
    }
    let speed = sounds.speed.max(0.0);
    let level = ((speed - ROLL_MIN_SPEED) / (ROLL_FULL_SPEED - ROLL_MIN_SPEED)).clamp(0.0, 1.0);
    let level = level.powf(0.6);
    let pitch = 0.85 + 0.35 * (speed / 10.0).min(1.0);
    for kind in Loop::ALL {
        audio.set(kind, 0.0, pitch);
    }
    let position = sounds.at;
    match sounds.rolling {
        Rolling::Grind => {
            let kind = if hints.near_metal(position) { Loop::GrindMetal } else { Loop::GrindConcrete };
            audio.set(kind, (0.35 + 0.65 * level).min(1.0), 0.9 + 0.2 * (speed / 8.0).min(1.0));
        }
        Rolling::Ground => {
            // All AR real surfaces share one physics material, so the wheel material says nothing
            // there: placed ramps are wood, everything else rolls like smooth concrete. The rough and
            // metal loops are loaded for maps whose materials get mapped later.
            // On a wooden ramp the wheels sound lower, not different: the wood recording
            // (`roll_wood`) is a creaking board, a steady squeak under every roll.
            let (kind, pitch) = audio.rolling(hints.on_wood(position), pitch);
            if sounds.powersliding {
                audio.set(kind, level * 0.5, pitch);
                let (volume, pitch) = slide(speed);
                audio.set(Loop::Powerslide, volume, pitch);
            } else {
                audio.set(kind, level, pitch);
            }
        }
        Rolling::None => {}
    }
}

/// Keep the looping emitters on the skater.
fn follow_skater(audio: Option<Res<CleanAudio>>, mut transforms: Query<&mut Transform>) {
    let Some(audio) = audio.filter(|a| a.spatial) else { return };
    for (_, entity, _) in &audio.loops {
        if let Ok(mut transform) = transforms.get_mut(*entity) {
            if transform.translation != audio.source {
                transform.translation = audio.source;
            }
        }
    }
}

/// Mono or stereo 16-bit PCM from a canonical RIFF/WAVE file.
fn parse_wav(bytes: &[u8]) -> Option<(Vec<i16>, u16, u32)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let (mut at, mut format) = (12, None);
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let body = bytes.get(at + 8..(at + 8 + size).min(bytes.len()))?;
        if id == b"fmt " && body.len() >= 16 {
            let tag = u16::from_le_bytes([body[0], body[1]]);
            let channels = u16::from_le_bytes([body[2], body[3]]);
            let rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
            let bits = u16::from_le_bytes([body[14], body[15]]);
            if tag != 1 || bits != 16 || channels == 0 {
                return None;
            }
            format = Some((channels, rate));
        } else if id == b"data" {
            let (channels, rate) = format?;
            let samples = body
                .chunks_exact(2)
                .map(|s| i16::from_le_bytes([s[0], s[1]]))
                .collect();
            return Some((samples, channels, rate));
        }
        at += 8 + size + (size & 1);
    }
    None
}

fn setup(
    mut commands: Commands,
    mut assets: ResMut<Assets<CleanPcm>>,
    mut reverbs: ResMut<Assets<ReverbPcm>>,
    spatial: Option<Res<SpatialSkateAudio>>,
    file: Option<Res<TuningFile>>,
) {
    // In 3D the sounds go through Steam Audio's HRTF where it is there, and are panned where not.
    let emitter = (spatial.is_some() && crate::binaural::available()).then(|| Arc::new(Emitter::default()));
    let spatial = spatial.is_some() && emitter.is_none();
    let (mut voices, mut loops, mut shots) = (Vec::new(), Vec::new(), Vec::new());
    let (mut strips, mut buffers, mut recorded) = (Vec::new(), Vec::new(), Vec::new());
    let mut count = 0;
    for (k, kind) in KINDS.iter().enumerate() {
        let strip = Arc::new(Strip::default());
        let (mut kind_buffers, mut kind_recorded, mut handles) = (Vec::new(), Vec::new(), Vec::new());
        for variant in 0..kind.variants {
            let name = kind.file(variant);
            let samples = match WAVS.iter().find(|(n, _)| *n == name).and_then(|(_, bytes)| parse_wav(bytes)) {
                Some((mut samples, 1, RATE)) => {
                    if name == Loop::Powerslide.file() {
                        lowpass_loop(&mut samples, 1, RATE as f32, SLIDE_LOWPASS_HZ);
                    }
                    count += 1;
                    Arc::new(samples)
                }
                _ => {
                    error!("SKATE_CLEAN_AUDIO {name}.wav is not mono 16-bit PCM at {RATE} Hz");
                    Arc::new(vec![0i16; 480])
                }
            };
            let buffer = Buffer::new(samples.clone());
            let voice = emitter.clone().map(Voice::new);
            let handle = assets.add(CleanPcm::live(buffer.clone(), strip.clone(), RATE, kind.looping, voice.clone()));
            kind_buffers.push(buffer);
            kind_recorded.push(samples);
            if kind.looping {
                voices.push(voice);
                // Inline-looping source played once: it never ends, and Bevy's whole-source loop
                // (a buffering repeater) is not needed.
                let entity = commands
                    .spawn((
                        AudioPlayer(handle),
                        PlaybackSettings::ONCE.with_volume(Volume::Linear(0.0)).with_spatial(spatial),
                        Transform::default(),
                    ))
                    .id();
                loops.push((Loop::ALL[k], entity, 0.0));
            } else {
                handles.push((handle, voice));
            }
        }
        if !kind.looping {
            shots.push((Shot::ALL[k - Loop::ALL.len()], handles));
        }
        strips.push(strip);
        buffers.push(kind_buffers);
        recorded.push(kind_recorded);
    }
    // The room's answer to all of them: one source that never ends.
    let room = emitter.as_ref().and_then(|_| crate::reverb::room());
    if let Some(room) = room {
        commands.spawn((AudioPlayer(reverbs.add(ReverbPcm(room.clone()))), PlaybackSettings::ONCE));
    }
    info!("SKATE_CLEAN_AUDIO loaded {count} sounds (spatial={spatial} hrtf={} reverb={})", emitter.is_some(), room.is_some());
    commands.insert_resource(SoundLab::load(file.map(|f| f.0.clone())));
    commands.insert_resource(CleanAudio {
        spatial,
        emitter,
        voices,
        strips,
        buffers,
        recorded,
        wood_loop: false,
        source: Vec3::ZERO,
        shots,
        loops,
        targets: [(0.0, 1.0); 7],
        last_pick: [usize::MAX; 8],
        rng: 0x9E37_79B9,
        pending_catch: None,
        last_land: -1.0,
        last_wipe: -1.0,
        paused: false,
        #[cfg(test)]
        log: Vec::new(),
    });
}

impl CleanAudio {
    /// The loop under the wheels and its pitch: on a wooden ramp the smooth loop played lower
    /// (the wood recording is a creaking board), or the synthesised wood loop where that is on.
    fn rolling(&self, on_wood: bool, pitch: f32) -> (Loop, f32) {
        match (on_wood, self.wood_loop) {
            (true, true) => (Loop::RollWood, pitch),
            (true, false) => (Loop::RollSmooth, pitch * WOOD_PITCH),
            _ => (Loop::RollSmooth, pitch),
        }
    }

    fn random(&mut self) -> f32 {
        // xorshift32: plenty for picking variants.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    fn play(&mut self, commands: &mut Commands, shot: Shot, gain: f32) {
        #[cfg(test)]
        self.log.push(format!("{shot:?} {gain:?}"));
        let slot = Shot::ALL.iter().position(|&s| s == shot).unwrap_or(0);
        let Some(count) = self
            .shots
            .iter()
            .find(|(s, _)| *s == shot)
            .map(|(_, v)| v.len())
            .filter(|&n| n > 0)
        else {
            return;
        };
        // A random variant, never the same one twice in a row.
        let mut pick = (self.random() * count as f32) as usize % count;
        if count > 1 && pick == self.last_pick[slot] {
            pick = (pick + 1) % count;
        }
        self.last_pick[slot] = pick;
        let pitch = 0.96 + 0.08 * self.random();
        let (handle, voice) = self
            .shots
            .iter()
            .find(|(s, _)| *s == shot)
            .map(|(_, v)| v[pick].clone())
            .expect("checked above");
        let level = (gain * MASTER).clamp(0.0, 1.5);
        if let Some(voice) = &voice {
            voice.set_level(level);
        }
        commands.spawn((
            AudioPlayer(handle),
            PlaybackSettings::DESPAWN
                .with_volume(Volume::Linear(level))
                .with_speed(pitch)
                .with_spatial(self.spatial),
            Transform::from_translation(self.source),
        ));
        debug!("SKATE_CLEAN_AUDIO {} gain={gain:.2}", shot.prefix());
    }

    fn set(&mut self, kind: Loop, volume: f32, pitch: f32) {
        let i = Loop::ALL.iter().position(|&k| k == kind).unwrap_or(0);
        self.targets[i] = (volume, pitch);
    }
}

/// The powerslide loop's volume and pitch at a board speed.
fn slide(speed: f32) -> (f32, f32) {
    let level = ((speed - SLIDE_MIN_SPEED) / (SLIDE_FULL_SPEED - SLIDE_MIN_SPEED)).clamp(0.0, 1.0);
    (SLIDE_GAIN * level, 0.8 + 0.15 * level)
}

/// Takes what is above `cutoff` out of a looping sound (two one-pole filters, 12 dB an octave),
/// run round the loop twice so its end still meets its start.
fn lowpass_loop(samples: &mut [i16], channels: usize, rate: f32, cutoff: f32) {
    let k = 1.0 - (-std::f32::consts::TAU * cutoff / rate).exp();
    for channel in 0..channels.max(1) {
        let (mut a, mut b) = (0.0f32, 0.0f32);
        for pass in 0..2 {
            for sample in samples.iter_mut().skip(channel).step_by(channels.max(1)) {
                a += k * (*sample as f32 - a);
                b += k * (a - b);
                if pass == 1 {
                    *sample = b.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                }
            }
        }
    }
}

// ---------------------------------------------------------------- the sound lab

/// A held loop's level in the lab, of a loop's full level; a hit's when it is played there.
const AUDITION_LEVEL: f32 = 0.8;
/// How often a held hit is played, seconds.
const AUDITION_EVERY_S: f32 = 0.9;
/// A sound is made again this long after its values were last changed, seconds.
const REMAKE_AFTER_S: f32 = 0.15;
/// The tuning is saved this long after the last change, seconds.
const SAVE_AFTER_S: f32 = 1.0;
const TUNING_FILE: &str = "sound-tuning.json";
/// The tuning the app is built with: `sound-tuning.json` from a headset, kept beside the sounds.
const BAKED: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/clean-audio/tuning.json"));

/// Where the tuning is kept (`sound-tuning.json` in this directory). Without it, `SK8_SOUND_TUNING`
/// names the file, or nothing is saved.
#[derive(Resource, Clone)]
pub(crate) struct TuningFile(pub std::path::PathBuf);

type Made = (usize, u32, Vec<Arc<Vec<i16>>>);

/// Every kind of sound as tuned ([`KINDS`]' order), and what the sound lab's panel
/// (`xr::ui::sound_lab`) is doing: which is selected, soloed, held playing.
#[derive(Resource)]
pub(crate) struct SoundLab {
    tunes: Vec<Tune>,
    pub selected: usize,
    /// Only the selected kind is heard.
    pub solo: bool,
    /// The selected kind keeps playing: a loop held on, a hit again and again.
    pub audition: bool,
    /// The pitch a held loop plays at (the game plays loops at 0.85 to 1.2 by speed).
    pub pitch: f32,
    /// The room's reverberation: its level, and how absorbent the room is taken to be.
    pub reverb: f32,
    pub damping: f32,
    play: bool,
    path: Option<std::path::PathBuf>,
    now: f32,
    last_played: f32,
    save_wanted: Option<f32>,
    /// Since when each kind has wanted making again, and the count of its changes.
    remake: Vec<Option<f32>>,
    changes: Vec<u32>,
    making: Vec<bool>,
    /// Whether each kind's sounds are synthesised ones as they stand.
    synthesised: Vec<bool>,
    made: (std::sync::mpsc::Sender<Made>, std::sync::Mutex<std::sync::mpsc::Receiver<Made>>),
}

impl SoundLab {
    fn load(directory: Option<std::path::PathBuf>) -> Self {
        let path = directory.map(|d| d.join(TUNING_FILE)).or_else(|| std::env::var_os("SK8_SOUND_TUNING").map(Into::into));
        let mut tuning: Tuning = serde_json::from_str(BAKED).unwrap_or_else(|e| {
            error!("SOUND LAB the built-in tuning doesn't parse: {e}");
            Tuning::default()
        });
        if let Some(bytes) = path.as_ref().and_then(|p| std::fs::read(p).ok()) {
            match serde_json::from_slice::<Tuning>(&bytes) {
                Ok(saved) => {
                    info!("SOUND LAB loaded {} ({} sounds tuned)", path.as_ref().unwrap().display(), saved.sounds.len());
                    tuning.sounds.extend(saved.sounds);
                }
                Err(e) => warn!("SOUND LAB ignoring {}: {e}", path.as_ref().unwrap().display()),
            }
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let tunes: Vec<Tune> = KINDS.iter().map(|kind| tuning.sounds.get(kind.name).cloned().unwrap_or_default()).collect();
        Self {
            remake: tunes.iter().map(|tune| tune.synth.then_some(f32::MIN)).collect(),
            tunes,
            selected: 0,
            solo: false,
            audition: false,
            pitch: 1.0,
            reverb: tuning.reverb,
            damping: tuning.damping,
            play: false,
            path,
            now: 0.0,
            last_played: 0.0,
            save_wanted: None,
            changes: vec![0; KINDS.len()],
            making: vec![false; KINDS.len()],
            synthesised: vec![false; KINDS.len()],
            made: (sender, std::sync::Mutex::new(receiver)),
        }
    }

    #[cfg(feature = "xr")]
    pub(crate) fn tune(&self, kind: usize) -> &Tune {
        &self.tunes[kind]
    }

    /// Where the tuning is saved, if anywhere.
    #[cfg(feature = "xr")]
    pub(crate) fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    /// A generator value of a kind: as tuned, else its default.
    pub(crate) fn value(&self, kind: usize, param: usize) -> f32 {
        let (k, tune) = (&KINDS[kind], &self.tunes[kind]);
        tune.params.get(k.params[param].key).copied().unwrap_or(k.defaults[param]).clamp(k.params[param].min, k.params[param].max)
    }

    /// Change a kind's tuning: saved, and its sounds made again if what they are made from changed.
    #[cfg(any(feature = "xr", test))]
    pub(crate) fn edit(&mut self, kind: usize, f: impl FnOnce(&mut Tune)) {
        let before = self.tunes[kind].clone();
        f(&mut self.tunes[kind]);
        let after = &self.tunes[kind];
        if *after == before {
            return;
        }
        self.save_wanted = Some(self.now);
        if after.synth && (!before.synth || after.params != before.params) {
            self.remake[kind] = Some(self.now);
            self.changes[kind] += 1;
        }
    }

    #[cfg(any(feature = "xr", test))]
    pub(crate) fn set_value(&mut self, kind: usize, param: usize, value: f32) {
        let (key, default) = (KINDS[kind].params[param].key, KINDS[kind].defaults[param]);
        self.edit(kind, |tune| {
            if (value - default).abs() <= 1e-6 * default.abs().max(1.0) {
                tune.params.remove(key);
            } else {
                tune.params.insert(key.to_string(), value);
            }
        });
    }

    /// Set the room's reverberation (saved).
    #[cfg(feature = "xr")]
    pub(crate) fn set_reverb(&mut self, reverb: f32, damping: f32) {
        if (self.reverb, self.damping) != (reverb, damping) {
            (self.reverb, self.damping) = (reverb, damping);
            self.save_wanted = Some(self.now);
        }
    }

    /// Play the selected hit once.
    #[cfg(feature = "xr")]
    pub(crate) fn play_once(&mut self) {
        self.play = true;
    }

    /// The file's contents: the kinds that are not as they come.
    fn tuning(&self) -> Tuning {
        let sounds = KINDS.iter().zip(&self.tunes).filter(|(_, tune)| **tune != Tune::default()).map(|(kind, tune)| (kind.name.to_string(), tune.clone()));
        Tuning { sounds: sounds.collect(), reverb: self.reverb, damping: self.damping }
    }
}

/// The tuning into what plays: levels and EQs, synthesised sounds made (off the frame) and put in
/// place, the selected hit played, the file saved.
fn lab(mut commands: Commands, audio: Option<ResMut<CleanAudio>>, lab: Option<ResMut<SoundLab>>, time: Res<Time<bevy::time::Real>>) {
    let (Some(mut audio), Some(mut lab)) = (audio, lab) else { return };
    let lab = &mut *lab;
    let now = time.elapsed_secs();
    lab.now = now;
    for (k, tune) in lab.tunes.iter().enumerate() {
        let heard = !lab.solo || k == lab.selected;
        let level = if heard { 10f32.powf(tune.level_db / 20.0) } else { 0.0 };
        audio.strips[k].set(level, tune.eq);
    }
    audio.wood_loop = lab.tunes[Loop::ALL.iter().position(|&l| l == Loop::RollWood).unwrap_or(0)].synth;
    // What was made since the last frame, unless its values have changed again since.
    let made: Vec<Made> = lab.made.1.lock().map(|receiver| receiver.try_iter().collect()).unwrap_or_default();
    for (k, change, variants) in made {
        lab.making[k] = false;
        if lab.tunes[k].synth && change == lab.changes[k] {
            for (buffer, samples) in audio.buffers[k].iter().zip(variants) {
                buffer.set(samples);
            }
            lab.synthesised[k] = true;
            lab.remake[k] = None;
        }
    }
    for (k, kind) in KINDS.iter().enumerate() {
        if !lab.tunes[k].synth {
            if std::mem::take(&mut lab.synthesised[k]) {
                for (buffer, samples) in audio.buffers[k].iter().zip(&audio.recorded[k]) {
                    buffer.set(samples.clone());
                }
            }
            lab.remake[k] = None;
        } else if !lab.making[k] && lab.remake[k].is_some_and(|since| now - since >= REMAKE_AFTER_S) {
            lab.making[k] = true;
            let values: Vec<f32> = (0..kind.params.len()).map(|p| lab.value(k, p)).collect();
            let (change, sender) = (lab.changes[k], lab.made.0.clone());
            let work = move || {
                let variants = (0..kind.variants).map(|variant| Arc::new(kind.render(&values, variant))).collect();
                let _ = sender.send((k, change, variants));
            };
            if let Err(e) = std::thread::Builder::new().name("sound-lab".into()).spawn(work) {
                error!("SOUND LAB could not make {}: {e}", kind.name);
                lab.making[k] = false;
                lab.remake[k] = None;
            }
        }
    }
    // The selected hit: once when asked, or again and again while held.
    if let Some(&shot) = lab.selected.checked_sub(Loop::ALL.len()).and_then(|i| Shot::ALL.get(i)) {
        if std::mem::take(&mut lab.play) || (lab.audition && now - lab.last_played >= AUDITION_EVERY_S) {
            lab.last_played = now;
            audio.play(&mut commands, shot, AUDITION_LEVEL);
        }
    }
    lab.play = false;
    if lab.save_wanted.is_some_and(|since| now - since >= SAVE_AFTER_S) {
        lab.save_wanted = None;
        if let Some(path) = &lab.path {
            match serde_json::to_string_pretty(&lab.tuning()).map_err(|e| e.to_string()).and_then(|json| std::fs::write(path, json + "\n").map_err(|e| e.to_string())) {
                Ok(()) => info!("SOUND LAB saved {}", path.display()),
                Err(e) => warn!("SOUND LAB could not save {}: {e}", path.display()),
            }
        }
    }
}

fn mix(
    audio: Option<ResMut<CleanAudio>>,
    mut sinks: Query<&mut AudioSink>,
    mut spatial_sinks: Query<&mut SpatialAudioSink>,
    time: Res<Time<bevy::time::Real>>,
    listener: Query<&Transform, With<bevy::audio::SpatialListener>>,
    scale: Res<bevy::audio::DefaultSpatialScale>,
    lab: Option<Res<SoundLab>>,
    room_from_world: Res<RoomFromWorld>,
    engines: Option<Res<crate::engines::Engines>>,
) {
    let Some(mut audio) = audio else { return };
    // Where the skater is in the real room, for its reverberation (SK8's; an engine the driver hosts puts its own
    // player there: `engine_driver::aim`).
    let sk8 = engines.is_some_and(|e| e.current() == Some(crate::engines::Engine::Sk8));
    if let (Some(room), Some(to_room), Some(_), true) = (crate::reverb::room(), room_from_world.0, &audio.emitter, sk8) {
        room.set_source(to_room.transform_point3(audio.source));
    }
    // (the sound lab's level and deadness of the room: every engine's)
    if let (Some(room), Some(lab)) = (crate::reverb::room(), &lab) {
        room.set_mix(lab.reverb, lab.damping);
    }
    // The sound lab holds the loop being tuned on, whatever the skater is doing.
    let held = lab.filter(|lab| lab.audition && lab.selected < Loop::ALL.len()).map(|lab| (lab.selected, lab.pitch));
    // Through the HRTF: where the skater is from the head, and as loud as the panned sounds were
    // at that distance (full level within one spatial unit, the inverse square beyond it).
    if let (Some(emitter), Ok(head)) = (&audio.emitter, listener.single()) {
        crate::binaural::place(emitter, audio.source, head, scale.0.0.x);
    }
    let voices = audio.voices.clone();
    let blend = (time.delta_secs() / FADE_S).min(1.0);
    let paused = audio.paused;
    let targets = audio.targets;
    for (i, (_, entity, volume)) in audio.loops.iter_mut().enumerate() {
        let (target, pitch) = targets[i];
        let target = if paused { 0.0 } else { target };
        let (target, pitch) = if held.is_some_and(|(k, _)| k == i) { (AUDITION_LEVEL, held.map_or(1.0, |h| h.1)) } else { (target, pitch) };
        *volume += (target - *volume) * blend;
        let level = Volume::Linear(*volume * LOOP_GAIN * MASTER);
        let speed = pitch.clamp(0.5, 2.0);
        if let Some(voice) = voices.get(i).and_then(Option::as_ref) {
            voice.set_pitch(speed);
            voice.set_level(*volume * LOOP_GAIN * MASTER);
            if let Ok(mut sink) = sinks.get_mut(*entity) {
                sink.set_volume(level);
            }
        } else if let Ok(mut sink) = sinks.get_mut(*entity) {
            sink.set_volume(level);
            sink.set_speed(speed);
        } else if let Ok(mut sink) = spatial_sinks.get_mut(*entity) {
            sink.set_volume(level);
            sink.set_speed(speed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_powerslide_is_silent_at_walking_pace_and_stays_under_the_wheels() {
        assert_eq!(slide(0.0).0, 0.0);
        assert_eq!(slide(SLIDE_MIN_SPEED).0, 0.0);
        let mut last = 0.0;
        for speed in [2.0, 4.0, 7.0, 20.0] {
            let (volume, pitch) = slide(speed);
            assert!(volume >= last && volume <= SLIDE_GAIN, "{speed}: {volume}");
            assert!((0.8..=0.951).contains(&pitch));
            last = volume;
        }
        assert!(last > 0.3);
    }

    #[test]
    fn the_powerslide_loses_its_squeal_and_still_loops() {
        let (_, bytes) = WAVS.iter().find(|(n, _)| *n == "powerslide").unwrap();
        let (original, channels, rate) = parse_wav(bytes).unwrap();
        let mut soft = original.clone();
        lowpass_loop(&mut soft, channels as usize, rate as f32, SLIDE_LOWPASS_HZ);
        // The energy of the sample-to-sample changes is the top of the spectrum.
        let harsh = |s: &[i16]| s.windows(2).map(|w| (w[1] as f64 - w[0] as f64).powi(2)).sum::<f64>();
        let level = |s: &[i16]| s.iter().map(|&v| (v as f64).powi(2)).sum::<f64>();
        assert!(harsh(&soft) < 0.25 * harsh(&original), "{} of {}", harsh(&soft), harsh(&original));
        assert!(level(&soft) > 0.3 * level(&original), "the body of the sound is kept");
        // The seam is no bigger a step than the loop's own.
        let seam = (soft[0] as f64 - soft[soft.len() - 1] as f64).abs();
        let biggest = soft.windows(2).map(|w| (w[1] as f64 - w[0] as f64).abs()).fold(0.0, f64::max);
        assert!(seam <= biggest, "{seam} > {biggest}");
    }

    #[test]
    fn every_clean_sound_parses() {
        for (name, bytes) in WAVS {
            let (samples, channels, rate) = parse_wav(bytes).unwrap_or_else(|| panic!("{name}"));
            assert_eq!((channels, rate), (1, 48_000), "{name}");
            assert!(samples.len() > 1000, "{name}");
        }
    }

    #[test]
    fn the_kinds_are_the_drivers_own_and_each_has_its_recordings() {
        assert_eq!(KINDS.len(), Loop::ALL.len() + Shot::ALL.len());
        for (kind, sound) in KINDS.iter().zip(Loop::ALL) {
            assert!(kind.looping && kind.name == sound.file(), "{}", kind.name);
        }
        for (kind, shot) in KINDS[Loop::ALL.len()..].iter().zip(Shot::ALL) {
            assert!(!kind.looping && kind.name == shot.prefix(), "{}", kind.name);
        }
        let mut files = 0;
        for kind in &KINDS {
            for variant in 0..kind.variants {
                assert!(WAVS.iter().any(|(name, _)| *name == kind.file(variant)), "{}", kind.file(variant));
                files += 1;
            }
        }
        assert_eq!(files, WAVS.len());
    }

    #[test]
    fn the_lab_keeps_what_is_tuned_and_only_that() {
        let mut lab = SoundLab::load(None);
        lab.path = None;
        assert_eq!(lab.tuning(), serde_json::from_str::<Tuning>(BAKED).unwrap());
        let pop = KINDS.iter().position(|k| k.name == "pop").unwrap();
        let click = KINDS[pop].params.iter().position(|p| p.key == "click").unwrap();
        lab.now = 5.0;
        lab.edit(pop, |tune| tune.synth = true);
        lab.set_value(pop, click, 0.5);
        lab.edit(pop, |tune| tune.eq[1].gain = -6.0);
        assert_eq!(lab.value(pop, click), 0.5);
        assert_eq!((lab.remake[pop], lab.changes[pop]), (Some(5.0), 2), "made again for the source and the value, not the EQ");
        let saved = lab.tuning();
        assert_eq!(saved.sounds["pop"].params["click"], 0.5);
        assert_eq!(saved.sounds["pop"].eq[1].gain, -6.0);
        // Back at its default, a value is no longer in the file.
        lab.set_value(pop, click, KINDS[pop].defaults[click]);
        assert!(lab.tuning().sounds["pop"].params.is_empty());
        lab.edit(pop, |tune| *tune = Tune::default());
        assert!(!lab.tuning().sounds.contains_key("pop"));
    }

    #[test]
    fn hints_find_rails_and_ramps() {
        let hints = SurfaceHints {
            metal_rails: vec![(Vec3::ZERO, Vec3::X * 2.0)],
            wood_boxes: vec![(Vec3::new(5.0, 0.0, 0.0), Vec3::new(6.0, 0.5, 1.0))],
        };
        assert!(hints.near_metal(Vec3::new(1.0, 0.3, 0.0)));
        assert!(!hints.near_metal(Vec3::new(1.0, 2.0, 0.0)));
        assert!(hints.on_wood(Vec3::new(5.5, 1.0, 0.5)));
        assert!(!hints.on_wood(Vec3::new(3.0, 0.5, 0.5)));
    }

}

/// What the driver decides (the shots it plays, its loops' targets), frame by frame, over scripted runs of each
/// engine: digested, so moving the engines' knowledge out of the driver can be checked to change nothing.
#[cfg(test)]
mod parity {
    use super::*;
    use std::fmt::Write;
    use std::time::Duration;

    pub(super) fn bare() -> CleanAudio {
        CleanAudio {
            spatial: false,
            source: Vec3::ZERO,
            shots: Vec::new(),
            loops: Vec::new(),
            emitter: None,
            voices: Vec::new(),
            strips: Vec::new(),
            buffers: Vec::new(),
            recorded: Vec::new(),
            wood_loop: false,
            targets: [(0.0, 1.0); 7],
            last_pick: [usize::MAX; 8],
            rng: 0x9E37_79B9,
            pending_catch: None,
            last_land: -1.0,
            last_wipe: -1.0,
            paused: false,
            log: Vec::new(),
        }
    }

    pub(super) fn hints() -> SurfaceHints {
        SurfaceHints {
            metal_rails: vec![(Vec3::new(0.0, 0.3, 5.0), Vec3::new(3.0, 0.3, 5.0))],
            wood_boxes: vec![(Vec3::new(-3.0, 0.0, -3.0), Vec3::new(-1.0, 1.0, -1.0))],
        }
    }

    /// Where the scripted skater is at a tick: on the rail while grinding, in the wooden box while sliding.
    pub(super) fn position(t: u64) -> Vec3 {
        match t {
            100..=140 => Vec3::new((t - 100) as f32 * 0.07, 0.35, 5.1),
            160..=200 => Vec3::new(-2.0, 0.2, -2.0),
            _ => Vec3::new(t as f32 * 0.01, 0.0, 0.0),
        }
    }

    pub(super) const FRAME: Duration = Duration::from_nanos(1_000_000_000 / 72);
    pub(super) const FRAMES: u64 = 420;
    /// The frames the game is paused for (a menu): no ticks.
    pub(super) fn paused(frame: u64) -> bool {
        (300..330).contains(&frame)
    }

    /// The ticks due by each frame (60 Hz under 72 Hz frames: none, one or two), none while paused.
    pub(super) fn ticks() -> Vec<std::ops::Range<u64>> {
        let (mut out, mut done, mut running) = (Vec::new(), 0, 0u64);
        for frame in 0..FRAMES {
            if !paused(frame) {
                running += 1;
            }
            let due = running * 60 / 72;
            out.push(done..due);
            done = due;
        }
        out
    }

    pub(super) fn fnv(text: &str) -> u64 {
        text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
    }

    pub(super) fn frame_line(out: &mut String, frame: u64, audio: &mut CleanAudio) {
        let _ = writeln!(out, "{frame}: {:?} {:?} {:?} {:?}", std::mem::take(&mut audio.log), audio.targets, audio.pending_catch, audio.source);
    }

    #[cfg(feature = "retail")]
    pub(super) fn observation(t: u64) -> skate3_sim::skate_audio::PlayerAudioObservation {
        use skate_core::physics::phase::PhysicsEvent;
        use skate3_sim::skate_audio::{PlayerAudioObservation, RetailAudioInputs};
        let mut o = PlayerAudioObservation {
            tick: t,
            state: 100,
            board_speed: (t as f32 * 0.1).min(6.0),
            rider_speed: 0.0,
            grounded: true,
            grinding: false,
            grind_family: 0,
            grind_substate: 0,
            grind_audio_surface: 0,
            grind_impact_speed: 0.0,
            wiping_out: false,
            landed: false,
            landing_impact_speed: 0.0,
            landing_clean: false,
            landing_sketchy: false,
            landing_type: 0,
            landing_spin: 0.0,
            landing_sideways_speed: 0.0,
            footstep_strength: 0.0,
            footstep_bone: -1,
            foot_push_speed: 0.0,
            feet_supported: [false; 2],
            foot_surface: 0,
            contact_count: 0,
            powersliding: false,
            trick_id: None,
            trick_identifier: None,
            animation_name: None,
            riding_switch: false,
            riding_fakie: false,
            nollie: false,
            wheel_surface: (t / 37) as u32 % 5,
            events: Vec::new(),
            frontend_sounds: Vec::new(),
            retail: RetailAudioInputs::default(),
        };
        // (the sim's world: the room's raised to its floor)
        o.retail.position = skate3_engine::stage::to_sim(position(t)).to_array();
        let air = |o: &mut PlayerAudioObservation, trick: &str| {
            o.state = 200;
            o.grounded = false;
            o.trick_identifier = Some(trick.into());
        };
        match t {
            10..=14 | 40..=44 => o.foot_push_speed = 0.5,
            60..=69 => air(&mut o, "Ollie"),
            70..=89 => air(&mut o, "Kickflip"),
            90 => {
                o.landed = true;
                o.landing_impact_speed = -3.5;
                o.events.push(PhysicsEvent::Landing);
            }
            100..=140 => {
                o.state = 401;
                o.grinding = true;
                o.grounded = false;
            }
            141..=144 => air(&mut o, "Ollie"),
            145..=149 => air(&mut o, "Heelflip"),
            150 => o.events.push(PhysicsEvent::Landing),
            160..=200 => {
                o.state = 101;
                o.powersliding = true;
            }
            201..=204 | 210..=215 => {
                o.state = 300;
                o.grounded = false;
                o.wiping_out = t != 203;
                if t == 201 {
                    o.events.push(PhysicsEvent::Wipeout);
                }
            }
            205..=209 => o.state = 300,
            216..=229 => o.board_speed = 0.1,
            230..=280 => {
                o.state = 501;
                o.board_speed = 0.0;
                o.footstep_strength = if t % 15 < 3 { 0.6 } else { 0.0 };
            }
            281..=289 => air(&mut o, "Ollie"),
            290 | 292 => {
                o.landed = true;
                o.landing_impact_speed = -8.0;
            }
            300..=310 => {
                // (a grind the state doesn't name: the flag alone)
                o.state = 202;
                o.grounded = false;
                o.grinding = true;
            }
            311..=320 => air(&mut o, "varial_heelflip"),
            _ => {}
        }
        o
    }

    #[cfg(feature = "retail")]
    #[test]
    fn skate3s_sounds_are_decided_as_recorded() {
        let mut world = World::new();
        world.insert_resource(bare());
        world.insert_resource(hints());
        world.insert_resource(Time::<bevy::time::Real>::default());
        world.init_resource::<SkaterSounds>();
        let drive = world.register_system(drive);
        // (as Skate 3's adapter fed it: each tick observed, the front-end sounds one chime, a frame's sounds taken)
        let mut heard = skate3_engine::sound::Heard::default();
        let mut out = String::new();
        for (frame, ticks) in ticks().into_iter().enumerate() {
            let frame = frame as u64;
            world.resource_mut::<Time<bevy::time::Real>>().update_with_duration(FRAME);
            for t in ticks {
                heard.observe(&observation(t));
            }
            if frame == 120 || frame == 121 || frame == 250 {
                heard.front_end();
            }
            world.insert_resource(SkaterSounds {
                engine: Some(crate::engines::Engine::Skate3),
                paused: paused(frame),
                sounds: heard.take(),
            });
            world.run_system(drive).unwrap();
            frame_line(&mut out, frame, &mut world.resource_mut::<CleanAudio>());
        }
        let shots = out.matches(|c| c == '"').count() / 2;
        eprintln!("skate3 clean sounds: {} shots, digest {:016x}", shots, fnv(&out));
        if std::env::var_os("PARITY_DUMP").is_some() {
            std::fs::write(std::env::temp_dir().join("skate3-sounds.txt"), &out).unwrap();
        }
        assert!(shots > 15, "{out}");
        assert_eq!(fnv(&out), SKATE3_DIGEST);
    }

    /// Recorded at c45e939 (`8c0d54649b762692`, Skate 3's knowledge in the driver), and again through the engine's
    /// `skate3_engine::sound::Heard`: one frame differs, the first, before any tick (the loops' silent targets now have
    /// the standing pitch). Again (from `c46c91528e778afe`) when Skate 3 took the room's floor to its own: the same
    /// sounds every frame; only the positions written down differ, by the last bit of a y gone into the sim's world
    /// and back (0.35 as 0.35000002).
    #[cfg(feature = "retail")]
    const SKATE3_DIGEST: u64 = 0x1f76_48db_46d1_1ad7;

    /// What SK8 said a frame, as its adapter had it before the skate mode's sounds (`crate::clean::feed_sounds` makes
    /// those from the same).
    #[derive(Default)]
    pub(super) struct Sk8Sounds {
        position: Vec3,
        speed: f32,
        grounded: bool,
        grinding: bool,
        powersliding: bool,
        paused: bool,
        cues: Vec<Sk8Cue>,
    }

    pub(super) enum Sk8Cue {
        Pop,
        Flip,
        Land(f32),
        Push,
        Bail,
        Chime,
    }

    impl Sk8Sounds {
        fn said(self) -> SkaterSounds {
            let cues = self
                .cues
                .into_iter()
                .map(|c| match c {
                    Sk8Cue::Pop => Cue::Pop,
                    Sk8Cue::Flip => Cue::Flip,
                    Sk8Cue::Land(impact) => Cue::Land { impact },
                    Sk8Cue::Push => Cue::Push,
                    Sk8Cue::Bail => Cue::Bail,
                    Sk8Cue::Chime => Cue::Chime,
                })
                .collect();
            let rolling = if self.grinding { Rolling::Grind } else if self.grounded { Rolling::Ground } else { Rolling::None };
            SkaterSounds {
                engine: None,
                paused: self.paused,
                sounds: SkateSounds {
                    at: self.position,
                    speed: self.speed,
                    rolling,
                    powersliding: self.powersliding,
                    airborne: !self.grounded && !self.grinding,
                    cues,
                },
            }
        }
    }

    /// SK8's sounds a frame, scripted.
    pub(super) fn sk8(frame: u64) -> Sk8Sounds {
        let t = frame;
        let mut s = Sk8Sounds {
            position: position(t),
            speed: (t as f32 * 0.08).min(7.0),
            grounded: true,
            grinding: false,
            powersliding: false,
            paused: paused(frame),
            cues: Vec::new(),
        };
        match t {
            10 | 40 => s.cues.push(Sk8Cue::Push),
            60 => s.cues.push(Sk8Cue::Pop),
            70 => {
                s.grounded = false;
                s.cues.push(Sk8Cue::Flip);
            }
            61..=89 => s.grounded = false,
            90 => s.cues.push(Sk8Cue::Land(3.5)),
            100..=140 => {
                s.grounded = false;
                s.grinding = true;
            }
            141 => {
                s.grounded = false;
                s.cues.push(Sk8Cue::Pop);
            }
            142..=148 => s.grounded = false,
            // (a flip into a grind: the catch comes on the rail)
            149 => {
                s.grounded = false;
                s.cues.push(Sk8Cue::Flip);
            }
            150..=158 => {
                s.grounded = false;
                s.grinding = true;
            }
            160..=200 => s.powersliding = true,
            201 | 210 => {
                s.grounded = false;
                s.cues.push(Sk8Cue::Bail);
            }
            250 => s.cues.extend([Sk8Cue::Chime, Sk8Cue::Chime]),
            290 | 292 => s.cues.push(Sk8Cue::Land(8.0)),
            _ => {}
        }
        s
    }

    #[test]
    fn sk8s_sounds_are_decided_as_recorded() {
        let mut world = World::new();
        world.insert_resource(bare());
        world.insert_resource(hints());
        world.insert_resource(Time::<bevy::time::Real>::default());
        let observe = world.register_system(drive);
        let mut out = String::new();
        for frame in 0..FRAMES {
            world.resource_mut::<Time<bevy::time::Real>>().update_with_duration(FRAME);
            world.insert_resource(sk8(frame).said());
            world.run_system(observe).unwrap();
            frame_line(&mut out, frame, &mut world.resource_mut::<CleanAudio>());
        }
        let shots = out.matches(|c| c == '"').count() / 2;
        eprintln!("sk8 sounds: {} shots, digest {:016x}", shots, fnv(&out));
        if std::env::var_os("PARITY_DUMP").is_some() {
            std::fs::write(std::env::temp_dir().join("sk8-sounds.txt"), &out).unwrap();
        }
        assert!(shots > 10, "{out}");
        assert_eq!(fnv(&out), SK8_DIGEST);
    }

    /// Recorded at c45e939 (`0ca7f5bd3fd01860`), and again with the skate mode's sounds: one change, a flip into a
    /// grind no longer has its catch on the rail (a catch is only heard in the air, as Skate 3's always was).
    const SK8_DIGEST: u64 = 0x2108_2035_b3d9_bffe;
}
