//! Sound choice for the voices of playback.
//!
//! The **rhythm** voice plays the written notes, accents and ghost notes; the
//! **metronome** voice plays the downbeat, the primary beats and the count-in;
//! the **counter rhythm** is the metronome's second layer. Each voice picks
//! any [`Sound`]. When the rhythm or the counter rhythm plays the metronome's
//! sound, its samples play higher, so the voices stay apart by ear.
//!
//! Every hit has a [`Level`] (strong, normal or soft). A sampled sound or kit
//! instrument has a normal recording and optionally a strong and a soft one
//! ([`SampleSet`]); a level without its own recording plays the normal one,
//! at the voice's mixer volume for that level, or, in the kit, louder or
//! quieter by a fixed stand-in gain. `Click` is the
//! configurable click from [`AudioSettings`] (waveform, base frequency,
//! decay, noise), pitched per sound type. Every other sound plays short WAV
//! samples embedded in the binary (see [`SampleId`]) with the volume of the
//! mixer.
//!
//! The heads of a drum kit score play the kit: a [`SampleSet`] per
//! instrument and technique ([`kit_samples`]), whatever sounds the voices have; a
//! [`Instrument::Rhythm`] head is the rhythm voice's.

use crate::schedule::{KitHit, SoundType};
use crate::wav::{self, SampleData};
use crate::{AudioSettings, Waveform};
use grooph_measure::{Instrument, Technique};
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
    /// Claves and xylophone have no accent recording as such: their higher
    /// hit or note is the accent.
    fn samples(self) -> Option<SampleSet> {
        Some(match self {
            Sound::Click => return None,
            Sound::Claves => SampleSet::new(SampleId::ClavesLow).with_strong(SampleId::ClavesHigh),
            Sound::Snare => SampleSet::new(SampleId::Snare)
                .with_strong(SampleId::SnareAccent)
                .with_soft(SampleId::SnareGhost),
            Sound::Xylophone => SampleSet::new(SampleId::XyloLow).with_strong(SampleId::XyloHigh),
        })
    }
}

/// How strongly a hit is played, whichever voice plays it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    Strong,
    Normal,
    Soft,
}

/// The recordings of a sound or a kit instrument: a normal one, and
/// optionally one for strong and one for soft hits. A level without its own
/// recording plays the normal one in its place ([`SampleSet::at`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SampleSet {
    normal: SampleId,
    strong: Option<SampleId>,
    soft: Option<SampleId>,
}

/// Volume of the normal recording standing in for a missing strong or soft
/// one, relative to a normal hit ([`SampleSet::stand_in_gain`]).
const STAND_IN_STRONG_GAIN: f32 = 4.0 / 3.0;
const STAND_IN_SOFT_GAIN: f32 = 0.4;

impl SampleSet {
    /// A set of one recording, which plays at every level.
    const fn new(normal: SampleId) -> Self {
        Self { normal, strong: None, soft: None }
    }

    const fn with_strong(self, strong: SampleId) -> Self {
        Self { strong: Some(strong), ..self }
    }

    const fn with_soft(self, soft: SampleId) -> Self {
        Self { soft: Some(soft), ..self }
    }

    /// The recording of `level`, or the normal one where there is none.
    fn at(&self, level: Level) -> SampleId {
        self.recorded(level).unwrap_or(self.normal)
    }

    /// The set's own recording of `level`, if it has one.
    fn recorded(&self, level: Level) -> Option<SampleId> {
        match level {
            Level::Strong => self.strong,
            Level::Normal => Some(self.normal),
            Level::Soft => self.soft,
        }
    }

    /// Volume of a hit at `level` relative to a normal one where nothing
    /// else sets it: 1.0 where the set has a recording of the level (the
    /// recording carries the dynamics), louder or quieter where the normal
    /// recording stands in. The voices have a mixer volume per level
    /// instead; the kit, with one volume per instrument, uses this.
    fn stand_in_gain(&self, level: Level) -> f32 {
        match (level, self.recorded(level)) {
            (_, Some(_)) => 1.0,
            (Level::Strong, None) => STAND_IN_STRONG_GAIN,
            (_, None) => STAND_IN_SOFT_GAIN,
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

/// Declares [`SampleId`] and its files in one table, so an id cannot point
/// at another id's file.
macro_rules! samples {
    ($($id:ident => $path:literal,)*) => {
        /// An embedded sample, decoded on first use.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) enum SampleId {
            $($id,)*
        }

        impl SampleId {
            /// Every sample, in declaration order (its index).
            const ALL: &[SampleId] = &[$(SampleId::$id,)*];

            fn file(self) -> SampleFile {
                match self {
                    $(SampleId::$id => SampleFile {
                        name: $path,
                        bytes: include_bytes!(concat!("../samples/", $path)),
                    },)*
                }
            }
        }
    };
}

samples! {
    ClavesHigh => "claves3.wav",
    ClavesLow => "claves4.wav",
    Snare => "snare-normal.wav",
    SnareAccent => "snare-accent.wav",
    SnareGhost => "snare-ghost.wav",
    XyloHigh => "xylo-high.wav",
    XyloLow => "xylo-low.wav",
    // The drum kit (`samples/kit`, made by its `synth.py`).
    Kick => "kit/kick.wav",
    CrossStick => "kit/cross-stick.wav",
    HiHatClosed => "kit/hihat-closed.wav",
    HiHatHalf => "kit/hihat-half.wav",
    HiHatOpen => "kit/hihat-open.wav",
    HiHatFoot => "kit/hihat-foot.wav",
    Ride => "kit/ride.wav",
    RideBell => "kit/ride-bell.wav",
    Crash => "kit/crash.wav",
    CrashChoke => "kit/crash-choke.wav",
    TomHigh => "kit/tom-high.wav",
    TomMid => "kit/tom-mid.wav",
    TomFloor => "kit/tom-floor.wav",
}

impl SampleId {
    /// The decoded sample; an unreadable file becomes silence (logged once).
    fn data(self) -> Arc<SampleData> {
        static DECODED: OnceLock<Vec<Arc<SampleData>>> = OnceLock::new();
        let all = DECODED.get_or_init(|| {
            SampleId::ALL
                .iter()
                .map(|id| {
                    let file = id.file();
                    Arc::new(wav::decode(file.bytes).unwrap_or_else(|err| {
                        log::error!("Cannot decode sample {}: {err:?}", file.name);
                        SampleData { rate: 44_100, frames: Vec::new() }
                    }))
                })
                .collect()
        });
        all[self as usize].clone()
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
    pub(crate) fn is_rhythm(self) -> bool {
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
            SoundType::Kit(hit) => hit.level,
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
        if let SoundType::Kit(hit) = self {
            let gain = self.profile(settings).gain;
            return Patch::Sample { data: kit_samples(hit).at(hit.level).data(), rate: 1.0, gain };
        }
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

/// The kit's recordings for `hit`'s instrument and technique. The snare has
/// one per level; the other instruments have one, played louder or quieter
/// in place of the others ([`KitHit::level_gain`]).
fn kit_samples(hit: KitHit) -> SampleSet {
    use Instrument::*;
    use Technique::*;
    let snare = Sound::Snare.samples().expect("recorded");
    SampleSet::new(match (hit.instrument, hit.technique) {
        (Snare, CrossStick) => SampleId::CrossStick,
        // No rimshot recording: the snare's accent stands in.
        (Snare, Rimshot) => SampleId::SnareAccent,
        (Snare, _) => return snare,
        (Kick, _) => SampleId::Kick,
        (HiHat, Open) => SampleId::HiHatOpen,
        (HiHat, HalfOpen) => SampleId::HiHatHalf,
        (HiHat, _) => SampleId::HiHatClosed,
        (HiHatFoot, _) => SampleId::HiHatFoot,
        (Ride, Bell) => SampleId::RideBell,
        (Ride, _) => SampleId::Ride,
        (Crash, Choke) => SampleId::CrashChoke,
        (Crash, _) => SampleId::Crash,
        (HighTom, _) => SampleId::TomHigh,
        (MidTom, _) => SampleId::TomMid,
        (FloorTom, _) => SampleId::TomFloor,
        // Never scheduled as a kit hit: the rhythm voice plays it.
        (Rhythm, _) => return snare,
    })
}

/// Volume of a normal kit hit relative to its instrument's in the mixer,
/// leaving room for the accents.
const KIT_NORMAL_GAIN: f32 = 0.75;

impl KitHit {
    /// Volume of the hit relative to its instrument's in the mixer: a
    /// recorded level plays at the normal volume, its recording carrying the
    /// dynamics; a level without one plays the normal recording louder or
    /// quieter ([`SampleSet::stand_in_gain`]).
    pub(crate) fn level_gain(self) -> f32 {
        KIT_NORMAL_GAIN * kit_samples(self).stand_in_gain(self.level)
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
        assert_eq!(high, SampleId::ClavesHigh.data());
        assert_eq!(gain, settings.downbeat);
        let (low, gain) = sample(SoundType::PrimaryBeat);
        assert_eq!(low, SampleId::ClavesLow.data());
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
        assert_eq!(data, SampleId::ClavesLow.data());
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
        let (high, low) = (SampleId::XyloHigh.data(), SampleId::XyloLow.data());
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
        assert_eq!(sample(SoundType::CounterStrong).0, SampleId::SnareAccent.data());
        assert_eq!(sample(SoundType::CounterMedium).0, SampleId::Snare.data());
        let (ghost, gain) = sample(SoundType::CounterSoft);
        assert_eq!(ghost, SampleId::SnareGhost.data());
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
        assert_eq!(sample(SoundType::Beat), (SampleId::Snare.data(), settings.beat));
        assert_eq!(
            sample(SoundType::AccentedBeat),
            (SampleId::SnareAccent.data(), settings.accent)
        );
        // The ghost volume scales the quiet ghost recording like any other sample.
        assert_eq!(sample(SoundType::Ghost), (SampleId::SnareGhost.data(), settings.ghost));
    }

    #[test]
    fn snare_dynamics_get_louder_from_ghost_to_accent() {
        let peak = |id: SampleId| id.data().frames.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let (ghost, normal, accent) =
            (peak(SampleId::SnareGhost), peak(SampleId::Snare), peak(SampleId::SnareAccent));
        assert!(ghost < normal && normal < accent, "{ghost} {normal} {accent}");
    }

    #[test]
    fn a_bar_of_four_four_plays_one_high_and_three_low_claves() {
        use crate::schedule::Schedule;
        use grooph_measure::tempo::ScoreTiming;
        use grooph_measure::{Measure, Score, TimeSignature};

        let score = Score::new(vec![Measure::new(TimeSignature::FOUR_FOUR)]);
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
        let (high, low) = (SampleId::ClavesHigh.data(), SampleId::ClavesLow.data());
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
        let ghost = SampleId::SnareGhost.data();
        for soft in [SoundType::SoftBeat, SoundType::CountInSoft, SoundType::CounterSoft] {
            assert_eq!(sample(soft), ghost, "{soft:?}");
        }
        // Without a soft recording, soft hits play the normal one.
        let claves = AudioSettings { metronome_sound: Sound::Claves, ..settings };
        assert!(matches!(
            SoundType::SoftBeat.patch(&claves),
            Patch::Sample { data, .. } if data == SampleId::ClavesLow.data()
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
        assert_eq!(sample(claves, SoundType::AccentedBeat).0, SampleId::ClavesHigh.data());
        assert_eq!(sample(claves, SoundType::Beat), (SampleId::ClavesLow.data(), 1.0));
        assert_eq!(sample(claves, SoundType::Ghost).0, SampleId::ClavesLow.data());
        let xylo = Sound::Xylophone;
        assert_eq!(sample(xylo, SoundType::AccentedBeat).0, SampleId::XyloHigh.data());
        assert_eq!(sample(xylo, SoundType::Beat).0, SampleId::XyloLow.data());
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
        assert_eq!(sample(SoundType::Downbeat), (SampleId::SnareAccent.data(), 1.0));
        assert_eq!(sample(SoundType::PrimaryBeat), (SampleId::Snare.data(), 1.0));
        assert_eq!(sample(SoundType::CountIn).0, SampleId::Snare.data());
        // A snare counter rhythm plays higher against it.
        assert_eq!(sample(SoundType::CounterMedium).1, SAME_SOUND_RATE);
    }

    #[test]
    fn embedded_samples_decode() {
        for (i, &id) in SampleId::ALL.iter().enumerate() {
            assert_eq!(id as usize, i, "{id:?}");
            let file = id.file();
            let data = wav::decode(file.bytes).unwrap_or_else(|e| panic!("{}: {e:?}", file.name));
            assert!(!data.frames.is_empty(), "{} is empty", file.name);
            assert_eq!(*id.data(), data);
        }
    }

    #[test]
    fn every_kit_instrument_and_technique_has_a_sample() {
        let settings = AudioSettings::default();
        let mut samples = Vec::new();
        for instrument in Instrument::ALL.into_iter().filter(|&i| i != Instrument::Rhythm) {
            for &technique in instrument.techniques() {
                let hit = KitHit { instrument, technique, level: Level::Normal };
                let Patch::Sample { data, rate, .. } = SoundType::Kit(hit).patch(&settings) else {
                    panic!("{instrument:?} {technique:?}: sample expected");
                };
                assert!(!data.frames.is_empty(), "{instrument:?} {technique:?}");
                assert_eq!(rate, 1.0, "the kit plays as recorded, whatever the voices' sounds");
                samples.push((instrument, technique, data));
            }
        }
        let of = |i, t| samples.iter().find(|s| s.0 == i && s.1 == t).unwrap().2.clone();
        use Instrument::{Crash, HiHat, Kick, Ride, Snare};
        use Technique::{Bell, Choke, Closed, CrossStick, HalfOpen, Normal, Open};
        // An open hi-hat rings longer than a closed one, a half-open one between.
        let len = |i, t| of(i, t).frames.len();
        assert!(len(HiHat, Closed) < len(HiHat, HalfOpen));
        assert!(len(HiHat, HalfOpen) < len(HiHat, Open));
        assert_eq!(of(HiHat, Normal), of(HiHat, Closed));
        assert!(len(Crash, Choke) < len(Crash, Normal));
        assert_ne!(of(Ride, Bell), of(Ride, Normal));
        assert_ne!(of(Snare, CrossStick), of(Snare, Normal));
        assert_ne!(of(Kick, Normal), of(Snare, Normal));
    }

    #[test]
    fn the_kit_snare_plays_its_recording_per_level() {
        let hit = |instrument, level| KitHit { instrument, technique: Technique::Normal, level };
        let at = |level| kit_samples(hit(Instrument::Snare, level)).at(level);
        assert_eq!(at(Level::Strong), SampleId::SnareAccent);
        assert_eq!(at(Level::Normal), SampleId::Snare);
        assert_eq!(at(Level::Soft), SampleId::SnareGhost);
        // Its recordings carry the dynamics: every level at one volume.
        let snare = |level| hit(Instrument::Snare, level).level_gain();
        assert_eq!(snare(Level::Strong), snare(Level::Normal));
        assert_eq!(snare(Level::Soft), snare(Level::Normal));
        // Other instruments have one recording, played louder or quieter.
        let kick = |level| kit_samples(hit(Instrument::Kick, level)).at(level);
        assert_eq!(kick(Level::Soft), kick(Level::Strong));
        let kick = |level| hit(Instrument::Kick, level).level_gain();
        assert!(
            kick(Level::Soft) < kick(Level::Normal) && kick(Level::Normal) < kick(Level::Strong)
        );
    }

    #[test]
    fn a_missing_recording_is_stood_in_for_by_the_normal_one() {
        let set = SampleSet::new(SampleId::Kick);
        for level in [Level::Strong, Level::Normal, Level::Soft] {
            assert_eq!(set.at(level), SampleId::Kick);
        }
        assert_eq!(set.stand_in_gain(Level::Normal), 1.0);
        assert!(set.stand_in_gain(Level::Strong) > 1.0);
        assert!(set.stand_in_gain(Level::Soft) < 1.0);
        let full = set.with_strong(SampleId::SnareAccent).with_soft(SampleId::SnareGhost);
        assert_eq!(full.at(Level::Strong), SampleId::SnareAccent);
        assert_eq!(full.at(Level::Soft), SampleId::SnareGhost);
        assert_eq!(full.stand_in_gain(Level::Strong), 1.0);
        assert_eq!(full.stand_in_gain(Level::Soft), 1.0);
    }
}
