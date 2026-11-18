//! Decoded 16-bit PCM as a Bevy audio source: THUG2's sound effects play through it (`thug2.rs`).
use bevy::audio::{Decodable, Source};
use bevy::prelude::*;
use std::sync::Arc;
use std::time::Duration;

/// Decoded interleaved 16-bit PCM, ready to hand to the mixer.
#[derive(Asset, TypePath, Clone)]
pub struct StreamPcm {
    pub(crate) samples: Arc<Vec<i16>>,
    pub(crate) channels: u16,
    pub(crate) sample_rate: u32,
    /// Frame interval to repeat inline (a looping clip repeats the whole of itself).
    pub(crate) loop_range: Option<(usize, usize)>,
}

impl StreamPcm {
    /// Interleaved 16-bit PCM; `looping` repeats the whole clip inline, forever.
    pub(crate) fn from_i16(samples: Arc<Vec<i16>>, channels: u16, sample_rate: u32, looping: bool) -> Self {
        let frames = if channels == 0 { 0 } else { samples.len() / usize::from(channels) };
        Self {
            samples,
            channels,
            sample_rate,
            loop_range: (looping && frames > 0).then_some((0, frames)),
        }
    }
}

/// Plays a [`StreamPcm`] once, converting to the float samples the mixer wants.
pub struct PcmPlayback {
    samples: Arc<Vec<i16>>,
    at: usize,
    channels: u16,
    sample_rate: u32,
    loop_range: Option<(usize, usize)>,
}

impl Iterator for PcmPlayback {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if let Some((start, end)) = self.loop_range {
            let channels = usize::from(self.channels);
            if self.at == end.saturating_mul(channels) {
                self.at = start.saturating_mul(channels);
            }
        }
        let s = *self.samples.get(self.at)?;
        self.at += 1;
        // i16::MIN has no positive counterpart, so dividing by 32768 keeps the full range inside
        // [-1, 1] instead of letting the one extreme sample clip a fraction above it.
        Some(f32::from(s) / 32768.0)
    }
}

impl Source for PcmPlayback {
    fn current_frame_len(&self) -> Option<usize> {
        self.loop_range
            .is_none()
            .then(|| self.samples.len().saturating_sub(self.at))
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        if self.loop_range.is_some() {
            return None;
        }
        let frames = if self.channels == 0 {
            0
        } else {
            self.samples.len() / usize::from(self.channels)
        };
        (self.sample_rate != 0)
            .then(|| Duration::from_secs_f64(frames as f64 / f64::from(self.sample_rate)))
    }
}

impl Decodable for StreamPcm {
    type DecoderItem = f32;
    type Decoder = PcmPlayback;

    fn decoder(&self) -> PcmPlayback {
        PcmPlayback {
            samples: self.samples.clone(),
            at: 0,
            channels: self.channels,
            sample_rate: self.sample_rate,
            loop_range: self.loop_range,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_converts_full_scale_without_exceeding_unity() {
        // i16::MIN / 32767 would be 1.000031, which clips on a mixer that trusts the range.
        let pcm = StreamPcm::from_i16(Arc::new(vec![i16::MIN, i16::MAX, 0]), 1, 48_000, false);
        let got: Vec<f32> = pcm.decoder().collect();
        assert_eq!(got.len(), 3);
        assert!(got.iter().all(|s| (-1.0..=1.0).contains(s)), "{got:?}");
        assert_eq!(got[0], -1.0);
        assert_eq!(got[2], 0.0);
    }

    #[test]
    fn duration_counts_frames_not_samples() {
        // Five channels at 48 kHz: 240,000 samples are one second, not five.
        let pcm = StreamPcm::from_i16(Arc::new(vec![0i16; 48_000 * 5]), 5, 48_000, false);
        assert_eq!(pcm.decoder().total_duration(), Some(Duration::from_secs(1)));
    }

    #[test]
    fn a_looping_clip_repeats_without_end() {
        let pcm = StreamPcm::from_i16(Arc::new(vec![100i16, 200, 300]), 1, 48_000, true);
        let mut playback = pcm.decoder();
        let got: Vec<i16> = (0..7).map(|_| (playback.next().unwrap() * 32768.0).round() as i16).collect();
        assert_eq!(got, [100, 200, 300, 100, 200, 300, 100]);
        assert_eq!(playback.current_frame_len(), None);
        assert_eq!(playback.total_duration(), None);
    }

    #[test]
    fn inline_loop_restarts_at_its_frame_not_at_zero() {
        let pcm = StreamPcm {
            samples: Arc::new(vec![100i16, 200, 300]),
            channels: 1,
            sample_rate: 48_000,
            loop_range: Some((1, 3)),
        };
        let mut playback = pcm.decoder();
        let got: Vec<i16> = (0..6).map(|_| (playback.next().unwrap() * 32768.0).round() as i16).collect();
        assert_eq!(got, [100, 200, 300, 200, 300, 200]);
    }
}
