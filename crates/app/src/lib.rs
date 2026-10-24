mod accuracy;
mod calibration;
mod clock;
mod compact;
mod generator;
mod help;
mod host;
mod keyboard_input;
mod library;
mod library_panel;
mod measure_panel;
mod metronome_trainer;
mod metronome_view;
mod midi_input_widget;
mod mixer_panel;
mod pickers;
mod platform;
mod play_along;
mod playback_clock;
mod preset_panel;
mod presets;
mod settings_panel;
mod share;
mod state;
mod style;
mod time_signature_dialog;
mod tool_palette;
pub mod tools;
mod undo;
mod update_check;
#[cfg(target_arch = "wasm32")]
mod web;

pub use host::Host;

use grooph_measure::duration::{Duration, NoteValue, TupletSpec, q};
use grooph_measure::{BeatIdx, Cursor, Measure, Score, TimeSignature};

use crate::accuracy::AccuracyState;
use crate::calibration::CalibrationPhase;
use crate::compact::{CompactState, CompactTab};
use crate::generator::GeneratorState;
use crate::library::PatternLibrary;
use crate::metronome_view::MetronomeState;
use crate::platform::{PlatformRuntime, VisibilityEvent};
use crate::play_along::{PlayAlongSettings, PlayAlongState};
use crate::presets::PresetLibrary;
use crate::state::{
    AudioConfig, CountInState, EditorState, LayoutSettings, MicState, MidiState, PendingLoad,
    PlaybackController, PlaybackState, UiShell,
};
use crate::tools::ToolKind;
use crate::tools::{BeatTemplate, Modifier, all_tools};
use crate::undo::{DEFAULT_UNDO_LIMIT, EditorSnapshot, UndoHistory};
use eframe::egui::{Context, Ui};
use eframe::epaint::text::{FontInsert, InsertFontFamily};
use eframe::epaint::{FontFamily, FontId};
use eframe::{App, CreationContext, egui};
use grooph_audio::{AudioSettings, GapClick, PlaybackOptions, PlayerState, TempoRamp};
use grooph_link::{LinkContent, SharedLink};
use grooph_measure::click::ClickBars;
use grooph_measure::counter::{CounterBars, Ratio};
use grooph_measure::counting::{
    CountConfig, CountLayer, CountScope, LabelPattern, LabelToken, Subdiv,
};
use grooph_measure::duration::NoteValue::*;
use grooph_measure::editing::Modification;
use grooph_measure::generator::GeneratorSettings;
use grooph_measure::swing::Swing;
use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{Beat, BeatKind};
use grooph_midi::MidiInput;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Edit,
    Playback,
    Mixer,
    Settings,
    Help,
    Library,
    Generator,
    /// The compact layout's "More" list of panels.
    MoreMenu,
    /// The time signature dialog, holding the meter being picked.
    TimeSignature(TimeSignature),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransportState {
    Stopped,
    Playing,
}

const APP_STATE_KEY: &str = "grooph_state";

/// Top-level app object. Holds three subsystems:
/// - [`EditorState`] — score, cursor, undo history, button thumbnails.
/// - [`PlaybackController`] — transport, audio, MIDI, accuracy.
/// - [`UiShell`] — modes, fonts, layout knobs, counting overlay, platform glue.
///
/// `Grooph` itself is a facade: most methods coordinate across subsystems,
/// which is why they stay attached to `Self` rather than moving onto the
/// individual structs.
pub struct Grooph {
    pub(crate) editor: EditorState,
    pub(crate) playback_ctl: PlaybackController,
    pub(crate) ui: UiShell,
}

const PERSISTED_STATE_VERSION: u8 = 2;

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct PersistedState {
    version: u8,
    score: Score,
    cursor: Cursor,
    bpm: u32,
    audio_settings: AudioSettings,
    audio_latency_enabled: bool,
    audio_offset: f32,
    counting: CountingSettings,
    flash_enabled: bool,
    midi_selected_port_id: Option<String>,
    accuracy_enabled: bool,
    library: PatternLibrary,
    active_pattern_id: Option<u64>,
    dirty: bool,
    generator_settings: GeneratorSettings,
    endless: bool,
    ghost_notes: bool,
    count_in: bool,
    swing: Swing,
    play_along: PlayAlongSettings,
    gap_click: GapClick,
    tempo_ramp: TempoRamp,
    presets: PresetLibrary,
    click_bars: ClickBars,
    counter_bars: CounterBars,
    /// The ratio each counter bar was picked as (see [`CounterBars::names`]).
    counter_names: Vec<(TimeSignature, Ratio)>,
    metronome_meter: TimeSignature,
    /// Tab open when the state was saved, reopened at the next start.
    tab: CompactTab,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            version: PERSISTED_STATE_VERSION,
            score: Score::single(Grooph::default_measure()),
            cursor: Cursor::start(),
            bpm: 120,
            audio_settings: AudioSettings::default(),
            audio_latency_enabled: true,
            audio_offset: 0.0,
            counting: CountingSettings::default(),
            flash_enabled: false,
            midi_selected_port_id: None,
            accuracy_enabled: false,
            library: PatternLibrary::default(),
            active_pattern_id: None,
            dirty: false,
            generator_settings: GeneratorSettings::default(),
            endless: false,
            ghost_notes: false,
            count_in: false,
            swing: Swing::default(),
            play_along: PlayAlongSettings::default(),
            gap_click: GapClick::default(),
            tempo_ramp: TempoRamp::default(),
            presets: PresetLibrary::default(),
            click_bars: ClickBars::default(),
            counter_bars: CounterBars::default(),
            counter_names: Vec::new(),
            metronome_meter: TimeSignature::FOUR_FOUR,
            tab: CompactTab::Practice,
        }
    }
}

impl PersistedState {
    fn from_app(app: &Grooph) -> Self {
        // A running calibration has swapped in its click score, the metronome
        // tab its click bar: save what they will restore instead.
        let stash = app.calibration_stash();
        let rhythm = app.metronome_stash();
        let generator = &app.editor.generator;
        Self {
            version: PERSISTED_STATE_VERSION,
            score: stash
                .map_or(rhythm.map_or(&app.editor.score, |r| &r.score), |s| &s.score)
                .clone(),
            cursor: stash.map_or(rhythm.map_or(app.editor.cursor, |r| r.cursor), |s| s.cursor),
            bpm: stash.map_or(app.playback_ctl.bpm, |s| s.bpm),
            audio_settings: app.playback_ctl.audio_cfg.settings,
            audio_latency_enabled: app.playback_ctl.audio_cfg.latency_enabled,
            audio_offset: app.playback_ctl.audio_cfg.offset,
            counting: app.ui.counting,
            flash_enabled: app.ui.flash_enabled,
            midi_selected_port_id: app.playback_ctl.midi.selected_port_id.clone(),
            accuracy_enabled: app.playback_ctl.accuracy.enabled,
            library: app.editor.library.clone(),
            active_pattern_id: stash.map_or(
                rhythm.map_or(app.editor.active_pattern_id, |r| r.active_pattern_id),
                |s| s.active_pattern_id,
            ),
            dirty: stash.map_or(rhythm.map_or(app.editor.dirty, |r| r.dirty), |s| s.dirty),
            generator_settings: generator.settings,
            endless: stash.map_or(generator.endless, |s| s.endless),
            ghost_notes: stash.map_or(generator.ghost_notes, |s| s.ghost_notes),
            count_in: stash.map_or(app.playback_ctl.audio_cfg.count_in, |s| s.count_in),
            swing: stash.map_or(app.playback_ctl.audio_cfg.swing, |s| s.swing),
            play_along: app.playback_ctl.play_along.settings,
            gap_click: stash.map_or(app.playback_ctl.audio_cfg.gap, |s| s.gap),
            tempo_ramp: stash.map_or(app.playback_ctl.audio_cfg.ramp, |s| s.ramp),
            presets: app.editor.presets.clone(),
            click_bars: app.playback_ctl.audio_cfg.click.clone(),
            counter_bars: app.playback_ctl.audio_cfg.counter.clone(),
            counter_names: app.playback_ctl.audio_cfg.counter.names().to_vec(),
            metronome_meter: app.editor.metronome.meter,
            tab: app.compact_tab(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
enum CountingBase {
    Off,
    Primary,
    Ands,
    Sixteenth,
    Triplet,
}

impl CountingBase {
    /// The overlay layer counting in this subdivision, `None` for `Off`.
    fn layer(self, id: u32) -> Option<CountLayer> {
        let (scope, subdiv, labels) = match self {
            Self::Off => return None,
            Self::Primary => (
                CountScope::PrimaryGroup,
                1,
                LabelPattern { slots: vec![vec![LabelToken::GroupNum]] },
            ),
            Self::Ands => (CountScope::BeatUnit, 2, LabelPattern::ands()),
            Self::Sixteenth => (CountScope::BeatUnit, 4, LabelPattern::sixteenth()),
            Self::Triplet => (CountScope::PrimaryGroup, 3, LabelPattern::triplet()),
        };
        let mut layer = CountLayer::new(id, scope, Subdiv::Fixed(subdiv));
        layer.labels = Some(labels);
        // Triplets of a group of eighths are the eighths themselves: count
        // them as "1 la li" in x/8 and x/16.
        layer.group_units = self == Self::Triplet;
        Some(layer)
    }

    /// Whether the subdivision counts sensibly in some measure of `score`.
    fn fits(self, score: &Score) -> bool {
        self.layer(0).is_none_or(|l| score.measures.iter().any(|m| l.fits(&m.time_signature())))
    }
}

/// The counting overlay: a subdivision to count in (or `Off`) plus an
/// optional tuplet layer. `Off` without tuplets hides the overlay.
#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(from = "StoredCountingSettings")]
struct CountingSettings {
    base: CountingBase,
    show_tuplets: bool,
}

impl Default for CountingSettings {
    fn default() -> Self {
        Self { base: CountingBase::Off, show_tuplets: false }
    }
}

/// Saved form, still reading the `enabled` and `show_labels` switches that
/// older versions stored next to `base`: either one off hid the overlay.
#[derive(Deserialize)]
struct StoredCountingSettings {
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default = "yes")]
    show_labels: bool,
    base: CountingBase,
    show_tuplets: bool,
}

fn yes() -> bool {
    true
}

impl From<StoredCountingSettings> for CountingSettings {
    fn from(s: StoredCountingSettings) -> Self {
        if s.enabled && s.show_labels {
            Self { base: s.base, show_tuplets: s.show_tuplets }
        } else {
            Self::default()
        }
    }
}

fn add_font(ctx: &Context) {
    ctx.add_font(FontInsert::new(
        "Bravura",
        egui::FontData::from_static(include_bytes!("../assets/fonts/Bravura.otf")),
        vec![InsertFontFamily {
            family: FontFamily::Name("music".into()),
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    ));
}

impl App for Grooph {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        self.apply_style(ui);
        self.ui.narrow_screen = ui.available_width() < compact::NARROW_MAX_WIDTH;
        self.compact_ui(ui);
        self.sync_click_bar();
        self.update_metronome_trainer();
        self.update_endless();
        self.run_summary_window(ui);
        self.calibration_window(ui);

        if matches!(self.ui.mode, Mode::TimeSignature(_)) {
            self.time_signature_dialog(ui);
        }

        if self.ui.pending_load.is_some() {
            self.load_confirm_dialog(ui);
        }

        if self.ui.link_error.is_some() {
            self.link_error_dialog(ui);
        }

        self.update_notice(ui, frame);

        self.handle_keyboard_input(ui);
        self.handle_play_along_input(ui);

        if let Some(ev) = self.ui.platform.take_visibility_event() {
            match ev {
                VisibilityEvent::Hidden => self.hidden(),
                VisibilityEvent::Visible | VisibilityEvent::PageShow => {
                    self.playback_ctl.audio = None;
                }
            }
        }

        let audio_state = self.audio_state();

        // If we should be playing audio but have no audio engine, try to create it (works after user gesture on iOS)
        if audio_state == PlayerState::Playing && self.playback_ctl.audio.is_none() {
            debug!("Creating audio engine.");
            self.playback_ctl.audio = grooph_audio::Audio::new(self.playback_ctl.bpm);
            if let Some(audio) = &mut self.playback_ctl.audio {
                audio.set_seed(crate::platform::random_seed());
            }
        }

        let playback_options = self.playback_options();
        let metronome = self.metronome_mode().then(|| self.metronome_score());
        let score = metronome.as_ref().unwrap_or(&self.editor.score);
        if let Some(audio) = &mut self.playback_ctl.audio {
            audio.set_audio_settings(self.playback_ctl.audio_cfg.settings);
            audio.set_playback_options(playback_options);
            if audio.update(&audio_state, self.playback_ctl.bpm, score) {
                ui.ctx().request_repaint();
            }
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let state = PersistedState::from_app(self);
        eframe::set_value(storage, APP_STATE_KEY, &state);
    }
}

impl Grooph {
    fn playback_options(&self) -> PlaybackOptions {
        let generator = &self.editor.generator;
        let calibrating = self.calibration_stash().is_some();
        PlaybackOptions {
            count_in: self.playback_ctl.audio_cfg.count_in,
            ghost_notes: (generator.ghost_notes && !self.metronome_mode())
                .then_some(generator.settings.subdivision),
            swing: self.playback_ctl.audio_cfg.swing,
            gap: self.playback_ctl.audio_cfg.gap,
            ramp: self.played_ramp(),
            // Calibration clicks every beat, whatever the user's click bars.
            click: if calibrating {
                ClickBars::default()
            } else {
                self.playback_ctl.audio_cfg.click.clone()
            },
            counter: self.played_counter(),
        }
    }

    /// Tempo backbone of the working score, including swing. Must agree with
    /// the timing the audio engine builds from [`Self::playback_options`],
    /// including the tempo ramp of the bar being played.
    pub(crate) fn score_timing(&self) -> ScoreTiming {
        ScoreTiming::from_score(&self.editor.score, self.current_bpm())
            .with_swing(self.playback_ctl.audio_cfg.swing)
    }

    /// Tempo being played: the ramp tempo while a ramp runs, else the BPM setting.
    pub(crate) fn current_bpm(&self) -> u32 {
        self.playback_ctl.playback.ramp_bpm.unwrap_or(self.playback_ctl.bpm)
    }

    fn clear_accuracy_for_edit(&mut self) {
        self.playback_ctl.accuracy.tracker.clear_for_edit();
    }

    pub(crate) fn current_measure(&self) -> &Measure {
        self.editor.score.current(self.editor.cursor.measure_idx)
    }

    pub(crate) fn current_measure_mut(&mut self) -> &mut Measure {
        self.editor.score.current_mut(self.editor.cursor.measure_idx)
    }

    fn current_snapshot(&self) -> EditorSnapshot {
        EditorSnapshot {
            score: self.editor.score.clone(),
            cursor: self.editor.cursor,
            counter: self.metronome_mode().then(|| self.playback_ctl.audio_cfg.counter.clone()),
            click: self.metronome_mode().then(|| self.playback_ctl.audio_cfg.click.clone()),
            voice: self.edited_voice(),
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.editor.history.can_undo()
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.editor.history.can_redo()
    }

    /// Snapshot the current state, run `op`, and commit only if it reports a change.
    /// On commit, clears redo and accuracy edit state. On no-op, the snapshot is discarded.
    pub(crate) fn with_undo_snapshot<F>(&mut self, op: F) -> bool
    where
        F: FnOnce(&mut Self) -> bool,
    {
        let snap = self.current_snapshot();
        if op(self) {
            self.editor.history.push(snap);
            self.clear_accuracy_for_edit();
            self.editor.dirty = true;
            true
        } else {
            false
        }
    }

    fn undo(&mut self) {
        // The bar being edited goes back to its voice first: the restored
        // snapshot may hold the other one.
        self.sync_click_bar();
        let current = self.current_snapshot();
        if let Some(prev) = self.editor.history.pop_undo(current) {
            self.editor.score = prev.score;
            self.editor.cursor = prev.cursor;
            if let Some(counter) = prev.counter {
                self.playback_ctl.audio_cfg.counter = counter;
                self.editor.metronome.voice = prev.voice;
            }
            if let Some(click) = prev.click {
                self.playback_ctl.audio_cfg.click = click;
            }
            self.clear_accuracy_for_edit();
            self.editor.dirty = true;
        }
    }

    fn redo(&mut self) {
        // The bar being edited goes back to its voice first: the restored
        // snapshot may hold the other one.
        self.sync_click_bar();
        let current = self.current_snapshot();
        if let Some(next) = self.editor.history.pop_redo(current) {
            self.editor.score = next.score;
            self.editor.cursor = next.cursor;
            if let Some(counter) = next.counter {
                self.playback_ctl.audio_cfg.counter = counter;
                self.editor.metronome.voice = next.voice;
            }
            if let Some(click) = next.click {
                self.playback_ctl.audio_cfg.click = click;
            }
            self.clear_accuracy_for_edit();
            self.editor.dirty = true;
        }
    }

    fn set_beat(
        &mut self,
        idx: usize,
        base: NoteValue,
        beat_kind: Option<BeatKind>,
    ) -> Option<Modification> {
        let result = self.current_measure_mut().modify_beat(idx, base, beat_kind);
        if result.is_some() {
            let new_len = self.current_measure().beats().len();
            if new_len > 0 {
                let last = new_len - 1;
                if self.editor.cursor.beat_idx < last {
                    self.editor.cursor.beat_idx += 1;
                }
            }
        }

        result
    }

    fn set_tuplet(&mut self, idx: usize, tuplet_spec: Option<TupletSpec>) -> Option<Modification> {
        let result = self.current_measure_mut().set_tuplet(idx, tuplet_spec, true);
        match &result {
            Some(Modification::DissolveTuplet(tuplet_idx, _)) => {
                let new_len = self.current_measure().beats().len();
                if new_len > 0 {
                    self.editor.cursor.beat_idx = tuplet_idx.start_idx.min(new_len - 1);
                } else {
                    self.editor.cursor.beat_idx = 0;
                }
            }
            Some(Modification::SetTuplet(group_span, ..)) => {
                self.editor.cursor.beat_idx =
                    (group_span.end_idx + 1).min(self.current_measure().beats().len() - 1);
            }
            _ => {}
        }
        result
    }

    /// Append a new empty measure after the current one. The new measure
    /// inherits the time signature of the active measure. Moves the cursor to
    /// the start of the new measure.
    pub(crate) fn append_measure(&mut self) {
        self.with_undo_snapshot(|app| {
            let ts = app.current_measure().time_signature();
            let new_measure = Measure::new(ts);
            let insert_at = app.editor.cursor.measure_idx + 1;
            app.editor.score.measures.insert(insert_at, new_measure);
            app.editor.cursor.measure_idx = insert_at;
            app.editor.cursor.beat_idx = 0;
            true
        });
    }

    /// Remove the currently active measure. Does nothing if only one measure
    /// remains (Score invariant: at least one measure).
    pub(crate) fn remove_current_measure(&mut self) {
        if self.editor.score.len() <= 1 {
            return;
        }
        self.with_undo_snapshot(|app| {
            let idx = app.editor.cursor.measure_idx;
            app.editor.score.measures.remove(idx);
            if app.editor.cursor.measure_idx >= app.editor.score.len() {
                app.editor.cursor.measure_idx = app.editor.score.len() - 1;
            }
            let new_len = app.editor.score.measures[app.editor.cursor.measure_idx].beats().len();
            if new_len == 0 {
                app.editor.cursor.beat_idx = 0;
            } else if app.editor.cursor.beat_idx >= new_len {
                app.editor.cursor.beat_idx = new_len - 1;
            }
            true
        });
    }

    /// "Save": if a pattern is active, overwrite its score + tempo in place
    /// (keeping id and name). Otherwise create a new entry from `name` and make
    /// it active. An empty name falls back to a numbered default.
    pub(crate) fn save_active_pattern(&mut self, name: String) {
        let score = self.editor.score.clone();
        let bpm = self.playback_ctl.bpm;
        match self.editor.active_pattern_id {
            Some(id) if self.editor.library.contains(id) => {
                self.editor.library.update(id, score, bpm);
            }
            _ => {
                let id = self.editor.library.add(
                    Self::pattern_name_or_default(&name, &self.editor.library),
                    score,
                    bpm,
                );
                self.editor.active_pattern_id = Some(id);
            }
        }
        self.editor.dirty = false;
    }

    /// "Save as": always create a new library entry from `name` and make it the
    /// active pattern. An empty name falls back to a numbered default.
    pub(crate) fn save_pattern_as(&mut self, name: String) {
        let score = self.editor.score.clone();
        let bpm = self.playback_ctl.bpm;
        let name = Self::pattern_name_or_default(&name, &self.editor.library);
        let id = self.editor.library.add(name, score, bpm);
        self.editor.active_pattern_id = Some(id);
        self.editor.dirty = false;
    }

    fn pattern_name_or_default(name: &str, library: &PatternLibrary) -> String {
        if name.trim().is_empty() {
            format!("Pattern {}", library.patterns.len() + 1)
        } else {
            name.trim().to_string()
        }
    }

    /// Replace the current score + tempo with the saved pattern `id`. The score
    /// swap is undoable; the tempo change is applied directly (tempo is not part
    /// of the undo snapshot). Transport is stopped so playback restarts cleanly.
    pub(crate) fn load_pattern(&mut self, id: u64) {
        let Some(pattern) = self.editor.library.get(id) else {
            return;
        };
        let score = pattern.score.clone();
        let bpm = pattern.bpm;

        self.stop_transport();
        self.with_undo_snapshot(|app| {
            app.editor.score = score;
            // measure_idx 0 is valid by Score's non-empty invariant; start at the
            // beginning and clamp the beat index to the first measure.
            let mut cursor = Cursor::start();
            let beats_len = app.editor.score.current(0).beats().len();
            if beats_len > 0 {
                cursor.beat_idx = cursor.beat_idx.min(beats_len - 1);
            }
            app.editor.cursor = cursor;
            true
        });
        self.playback_ctl.bpm = bpm;
        self.playback_ctl.playback.reset();
        self.editor.active_pattern_id = Some(id);
        self.editor.dirty = false;
    }

    /// Entry point for the load button: if the working score has unsaved changes,
    /// defer to the confirmation dialog; otherwise load immediately.
    pub(crate) fn request_load_pattern(&mut self, id: u64) {
        if self.editor.active_pattern_id == Some(id) && !self.editor.dirty {
            return; // already loaded and unchanged
        }
        if self.editor.dirty {
            self.ui.pending_load = Some(PendingLoad::Pattern(id));
        } else {
            self.load_pattern(id);
        }
    }

    pub(crate) fn delete_pattern(&mut self, id: u64) {
        self.editor.library.remove(id);
        if self.editor.active_pattern_id == Some(id) {
            self.editor.active_pattern_id = None;
        }
        if matches!(self.ui.pending_load, Some(PendingLoad::Pattern(p)) if p == id) {
            self.ui.pending_load = None;
        }
    }

    /// Open a shared link. Replacing the working score asks first if it has
    /// unsaved changes; a tempo-only link applies right away.
    pub(crate) fn open_link(&mut self, link: SharedLink) {
        // A metronome link leaves the rhythm alone: nothing to lose.
        let replaces_rhythm =
            link.content.as_ref().is_some_and(|c| !matches!(c, LinkContent::Metronome(_)));
        if replaces_rhythm && self.editor.dirty {
            self.ui.pending_load = Some(PendingLoad::Link(Box::new(link)));
        } else {
            self.apply_link(link);
        }
    }

    pub(crate) fn apply_link(&mut self, link: SharedLink) {
        // A rhythm link puts the metronome's click bar away first; a
        // metronome link reopens it below with its meter and click bar.
        if self.metronome_mode() {
            self.stop_transport();
            self.close_metronome();
            self.ui.mode = Mode::Playback;
        }
        let metronome = matches!(link.content, Some(LinkContent::Metronome(_)));
        match link.content {
            Some(LinkContent::Score(score)) => self.replace_score_from_link(score),
            Some(LinkContent::Metronome(ts)) => {
                self.stop_transport();
                self.editor.metronome.meter = ts;
                // The meter plays the link's counter rhythm, or none.
                self.playback_ctl.audio_cfg.counter.replace(ts, link.counter.clone());
            }
            Some(LinkContent::Generator { settings, seed }) => {
                self.stop_transport();
                self.editor.generator.settings = GeneratorSettings {
                    custom_patterns: self.editor.generator.settings.custom_patterns,
                    ..settings
                };
                self.generate_new_score_with_seed(seed);
                self.ui.mode = Mode::Generator;
            }
            None => {}
        }
        if let Some(bpm) = link.bpm {
            self.playback_ctl.bpm = bpm;
            self.handle_bpm_change();
        }
        if let Some(swing) = link.swing {
            self.playback_ctl.audio_cfg.swing = swing;
        }
        if let Some(bar) = link.click {
            self.playback_ctl.audio_cfg.click.set(bar);
        }
        self.playback_ctl.playback.reset();
        if metronome {
            self.open_metronome();
            self.ui.mode = Mode::Playback;
        } else {
            // The content is reproducible from the link, so it is not "unsaved".
            self.editor.dirty = false;
        }
    }

    fn replace_score_from_link(&mut self, score: Score) {
        self.stop_transport();
        self.with_undo_snapshot(|app| {
            app.editor.score = score;
            app.editor.cursor = Cursor::start();
            true
        });
        self.editor.active_pattern_id = None;
    }

    /// Run a pending load after the unsaved-changes dialog.
    pub(crate) fn finish_pending_load(&mut self, pending: PendingLoad) {
        match pending {
            PendingLoad::Pattern(id) => self.load_pattern(id),
            PendingLoad::Link(link) => self.apply_link(*link),
        }
    }

    fn build_button_measure(template: BeatTemplate) -> Measure {
        let beat_count =
            if let Duration::Tuplet(TupletSpec { m, .. }) = template.duration { m } else { 2 };

        let mut measure = Measure::new_init(
            TimeSignature::new(beat_count, template.duration.base_note().denominator()),
            template.kind,
        );

        if let Duration::Tuplet(TupletSpec { n, .. }) = template.duration {
            for i in 0..n {
                measure.set_beat(i as BeatIdx, Beat::note(template.duration)).unwrap();
            }
        }

        if let Duration::Dotted { .. } = template.duration {
            measure.toggle_dotted(0);
        }

        if let Duration::Simple(..) = template.duration
            && template.accented
        {
            measure.toggle_accent(0);
        }

        if let Duration::Simple(..) = template.duration
            && template.soft
        {
            measure.toggle_soft(0);
        }

        if let Duration::Tuplet(..) = template.duration {
            //
        } else {
            measure.delete_beat(1);
        }

        measure
    }

    /// Give the app a host's share sheet and permissions (Android).
    pub fn set_host(&mut self, host: Box<dyn Host>) {
        self.ui.host = Some(host);
    }

    /// The system back button: close what is open on top, one step at a
    /// time. `false` when nothing is, so the host can leave the app.
    pub fn back(&mut self) -> bool {
        // Calibration and the run summary float above every page.
        if let Some(cal) = &self.playback_ctl.play_along.calibration {
            if matches!(cal.phase, CalibrationPhase::Done { .. }) {
                self.playback_ctl.play_along.calibration = None;
            } else {
                // Cancel, which also puts the score back.
                self.stop_transport();
            }
            return true;
        }
        if self.playback_ctl.transport_state == TransportState::Stopped
            && self.playback_ctl.accuracy.tracker.last_summary().is_some()
        {
            self.playback_ctl.accuracy.tracker.dismiss_summary();
            return true;
        }
        let practice_page = self.compact_tab() == CompactTab::Practice;
        let metronome = self.metronome_mode();
        let ui = &mut self.ui;
        if ui.link_error.take().is_some()
            || ui.pending_load.take().is_some()
            || ui.renaming_preset.take().is_some()
        {
            return true;
        }
        if ui.picker.take().is_some() {
            // As when the picker closes itself.
            ui.tap_tempo.reset();
            return true;
        }
        ui.mode = match ui.mode {
            Mode::TimeSignature(_) => Mode::Edit,
            Mode::Mixer | Mode::Settings | Mode::Help | Mode::Library if ui.narrow_screen => {
                Mode::MoreMenu
            }
            Mode::Mixer | Mode::Settings | Mode::Help | Mode::Library => Mode::Playback,
            // The metronome's click bar editor ("Done").
            Mode::Edit if metronome => Mode::Playback,
            // The practice page's options sheet, when unfolded.
            _ if ui.narrow_screen && practice_page && !ui.compact.options_collapsed => {
                ui.compact.options_collapsed = true;
                return true;
            }
            _ => return false,
        };
        true
    }

    /// The app went to the background: stop playing and let go of the audio
    /// device, as the web page does when its tab is hidden, and of the
    /// microphone, which no frame would close while hidden.
    pub fn hidden(&mut self) {
        self.stop_transport();
        self.playback_ctl.audio = None;
        self.playback_ctl.mic.input = None;
    }

    fn toggle_mode(&mut self, mode: Mode) {
        if self.ui.mode == mode {
            self.ui.mode = Mode::Playback;
        } else {
            self.ui.mode = mode
        }
    }

    fn audio_state(&self) -> PlayerState {
        if self.playback_ctl.transport_state == TransportState::Playing {
            PlayerState::Playing
        } else {
            PlayerState::Stopped
        }
    }

    fn start_playback(&mut self) {
        if self.playback_ctl.transport_state == TransportState::Playing {
            return;
        }
        self.playback_ctl.transport_state = TransportState::Playing;
        let midi_connected =
            self.playback_ctl.midi.input.as_ref().is_some_and(|input| input.is_connected());
        let accuracy_start = midi_connected.then(clock::now_seconds);
        self.playback_ctl.play_along.engaged = false;
        self.playback_ctl.accuracy.tracker.dismiss_summary();
        // The metronome plays no rhythm to play along with.
        if self.playback_ctl.accuracy.enabled && !self.metronome_mode() {
            self.playback_ctl.accuracy.tracker.on_playback_start(accuracy_start);
        } else {
            self.playback_ctl.accuracy.tracker.on_playback_stop();
        }
        self.playback_ctl.playback.reset();
        // Mark the count-in as pending until the UI clock is known (first
        // frame of the measure panel fills in `since`).
        if self.playback_ctl.audio_cfg.count_in {
            self.playback_ctl.playback.count_in = CountInState::Waiting { since: f64::NAN };
        }

        self.ui.platform.acquire_wake_lock();
    }

    fn stop_transport(&mut self) {
        if self.playback_ctl.transport_state == TransportState::Stopped {
            return;
        }
        self.playback_ctl.transport_state = TransportState::Stopped;
        if self.playback_ctl.audio_cfg.ramp.keep_reached
            && let Some(bpm) = self.playback_ctl.playback.ramp_bpm
            && bpm != self.playback_ctl.bpm
            && self.playback_ctl.play_along.calibration.is_none()
        {
            self.playback_ctl.bpm = bpm;
            self.editor.dirty = true;
        }
        if self.playback_ctl.play_along.calibration.is_some() {
            self.playback_ctl.accuracy.tracker.discard_session();
        } else {
            self.playback_ctl.accuracy.tracker.on_playback_stop();
        }
        self.playback_ctl.play_along.engaged = false;
        self.playback_ctl.playback.reset();
        self.end_calibration_run();

        self.ui.platform.release_wake_lock();
    }

    pub fn toggle_playback(&mut self) {
        match self.playback_ctl.transport_state {
            TransportState::Stopped => self.start_playback(),
            TransportState::Playing => self.stop_transport(),
        }
        info!("Toggle playback: {:?}", self.playback_ctl.transport_state);
    }

    fn set_accuracy_enabled(&mut self, enabled: bool) {
        let transport = self.playback_ctl.transport_state;
        self.playback_ctl.accuracy.set_enabled(enabled, transport);
    }

    fn handle_bpm_change(&mut self) {
        self.realign_accuracy_clock();
    }

    /// Re-anchor the accuracy session to the current playback position, e.g.
    /// after a tempo change or when the count-in bar ends.
    pub(crate) fn realign_accuracy_clock(&mut self) {
        if self.playback_ctl.transport_state != TransportState::Playing {
            return;
        }
        if !self.playback_ctl.accuracy.tracker.has_start_time() {
            return;
        }

        let timing = self.score_timing();
        if timing.total_loop_seconds() <= 0.0 {
            return;
        }

        let now_seconds = clock::now_seconds();
        let start_time =
            now_seconds - timing.global_tick_to_seconds(self.playback_ctl.playback.smooth_tick);
        self.playback_ctl
            .accuracy
            .tracker
            .realign_start_time(start_time, self.playback_ctl.playback.smooth_tick);
    }

    fn default_measure() -> Measure {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.set_beat(0, Beat::note(q())).unwrap();
        m.set_beat(1, Beat::note(q())).unwrap();
        m.set_beat(2, Beat::note(q())).unwrap();
        m.set_beat(3, Beat::note(q())).unwrap();
        m
    }

    fn build_count_config(&self) -> Option<CountConfig> {
        let mut layers = Vec::new();
        let mut next_id = 1u32;

        let base_layer = self.ui.counting.base.layer(next_id);

        if let Some(layer) = base_layer {
            layers.push(layer);
            next_id = next_id.saturating_add(1);
        }

        if self.ui.counting.show_tuplets {
            let mut layer = CountLayer::new(next_id, CountScope::TupletAll, Subdiv::TupletN);
            layer.labels = Some(LabelPattern::triplet());
            layer.priority = 10;
            layers.push(layer);
        }

        if layers.is_empty() { None } else { Some(CountConfig::new(layers)) }
    }

    pub fn new(cc: &CreationContext) -> Self {
        add_font(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let ff = FontFamily::Name("music".into());

        let mut state = cc
            .storage
            .and_then(|storage| eframe::get_value::<PersistedState>(storage, APP_STATE_KEY))
            .filter(|s| s.version == PERSISTED_STATE_VERSION)
            .unwrap_or_default();

        if state.score.is_empty() {
            state.score = Score::single(Self::default_measure());
            state.cursor = Cursor::start();
        } else {
            let measure_count = state.score.len();
            if state.cursor.measure_idx >= measure_count {
                state.cursor.measure_idx = measure_count - 1;
            }
            let beats_len = state.score.current(state.cursor.measure_idx).beats().len();
            if beats_len == 0 {
                state.score.measures[state.cursor.measure_idx] = Self::default_measure();
                state.cursor.beat_idx = 0;
            } else {
                state.cursor.beat_idx = state.cursor.beat_idx.min(beats_len - 1);
            }
        }

        state.audio_settings = state.audio_settings.clamped();

        // Precompute button measures for all insert-beat tools
        let mut button_measures: HashMap<&'static str, Measure> = HashMap::new();
        for t in all_tools() {
            match t.kind {
                ToolKind::InsertBeat(template) => {
                    button_measures.insert(t.id, Self::build_button_measure(template));
                }
                ToolKind::Modify(Modifier::ToggleDotted { dots }) => {
                    button_measures.insert(
                        t.id,
                        Self::build_button_measure(BeatTemplate {
                            kind: BeatKind::Note,
                            duration: Duration::Dotted { dots, base: Quarter },
                            accented: false,
                            soft: false,
                        }),
                    );
                }
                ToolKind::Modify(Modifier::ToggleAccent) => {
                    button_measures.insert(
                        t.id,
                        Self::build_button_measure(BeatTemplate {
                            kind: BeatKind::Note,
                            duration: Duration::Simple(Quarter),
                            accented: true,
                            soft: false,
                        }),
                    );
                }
                ToolKind::Modify(Modifier::ToggleSoft) => {
                    button_measures.insert(
                        t.id,
                        Self::build_button_measure(BeatTemplate {
                            kind: BeatKind::Note,
                            duration: Duration::Simple(Quarter),
                            accented: false,
                            soft: true,
                        }),
                    );
                }
                _ => {}
            }
        }

        let (mut midi_input, midi_input_ports) = match MidiInput::new() {
            Ok(mut input) => {
                let ports = input.available_ports().unwrap_or_default();
                (Some(input), ports)
            }
            Err(err) => {
                warn!("Failed to initialize MIDI input: {:?}", err);
                (None, Vec::new())
            }
        };

        if let Some(ref mut input) = midi_input {
            let ctx = cc.egui_ctx.clone();
            input.set_event_notifier(Some(Arc::new(move || ctx.request_repaint())));
        }

        let midi_selected_port_id = state.midi_selected_port_id.clone();

        if let (Some(input), Some(port_id)) = (midi_input.as_mut(), midi_selected_port_id.as_ref())
            && let Some(idx) = input.find_port_index_by_id(port_id)
        {
            let _ = input.connect(idx);
        }

        let platform = PlatformRuntime::new();
        platform.install_listeners(cc.egui_ctx.clone());
        update_check::start(cc.egui_ctx.clone());

        let mut app = Self {
            editor: EditorState {
                score: state.score,
                cursor: state.cursor,
                history: UndoHistory::new(DEFAULT_UNDO_LIMIT),
                button_measures,
                active_pattern_id: state.active_pattern_id.filter(|id| state.library.contains(*id)),
                dirty: state.dirty,
                library: state.library,
                presets: state.presets,
                active_preset: None,
                metronome: MetronomeState::new(state.metronome_meter),
                generator: GeneratorState::new(
                    state.generator_settings,
                    state.endless,
                    state.ghost_notes,
                ),
            },
            playback_ctl: PlaybackController {
                transport_state: TransportState::Stopped,
                bpm: state.bpm,
                audio: None,
                audio_cfg: AudioConfig {
                    settings: state.audio_settings,
                    offset: state.audio_offset,
                    latency_enabled: state.audio_latency_enabled,
                    count_in: state.count_in,
                    swing: state.swing,
                    gap: state.gap_click,
                    ramp: state.tempo_ramp,
                    click: state.click_bars,
                    counter: state.counter_bars.with_names(&state.counter_names),
                },
                playback: PlaybackState::default(),
                accuracy: AccuracyState::new(state.accuracy_enabled),
                midi: MidiState {
                    input: midi_input,
                    available_ports: midi_input_ports,
                    selected_port_id: midi_selected_port_id,
                },
                mic: MicState::default(),
                play_along: PlayAlongState { settings: state.play_along, ..Default::default() },
            },
            ui: UiShell {
                mode: Mode::Playback,
                music_font_id: FontId::new(16.0, ff),
                font_bump: 8.0,
                baseline_dark: None,
                baseline_light: None,
                layout: LayoutSettings::default(),
                counting: state.counting,
                flash_enabled: state.flash_enabled,
                platform,
                save_name_buffer: String::new(),
                preset_name_buffer: String::new(),
                renaming_preset: None,
                pending_load: None,
                link_error: None,
                link_copied_at: None,
                narrow_screen: false,
                staff_em: 40.0,
                followed_cursor: None,
                compact: CompactState::default(),
                picker: None,
                tap_tempo: Default::default(),
                host: None,
            },
        };
        app.editor.active_preset = app.matching_preset();
        // A link opens after the saved tab, so its tab wins.
        app.select_compact_tab(state.tab);
        app.open_link_from_url();
        app
    }

    /// Open the link the app was started with (web only) and drop its
    /// parameters from the address bar, so a reload keeps later edits.
    fn open_link_from_url(&mut self) {
        let Some(query) = platform::link_query() else {
            return;
        };
        match grooph_link::parse_query(&query) {
            Ok(None) => return,
            Ok(Some(link)) => self.open_link(link),
            Err(err) => {
                warn!("Cannot open link: {err}");
                self.ui.link_error = Some(err);
            }
        }
        platform::clear_link_query();
    }
}

#[cfg(test)]
mod tests {
    use super::{CountingBase, CountingSettings, PersistedState};
    use crate::compact::CompactTab;

    #[test]
    fn back_closes_one_layer_at_a_time() {
        use super::{Grooph, Mode};
        use crate::pickers::Picker;

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.ui.narrow_screen = true;
        app.select_compact_tab(CompactTab::More);
        app.ui.mode = Mode::Settings;
        app.ui.picker = Some(Picker::Bpm);
        assert!(app.back());
        assert_eq!(app.ui.picker, None);
        assert_eq!(app.ui.mode, Mode::Settings);
        assert!(app.back());
        assert_eq!(app.ui.mode, Mode::MoreMenu);
        // Nothing left to close: the host leaves the app. The practice
        // options sheet is not on this page, so it does not count.
        app.ui.compact.options_collapsed = false;
        assert!(!app.back());
        assert_eq!(app.ui.mode, Mode::MoreMenu);
        // On the practice page, back folds the sheet first.
        app.select_compact_tab(CompactTab::Practice);
        assert!(app.back());
        assert!(app.ui.compact.options_collapsed);
        assert!(!app.back());
    }

    fn load(saved: &str) -> CountingSettings {
        ron::from_str(saved).unwrap()
    }

    #[test]
    fn counting_overlay_is_off_by_default() {
        let c = CountingSettings::default();
        assert!(c.base == CountingBase::Off && !c.show_tuplets);
    }

    #[test]
    fn fresh_practice_has_no_space_and_no_timing_feedback() {
        let s = PersistedState::default();
        assert_eq!(s.generator_settings.space, 0.0);
        assert!(!s.accuracy_enabled);
    }

    #[test]
    fn saved_tab_round_trips_and_older_states_open_practice() {
        let s = PersistedState { tab: CompactTab::Metronome, ..Default::default() };
        let back: PersistedState = ron::from_str(&ron::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.tab, CompactTab::Metronome);
        let older: PersistedState = ron::from_str("(version: 2)").unwrap();
        assert_eq!(older.tab, CompactTab::Practice);
    }

    #[test]
    fn counting_settings_round_trip() {
        let c = CountingSettings { base: CountingBase::Sixteenth, show_tuplets: true };
        let back = load(&ron::to_string(&c).unwrap());
        assert!(back.base == CountingBase::Sixteenth && back.show_tuplets);
    }

    #[test]
    fn older_counting_switches_still_hide_the_overlay() {
        let on = load("(enabled: true, show_labels: true, base: Ands, show_tuplets: true)");
        assert!(on.base == CountingBase::Ands && on.show_tuplets);
        for hidden in [
            "(enabled: false, show_labels: true, base: Ands, show_tuplets: true)",
            "(enabled: true, show_labels: false, base: Ands, show_tuplets: true)",
        ] {
            let c = load(hidden);
            assert!(c.base == CountingBase::Off && !c.show_tuplets, "{hidden}");
        }
    }

    #[test]
    fn generator_settings_with_an_old_figure_selection_still_load() {
        use grooph_measure::generator::GeneratorSettings;
        let settings = GeneratorSettings { complexity: 4, ..GeneratorSettings::default() };
        // A selection stored by figure ids before the computed patterns (and
        // the switch from before the figures rename) loads without it.
        let stored = ron::to_string(&settings).unwrap();
        for old in ["custom_figures: (5)", "custom_groupings: (5)"] {
            let stored = stored.replacen("(", &format!("({old}, "), 1);
            let back: GeneratorSettings = ron::from_str(&stored).unwrap();
            assert_eq!(back.complexity, 4, "{stored}");
            assert!(back.custom_patterns.is_empty(), "{stored}");
        }
    }
}
