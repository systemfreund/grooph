//! Sound choice for the voices of playback.
//!
//! The **rhythm** voice plays the written notes, accents and ghost notes; the
//! **metronome** voice plays the downbeat, the primary beats and the count-in;
//! the **counter rhythm** is the metronome's second layer. Each voice picks
//! any [`Sound`]. When the rhythm or the counter rhythm plays the metronome's
//! sound, its samples play higher, so the voices stay apart by ear.
//!
//! Every hit has a [`Level`] (strong, normal or soft), and a sound has one
//! recording per level ([`SampleSet`]); a sound without a soft recording
//! plays its normal one there, quieter through the mixer. `Click` is the
//! configurable click from [`AudioSettings`] (waveform, base frequency,
//! decay, noise), pitched per sound type. Every other sound plays short WAV
//! samples embedded in the binary (see [`SampleId`]) with the volume of the
//! mixer.

use crate::schedule::SoundType;
use crate::wav::{self, SampleData};
use crate::{AudioSettings, Waveform};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

/// What a voice sounds like. Every voice offers every sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sound {
    /// The click from the sound settings.
    Click,
    /// Claves: a higher and a lower hit.
    Claves,
    /// Snare drum, recorded at three dynamics.
    Snare,
    /// Xylophone: a higher and a lower note.
    Xylophone,
}

impl Sound {
    pub const ALL: [Sound; 4] = [Sound::Click, Sound::Claves, Sound::Snare, Sound::Xylophone];
    /// The rhythm's sound out of the box.
    pub const RHYTHM_DEFAULT: Sound = Sound::Snare;
    /// The metronome's sound out of the box.
    pub const METRONOME_DEFAULT: Sound = Sound::Claves;
    /// The counter rhythm's sound out of the box, apart from the metronome's.
    pub const COUNTER_DEFAULT: Sound = Sound::Xylophone;

    pub fn label(self) -> &'static str {
        match self {
            Sound::Click => "Click",
            Sound::Claves => "Claves",
            Sound::Snare => "Snare",
            Sound::Xylophone => "Xylophone",
        }
    }

    /// The recordings of this sound; `None` for the synthesized click.
    fn samples(self) -> Option<SampleSet> {
        Some(match self {
            Sound::Click => return None,
            Sound::Claves => SampleSet {
                strong: SampleId::CLAVES_HIGH,
                normal: SampleId::CLAVES_LOW,
                soft: None,
            },
            Sound::Snare => SampleSet {
                strong: SampleId::SNARE_ACCENT,
                normal: SampleId::SNARE,
                soft: Some(SampleId::SNARE_GHOST),
            },
            Sound::Xylophone => {
                SampleSet { strong: SampleId::XYLO_HIGH, normal: SampleId::XYLO_LOW, soft: None }
            }
        })
    }
}

/// How strongly a hit is played, whichever voice plays it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    Strong,
    Normal,
    Soft,
}

/// The recordings of a sound, one per [`Level`].
struct SampleSet {
    strong: SampleId,
    normal: SampleId,
    /// A quiet recording; without one, soft hits play `normal`.
    soft: Option<SampleId>,
}

impl SampleSet {
    fn at(&self, level: Level) -> SampleId {
        match level {
            Level::Strong => self.strong,
            Level::Normal => self.normal,
            Level::Soft => self.soft.unwrap_or(self.normal),
        }
    }
}

/// Playback rate of the rhythm's or counter rhythm's samples while the
/// metronome plays the same sound, so the voices stay apart by ear.
const SAME_SOUND_RATE: f32 = 1.5;

/// An embedded WAV file.
struct SampleFile {
    name: &'static str,
    bytes: &'static [u8],
}

/// Every embedded sample; [`SampleId`] indexes into it.
const SAMPLES: &[SampleFile] = &[
    SampleFile { name: "claves3.wav", bytes: include_bytes!("../samples/claves3.wav") },
    SampleFile { name: "claves4.wav", bytes: include_bytes!("../samples/claves4.wav") },
    SampleFile { name: "snare-normal.wav", bytes: include_bytes!("../samples/snare-normal.wav") },
    SampleFile { name: "snare-accent.wav", bytes: include_bytes!("../samples/snare-accent.wav") },
    SampleFile { name: "snare-ghost.wav", bytes: include_bytes!("../samples/snare-ghost.wav") },
    SampleFile { name: "xylo-high.wav", bytes: include_bytes!("../samples/xylo-high.wav") },
    SampleFile { name: "xylo-low.wav", bytes: include_bytes!("../samples/xylo-low.wav") },
];

/// An embedded sample, decoded on first use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SampleId(usize);

impl SampleId {
    const CLAVES_HIGH: SampleId = SampleId(0);
    const CLAVES_LOW: SampleId = SampleId(1);
    const SNARE: SampleId = SampleId(2);
    const SNARE_ACCENT: SampleId = SampleId(3);
    const SNARE_GHOST: SampleId = SampleId(4);
    const XYLO_HIGH: SampleId = SampleId(5);
    const XYLO_LOW: SampleId = SampleId(6);

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

    pub(crate) fn is_counter(self) -> bool {
        matches!(self, SoundType::CounterStrong | SoundType::CounterMedium | SoundType::CounterSoft)
    }

    /// Strong: the downbeat, accents and the counter rhythm's strong clicks.
    /// Soft: soft beats, ghost notes and the quiet clicks. The count-in
    /// plays its normal hits on every beat, like the primary beats.
    fn level(self) -> Level {
        match self {
            SoundType::Downbeat | SoundType::AccentedBeat | SoundType::CounterStrong => {
                Level::Strong
            }
            SoundType::PrimaryBeat
            | SoundType::Beat
            | SoundType::CountIn
            | SoundType::CounterMedium => Level::Normal,
            SoundType::SoftBeat
            | SoundType::Ghost
            | SoundType::CountInSoft
            | SoundType::CounterSoft => Level::Soft,
        }
    }

    /// The patch for this hit: the voice's chosen sound at this hit's level.
    pub(crate) fn patch(self, settings: &AudioSettings) -> Patch {
        let metronome = settings.metronome_sound;
        let sound = if self.is_rhythm() {
            settings.rhythm_sound
        } else if self.is_counter() {
            settings.counter_sound
        } else {
            metronome
        };
        let gain = self.profile(settings).gain;
        let same = (self.is_rhythm() || self.is_counter()) && sound == metronome;
        let rate = if same { SAME_SOUND_RATE } else { 1.0 };
        match sound.samples() {
            Some(set) => Patch::Sample { data: set.at(self.level()).data(), rate, gain },
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
    use grooph_measure::click::ClickBars;

    #[test]
    fn click_patch_matches_the_sound_settings() {
        let settings = AudioSettings {
            metronome_sound: Sound::Click,
            rhythm_sound: Sound::Click,
            ..AudioSettings::default()
        };
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
        let settings = AudioSettings { metronome_sound: Sound::Claves, ..AudioSettings::default() };
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
        // Both claves are audible out of the box.
        assert!(settings.downbeat > 0.0 && settings.primary > 0.0);
        assert_eq!(sample(SoundType::CountIn).0, low);
        // The rhythm keeps its own sound.
        assert_ne!(sample(SoundType::Beat).0, low);
    }

    #[test]
    fn counter_sounds_stay_apart_from_the_metronome() {
        let rate_of = |counter_sound, metronome_sound| {
            let settings =
                AudioSettings { counter_sound, metronome_sound, ..AudioSettings::default() };
            match SoundType::CounterMedium.patch(&settings) {
                Patch::Sample { data, rate, .. } => Some((data, rate)),
                Patch::Click { .. } => None,
            }
        };
        // Claves against claves play higher, against the click as recorded.
        let (data, rate) = rate_of(Sound::Claves, Sound::Claves).unwrap();
        assert_eq!(data, SampleId::CLAVES_LOW.data());
        assert_eq!(rate, SAME_SOUND_RATE);
        assert_eq!(rate_of(Sound::Claves, Sound::Click).unwrap().1, 1.0);
        // The click is pitched above the metronome's.
        assert!(rate_of(Sound::Click, Sound::Click).is_none());
        let settings = AudioSettings::default();
        let profile = |s: SoundType| s.profile(&settings).freq_mult;
        assert!(profile(SoundType::CounterMedium) > profile(SoundType::Downbeat));
        // The out-of-the-box counter sound differs from the metronome's.
        assert_ne!(settings.counter_sound.label(), settings.metronome_sound.label());
    }

    #[test]
    fn xylophone_plays_high_on_the_downbeat_and_strong_counter_clicks() {
        let settings = AudioSettings {
            metronome_sound: Sound::Xylophone,
            counter_sound: Sound::Xylophone,
            ..AudioSettings::default()
        };
        let sample = |sound: SoundType| match sound.patch(&settings) {
            Patch::Sample { data, rate, .. } => (data, rate),
            other => panic!("sample expected, got {other:?}"),
        };
        let (high, low) = (SampleId::XYLO_HIGH.data(), SampleId::XYLO_LOW.data());
        assert_eq!(sample(SoundType::Downbeat), (high.clone(), 1.0));
        assert_eq!(sample(SoundType::PrimaryBeat), (low.clone(), 1.0));
        assert_eq!(sample(SoundType::CountIn).0, low);
        // Against the xylophone metronome the counter rhythm plays higher.
        assert_eq!(sample(SoundType::CounterStrong), (high, SAME_SOUND_RATE));
        assert_eq!(sample(SoundType::CounterMedium), (low.clone(), SAME_SOUND_RATE));
        let against_claves = AudioSettings { metronome_sound: Sound::Claves, ..settings };
        assert!(matches!(
            SoundType::CounterMedium.patch(&against_claves),
            Patch::Sample { rate: 1.0, .. }
        ));
    }

    #[test]
    fn snare_counter_clicks_use_the_three_recordings() {
        let settings = AudioSettings { counter_sound: Sound::Snare, ..Default::default() };
        let sample = |sound: SoundType| match sound.patch(&settings) {
            Patch::Sample { data, gain, .. } => (data, gain),
            other => panic!("sample expected, got {other:?}"),
        };
        assert_eq!(sample(SoundType::CounterStrong).0, SampleId::SNARE_ACCENT.data());
        assert_eq!(sample(SoundType::CounterMedium).0, SampleId::SNARE.data());
        let (ghost, gain) = sample(SoundType::CounterSoft);
        assert_eq!(ghost, SampleId::SNARE_GHOST.data());
        // The soft clicks' volume scales the ghost recording directly.
        assert!((gain - crate::schedule::COUNTER_SOFT_GAIN * settings.counter).abs() < 1e-6);
    }

    #[test]
    fn snare_plays_its_own_sample_for_notes_accents_and_ghost_notes() {
        let settings = AudioSettings::default();
        assert_eq!(settings.rhythm_sound, Sound::Snare);
        let sample = |sound: SoundType| match sound.patch(&settings) {
            Patch::Sample { data, gain, .. } => (data, gain),
            other => panic!("sample expected, got {other:?}"),
        };
        assert_eq!(sample(SoundType::Beat), (SampleId::SNARE.data(), settings.beat));
        assert_eq!(
            sample(SoundType::AccentedBeat),
            (SampleId::SNARE_ACCENT.data(), settings.accent)
        );
        // The ghost volume scales the quiet ghost recording like any other sample.
        assert_eq!(sample(SoundType::Ghost), (SampleId::SNARE_GHOST.data(), settings.ghost));
    }

    #[test]
    fn snare_dynamics_get_louder_from_ghost_to_accent() {
        let peak = |id: SampleId| id.data().frames.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let (ghost, normal, accent) =
            (peak(SampleId::SNARE_GHOST), peak(SampleId::SNARE), peak(SampleId::SNARE_ACCENT));
        assert!(ghost < normal && normal < accent, "{ghost} {normal} {accent}");
    }

    #[test]
    fn a_bar_of_four_four_plays_one_high_and_three_low_claves() {
        use crate::schedule::Schedule;
        use grooph_measure::tempo::ScoreTiming;
        use grooph_measure::{Measure, Score, TimeSignature};

        let score = Score { measures: vec![Measure::new(TimeSignature::FOUR_FOUR)] };
        let timing = ScoreTiming::from_score(&score, 120);
        let mut sounds = Vec::new();
        Schedule::build(&score, &timing, None, &ClickBars::default()).collect_in_range(
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
    fn soft_hits_play_the_soft_recording_where_there_is_one() {
        let settings = AudioSettings {
            metronome_sound: Sound::Snare,
            counter_sound: Sound::Snare,
            ..AudioSettings::default()
        };
        let sample = |sound: SoundType| match sound.patch(&settings) {
            Patch::Sample { data, .. } => data,
            other => panic!("sample expected, got {other:?}"),
        };
        let ghost = SampleId::SNARE_GHOST.data();
        for soft in [SoundType::SoftBeat, SoundType::CountInSoft, SoundType::CounterSoft] {
            assert_eq!(sample(soft), ghost, "{soft:?}");
        }
        // Without a soft recording, soft hits play the normal one.
        let claves = AudioSettings { metronome_sound: Sound::Claves, ..settings };
        assert!(matches!(
            SoundType::SoftBeat.patch(&claves),
            Patch::Sample { data, .. } if data == SampleId::CLAVES_LOW.data()
        ));
    }

    #[test]
    fn rhythm_claves_and_xylophone_play_the_high_sample_on_accents() {
        let sample = |rhythm_sound, sound: SoundType| {
            let settings = AudioSettings {
                rhythm_sound,
                metronome_sound: Sound::Click,
                ..AudioSettings::default()
            };
            match sound.patch(&settings) {
                Patch::Sample { data, rate, .. } => (data, rate),
                other => panic!("sample expected, got {other:?}"),
            }
        };
        let claves = Sound::Claves;
        assert_eq!(sample(claves, SoundType::AccentedBeat).0, SampleId::CLAVES_HIGH.data());
        assert_eq!(sample(claves, SoundType::Beat), (SampleId::CLAVES_LOW.data(), 1.0));
        assert_eq!(sample(claves, SoundType::Ghost).0, SampleId::CLAVES_LOW.data());
        let xylo = Sound::Xylophone;
        assert_eq!(sample(xylo, SoundType::AccentedBeat).0, SampleId::XYLO_HIGH.data());
        assert_eq!(sample(xylo, SoundType::Beat).0, SampleId::XYLO_LOW.data());
    }

    #[test]
    fn rhythm_plays_higher_on_the_metronomes_sound() {
        let rate = |rhythm_sound, metronome_sound| {
            let settings =
                AudioSettings { rhythm_sound, metronome_sound, ..AudioSettings::default() };
            match SoundType::Beat.patch(&settings) {
                Patch::Sample { rate, .. } => rate,
                other => panic!("sample expected, got {other:?}"),
            }
        };
        assert_eq!(rate(Sound::Claves, Sound::Claves), SAME_SOUND_RATE);
        assert_eq!(rate(Sound::Snare, Sound::Snare), SAME_SOUND_RATE);
        assert_eq!(rate(Sound::Snare, Sound::Claves), 1.0);
        // The metronome itself stays as recorded.
        let settings = AudioSettings {
            rhythm_sound: Sound::Claves,
            metronome_sound: Sound::Claves,
            ..AudioSettings::default()
        };
        assert!(matches!(SoundType::Downbeat.patch(&settings), Patch::Sample { rate: 1.0, .. }));
    }

    #[test]
    fn snare_metronome_accents_the_downbeat() {
        let settings = AudioSettings {
            metronome_sound: Sound::Snare,
            counter_sound: Sound::Snare,
            ..AudioSettings::default()
        };
        let sample = |sound: SoundType| match sound.patch(&settings) {
            Patch::Sample { data, rate, .. } => (data, rate),
            other => panic!("sample expected, got {other:?}"),
        };
        assert_eq!(sample(SoundType::Downbeat), (SampleId::SNARE_ACCENT.data(), 1.0));
        assert_eq!(sample(SoundType::PrimaryBeat), (SampleId::SNARE.data(), 1.0));
        assert_eq!(sample(SoundType::CountIn).0, SampleId::SNARE.data());
        // A snare counter rhythm plays higher against it.
        assert_eq!(sample(SoundType::CounterMedium).1, SAME_SOUND_RATE);
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
