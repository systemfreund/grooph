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
mod tempo;
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
use crate::state::{AudioConfig, LayoutSettings, MidiState, PlaybackState};
use crate::tempo::TempoMap;
use crate::tools::ToolKind;
use crate::tools::{BeatTemplate, Modifier, all_tools};
use crate::undo::{DEFAULT_UNDO_LIMIT, EditorSnapshot, UndoHistory};
use eframe::egui::{Context, TextStyle, Widget};
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
enum Mode {
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

pub struct Grooph {
    mode: Mode,
    music_font_id: FontId,
    score: Score,
    cursor: Cursor,
    // Prebuilt measures for note/rest/tuplet tool buttons to avoid per-frame reconstruction
    button_measures: HashMap<&'static str, Measure>,
    history: UndoHistory,
    // Global UI font bump configuration and per-theme baselines (so bump applies to dark & light)
    font_bump: f32,
    baseline_dark: Option<Vec<(TextStyle, f32)>>,
    baseline_light: Option<Vec<(TextStyle, f32)>>,
    transport_state: TransportState,
    bpm: u32,
    audio: Option<grooph_audio::Audio>,
    pub(crate) audio_cfg: AudioConfig,
    pub(crate) layout: LayoutSettings,
    pub(crate) playback: PlaybackState,
    pub(crate) accuracy: AccuracyState,
    pub(crate) midi: MidiState,
    counting: CountingSettings,
    platform: PlatformRuntime,
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
            score: app.score.clone(),
            cursor: app.cursor,
            bpm: app.bpm,
            audio_settings: app.audio_cfg.settings,
            audio_latency_enabled: app.audio_cfg.latency_enabled,
            audio_offset: app.audio_cfg.offset,
            counting: app.counting,
            midi_selected_port_id: app.midi.selected_port_id.clone(),
            accuracy_enabled: app.accuracy.enabled,
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
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.apply_style(ctx);
        self.main_menu(ctx);
        self.help_panel(ctx);
        self.settings_panel(ctx);
        self.mixer_panel(ctx);
        self.tool_palette_panel(ctx);
        self.measure_panel(ctx);

        if matches!(self.mode, Mode::TimeSignature { .. }) {
            self.time_signature_dialog(ctx);
        }

        self.handle_keyboard_input(ctx);
        self.handle_midi_input_events();

        if let Some(ev) = self.platform.take_visibility_event() {
            match ev {
                VisibilityEvent::Hidden => {
                    self.stop_transport();
                    self.audio = None;
                }
                VisibilityEvent::Visible | VisibilityEvent::PageShow => {
                    self.audio = None;
                }
            }
        }

        let audio_state = self.audio_state();

        // If we should be playing audio but have no audio engine, try to create it (works after user gesture on iOS)
        if audio_state == PlayerState::Playing && self.audio.is_none() {
            debug!("Creating audio engine.");
            self.audio = grooph_audio::Audio::new(self.bpm);
        }

        if let Some(audio) = &mut self.audio {
            audio.set_audio_settings(self.audio_cfg.settings);
            let active_measure = &self.score.measures[self.cursor.measure_idx];
            if audio.update(&audio_state, self.bpm, active_measure) {
                ctx.request_repaint();
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
        let (events, now_seconds, is_connected) = match self.midi.input.as_ref() {
            Some(input) => (input.drain_events(), input.now_seconds(), input.is_connected()),
            None => return,
        };

        if !self.accuracy.enabled {
            return;
        }

        let tempo = TempoMap::new(self.bpm, &self.current_measure().time_signature());

        if self.transport_state == TransportState::Playing
            && is_connected
            && !self.accuracy.tracker.has_start_time()
        {
            let (start_time, last_tick) = if tempo.ticks_per_sec > 0.0 {
                (
                    now_seconds - (self.playback.smooth_tick / tempo.ticks_per_sec),
                    self.playback.smooth_tick,
                )
            } else {
                (now_seconds, 0.0)
            };
            self.accuracy.tracker.on_playback_start_at(start_time, last_tick);
        }

        let accuracy_active = self.accuracy.tracker.update_state(
            self.transport_state == TransportState::Playing,
            is_connected,
            now_seconds,
        );
        let beats = self.score.measures[self.cursor.measure_idx].beats();
        let ready = accuracy_active && tempo.valid() && !beats.is_empty();

        for event in events {
            match event {
                MidiInputEvent::NoteOn { channel, note, velocity, timestamp } => {
                    if ready {
                        self.accuracy.tracker.record_hit(timestamp, &tempo, beats);
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
            self.accuracy.tracker.update_progress(now_seconds, &tempo, beats);
        }
    }

    fn clear_accuracy_for_edit(&mut self) { self.accuracy.tracker.clear_for_edit(); }

    pub(crate) fn current_measure(&self) -> &Measure { self.score.current(self.cursor.measure_idx) }

    pub(crate) fn current_measure_mut(&mut self) -> &mut Measure {
        self.score.current_mut(self.cursor.measure_idx)
    }

    fn current_snapshot(&self) -> EditorSnapshot {
        EditorSnapshot { score: self.score.clone(), cursor: self.cursor }
    }

    pub(crate) fn can_undo(&self) -> bool { self.history.can_undo() }

    pub(crate) fn can_redo(&self) -> bool { self.history.can_redo() }

    /// Snapshot the current state, run `op`, and commit only if it reports a change.
    /// On commit, clears redo and accuracy edit state. On no-op, the snapshot is discarded.
    pub(crate) fn with_undo_snapshot<F>(&mut self, op: F) -> bool
    where
        F: FnOnce(&mut Self) -> bool,
    {
        let snap = self.current_snapshot();
        if op(self) {
            self.history.push(snap);
            self.clear_accuracy_for_edit();
            true
        } else {
            false
        }
    }

    fn undo(&mut self) {
        let current = self.current_snapshot();
        if let Some(prev) = self.history.pop_undo(current) {
            self.score = prev.score;
            self.cursor = prev.cursor;
            self.clear_accuracy_for_edit();
        }
    }

    fn redo(&mut self) {
        let current = self.current_snapshot();
        if let Some(next) = self.history.pop_redo(current) {
            self.score = next.score;
            self.cursor = next.cursor;
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
                if self.cursor.beat_idx < last {
                    self.cursor.beat_idx += 1;
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
                    self.cursor.beat_idx = tuplet_idx.start_idx.min(new_len - 1);
                } else {
                    self.cursor.beat_idx = 0;
                }
            }
            Some(Modification::SetTuplet(group_span, ..)) => {
                self.cursor.beat_idx =
                    (group_span.end_idx + 1).min(self.current_measure().beats().len() - 1);
            }
            _ => {}
        }
        result
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
        if self.mode == mode {
            self.mode = Mode::Playback;
        } else {
            self.mode = mode
        }
    }

    fn audio_state(&self) -> PlayerState {
        if self.transport_state == TransportState::Playing {
            PlayerState::Playing
        } else {
            PlayerState::Stopped
        }
    }

    fn start_playback(&mut self) {
        if self.transport_state == TransportState::Playing {
            return;
        }
        self.transport_state = TransportState::Playing;
        let accuracy_start =
            self.midi.input.as_ref().and_then(|input| {
                if input.is_connected() { Some(input.now_seconds()) } else { None }
            });
        if self.accuracy.enabled {
            self.accuracy.tracker.on_playback_start(accuracy_start);
        } else {
            self.accuracy.tracker.on_playback_stop();
        }
        self.playback.reset();

        self.platform.acquire_wake_lock();
    }

    fn stop_transport(&mut self) {
        if self.transport_state == TransportState::Stopped {
            return;
        }
        self.transport_state = TransportState::Stopped;
        self.accuracy.tracker.on_playback_stop();
        self.playback.reset();

        self.platform.release_wake_lock();
    }

    pub fn toggle_playback(&mut self) {
        match self.transport_state {
            TransportState::Stopped => self.start_playback(),
            TransportState::Playing => self.stop_transport(),
        }
        info!("Toggle playback: {:?}", self.transport_state);
    }

    fn set_accuracy_enabled(&mut self, enabled: bool) {
        self.accuracy.set_enabled(enabled, self.transport_state);
    }

    fn handle_bpm_change(&mut self) {
        if !self.accuracy.enabled {
            return;
        }
        if self.transport_state != TransportState::Playing {
            return;
        }
        let Some(input) = self.midi.input.as_ref() else {
            return;
        };
        if !input.is_connected() {
            return;
        }

        let tempo = TempoMap::new(self.bpm, &self.current_measure().time_signature());
        if !tempo.valid() {
            return;
        }

        let now_seconds = input.now_seconds();
        let start_time = now_seconds - (self.playback.smooth_tick / tempo.ticks_per_sec);
        self.accuracy.tracker.realign_start_time(start_time, self.playback.smooth_tick);
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
        if !self.counting.enabled {
            return None;
        }
        if !self.counting.show_colors && !self.counting.show_labels {
            return None;
        }

        let palette: Vec<ColorId> = (0u8..6).map(ColorId).collect();
        let color_pattern = if self.counting.show_colors {
            Some(ColorPattern { palette: palette.clone(), mode: ColorMode::Scope })
        } else {
            None
        };

        let mut layers = Vec::new();
        let mut next_id = 1u32;

        let mut base_layer = match self.counting.base {
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
            layer.show_labels = self.counting.show_labels;
            layer.show_colors = self.counting.show_colors;
            layer.colors = color_pattern.clone();
            layers.push(layer.clone());
            next_id = next_id.saturating_add(1);
        }

        if self.counting.show_tuplets {
            let mut layer = CountLayer::new(next_id, CountScope::TupletAll, Subdiv::TupletN);
            layer.labels = Some(LabelPattern::triplet());
            layer.show_labels = self.counting.show_labels;
            layer.show_colors = self.counting.show_colors;
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
            mode: Mode::Playback,
            music_font_id: FontId::new(16.0, ff),
            score: state.score,
            cursor: state.cursor,
            button_measures,
            history: UndoHistory::new(DEFAULT_UNDO_LIMIT),
            font_bump: 8.0,
            baseline_dark: None,
            baseline_light: None,
            transport_state: TransportState::Stopped,
            bpm: state.bpm,
            audio: None,
            audio_cfg: AudioConfig {
                settings: state.audio_settings,
                offset: state.audio_offset,
                latency_enabled: state.audio_latency_enabled,
            },
            layout: LayoutSettings::default(),
            playback: PlaybackState::default(),
            accuracy: AccuracyState::new(state.accuracy_enabled),
            midi: MidiState {
                input: midi_input,
                available_ports: midi_input_ports,
                selected_port_id: midi_selected_port_id,
            },
            counting: state.counting,
            platform,
        }
    }
}
