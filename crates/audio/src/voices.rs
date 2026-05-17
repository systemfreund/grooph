//! Voice lifecycle and synthesis for the metronome.
//!
//! A [`VoiceMixer`] owns the currently sounding voices. A new voice is spawned
//! via [`VoiceMixer::trigger`] when a scheduled sound is reached, and decays
//! according to its `tone_decay` / `noise_decay` envelopes. [`VoiceMixer::next_sample`]
//! advances every voice by one audio sample and returns the soft-clipped mix.
//!
//! This module deliberately knows nothing about timing, the score, or
//! the playback cursor — those concerns live in `tick_source` and the
//! orchestrator in `lib.rs`.

use crate::AudioSettings;
use crate::schedule::SoundType;
use rodio::Source;
use rodio::source::SignalGenerator;
use rodio::source::noise::WhiteUniform;

struct ActiveVoice {
    // Delegated signal generators (rodio). Basis + optional Noise getrennt.
    base_signal: Box<dyn Iterator<Item = f32> + Send>,
    noise_signal: Option<Box<dyn Iterator<Item = f32> + Send>>,
    // Age of this voice in seconds to drive our envelope/decay bookkeeping
    age: f32,
    gain: f32,
    tone_decay: f32,
    noise_decay: f32,
}

pub(crate) struct VoiceMixer {
    voices: Vec<ActiveVoice>,
    sample_rate: u32,
}

impl VoiceMixer {
    pub(crate) const MAX_VOICES: usize = 8;

    pub(crate) fn new(sample_rate: u32) -> Self {
        Self { voices: Vec::with_capacity(Self::MAX_VOICES), sample_rate }
    }

    /// `true` when no voice is currently sounding. Used by the orchestrator to
    /// decide when it is safe to pause the audio sink without a click.
    pub(crate) fn is_silent(&self) -> bool { self.voices.is_empty() }

    /// Spawn a voice for `sound` using `settings` for frequency/gain/decay.
    /// If the polyphony cap is reached the oldest voice is dropped first.
    pub(crate) fn trigger(&mut self, sound: SoundType, settings: &AudioSettings) {
        if self.voices.len() >= Self::MAX_VOICES {
            // drop the oldest to keep CPU bounded
            self.voices.remove(0);
        }

        let profile = sound.profile(settings);
        let hz = (settings.base_frequency * profile.freq_mult).max(1.0);
        let base = SignalGenerator::new(self.sample_rate, hz, settings.waveform.to_rodio());

        let noise_signal: Option<Box<dyn Iterator<Item = f32> + Send>> =
            if settings.noise_mix > 0.0001 {
                let cutoff_hz_u32 = settings.noise_hpf_hz as u32;
                let noise = WhiteUniform::new(self.sample_rate).high_pass(cutoff_hz_u32);
                Some(Box::new(noise))
            } else {
                None
            };

        self.voices.push(ActiveVoice {
            base_signal: Box::new(base),
            noise_signal,
            age: 0.0,
            gain: profile.gain,
            tone_decay: settings.decay,
            noise_decay: settings.noise_decay,
        });
    }

    /// Advance every active voice by one audio sample, mix them, and return
    /// the soft-clipped sum. Voices whose tone- *and* noise-envelope have
    /// elapsed are removed during this pass.
    ///
    /// `noise_mix` is read as a parameter (not from a stored `AudioSettings`)
    /// so the mixer never needs the full settings object during synthesis.
    pub(crate) fn next_sample(&mut self, noise_mix: f32) -> f32 {
        let dt = 1.0 / (self.sample_rate as f32);
        const ATTACK: f32 = 0.0005; // ~1 ms

        if self.voices.is_empty() {
            // If not playing and no active voices, stay silent
            return 0.0;
        }

        // Advance phases and compute sum
        let mut mixed: f32 = 0.0;
        let mut i = 0;
        while i < self.voices.len() {
            let voice = &mut self.voices[i];
            voice.age += dt;
            let p = voice.age;
            // Voice bleibt aktiv bis beide Decays vorbei sind (Basis und ggf. Noise)
            let tone_alive = p <= voice.tone_decay;
            let noise_alive = p <= voice.noise_decay && voice.noise_signal.is_some();
            if !tone_alive && !noise_alive {
                self.voices.remove(i);
                continue;
            }

            let env_attack = (p / ATTACK).min(1.0);
            let env_tone_decay =
                if voice.tone_decay > 0.0 { 1.0 - (p / voice.tone_decay) } else { 0.0 };
            let env_noise_decay =
                if voice.noise_decay > 0.0 { 1.0 - (p / voice.noise_decay) } else { 0.0 };
            let env_tone = (env_attack * env_tone_decay).clamp(0.0, 1.0);
            let env_noise = (env_attack * env_noise_decay).clamp(0.0, 1.0);

            let base_sample =
                if tone_alive { voice.base_signal.next().unwrap_or(0.0) } else { 0.0 };

            let noise_sample = if noise_alive {
                if let Some(noise) = &mut voice.noise_signal {
                    noise.next().unwrap_or(0.0)
                } else {
                    0.0
                }
            } else {
                0.0
            };

            let tone_contrib = base_sample * (1.0 - noise_mix) * env_tone;
            let noise_contrib = noise_sample * noise_mix * env_noise;

            mixed += (tone_contrib + noise_contrib) * 0.6 * voice.gain; // master gain 0.6 to leave headroom

            i += 1;
        }

        // Soft clip to avoid hard clipping when multiple voices overlap
        mixed.tanh()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_mixer_is_silent() {
        let mixer = VoiceMixer::new(48000);
        assert!(mixer.is_silent());
    }

    #[test]
    fn trigger_adds_voice() {
        let mut mixer = VoiceMixer::new(48000);
        mixer.trigger(SoundType::Beat, &AudioSettings::default());
        assert!(!mixer.is_silent());
    }

    #[test]
    fn voice_dies_after_decay_elapses() {
        let mut mixer = VoiceMixer::new(48000);
        let settings = AudioSettings::default();
        // Default decays: tone=0.042s, noise=0.017s → tone dominates; 0.042 * 48000 ≈ 2016 samples.
        mixer.trigger(SoundType::Beat, &settings);
        for _ in 0..2200 {
            let _ = mixer.next_sample(settings.noise_mix);
        }
        assert!(mixer.is_silent());
    }

    #[test]
    fn polyphony_is_capped_to_max_voices() {
        let mut mixer = VoiceMixer::new(48000);
        let settings = AudioSettings::default();
        for _ in 0..(VoiceMixer::MAX_VOICES + 3) {
            mixer.trigger(SoundType::Beat, &settings);
        }
        assert_eq!(mixer.voices.len(), VoiceMixer::MAX_VOICES);
    }

    #[test]
    fn next_sample_when_silent_returns_zero() {
        let mut mixer = VoiceMixer::new(48000);
        assert_eq!(mixer.next_sample(0.0), 0.0);
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
        mixer.trigger(SoundType::Beat, &settings);
        // After tone decay but before noise decay, mixer must still be active.
        for _ in 0..(48000 / 100) {
            // 10 ms
            let _ = mixer.next_sample(settings.noise_mix);
        }
        assert!(!mixer.is_silent());
    }
}
