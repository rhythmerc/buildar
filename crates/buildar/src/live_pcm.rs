//! A continuous audio source a thread renders into: an engine's own mix (`engine_audio::EngineStream`).
use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use bevy::audio::{Decodable, Source};
use bevy::prelude::*;

/// A bounded, continuous float source for the recovered audio graph.
///
/// The audio thread never waits for the game thread. When a graph block has not arrived yet (or a
/// producer momentarily owns the queue), its decoder supplies silence and remains live for the
/// next block. Keeping the queue bounded also prevents a stalled output device from turning into
/// delayed gameplay audio.
#[derive(Asset, TypePath, Clone)]
pub struct LivePcm {
    samples: Arc<Mutex<VecDeque<f32>>>,
    stats: Arc<LivePcmStats>,
    channels: u16,
    sample_rate: u32,
    capacity: usize,
}

impl LivePcm {
    /// Hold at most `capacity_frames` rendered frames. A zero channel count, rate, or capacity is
    /// not a useful continuous audio source and is rejected at construction.
    pub fn new(channels: u16, sample_rate: u32, capacity_frames: usize) -> Result<Self, String> {
        if channels == 0 || sample_rate == 0 || capacity_frames == 0 {
            return Err("live PCM needs nonzero channels, sample rate, and capacity".into());
        }
        Ok(Self {
            samples: Arc::new(Mutex::new(VecDeque::with_capacity(
                capacity_frames.saturating_mul(usize::from(channels)),
            ))),
            stats: Arc::new(LivePcmStats::default()),
            channels,
            sample_rate,
            capacity: capacity_frames.saturating_mul(usize::from(channels)),
        })
    }

    /// Append complete interleaved frames. If the renderer gets ahead of output, discard the
    /// oldest complete frames so current gameplay remains current. The returned count is the
    /// number of samples discarded from the front.
    pub fn push(&self, samples: &[f32]) -> Result<usize, String> {
        if samples.len() % usize::from(self.channels) != 0 {
            return Err("live PCM input does not end on a frame boundary".into());
        }
        if samples.iter().any(|value| !value.is_finite()) {
            return Err("live PCM contains a non-finite sample".into());
        }
        let mut queue = self
            .samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let overflow = queue
            .len()
            .saturating_add(samples.len())
            .saturating_sub(self.capacity);
        let dropped = overflow.min(queue.len());
        for _ in 0..dropped {
            queue.pop_front();
        }
        // A block can be larger than this source's whole latency budget. Keep its newest complete
        // frames for the same reason as normal queue overflow.
        let keep = samples.len().min(self.capacity);
        let skip = samples.len() - keep;
        queue.extend(samples[skip..].iter().copied());
        self.stats.pushed.fetch_add(keep as u64, Ordering::Relaxed);
        Ok(dropped + skip)
    }

    /// Frames waiting in the shared queue, excluding the decoder's current small block.
    pub fn queued_frames(&self) -> usize {
        self.samples.lock().unwrap_or_else(|p| p.into_inner()).len() / usize::from(self.channels)
    }

    pub fn snapshot(&self) -> LivePcmSnapshot {
        LivePcmSnapshot {
            pushed: self.stats.pushed.load(Ordering::Relaxed),
            consumed: self.stats.consumed.load(Ordering::Relaxed),
            consumed_nonzero: self.stats.consumed_nonzero.load(Ordering::Relaxed),
            underflow: self.stats.underflow.load(Ordering::Relaxed),
        }
    }
}

#[derive(Default)]
struct LivePcmStats {
    pushed: AtomicU64,
    consumed: AtomicU64,
    consumed_nonzero: AtomicU64,
    underflow: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LivePcmSnapshot {
    pub pushed: u64,
    pub consumed: u64,
    pub consumed_nonzero: u64,
    pub underflow: u64,
}

/// The mixer-facing decoder for [`LivePcm`].
pub struct LivePlayback {
    samples: Arc<Mutex<VecDeque<f32>>>,
    stats: Arc<LivePcmStats>,
    channels: u16,
    sample_rate: u32,
    block: VecDeque<f32>,
}

impl Iterator for LivePlayback {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.block.is_empty() {
            // Transfer only whole frames. Locking once per sample could insert silence in the
            // middle of a stereo frame, permanently swapping L/R after contention or underflow.
            if let Ok(mut queue) = self.samples.try_lock() {
                let count = queue.len().min(256 * usize::from(self.channels));
                self.block.extend(queue.drain(..count));
            }
            if self.block.is_empty() {
                self.block.resize(usize::from(self.channels), 0.0);
                self.stats
                    .underflow
                    .fetch_add(u64::from(self.channels), Ordering::Relaxed);
            }
        }
        let sample = self.block.pop_front();
        if let Some(value) = sample {
            self.stats.consumed.fetch_add(1, Ordering::Relaxed);
            if value.abs() > 1.0e-8 {
                self.stats.consumed_nonzero.fetch_add(1, Ordering::Relaxed);
            }
        }
        sample
    }
}

impl Source for LivePlayback {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl Decodable for LivePcm {
    type DecoderItem = f32;
    type Decoder = LivePlayback;

    fn decoder(&self) -> LivePlayback {
        LivePlayback {
            samples: self.samples.clone(),
            stats: self.stats.clone(),
            channels: self.channels,
            sample_rate: self.sample_rate,
            block: VecDeque::with_capacity(256 * usize::from(self.channels)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_pcm_is_continuous_and_keeps_the_newest_complete_frames() {
        let pcm = LivePcm::new(2, 48_000, 2).unwrap();
        let mut playback = pcm.decoder();
        assert_eq!(playback.next(), Some(0.0), "an empty live block is silence");
        assert_eq!(
            playback.next(),
            Some(0.0),
            "silence completes the stereo frame"
        );
        assert_eq!(playback.current_frame_len(), None);
        assert_eq!(playback.total_duration(), None);
        assert_eq!(pcm.push(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap(), 2);
        assert_eq!(
            [
                playback.next(),
                playback.next(),
                playback.next(),
                playback.next()
            ],
            [Some(3.0), Some(4.0), Some(5.0), Some(6.0)]
        );
        assert_eq!(playback.next(), Some(0.0));
        assert!(pcm.push(&[1.0]).is_err(), "partial frames are refused");
        assert!(pcm.push(&[f32::NAN, 0.0]).is_err());
    }

    #[test]
    fn live_pcm_underflow_cannot_swap_stereo_channels() {
        let pcm = LivePcm::new(2, 48_000, 2).unwrap();
        let mut playback = pcm.decoder();
        assert_eq!(playback.next(), Some(0.0));
        pcm.push(&[0.25, -0.5]).unwrap();
        assert_eq!(playback.next(), Some(0.0));
        assert_eq!(playback.next(), Some(0.25));
        assert_eq!(playback.next(), Some(-0.5));
    }
}
