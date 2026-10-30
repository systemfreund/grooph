//! Voice lifecycle and synthesis for the metronome.
//!
//! A [`VoiceMixer`] owns the currently sounding voices. A new voice is spawned
//! via [`VoiceMixer::trigger`] with a [`Patch`] when a scheduled sound is
//! reached: a click decays along its tone and noise envelopes, a sample plays
//! to its end. [`VoiceMixer::next_sample`]
//! advances every voice by one audio sample and returns the soft-clipped mix.
//!
//! This module deliberately knows nothing about timing, the score, or
//! the playback cursor — those concerns live in `tick_source` and the
//! orchestrator in `lib.rs`.

use crate::sounds::{Noise, Patch, Tone};
use crate::wav::SampleData;
use rodio::Source;
use rodio::source::SignalGenerator;
use rodio::source::noise::WhiteUniform;
use std::sync::Arc;

type Signal = Box<dyn Iterator<Item = f32> + Send>;

enum ActiveVoice {
    /// The synthesized click: one tone plus an optional noise burst, each
    /// with its own linear decay.
    Click {
        tone_signal: Signal,
        tone: Tone,
        noise: Option<(Signal, Noise)>,
        // Age of this voice in seconds to drive the envelopes.
        age: f32,
        gain: f32,
    },
    /// An embedded sample, read with linear interpolation at `step` source
    /// frames per output sample.
    Sample { data: Arc<SampleData>, pos: f64, step: f64, gain: f32 },
}

impl ActiveVoice {
    /// Next sample of this voice, or `None` once it has finished.
    fn next(&mut self, dt: f32) -> Option<f32> {
        const ATTACK: f32 = 0.0005; // ~1 ms
        match self {
            ActiveVoice::Click { tone_signal, tone, noise, age, gain } => {
                *age += dt;
                let p = *age;
                // The voice stays active until both decays are over.
                let tone_alive = p <= tone.decay;
                let noise_alive = noise.as_ref().is_some_and(|(_, n)| p <= n.decay);
                if !tone_alive && !noise_alive {
                    return None;
                }
                let env = |decay: f32| {
                    let d = if decay > 0.0 { 1.0 - p / decay } else { 0.0 };
                    ((p / ATTACK).min(1.0) * d).clamp(0.0, 1.0)
                };
                let tone_contrib = if tone_alive {
                    tone_signal.next().unwrap_or(0.0) * tone.level * env(tone.decay)
                } else {
                    0.0
                };
                let noise_contrib = match noise {
                    Some((signal, n)) if noise_alive => {
                        signal.next().unwrap_or(0.0) * n.level * env(n.decay)
                    }
                    _ => 0.0,
                };
                Some((tone_contrib + noise_contrib) * *gain)
            }
            ActiveVoice::Sample { data, pos, step, gain } => {
                let i = *pos as usize;
                let a = *data.frames.get(i)?;
                let b = data.frames.get(i + 1).copied().unwrap_or(0.0);
                let frac = (*pos - i as f64) as f32;
                *pos += *step;
                Some((a + (b - a) * frac) * *gain)
            }
        }
    }
}

pub(crate) struct VoiceMixer {
    voices: Vec<ActiveVoice>,
    sample_rate: u32,
}

impl VoiceMixer {
    pub(crate) const MAX_VOICES: usize = 16;

    pub(crate) fn new(sample_rate: u32) -> Self {
        Self { voices: Vec::with_capacity(Self::MAX_VOICES), sample_rate }
    }

    /// `true` when no voice is currently sounding.
    #[cfg(test)]
    pub(crate) fn is_silent(&self) -> bool {
        self.voices.is_empty()
    }

    /// Spawn a voice playing `patch`. If the polyphony cap is reached the
    /// oldest voice is dropped first.
    pub(crate) fn trigger(&mut self, patch: Patch) {
        if self.voices.len() >= Self::MAX_VOICES {
            // drop the oldest to keep CPU bounded
            self.voices.remove(0);
        }

        let voice = match patch {
            Patch::Click { tone, noise, gain } => {
                let hz = tone.hz.max(1.0);
                let tone_signal: Signal =
                    Box::new(SignalGenerator::new(self.sample_rate, hz, tone.waveform.to_rodio()));
                let noise = noise.map(|n| {
                    let signal: Signal =
                        Box::new(WhiteUniform::new(self.sample_rate).high_pass(n.hpf_hz as u32));
                    (signal, n)
                });
                ActiveVoice::Click { tone_signal, tone, noise, age: 0.0, gain }
            }
            Patch::Sample { data, rate, gain } => {
                let step = rate as f64 * data.rate as f64 / self.sample_rate as f64;
                ActiveVoice::Sample { data, pos: 0.0, step, gain }
            }
        };
        self.voices.push(voice);
    }

    /// Advance every active voice by one audio sample, mix them, and return
    /// the soft-clipped sum. Finished voices are removed during this pass.
    pub(crate) fn next_sample(&mut self) -> f32 {
        if self.voices.is_empty() {
            return 0.0;
        }
        let dt = 1.0 / (self.sample_rate as f32);
        let mut mixed: f32 = 0.0;
        let mut i = 0;
        while i < self.voices.len() {
            match self.voices[i].next(dt) {
                Some(sample) => {
                    mixed += sample * 0.6; // master gain 0.6 to leave headroom
                    i += 1;
                }
                None => {
                    self.voices.remove(i);
                }
            }
        }
        // Soft clip to avoid hard clipping when multiple voices overlap
        mixed.tanh()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::SoundType;
    use crate::{AudioSettings, Sound};

    fn beat() -> Patch {
        let settings = AudioSettings { rhythm_sound: Sound::Click, ..AudioSettings::default() };
        SoundType::Beat.patch(&settings)
    }

    fn sample(frames: Vec<f32>, rate: u32, playback_rate: f32) -> Patch {
        Patch::Sample {
            data: Arc::new(SampleData { rate, frames }),
            rate: playback_rate,
            gain: 1.0,
        }
    }

    #[test]
    fn new_mixer_is_silent() {
        let mixer = VoiceMixer::new(48000);
        assert!(mixer.is_silent());
    }

    #[test]
    fn trigger_adds_voice() {
        let mut mixer = VoiceMixer::new(48000);
        mixer.trigger(beat());
        assert!(!mixer.is_silent());
    }

    #[test]
    fn voice_dies_after_decay_elapses() {
        let mut mixer = VoiceMixer::new(48000);
        // Default decays: tone=0.042s, noise=0.017s → tone dominates; 0.042 * 48000 ≈ 2016 samples.
        mixer.trigger(beat());
        for _ in 0..2200 {
            let _ = mixer.next_sample();
        }
        assert!(mixer.is_silent());
    }

    #[test]
    fn polyphony_is_capped_to_max_voices() {
        let mut mixer = VoiceMixer::new(48000);
        for _ in 0..(VoiceMixer::MAX_VOICES + 3) {
            mixer.trigger(beat());
        }
        assert_eq!(mixer.voices.len(), VoiceMixer::MAX_VOICES);
    }

    #[test]
    fn next_sample_when_silent_returns_zero() {
        let mut mixer = VoiceMixer::new(48000);
        assert_eq!(mixer.next_sample(), 0.0);
    }

    #[test]
    fn noise_voice_outlasts_tone_if_decay_is_longer() {
        let mut mixer = VoiceMixer::new(48000);
        let settings = AudioSettings {
            decay: 0.005,      // tone dies quickly
            noise_decay: 0.05, // noise lives longer
            noise_mix: 0.5,
            ..AudioSettings::default()
        };
        mixer.trigger(SoundType::Beat.patch(&settings));
        // After tone decay but before noise decay, mixer must still be active.
        for _ in 0..(48000 / 100) {
            // 10 ms
            let _ = mixer.next_sample();
        }
        assert!(!mixer.is_silent());
    }

    #[test]
    fn sample_plays_to_its_end_and_is_resampled() {
        // A 4-frame ramp at half the output rate lasts 8 output samples.
        let mut mixer = VoiceMixer::new(48000);
        mixer.trigger(sample(vec![0.0, 0.2, 0.4, 0.6], 24000, 1.0));
        let out: Vec<f32> = (0..8).map(|_| mixer.next_sample()).collect();
        // The finished voice is dropped on the next sample.
        assert_eq!(mixer.next_sample(), 0.0);
        assert!(mixer.is_silent());
        // Interpolated halfway between the first two frames.
        assert!((out[1] - (0.1f32 * 0.6).tanh()).abs() < 1e-6, "{out:?}");
        assert!(out.windows(2).take(6).all(|w| w[1] >= w[0]), "{out:?}");
    }

    #[test]
    fn higher_playback_rate_shortens_the_sample() {
        let mut mixer = VoiceMixer::new(48000);
        mixer.trigger(sample(vec![0.5; 100], 48000, 2.0));
        let samples =
            std::iter::from_fn(|| (!mixer.is_silent()).then(|| mixer.next_sample())).count();
        // 50 output samples plus the one that drops the finished voice.
        assert_eq!(samples, 51);
    }
}
