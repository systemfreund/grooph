use crate::accuracy::AccuracyState;
use crate::library::PatternLibrary;
use crate::platform::PlatformRuntime;
use crate::undo::UndoHistory;
use crate::{CountingSettings, Mode, TransportState};
use eframe::egui::TextStyle;
use eframe::epaint::FontId;
use grooph_audio::{Audio, AudioSettings};
use grooph_measure::{Cursor, Measure, Score};
use grooph_midi::MidiInput;
use std::collections::HashMap;

pub(crate) struct AudioConfig {
    pub(crate) settings: AudioSettings,
    pub(crate) offset: f32,
    pub(crate) latency_enabled: bool,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self { settings: AudioSettings::default(), offset: 0.0, latency_enabled: true }
    }
}

pub(crate) struct LayoutSettings {
    pub(crate) width_cap_factor: f32,
    pub(crate) accent_below: bool,
    pub(crate) proportional_spacing: bool,
    pub(crate) stem_length_factor: f32,
    pub(crate) debug_bbox: bool,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            width_cap_factor: 0.1,
            accent_below: false,
            proportional_spacing: true,
            stem_length_factor: 0.9,
            debug_bbox: false,
        }
    }
}

#[derive(Default)]
pub(crate) struct PlaybackState {
    /// Cursor position in **global ticks** across the entire score loop.
    pub(crate) smooth_tick: f64,
    pub(crate) last_update: Option<f64>,
    pub(crate) flash_intensity: f32,
    /// Last (measure_idx, primary_beat_in_measure) that fired the flash. Tracks
    /// both axes so the flash fires once per primary beat in every measure, not
    /// only when the beat number changes.
    pub(crate) last_primary_beat: Option<(usize, u32)>,
}

impl PlaybackState {
    pub(crate) fn reset(&mut self) { *self = Self::default(); }
}

#[derive(Default)]
pub(crate) struct MidiState {
    pub(crate) input: Option<MidiInput>,
    pub(crate) available_ports: Vec<String>,
    pub(crate) selected_port_id: Option<String>,
}

/// Score editing surface: the document, where the cursor sits, undo history,
/// and pre-built button thumbnails. Mutated by tools and the time-signature
/// dialog; read by the rendering pipeline.
pub(crate) struct EditorState {
    pub(crate) score: Score,
    pub(crate) cursor: Cursor,
    pub(crate) history: UndoHistory,
    /// Prebuilt measures for note/rest/tuplet tool buttons to avoid per-frame
    /// reconstruction.
    pub(crate) button_measures: HashMap<&'static str, Measure>,
    /// User-saved, named patterns (score + tempo). Persisted across sessions.
    pub(crate) library: PatternLibrary,
}

/// Realtime playback subsystem: transport, tempo, audio engine, MIDI input,
/// and the accuracy tracker that ties hits to score onsets.
pub(crate) struct PlaybackController {
    pub(crate) transport_state: TransportState,
    pub(crate) bpm: u32,
    pub(crate) audio: Option<Audio>,
    pub(crate) audio_cfg: AudioConfig,
    pub(crate) playback: PlaybackState,
    pub(crate) accuracy: AccuracyState,
    pub(crate) midi: MidiState,
}

/// UI shell state: which panel is visible, font configuration, layout knobs,
/// counting overlay settings, and the platform abstraction for visibility /
/// wake-lock.
pub(crate) struct UiShell {
    pub(crate) mode: Mode,
    pub(crate) music_font_id: FontId,
    pub(crate) font_bump: f32,
    pub(crate) baseline_dark: Option<Vec<(TextStyle, f32)>>,
    pub(crate) baseline_light: Option<Vec<(TextStyle, f32)>>,
    pub(crate) layout: LayoutSettings,
    pub(crate) counting: CountingSettings,
    pub(crate) platform: PlatformRuntime,
    /// Transient text buffer for the "save current pattern" name input. Not
    /// persisted.
    pub(crate) save_name_buffer: String,
}
