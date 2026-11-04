//! The app's settings, one group per feature. Each group is the same struct
//! at runtime, in the saved state ([`crate::PersistedState`]) and, where it
//! applies, in presets ([`crate::presets::PresetSettings`]), so a new
//! setting is a field in its group and nothing else lists it by hand.
//!
//! Every field falls back to its default on its own: a group is restored
//! field by field (see [`crate::persist`]), and a field missing from a
//! saved group reads as its default.

use crate::guide::Guide;
use crate::play_along::PlayAlongSettings;
use crate::{CountingSettings, StickingSettings};
use grooph_audio::{GapClick, TempoRamp};
use grooph_measure::generator::GeneratorSettings;
use grooph_measure::swing::Swing;
use grooph_measure::tempo::DEFAULT_BPM;
use serde::{Deserialize, Serialize};

/// How the rhythm or click is played: tempo, count-in, swing and the
/// metronome trainer (gap click, tempo ramp).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PlaybackSettings {
    pub(crate) bpm: u32,
    /// Play one bar of clicks before playback starts.
    pub(crate) count_in: bool,
    /// Swing feel of the playback (notation stays straight).
    pub(crate) swing: Swing,
    /// Metronome trainer: muted bars.
    pub(crate) gap_click: GapClick,
    /// Metronome trainer: stepwise tempo change from `bpm`.
    pub(crate) tempo_ramp: TempoRamp,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            bpm: DEFAULT_BPM,
            count_in: false,
            swing: Swing::default(),
            gap_click: GapClick::default(),
            tempo_ramp: TempoRamp::default(),
        }
    }
}

/// What Practice's rhythm generator writes, and how.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct GeneratorConfig {
    pub(crate) settings: GeneratorSettings,
    /// Keep adding measures while playing instead of looping the score.
    pub(crate) endless: bool,
    /// Play quiet ghost notes on the free slots of the generator subdivision.
    pub(crate) ghost_notes: bool,
}

/// The devices of this machine: the output latency and the MIDI input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct DeviceSettings {
    pub(crate) latency_enabled: bool,
    /// Output latency (seconds) the cursor and the timing feedback allow for.
    pub(crate) offset: f32,
    /// The MIDI input port played along on.
    pub(crate) midi_port_id: Option<String>,
}

impl Default for DeviceSettings {
    fn default() -> Self {
        Self { latency_enabled: true, offset: 0.0, midi_port_id: None }
    }
}

impl DeviceSettings {
    /// The latency offset in seconds, 0 while switched off.
    pub(crate) fn latency_s(&self) -> f64 {
        if self.latency_enabled { self.offset as f64 } else { 0.0 }
    }
}

/// Feedback while playing: timing feedback, the staff's beat flash and
/// how hits are played along (taps, MIDI, microphone).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct FeedbackSettings {
    /// Timing feedback (see `Grooph::accuracy_active`).
    pub(crate) accuracy_enabled: bool,
    /// Whether the staff flashes on every beat while playing.
    pub(crate) flash_enabled: bool,
    pub(crate) play_along: PlayAlongSettings,
}

/// How the staff is read: the counting and sticking rows, the practice
/// guide, and whether the score tab is shown.
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ViewSettings {
    pub(crate) counting: CountingSettings,
    pub(crate) sticking: StickingSettings,
    /// What the practice staff shows while it plays.
    pub(crate) guide: Guide,
    /// Whether the tab bar shows the score tab, which is not finished yet
    /// (a developer setting, off by default). A link or pattern that opens
    /// the score tab still shows it while it is open.
    pub(crate) show_score_tab: bool,
}
