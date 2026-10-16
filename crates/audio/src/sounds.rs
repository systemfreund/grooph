//! Sound choice for the two voices of playback.
//!
//! The **rhythm** voice plays the written notes, accents and ghost notes; the
//! **metronome** voice plays the downbeat, the primary beats and the count-in.
//! Each voice has its own [`RhythmSound`] / [`MetronomeSound`], so the two
//! stay apart by ear.
//!
//! `Click` is the configurable click from [`AudioSettings`] (waveform, base
//! frequency, decay, noise), pitched per sound type as before. Every other
//! sound plays short WAV samples embedded in the binary (see [`SampleId`]),
//! one per sound type where the sound has several, with the volume of the
//! mixer.

use crate::schedule::SoundType;
use crate::wav::{self, SampleData};
use crate::{AudioSettings, Waveform};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

/// Sound of the written rhythm.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RhythmSound {
    /// The click from the sound settings.
    #[default]
    Click,
}

impl RhythmSound {
    pub const ALL: [RhythmSound; 1] = [RhythmSound::Click];

    pub fn label(self) -> &'static str {
        match self {
            RhythmSound::Click => "Click",
        }
    }

    fn sample(self, _sound: SoundType) -> Option<SampleId> {
        match self {
            RhythmSound::Click => None,
        }
    }
}

/// Sound of the metronome (downbeat, primary beats, count-in).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetronomeSound {
    /// Claves: a higher hit on the downbeat, a lower one on the other
    /// primary beats and the count-in.
    #[default]
    Claves,
    /// The click from the sound settings.
    Click,
}

impl MetronomeSound {
    pub const ALL: [MetronomeSound; 2] = [MetronomeSound::Claves, MetronomeSound::Click];

    pub fn label(self) -> &'static str {
        match self {
            MetronomeSound::Claves => "Claves",
            MetronomeSound::Click => "Click",
        }
    }

    fn sample(self, sound: SoundType) -> Option<SampleId> {
        match self {
            MetronomeSound::Claves if sound == SoundType::Downbeat => Some(SampleId::CLAVES_HIGH),
            MetronomeSound::Claves => Some(SampleId::CLAVES_LOW),
            MetronomeSound::Click => None,
        }
    }
}

/// An embedded WAV file.
struct SampleFile {
    name: &'static str,
    bytes: &'static [u8],
}

/// Every embedded sample; [`SampleId`] indexes into it.
const SAMPLES: &[SampleFile] = &[
    SampleFile { name: "claves3.wav", bytes: include_bytes!("../samples/claves3.wav") },
    SampleFile { name: "claves4.wav", bytes: include_bytes!("../samples/claves4.wav") },
];

/// An embedded sample, decoded on first use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SampleId(usize);

impl SampleId {
    const CLAVES_HIGH: SampleId = SampleId(0);
    const CLAVES_LOW: SampleId = SampleId(1);

    /// The decoded sample; an unreadable file becomes silence (logged once).
    fn data(self) -> Arc<SampleData> {
        static DECODED: OnceLock<Vec<Arc<SampleData>>> = OnceLock::new();
        let all = DECODED.get_or_init(|| {
            SAMPLES
                .iter()
                .map(|file| {
                    Arc::new(wav::decode(file.bytes).unwrap_or_else(|err| {
                        log::error!("Cannot decode sample {}: {err:?}", file.name);
                        SampleData { rate: 44_100, frames: Vec::new() }
                    }))
                })
                .collect()
        });
        all[self.0].clone()
    }
}

/// One decaying oscillator of the click.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tone {
    pub(crate) hz: f32,
    pub(crate) waveform: Waveform,
    /// Seconds until the linear decay reaches silence.
    pub(crate) decay: f32,
    pub(crate) level: f32,
}

/// High-passed white noise burst of the click.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Noise {
    pub(crate) hpf_hz: f32,
    pub(crate) decay: f32,
    pub(crate) level: f32,
}

/// What the voice mixer plays for one hit.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Patch {
    Click {
        tone: Tone,
        noise: Option<Noise>,
        gain: f32,
    },
    /// `rate` scales the playback speed (and pitch): 1.0 is as recorded.
    Sample {
        data: Arc<SampleData>,
        rate: f32,
        gain: f32,
    },
}

impl SoundType {
    fn is_rhythm(self) -> bool {
        matches!(self, SoundType::Beat | SoundType::AccentedBeat | SoundType::Ghost)
    }

    /// The patch for this hit: the voice's chosen sound in this sound type's
    /// variant.
    pub(crate) fn patch(self, settings: &AudioSettings) -> Patch {
        let sample = if self.is_rhythm() {
            settings.rhythm_sound.sample(self)
        } else {
            settings.metronome_sound.sample(self)
        };
        let gain = self.profile(settings).gain;
        match sample {
            Some(id) => Patch::Sample { data: id.data(), rate: 1.0, gain },
            None => click(self, settings),
        }
    }
}

/// The configurable click from [`AudioSettings`], pitched per sound type.
fn click(sound: SoundType, settings: &AudioSettings) -> Patch {
    let profile = sound.profile(settings);
    let hz = (settings.base_frequency * profile.freq_mult).max(1.0);
    let mix = settings.noise_mix.clamp(0.0, 1.0);
    Patch::Click {
        tone: Tone { hz, waveform: settings.waveform, decay: settings.decay, level: 1.0 - mix },
        noise: (mix > 0.0001).then_some(Noise {
            hpf_hz: settings.noise_hpf_hz,
            decay: settings.noise_decay,
            level: mix,
        }),
        gain: profile.gain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_patch_matches_the_sound_settings() {
        let settings =
            AudioSettings { metronome_sound: MetronomeSound::Click, ..AudioSettings::default() };
        assert!(matches!(SoundType::Downbeat.patch(&settings), Patch::Click { .. }));
        let Patch::Click { tone, noise, gain } = SoundType::Beat.patch(&settings) else {
            panic!("click expected");
        };
        assert_eq!(tone.hz, settings.base_frequency * 1.5);
        assert_eq!(tone.waveform, settings.waveform);
        assert_eq!(tone.decay, settings.decay);
        assert_eq!(noise.unwrap().level, settings.noise_mix);
        assert_eq!(gain, settings.beat);
    }

    #[test]
    fn claves_play_the_high_hit_on_the_downbeat_and_the_low_one_elsewhere() {
        let settings =
            AudioSettings { metronome_sound: MetronomeSound::Claves, ..AudioSettings::default() };
        let sample = |sound: SoundType| match sound.patch(&settings) {
            Patch::Sample { data, gain, .. } => (data, gain),
            other => panic!("sample expected, got {other:?}"),
        };
        let (high, gain) = sample(SoundType::Downbeat);
        assert_eq!(high, SampleId::CLAVES_HIGH.data());
        assert_eq!(gain, settings.downbeat);
        let (low, gain) = sample(SoundType::PrimaryBeat);
        assert_eq!(low, SampleId::CLAVES_LOW.data());
        assert_eq!(gain, settings.primary);
        assert_eq!(sample(SoundType::CountIn).0, low);
        // The rhythm keeps its own sound.
        assert!(matches!(SoundType::Beat.patch(&settings), Patch::Click { .. }));
    }

    #[test]
    fn a_bar_of_four_four_plays_one_high_and_three_low_claves() {
        use crate::schedule::Schedule;
        use grooph_measure::tempo::ScoreTiming;
        use grooph_measure::{Measure, Score, TimeSignature};

        let score = Score { measures: vec![Measure::new(TimeSignature::FOUR_FOUR)] };
        let timing = ScoreTiming::from_score(&score, 120);
        let mut sounds = Vec::new();
        Schedule::build(&score, &timing, None).collect_in_range(
            0,
            timing.total_loop_ticks() as f64,
            &mut sounds,
        );
        let settings = AudioSettings::default();
        let samples: Vec<_> = sounds
            .into_iter()
            .filter_map(|sound| match sound.patch(&settings) {
                Patch::Sample { data, .. } => Some(data),
                Patch::Click { .. } => None,
            })
            .collect();
        let (high, low) = (SampleId::CLAVES_HIGH.data(), SampleId::CLAVES_LOW.data());
        assert_eq!(samples, vec![high, low.clone(), low.clone(), low]);
    }

    #[test]
    fn embedded_samples_decode() {
        for (i, file) in SAMPLES.iter().enumerate() {
            let data = wav::decode(file.bytes).unwrap_or_else(|e| panic!("{}: {e:?}", file.name));
            assert!(!data.frames.is_empty(), "{} is empty", file.name);
            assert_eq!(*SampleId(i).data(), data);
        }
    }
}
