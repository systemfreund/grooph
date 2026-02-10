mod help;
mod keyboard_input;
mod main_menu;
mod measure_panel;
mod mixer_panel;
mod settings_panel;
mod style;
mod time_signature_dialog;
mod tool_palette;
pub mod tools;
#[cfg(target_arch = "wasm32")]
mod web;

use std::cell::RefCell;
use grooph_measure::duration::{Duration, NoteValue, TupletSpec, q};
use grooph_measure::{BeatIdx, Measure, TimeSignature};
use grooph_measure::grid::DEFAULT_GRID;

use grooph_audio::{AudioSettings, PlayerState};
use grooph_midi::{MidiInput, MidiInputEvent};
use grooph_measure::duration::NoteValue::*;
use grooph_measure::editing::Modification;
use grooph_measure::{Beat, BeatKind};
use grooph_measure::counting::{
    ColorId, ColorMode, ColorPattern, CountConfig, CountLayer, CountScope, LabelPattern, LabelToken,
    Subdiv,
};
use crate::tools::ToolKind;
use crate::tools::{BeatTemplate, Modifier, all_tools};
use eframe::egui::{Context, TextStyle, Widget};
use eframe::epaint::text::{FontInsert, InsertFontFamily};
use eframe::epaint::{FontFamily, FontId};
use eframe::{App, CreationContext, egui};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
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
enum TransportState {
    Stopped,
    Playing
}

const APP_STATE_KEY: &str = "grooph_state";

pub struct Grooph {
    mode: Mode,
    music_font_id: FontId,
    measure: Measure,
    cursor_idx: BeatIdx,
    // Prebuilt measures for note/rest/tuplet tool buttons to avoid per-frame reconstruction
    button_measures: HashMap<&'static str, Measure>,
    undo_stack: Vec<(Measure, BeatIdx)>,
    redo_stack: Vec<(Measure, BeatIdx)>,
    // Global UI font bump configuration and per-theme baselines (so bump applies to dark & light)
    font_bump: f32,
    baseline_dark: Option<Vec<(TextStyle, f32)>>,
    baseline_light: Option<Vec<(TextStyle, f32)>>,
    transport_state: TransportState,
    bpm: u32,
    audio: Option<grooph_audio::Audio>,
    // Mixer volumes [0.0, 1.0]
    audio_settings: AudioSettings,
    // Playback smoothing state
    playback_smooth_tick: f64,
    playback_last_update: Option<f64>,
    record_start_time: Option<f64>,
    record_loop_index: u64,
    accuracy_start_time: Option<f64>,
    accuracy_stats: AccuracyStats,
    accuracy_by_onset: HashMap<u32, AccuracyMark>,
    accuracy_hits_in_loop: HashSet<u32>,
    accuracy_last_tick: Option<f64>,
    midi_input_offset_ms: f32,

    // Visual flash on primary beats
    flash_intensity: f32,        // [0,1]
    last_primary_beat: Option<u32>,

    #[cfg(target_arch = "wasm32")]
    wake_lock: Rc<RefCell<Option<web_sys::WakeLockSentinel>>>,
    layout_width_cap_factor: f32,
    layout_accent_below: bool,
    layout_proportional_spacing: bool,
    layout_stem_length_factor: f32,
    layout_debug_bbox: bool,
    audio_offset: f32,
    audio_latency_enabled: bool,
    counting: CountingSettings,
    midi_input: Option<MidiInput>,
    midi_input_ports: Vec<String>,
    midi_selected_port_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct PersistedState {
    version: u8,
    measure: Measure,
    cursor_idx: BeatIdx,
    bpm: u32,
    audio_settings: AudioSettings,
    audio_latency_enabled: bool,
    audio_offset: f32,
    counting: CountingSettings,
    midi_selected_port_id: Option<String>,
    midi_input_offset_ms: f32,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            version: 1,
            measure: Grooph::default_measure(),
            cursor_idx: 0,
            bpm: 120,
            audio_settings: AudioSettings::default(),
            audio_latency_enabled: true,
            audio_offset: 0.0,
            counting: CountingSettings::default(),
            midi_selected_port_id: None,
            midi_input_offset_ms: 0.0,
        }
    }
}

impl PersistedState {
    fn from_app(app: &Grooph) -> Self {
        Self {
            version: 1,
            measure: app.measure.clone(),
            cursor_idx: app.cursor_idx,
            bpm: app.bpm,
            audio_settings: app.audio_settings,
            audio_latency_enabled: app.audio_latency_enabled,
            audio_offset: app.audio_offset,
            counting: app.counting,
            midi_selected_port_id: app.midi_selected_port_id.clone(),
            midi_input_offset_ms: app.midi_input_offset_ms,
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

#[derive(Clone, Copy, Default)]
struct AccuracyStats {
    count: u64,
    sum_ms: f64,
    sum_abs_ms: f64,
    sum_sq_ms: f64,
    last_delta_ms: Option<f64>,
}

#[derive(Clone, Copy)]
enum AccuracyMark {
    Hit(f64),
    Miss,
}

impl AccuracyStats {
    fn reset(&mut self) { *self = Self::default(); }

    fn push(&mut self, delta_ms: f64) {
        self.count = self.count.saturating_add(1);
        self.sum_ms += delta_ms;
        self.sum_abs_ms += delta_ms.abs();
        self.sum_sq_ms += delta_ms * delta_ms;
        self.last_delta_ms = Some(delta_ms);
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

        #[cfg(target_arch = "wasm32")]
        self.handle_visibility_change();

        let audio_state = self.audio_state();

        // If we should be playing audio but have no audio engine, try to create it (works after user gesture on iOS)
        if audio_state == PlayerState::Playing && self.audio.is_none() {
            debug!("Creating audio engine.");
            self.audio = grooph_audio::Audio::new(self.bpm);
        }

        if let Some(audio) = &mut self.audio {
            audio.set_audio_settings(self.audio_settings);
            if audio.update(&audio_state, self.bpm, &self.measure) {
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
        let (events, now_seconds, is_connected) = match self.midi_input.as_ref() {
            Some(input) => (input.drain_events(), input.now_seconds(), input.is_connected()),
            None => return,
        };

        let accuracy_active = self.update_accuracy_state(is_connected, now_seconds);
        let (ticks_per_measure, ticks_per_sec, beat_onsets) = if accuracy_active {
            let ts = self.measure.time_signature();
            let ticks_per_measure = DEFAULT_GRID.ticks_per_measure(&ts) as f64;
            let ticks_per_beat = DEFAULT_GRID.ticks_per_beat(&ts) as f64;
            let ticks_per_sec = (self.bpm as f64 / 60.0) * ticks_per_beat;
            let beats = self.measure.beats();
            let beat_onsets = DEFAULT_GRID.compute_onset_ticks(beats);
            (ticks_per_measure, ticks_per_sec, beat_onsets)
        } else {
            (0.0, 0.0, Vec::new())
        };

        for event in events {
            match event {
                MidiInputEvent::NoteOn { channel, note, velocity, timestamp } => {
                    if accuracy_active
                        && ticks_per_sec > 0.0
                        && ticks_per_measure > 0.0
                        && !beat_onsets.is_empty()
                    {
                        self.record_accuracy_hit(
                            timestamp,
                            ticks_per_sec,
                            ticks_per_measure,
                            &beat_onsets,
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

        if accuracy_active
            && ticks_per_sec > 0.0
            && ticks_per_measure > 0.0
            && !beat_onsets.is_empty()
        {
            self.update_accuracy_progress(
                now_seconds,
                ticks_per_sec,
                ticks_per_measure,
                &beat_onsets,
            );
        }
    }

    fn update_accuracy_state(&mut self, is_connected: bool, now_seconds: f64) -> bool {
        if self.transport_state == TransportState::Playing && is_connected {
            if self.accuracy_start_time.is_none() {
                self.accuracy_start_time = Some(now_seconds);
                self.accuracy_stats.reset();
                self.accuracy_by_onset.clear();
                self.accuracy_hits_in_loop.clear();
                self.accuracy_last_tick = Some(0.0);
            }
            true
        } else {
            self.accuracy_start_time = None;
            self.accuracy_by_onset.clear();
            self.accuracy_hits_in_loop.clear();
            self.accuracy_last_tick = None;
            false
        }
    }

    fn clear_accuracy_for_edit(&mut self) {
        self.accuracy_stats.reset();
        self.accuracy_by_onset.clear();
        self.accuracy_hits_in_loop.clear();
        self.accuracy_last_tick = None;
    }

    fn record_accuracy_hit(
        &mut self,
        timestamp: f64,
        ticks_per_sec: f64,
        ticks_per_measure: f64,
        beat_onsets: &[u32],
    ) {
        let Some(start_time) = self.accuracy_start_time else {
            return;
        };
        let beats = self.measure.beats();
        if ticks_per_sec <= 0.0
            || ticks_per_measure <= 0.0
            || beats.is_empty()
            || beats.len() != beat_onsets.len()
        {
            return;
        }
        let offset_sec = (self.midi_input_offset_ms as f64) / 1000.0;
        let elapsed = timestamp - start_time + offset_sec;
        if elapsed < 0.0 {
            return;
        }
        let hit_tick = (elapsed * ticks_per_sec).rem_euclid(ticks_per_measure);
        let mut best: Option<(usize, f64)> = None;
        for (idx, &onset_tick_u32) in beat_onsets.iter().enumerate() {
            let onset_tick = onset_tick_u32 as f64;
            let mut diff = hit_tick - onset_tick;
            if diff > ticks_per_measure * 0.5 {
                diff -= ticks_per_measure;
            } else if diff < -ticks_per_measure * 0.5 {
                diff += ticks_per_measure;
            }
            if best.map_or(true, |(_, best_diff)| diff.abs() < best_diff.abs()) {
                best = Some((idx, diff));
            }
        }
        if let Some((best_idx, diff_ticks)) = best {
            if beats.get(best_idx).map_or(true, |b| b.kind != BeatKind::Note) {
                return;
            }
            let onset_tick = beat_onsets[best_idx];
            if self.accuracy_hits_in_loop.contains(&onset_tick) {
                return;
            }
            let delta_ms = (diff_ticks / ticks_per_sec) * 1000.0;
            self.accuracy_stats.push(delta_ms);
            self.accuracy_by_onset
                .insert(onset_tick, AccuracyMark::Hit(diff_ticks));
            self.accuracy_hits_in_loop.insert(onset_tick);
            info!(
                "Accuracy hit: onset_tick={} hit_tick={:.2} delta_ms={:+.2} bpm={} offset_ms={:+.1}",
                onset_tick,
                hit_tick,
                delta_ms,
                self.bpm,
                self.midi_input_offset_ms
            );
        }
    }

    fn update_accuracy_progress(
        &mut self,
        now_seconds: f64,
        ticks_per_sec: f64,
        ticks_per_measure: f64,
        beat_onsets: &[u32],
    ) {
        let Some(start_time) = self.accuracy_start_time else {
            return;
        };
        let beats = self.measure.beats();
        if beats.len() != beat_onsets.len() || beats.is_empty() {
            return;
        }
        let offset_sec = (self.midi_input_offset_ms as f64) / 1000.0;
        let elapsed = now_seconds - start_time + offset_sec;
        if elapsed < 0.0 {
            return;
        }
        let current_tick = (elapsed * ticks_per_sec).rem_euclid(ticks_per_measure);
        let Some(last_tick) = self.accuracy_last_tick else {
            self.accuracy_last_tick = Some(current_tick);
            return;
        };

        fn process_segment(
            beats: &[Beat],
            beat_onsets: &[u32],
            start: f64,
            end: f64,
            segment_offset: f64,
            ticks_per_measure: f64,
            accuracy_by_onset: &mut HashMap<u32, AccuracyMark>,
            accuracy_hits_in_loop: &mut HashSet<u32>,
        ) {
            if beats.is_empty() || beat_onsets.is_empty() {
                return;
            }
            let seg_start = segment_offset + start;
            let seg_end = segment_offset + end;
            for (idx, beat) in beats.iter().enumerate() {
                if beat.kind != BeatKind::Note {
                    continue;
                }
                let onset_tick = *beat_onsets.get(idx).unwrap_or(&0);
                let cur = onset_tick as f64;
                let next = if idx + 1 < beat_onsets.len() {
                    beat_onsets[idx + 1] as f64
                } else {
                    beat_onsets[0] as f64 + ticks_per_measure
                };
                let mut window_end = cur + (next - cur) * 0.5;
                if window_end < segment_offset {
                    window_end += ticks_per_measure;
                }
                let in_range = window_end > seg_start && window_end <= seg_end;
                if in_range {
                    if !accuracy_hits_in_loop.contains(&onset_tick) {
                        accuracy_by_onset.insert(onset_tick, AccuracyMark::Miss);
                    }
                    accuracy_hits_in_loop.remove(&onset_tick);
                }
            }
        }

        if current_tick >= last_tick {
            process_segment(
                beats,
                beat_onsets,
                last_tick,
                current_tick,
                0.0,
                ticks_per_measure,
                &mut self.accuracy_by_onset,
                &mut self.accuracy_hits_in_loop,
            );
        } else {
            // wrapped
            process_segment(
                beats,
                beat_onsets,
                last_tick,
                ticks_per_measure,
                0.0,
                ticks_per_measure,
                &mut self.accuracy_by_onset,
                &mut self.accuracy_hits_in_loop,
            );
            self.accuracy_hits_in_loop.clear();
            process_segment(
                beats,
                beat_onsets,
                0.0,
                current_tick,
                ticks_per_measure,
                ticks_per_measure,
                &mut self.accuracy_by_onset,
                &mut self.accuracy_hits_in_loop,
            );
        }

        self.accuracy_last_tick = Some(current_tick);
    }

    fn push_undo(&mut self) { self.undo_stack.push((self.measure.clone(), self.cursor_idx)); }

    fn clear_redo(&mut self) { self.redo_stack.clear(); }

    fn undo(&mut self) {
        if let Some((m, c)) = self.undo_stack.pop() {
            // Move the current state to redo, replace it with the popped snapshot
            let current = (std::mem::replace(&mut self.measure, m), self.cursor_idx);
            self.cursor_idx = c;
            self.redo_stack.push(current);
            self.clear_accuracy_for_edit();
        }
    }

    fn redo(&mut self) {
        if let Some((m, c)) = self.redo_stack.pop() {
            let current = (std::mem::replace(&mut self.measure, m), self.cursor_idx);
            self.cursor_idx = c;
            self.undo_stack.push(current);
            self.clear_accuracy_for_edit();
        }
    }

    fn set_beat(
        &mut self,
        idx: usize,
        base: NoteValue,
        beat_kind: Option<BeatKind>,
    ) -> Option<Modification> {
        let result = self.measure.modify_beat(idx, base, beat_kind);
        if result.is_some() {
            let new_len = self.measure.beats().len();
            if new_len > 0 {
                let last = new_len - 1;
                if self.cursor_idx < last {
                    self.cursor_idx += 1;
                }
            }
        }

        result
    }

    fn set_tuplet(&mut self, idx: usize, tuplet_spec: Option<TupletSpec>) -> Option<Modification> {
        let result = self.measure.set_tuplet(idx, tuplet_spec, true);
        match &result {
            Some(Modification::DissolveTuplet(tuplet_idx, _)) => {
                let new_len = self.measure.beats().len();
                if new_len > 0 {
                    self.cursor_idx = tuplet_idx.start_idx.min(new_len - 1);
                } else {
                    self.cursor_idx = 0;
                }
            }
            Some(Modification::SetTuplet(group_span, ..)) => {
                self.cursor_idx = (group_span.end_idx + 1).min(self.measure.beats().len() - 1);
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

    fn reset_playback_state(&mut self) {
        self.playback_last_update = None;
        self.playback_smooth_tick = 0.0;
        self.flash_intensity = 0.0;
        self.last_primary_beat = None;
    }

    fn start_playback(&mut self) {
        if self.transport_state == TransportState::Playing {
            return;
        }
        self.transport_state = TransportState::Playing;
        self.record_start_time = None;
        self.record_loop_index = 0;
        self.accuracy_stats.reset();
        self.accuracy_by_onset.clear();
        self.accuracy_hits_in_loop.clear();
        self.accuracy_last_tick = None;
        self.accuracy_start_time = self.midi_input.as_ref().and_then(|input| {
            if input.is_connected() {
                Some(input.now_seconds())
            } else {
                None
            }
        });
        if self.accuracy_start_time.is_some() {
            self.accuracy_last_tick = Some(0.0);
        }
        self.reset_playback_state();

        #[cfg(target_arch = "wasm32")]
        self.acquire_wake_lock();
    }

    fn stop_transport(&mut self) {
        if self.transport_state == TransportState::Stopped {
            return;
        }
        self.transport_state = TransportState::Stopped;
        self.record_start_time = None;
        self.record_loop_index = 0;
        self.accuracy_start_time = None;
        self.accuracy_by_onset.clear();
        self.accuracy_hits_in_loop.clear();
        self.accuracy_last_tick = None;
        self.reset_playback_state();

        #[cfg(target_arch = "wasm32")]
        self.release_wake_lock();
    }

    pub fn toggle_playback(&mut self) {
        match self.transport_state {
            TransportState::Stopped => self.start_playback(),
            TransportState::Playing => self.stop_transport(),
        }
        info!("Toggle playback: {:?}", self.transport_state);
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
                let mut layer = CountLayer::new(next_id, CountScope::PrimaryGroup, Subdiv::Fixed(1));
                layer.labels = Some(LabelPattern {
                    slots: vec![vec![LabelToken::GroupNum]],
                });
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
                let mut layer = CountLayer::new(next_id, CountScope::PrimaryGroup, Subdiv::Fixed(3));
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

        if layers.is_empty() {
            None
        } else {
            Some(CountConfig::new(layers))
        }
    }

    pub fn new(cc: &CreationContext) -> Self {
        add_font(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let ff = FontFamily::Name("music".into());

        let mut state = cc
            .storage
            .and_then(|storage| eframe::get_value::<PersistedState>(storage, APP_STATE_KEY))
            .unwrap_or_default();

        if state.measure.beats().is_empty() {
            state.measure = Self::default_measure();
            state.cursor_idx = 0;
        } else {
            state.cursor_idx = state.cursor_idx.min(state.measure.beats().len().saturating_sub(1));
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

        if let (Some(ref mut input), Some(ref port_id)) =
            (midi_input.as_mut(), midi_selected_port_id.as_ref())
        {
            if let Some(idx) = input.find_port_index_by_id(port_id) {
                let _ = input.connect(idx);
            }
        }

        let this = Self {
            mode: Mode::Playback,
            music_font_id: FontId::new(16.0, ff),
            measure: state.measure,
            cursor_idx: state.cursor_idx,
            button_measures,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            font_bump: 8.0,
            baseline_dark: None,
            baseline_light: None,
            transport_state: TransportState::Stopped,
            bpm: state.bpm,
            audio: None,
            audio_settings: state.audio_settings,
            playback_smooth_tick: 0.0,
            playback_last_update: None,
            record_start_time: None,
            record_loop_index: 0,
            accuracy_start_time: None,
            accuracy_stats: AccuracyStats::default(),
            accuracy_by_onset: HashMap::new(),
            accuracy_hits_in_loop: HashSet::new(),
            accuracy_last_tick: None,
            midi_input_offset_ms: state.midi_input_offset_ms,
            flash_intensity: 0.0,
            last_primary_beat: None,
            #[cfg(target_arch = "wasm32")]
            wake_lock: Rc::new(RefCell::new(None)),
            layout_width_cap_factor: 0.1,
            layout_accent_below: false,
            layout_proportional_spacing: true,
            layout_stem_length_factor: 0.9,
            layout_debug_bbox: false,
            audio_offset: state.audio_offset,
            audio_latency_enabled: state.audio_latency_enabled,
            counting: state.counting,
            midi_input,
            midi_input_ports,
            midi_selected_port_id,
        };

        // WASM: install visibilitychange/pageshow listeners once
        #[cfg(target_arch = "wasm32")]
        {
            web::install_visibility_listeners(cc.egui_ctx.clone());
        }

        this
    }
}
