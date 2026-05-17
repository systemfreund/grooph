mod accuracy;
mod help;
mod keyboard_input;
mod main_menu;
mod measure_panel;
mod midi_input_widget;
mod mixer_panel;
mod platform;
mod settings_panel;
mod state;
mod style;
mod time_signature_dialog;
mod tool_palette;
pub mod tools;
mod undo;
#[cfg(target_arch = "wasm32")]
mod web;

use grooph_measure::duration::{Duration, NoteValue, TupletSpec, q};
use grooph_measure::{BeatIdx, Cursor, Measure, Score, TimeSignature};

use crate::accuracy::AccuracyState;
use crate::platform::{PlatformRuntime, VisibilityEvent};
use crate::state::{
    AudioConfig, EditorState, LayoutSettings, MidiState, PlaybackController, PlaybackState,
    UiShell,
};
use grooph_measure::tempo::ScoreTiming;
use crate::tools::ToolKind;
use crate::tools::{BeatTemplate, Modifier, all_tools};
use crate::undo::{DEFAULT_UNDO_LIMIT, EditorSnapshot, UndoHistory};
use eframe::egui::{Context, TextStyle, Ui, Widget};
use eframe::epaint::text::{FontInsert, InsertFontFamily};
use eframe::epaint::{FontFamily, FontId};
use eframe::{App, CreationContext, egui};
use grooph_audio::{AudioSettings, PlayerState};
use grooph_measure::counting::{
    ColorId, ColorMode, ColorPattern, CountConfig, CountLayer, CountScope, LabelPattern,
    LabelToken, Subdiv,
};
use grooph_measure::duration::NoteValue::*;
use grooph_measure::editing::Modification;
use grooph_measure::{Beat, BeatKind};
use grooph_midi::{MidiInput, MidiInputEvent};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(PartialEq, Eq)]
pub(crate) enum Mode {
    Edit,
    Playback,
    Mixer,
    Settings,
    Help,
    TimeSignature { beats: u8, unit: u8 },
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
    midi_selected_port_id: Option<String>,
    accuracy_enabled: bool,
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
            midi_selected_port_id: None,
            accuracy_enabled: true,
        }
    }
}

impl PersistedState {
    fn from_app(app: &Grooph) -> Self {
        Self {
            version: PERSISTED_STATE_VERSION,
            score: app.editor.score.clone(),
            cursor: app.editor.cursor,
            bpm: app.playback_ctl.bpm,
            audio_settings: app.playback_ctl.audio_cfg.settings,
            audio_latency_enabled: app.playback_ctl.audio_cfg.latency_enabled,
            audio_offset: app.playback_ctl.audio_cfg.offset,
            counting: app.ui.counting,
            midi_selected_port_id: app.playback_ctl.midi.selected_port_id.clone(),
            accuracy_enabled: app.playback_ctl.accuracy.enabled,
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

#[derive(Serialize, Deserialize, Clone, Copy)]
struct CountingSettings {
    enabled: bool,
    show_colors: bool,
    show_labels: bool,
    base: CountingBase,
    show_tuplets: bool,
}

impl Default for CountingSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            show_colors: false,
            show_labels: true,
            base: CountingBase::Ands,
            show_tuplets: false,
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
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.apply_style(ui);
        self.main_menu(ui);
        self.help_panel(ui);
        self.settings_panel(ui);
        self.mixer_panel(ui);
        self.tool_palette_panel(ui);
        self.measure_panel(ui);

        if matches!(self.ui.mode, Mode::TimeSignature { .. }) {
            self.time_signature_dialog(ui);
        }

        self.handle_keyboard_input(ui);
        self.handle_midi_input_events();

        if let Some(ev) = self.ui.platform.take_visibility_event() {
            match ev {
                VisibilityEvent::Hidden => {
                    self.stop_transport();
                    self.playback_ctl.audio = None;
                }
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
        }

        if let Some(audio) = &mut self.playback_ctl.audio {
            audio.set_audio_settings(self.playback_ctl.audio_cfg.settings);
            if audio.update(&audio_state, self.playback_ctl.bpm, &self.editor.score) {
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
    fn handle_midi_input_events(&mut self) {
        let (events, now_seconds, is_connected) = match self.playback_ctl.midi.input.as_ref() {
            Some(input) => (input.drain_events(), input.now_seconds(), input.is_connected()),
            None => return,
        };

        if !self.playback_ctl.accuracy.enabled {
            return;
        }

        let timing = ScoreTiming::from_score(&self.editor.score, self.playback_ctl.bpm);
        let total_loop_seconds = timing.total_loop_seconds();

        if self.playback_ctl.transport_state == TransportState::Playing
            && is_connected
            && !self.playback_ctl.accuracy.tracker.has_start_time()
        {
            let (start_time, last_tick) = if total_loop_seconds > 0.0 {
                (
                    now_seconds
                        - timing.global_tick_to_seconds(self.playback_ctl.playback.smooth_tick),
                    self.playback_ctl.playback.smooth_tick,
                )
            } else {
                (now_seconds, 0.0)
            };
            self.playback_ctl.accuracy.tracker.on_playback_start_at(start_time, last_tick);
        }

        let accuracy_active = self.playback_ctl.accuracy.tracker.update_state(
            self.playback_ctl.transport_state == TransportState::Playing,
            is_connected,
            now_seconds,
        );
        let ready = accuracy_active
            && total_loop_seconds > 0.0
            && !self.editor.score.measures.is_empty();

        for event in events {
            match event {
                MidiInputEvent::NoteOn { channel, note, velocity, timestamp } => {
                    if ready {
                        self.playback_ctl.accuracy.tracker.record_hit(
                            timestamp,
                            &timing,
                            &self.editor.score,
                        );
                    }
                    debug!("MIDI NoteOn ch={} note={} vel={}", channel, note, velocity);
                }
                MidiInputEvent::NoteOff { channel, note, velocity, .. } => {
                    debug!("MIDI NoteOff ch={} note={} vel={}", channel, note, velocity);
                }
                MidiInputEvent::ControlChange { .. } => {}
            }
        }

        if ready {
            self.playback_ctl.accuracy.tracker.update_progress(
                now_seconds,
                &timing,
                &self.editor.score,
            );
        }
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
        EditorSnapshot { score: self.editor.score.clone(), cursor: self.editor.cursor }
    }

    pub(crate) fn can_undo(&self) -> bool { self.editor.history.can_undo() }

    pub(crate) fn can_redo(&self) -> bool { self.editor.history.can_redo() }

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
            true
        } else {
            false
        }
    }

    fn undo(&mut self) {
        let current = self.current_snapshot();
        if let Some(prev) = self.editor.history.pop_undo(current) {
            self.editor.score = prev.score;
            self.editor.cursor = prev.cursor;
            self.clear_accuracy_for_edit();
        }
    }

    fn redo(&mut self) {
        let current = self.current_snapshot();
        if let Some(next) = self.editor.history.pop_redo(current) {
            self.editor.score = next.score;
            self.editor.cursor = next.cursor;
            self.clear_accuracy_for_edit();
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
            let new_len =
                app.editor.score.measures[app.editor.cursor.measure_idx].beats().len();
            if new_len == 0 {
                app.editor.cursor.beat_idx = 0;
            } else if app.editor.cursor.beat_idx >= new_len {
                app.editor.cursor.beat_idx = new_len - 1;
            }
            true
        });
    }

    fn build_button_measure(template: BeatTemplate) -> Measure {
        let beat_count =
            if let Duration::Tuplet(TupletSpec { m, .. }) = template.duration { m } else { 2 };

        let mut measure = Measure::new_init(
            TimeSignature {
                beats: beat_count,
                beat_unit: template.duration.base_note().denominator(),
            },
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

        if let Duration::Tuplet(..) = template.duration {
            //
        } else {
            measure.delete_beat(1);
        }

        measure
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
        let accuracy_start =
            self.playback_ctl.midi.input.as_ref().and_then(|input| {
                if input.is_connected() { Some(input.now_seconds()) } else { None }
            });
        if self.playback_ctl.accuracy.enabled {
            self.playback_ctl.accuracy.tracker.on_playback_start(accuracy_start);
        } else {
            self.playback_ctl.accuracy.tracker.on_playback_stop();
        }
        self.playback_ctl.playback.reset();

        self.ui.platform.acquire_wake_lock();
    }

    fn stop_transport(&mut self) {
        if self.playback_ctl.transport_state == TransportState::Stopped {
            return;
        }
        self.playback_ctl.transport_state = TransportState::Stopped;
        self.playback_ctl.accuracy.tracker.on_playback_stop();
        self.playback_ctl.playback.reset();

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
        if !self.playback_ctl.accuracy.enabled {
            return;
        }
        if self.playback_ctl.transport_state != TransportState::Playing {
            return;
        }
        let Some(input) = self.playback_ctl.midi.input.as_ref() else {
            return;
        };
        if !input.is_connected() {
            return;
        }

        let timing = ScoreTiming::from_score(&self.editor.score, self.playback_ctl.bpm);
        if timing.total_loop_seconds() <= 0.0 {
            return;
        }

        let now_seconds = input.now_seconds();
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
        if !self.ui.counting.enabled {
            return None;
        }
        if !self.ui.counting.show_colors && !self.ui.counting.show_labels {
            return None;
        }

        let palette: Vec<ColorId> = (0u8..6).map(ColorId).collect();
        let color_pattern = if self.ui.counting.show_colors {
            Some(ColorPattern { palette: palette.clone(), mode: ColorMode::Scope })
        } else {
            None
        };

        let mut layers = Vec::new();
        let mut next_id = 1u32;

        let mut base_layer = match self.ui.counting.base {
            CountingBase::Off => None,
            CountingBase::Primary => {
                let mut layer =
                    CountLayer::new(next_id, CountScope::PrimaryGroup, Subdiv::Fixed(1));
                layer.labels = Some(LabelPattern { slots: vec![vec![LabelToken::GroupNum]] });
                Some(layer)
            }
            CountingBase::Ands => {
                let mut layer = CountLayer::new(next_id, CountScope::BeatUnit, Subdiv::Fixed(2));
                layer.labels = Some(LabelPattern::ands());
                Some(layer)
            }
            CountingBase::Sixteenth => {
                let mut layer = CountLayer::new(next_id, CountScope::BeatUnit, Subdiv::Fixed(4));
                layer.labels = Some(LabelPattern::sixteenth());
                Some(layer)
            }
            CountingBase::Triplet => {
                let mut layer =
                    CountLayer::new(next_id, CountScope::PrimaryGroup, Subdiv::Fixed(3));
                layer.labels = Some(LabelPattern::triplet());
                Some(layer)
            }
        };

        if let Some(ref mut layer) = base_layer {
            layer.show_labels = self.ui.counting.show_labels;
            layer.show_colors = self.ui.counting.show_colors;
            layer.colors = color_pattern.clone();
            layers.push(layer.clone());
            next_id = next_id.saturating_add(1);
        }

        if self.ui.counting.show_tuplets {
            let mut layer = CountLayer::new(next_id, CountScope::TupletAll, Subdiv::TupletN);
            layer.labels = Some(LabelPattern::triplet());
            layer.show_labels = self.ui.counting.show_labels;
            layer.show_colors = self.ui.counting.show_colors;
            layer.colors = color_pattern;
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

        Self {
            editor: EditorState {
                score: state.score,
                cursor: state.cursor,
                history: UndoHistory::new(DEFAULT_UNDO_LIMIT),
                button_measures,
            },
            playback_ctl: PlaybackController {
                transport_state: TransportState::Stopped,
                bpm: state.bpm,
                audio: None,
                audio_cfg: AudioConfig {
                    settings: state.audio_settings,
                    offset: state.audio_offset,
                    latency_enabled: state.audio_latency_enabled,
                },
                playback: PlaybackState::default(),
                accuracy: AccuracyState::new(state.accuracy_enabled),
                midi: MidiState {
                    input: midi_input,
                    available_ports: midi_input_ports,
                    selected_port_id: midi_selected_port_id,
                },
            },
            ui: UiShell {
                mode: Mode::Playback,
                music_font_id: FontId::new(16.0, ff),
                font_bump: 8.0,
                baseline_dark: None,
                baseline_light: None,
                layout: LayoutSettings::default(),
                counting: state.counting,
                platform,
            },
        }
    }
}
