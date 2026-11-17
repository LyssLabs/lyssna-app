//! The TTML Creator's own playback of a decoded song, for when Spotify
//! Soloist plays the music: Soloist cannot change speed, and its position
//! reports arrive late, so the creator plays the samples itself.

use crate::time_stretch::TimeStretch;
use librespot_playback::config::VolumeCtrl;
use librespot_playback::mixer::mappings::MappedCtrl;
use rodio::Source;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

const CHANNELS: usize = 2;
const SAMPLE_RATE: u32 = librespot_playback::SAMPLE_RATE;
const FULL_SCALE: f32 = 32768.0;
const CHUNK_FRAMES: usize = 2048;
// Starting or stopping mid-wave clicks; a short ramp removes it.
const EDGE_FRAMES: usize = SAMPLE_RATE as usize * 3 / 1000;

static GAIN: AtomicU32 = AtomicU32::new(0x3f80_0000);
static EPOCH: AtomicU64 = AtomicU64::new(0);

/// Each play or stop supersedes every play asked before it. Decoding takes a
/// while, so an older play could otherwise start after a newer stop.
pub(crate) fn supersede() -> u64 {
    EPOCH.fetch_add(1, Ordering::AcqRel) + 1
}

pub(crate) fn current(epoch: u64) -> bool {
    EPOCH.load(Ordering::Acquire) == epoch
}

/// Follows the volume slider through the same curve as Lyssna's own player.
pub(crate) fn set_volume(percent: u8) {
    store_volume(&GAIN, percent);
}

fn store_volume(gain: &AtomicU32, percent: u8) {
    let level = u32::from(percent.min(100)) * u32::from(u16::MAX) / 100;
    let mapped = VolumeCtrl::default().to_mapped(level as u16) as f32;
    gain.store(mapped.to_bits(), Ordering::Relaxed);
}

pub(crate) struct CreatorStream {
    pcm: Arc<Vec<i16>>,
    /// Next input frame, and the frame where input ends.
    next: usize,
    start: usize,
    end: usize,
    stretch: Option<TimeStretch>,
    pending: Vec<f32>,
    read: usize,
    drained: bool,
    /// Samples left to play: the span's length at this speed.
    remaining: usize,
    gain: &'static AtomicU32,
}

impl CreatorStream {
    pub(crate) fn new(pcm: Arc<Vec<i16>>, from_ms: f64, to_ms: Option<f64>, speed: f64) -> Self {
        Self::with_gain(pcm, from_ms, to_ms, speed, &GAIN)
    }

    fn with_gain(
        pcm: Arc<Vec<i16>>,
        from_ms: f64,
        to_ms: Option<f64>,
        speed: f64,
        gain: &'static AtomicU32,
    ) -> Self {
        let count = pcm.len() / CHANNELS;
        let frame =
            |ms: f64| ((ms.max(0.0) / 1000.0 * f64::from(SAMPLE_RATE)).round() as usize).min(count);
        let start = frame(from_ms);
        let end = to_ms.map_or(count, frame).max(start);
        let stretch = ((speed - 1.0).abs() > 0.001).then(|| {
            let mut stretch = TimeStretch::new();
            stretch.set_rate(speed);
            stretch
        });
        let remaining = ((end - start) as f64 / speed.clamp(0.25, 2.0)).round() as usize * CHANNELS;
        Self {
            pcm,
            remaining,
            next: start,
            start,
            end,
            stretch,
            pending: Vec::new(),
            read: 0,
            drained: false,
            gain,
        }
    }

    fn edge_gain(&self, frame: usize) -> f32 {
        let edge = EDGE_FRAMES.min((self.end - self.start) / 4).max(1);
        let from_start = frame - self.start;
        let to_end = self.end - 1 - frame;
        let ramp = |distance: usize| {
            let phase = std::f32::consts::PI * distance as f32 / edge as f32;
            0.5 - 0.5 * phase.cos()
        };
        if from_start < edge {
            ramp(from_start)
        } else if to_end < edge {
            ramp(to_end)
        } else {
            1.0
        }
    }

    fn refill(&mut self) {
        self.pending.clear();
        self.read = 0;
        while self.pending.is_empty() && !self.drained {
            if self.next >= self.end {
                // WSOLA holds back its last windows; silence flushes them out.
                match self.stretch.as_mut() {
                    Some(stretch) if self.next < self.end + CHUNK_FRAMES * 4 => {
                        self.pending = stretch.process(&[0.0; CHUNK_FRAMES * CHANNELS]);
                        self.next += CHUNK_FRAMES;
                        continue;
                    }
                    _ => {
                        self.drained = true;
                        break;
                    }
                }
            }
            let until = (self.next + CHUNK_FRAMES).min(self.end);
            let mut chunk = Vec::with_capacity((until - self.next) * CHANNELS);
            for frame in self.next..until {
                let edge = self.edge_gain(frame);
                for channel in 0..CHANNELS {
                    chunk.push(f32::from(self.pcm[frame * CHANNELS + channel]) / FULL_SCALE * edge);
                }
            }
            self.next = until;
            self.pending = match self.stretch.as_mut() {
                Some(stretch) => stretch.process(&chunk),
                None => chunk,
            };
        }
    }
}

impl Iterator for CreatorStream {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.remaining == 0 {
            return None;
        }
        if self.read >= self.pending.len() {
            self.refill();
        }
        let sample = *self.pending.get(self.read)?;
        self.read += 1;
        self.remaining -= 1;
        Some(sample * f32::from_bits(self.gain.load(Ordering::Relaxed)))
    }
}

impl Source for CreatorStream {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        CHANNELS as u16
    }
    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(seconds: usize) -> Arc<Vec<i16>> {
        let frames = SAMPLE_RATE as usize * seconds;
        Arc::new(
            (0..frames)
                .flat_map(|frame| {
                    let value =
                        ((frame as f32 * 440.0 * std::f32::consts::TAU / SAMPLE_RATE as f32).sin()
                            * 8000.0) as i16;
                    [value, value]
                })
                .collect(),
        )
    }

    static FULL: AtomicU32 = AtomicU32::new(0x3f80_0000);

    fn stream(seconds: usize, from_ms: f64, to_ms: Option<f64>, speed: f64) -> CreatorStream {
        CreatorStream::with_gain(song(seconds), from_ms, to_ms, speed, &FULL)
    }

    #[test]
    fn plays_from_the_asked_moment_to_the_asked_end() {
        let stream = stream(3, 1000.0, Some(1500.0), 1.0);
        let samples: Vec<f32> = stream.collect();
        assert_eq!(samples.len(), SAMPLE_RATE as usize / 2 * CHANNELS);
        assert_eq!(samples[0], 0.0);
        assert!(samples[samples.len() / 2].abs() <= 8000.0 / FULL_SCALE + 1e-6);
    }

    #[test]
    fn slowing_down_lengthens_the_song_by_the_speed() {
        let normal = SAMPLE_RATE as f64 * 2.0;
        for speed in [0.5, 0.75] {
            let frames = stream(3, 0.0, Some(2000.0), speed).count() / CHANNELS;
            let expected = normal / speed;
            assert!(
                (frames as f64 - expected).abs() < expected * 0.05,
                "{speed}: {frames} frames, expected about {expected}"
            );
        }
    }

    #[test]
    fn follows_the_volume_slider() {
        static SLIDER: AtomicU32 = AtomicU32::new(0);
        let level = || f32::from_bits(SLIDER.load(Ordering::Relaxed));
        store_volume(&SLIDER, 0);
        assert!(
            CreatorStream::with_gain(song(1), 100.0, Some(300.0), 1.0, &SLIDER)
                .all(|sample| sample == 0.0)
        );
        store_volume(&SLIDER, 100);
        assert_eq!(level(), 1.0);
        store_volume(&SLIDER, 50);
        assert!(level() > 0.0 && level() < 0.2);
    }

    #[test]
    fn ends_at_the_song_end_without_an_asked_end() {
        let frames = stream(2, 1500.0, None, 1.0).count() / CHANNELS;
        assert_eq!(frames, SAMPLE_RATE as usize / 2);
    }
}
