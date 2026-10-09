//! The sound's DSP: a sample player Bevy's audio runs through [`Voice`] (an audio asset),
//! with the three things the FMOD events ask for that `bevy_audio` does not do by
//! itself and that must change while the sound plays: its **gain** (fades, rolloff),
//! its **pan** (the 3D position, scaled by the event's pan level) and a **low-pass**
//! (the events' occlusion envelopes).
//!
//! `bevy_audio` 0.19 gives a sink volume and a speed and, for 3D, a spatial sink that pans
//! and attenuates by itself; there is no filter and the spatial sink's pan cannot be
//! scaled. So the sounds are not spatial sinks: each [`Voice`] decodes its MP3 and
//! applies a pan law and a one-pole low-pass in the sample loop, reading the values from
//! a [`Shared`] block the game writes every frame (atomics, no locks). The pitch is the
//! sink's speed. Looping is done here (the decoder is rebuilt at its end) because
//! rodio's own `repeat_infinite` buffers the processed samples and would freeze the DSP
//! of the first pass.

use bevy::audio::{AudioSource, ChannelCount, Decodable, SampleRate, Source};
use bevy::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// The cutoff that means "no filter".
pub const OPEN: f32 = 0.0;

/// What the game tells a playing sound, f32s in atomics.
#[derive(Debug)]
pub struct Shared {
    gain: AtomicU32,
    pan: AtomicU32,
    cutoff: AtomicU32,
}

impl Shared {
    pub fn new(gain: f32, pan: f32, cutoff_hz: f32) -> Arc<Self> {
        Arc::new(Self {
            gain: AtomicU32::new(gain.to_bits()),
            pan: AtomicU32::new(pan.to_bits()),
            cutoff: AtomicU32::new(cutoff_hz.to_bits()),
        })
    }

    pub fn set_gain(&self, g: f32) {
        self.gain.store(g.to_bits(), Ordering::Relaxed);
    }

    pub fn set_pan(&self, p: f32) {
        self.pan.store(p.to_bits(), Ordering::Relaxed);
    }

    /// 0 (`OPEN`) turns the filter off.
    pub fn set_cutoff(&self, hz: f32) {
        self.cutoff.store(hz.to_bits(), Ordering::Relaxed);
    }

    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }

    pub fn pan(&self) -> f32 {
        f32::from_bits(self.pan.load(Ordering::Relaxed))
    }

    pub fn cutoff(&self) -> f32 {
        f32::from_bits(self.cutoff.load(Ordering::Relaxed))
    }
}

/// The low-pass cutoff (Hz) an occlusion of 0..1 stands for: 20 kHz clear, ~5 kHz at
/// 0.4 (how far behind the listener a sound is muffled) and 600 Hz fully occluded.
pub fn occlusion_cutoff(occlusion: f32) -> f32 {
    if occlusion <= 0.01 {
        OPEN
    } else {
        20_000.0 * 0.03f32.powf(occlusion.clamp(0.0, 1.0))
    }
}

/// Equal-power left and right gains for a pan of −1 (left) … 1 (right); 1 each at the
/// centre, so a centred sound is as loud as it was before panning existed.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let theta = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (
        theta.cos() * std::f32::consts::SQRT_2,
        theta.sin() * std::f32::consts::SQRT_2,
    )
}

/// A sound to play: MP3 bytes, the block that steers it, and whether it loops.
#[derive(Asset, TypePath, Clone)]
pub struct Voice {
    pub bytes: Arc<[u8]>,
    pub shared: Arc<Shared>,
    pub looping: bool,
}

type Inner = <AudioSource as Decodable>::Decoder;

fn decode(bytes: &Arc<[u8]>) -> Inner {
    AudioSource {
        bytes: bytes.clone(),
    }
    .decoder()
}

impl Decodable for Voice {
    type Decoder = DspSource;

    fn decoder(&self) -> DspSource {
        DspSource::new(self.bytes.clone(), self.shared.clone(), self.looping)
    }
}

/// The decoder with the gain, pan and low-pass applied; always stereo out.
pub struct DspSource {
    inner: Inner,
    bytes: Arc<[u8]>,
    shared: Arc<Shared>,
    looping: bool,
    in_channels: u16,
    rate: u32,
    /// The right sample of the frame whose left one was just returned.
    queued: Option<f32>,
    tick: u32,
    gain: f32,
    pan: f32,
    target_gain: f32,
    target_pan: f32,
    /// One-pole low-pass state per channel and its coefficient (0 = bypass).
    lp: [f32; 2],
    alpha: f32,
    /// Per-sample smoothing of gain and pan (about 10 ms).
    smooth: f32,
}

impl DspSource {
    pub fn new(bytes: Arc<[u8]>, shared: Arc<Shared>, looping: bool) -> Self {
        let inner = decode(&bytes);
        let in_channels = inner.channels().get();
        let rate = inner.sample_rate().get();
        let target_gain = shared.gain();
        let target_pan = shared.pan();
        let mut s = Self {
            inner,
            bytes,
            shared,
            looping,
            in_channels,
            rate,
            queued: None,
            tick: 0,
            gain: target_gain,
            pan: target_pan,
            target_gain,
            target_pan,
            lp: [0.0; 2],
            alpha: 0.0,
            smooth: 1.0 - (-1.0 / (0.010 * rate as f32)).exp(),
        };
        s.refresh();
        s
    }

    fn refresh(&mut self) {
        self.target_gain = self.shared.gain();
        self.target_pan = self.shared.pan();
        let fc = self.shared.cutoff();
        self.alpha = if fc <= 0.0 || fc >= 0.45 * self.rate as f32 {
            0.0
        } else {
            1.0 - (-std::f32::consts::TAU * fc / self.rate as f32).exp()
        };
    }

    fn pull(&mut self) -> Option<f32> {
        if let Some(s) = self.inner.next() {
            return Some(s);
        }
        if self.looping {
            self.inner = decode(&self.bytes);
            return self.inner.next();
        }
        None
    }
}

impl Iterator for DspSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if let Some(r) = self.queued.take() {
            return Some(r);
        }
        let (a, b) = if self.in_channels == 1 {
            let m = self.pull()?;
            (m, m)
        } else {
            let l = self.pull()?;
            let r = self.pull().unwrap_or(l);
            (l, r)
        };
        self.tick = self.tick.wrapping_add(1);
        if self.tick & 31 == 0 {
            self.refresh();
        }
        self.gain += (self.target_gain - self.gain) * self.smooth;
        self.pan += (self.target_pan - self.pan) * self.smooth;
        let (gl, gr) = pan_gains(self.pan);
        let mut l = a * gl * self.gain;
        let mut r = b * gr * self.gain;
        if self.alpha > 0.0 {
            self.lp[0] += self.alpha * (l - self.lp[0]);
            self.lp[1] += self.alpha * (r - self.lp[1]);
            l = self.lp[0];
            r = self.lp[1];
        } else {
            self.lp = [l, r];
        }
        self.queued = Some(r);
        Some(l)
    }
}

impl Source for DspSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(2).unwrap()
    }

    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        if self.looping {
            None
        } else {
            self.inner.total_duration()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_law() {
        let (l, r) = pan_gains(0.0);
        assert!((l - 1.0).abs() < 1e-5 && (r - 1.0).abs() < 1e-5);
        let (l, r) = pan_gains(-1.0);
        assert!(l > 1.4 && r < 1e-5);
        let (l, r) = pan_gains(0.5);
        assert!(r > l);
        // equal power
        assert!(((l * l + r * r) - 2.0).abs() < 1e-4);
    }

    #[test]
    fn occlusion_cutoffs() {
        assert_eq!(occlusion_cutoff(0.0), OPEN);
        assert!(occlusion_cutoff(0.4) > 4_000.0 && occlusion_cutoff(0.4) < 6_000.0);
        assert!(occlusion_cutoff(1.0) < 700.0);
        assert!(occlusion_cutoff(0.8) > occlusion_cutoff(1.0));
    }

    /// A tiny constant-rate MP3-free check of the filter and pan on a stand-in source is
    /// covered by the game test; here the shared block round-trips.
    #[test]
    fn shared_roundtrip() {
        let s = Shared::new(0.5, -0.25, 800.0);
        assert_eq!((s.gain(), s.pan(), s.cutoff()), (0.5, -0.25, 800.0));
        s.set_gain(1.0);
        s.set_pan(0.75);
        s.set_cutoff(OPEN);
        assert_eq!((s.gain(), s.pan(), s.cutoff()), (1.0, 0.75, 0.0));
    }
}
