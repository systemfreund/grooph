mod accuracy;
mod calibration;
mod clock;
mod documents;
mod embed;
mod generator;
mod guide;
mod help;
mod host;
mod keyboard_input;
mod kit_palette;
mod library;
mod library_panel;
mod measure_panel;
mod metronome_trainer;
mod metronome_view;
mod midi_input;
mod mixer_panel;
mod persist;
mod pickers;
mod platform;
mod play_along;
mod play_along_settings;
mod playback_clock;
mod preset_panel;
mod presets;
mod settings_panel;
mod share;
mod shell;
mod state;
mod style;
mod tool_palette;
pub mod tools;
mod undo;
mod update_check;
#[cfg(target_arch = "wasm32")]
mod web;

pub use host::Host;

use grooph_layout::staff::Staff;
use grooph_measure::duration::{Duration, NoteValue, TupletSpec, q};
use grooph_measure::{BeatIdx, Cursor, Instrument, Measure, Score, TimeSignature, VoiceBar};

use crate::accuracy::AccuracyState;
use crate::calibration::CalibrationPhase;
use crate::documents::{Document, Documents, RhythmDoc};
use crate::embed::{Embed, EmbedParams, embed_params};
use crate::generator::GeneratorState;
use crate::guide::Guide;
use crate::library::PatternLibrary;
use crate::metronome_view::{MetronomeState, metronome_bar};
use crate::platform::{PlatformRuntime, VisibilityEvent};
use crate::play_along::{PlayAlongSettings, PlayAlongState};
use crate::presets::PresetLibrary;
use crate::shell::{ShellState, Tab};
use crate::state::{
    AudioConfig, CountInState, EditorState, LayoutSettings, MicState, MidiState, PendingLoad,
    PlaybackController, PlaybackState, UiShell,
};
use crate::tools::ToolKind;
use crate::tools::{BeatTemplate, Modifier, all_tools};
use crate::undo::EditorSnapshot;
use eframe::egui::{Context, Ui};
use eframe::epaint::text::{FontInsert, InsertFontFamily};
use eframe::epaint::{FontFamily, FontId};
use eframe::{App, CreationContext, egui};
use grooph_audio::{AudioSettings, GapClick, PlaybackOptions, Played, PlayerState, TempoRamp};
use grooph_link::{LinkContent, SharedLink};
use grooph_measure::click::ClickBars;
use grooph_measure::counter::{CounterBars, Ratio};
use grooph_measure::counting::{CountConfig, CountingSystem, Subdivision, build_count_slots};
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
    /// A page's editor: tool palette on its document.
    Edit,
    /// A page playing its document: the metronome's ring, Practice's and
    /// the score tab's options sheet.
    Playback,
    /// The "More" tab's list of dialogs.
    MoreMenu,
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

pub(crate) const PERSISTED_STATE_VERSION: u8 = 3;

/// A rhythm document as saved: the same entry for the score tab's and
/// Practice's.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SavedDoc {
    score: Score,
    cursor: Cursor,
    /// Changed since it was saved to or loaded from the library.
    dirty: bool,
    /// The library pattern it is.
    active_pattern_id: Option<u64>,
}

impl Default for SavedDoc {
    fn default() -> Self {
        Self {
            score: Score::single(Grooph::default_measure()),
            cursor: Cursor::start(),
            dirty: false,
            active_pattern_id: None,
        }
    }
}

impl SavedDoc {
    fn of(doc: &RhythmDoc) -> Self {
        Self {
            score: doc.score.clone(),
            cursor: doc.cursor,
            dirty: doc.dirty,
            active_pattern_id: doc.active_pattern_id,
        }
    }

    /// The document again, with nothing to undo: an empty score is one bar,
    /// and a pattern no longer in `library` is forgotten.
    fn into_doc(self, library: &PatternLibrary) -> RhythmDoc {
        // A measure always has beats (`VoiceBar`'s invariant, checked when
        // it loads), so the cursor clamps into a score with one.
        let mut score = if self.score.is_empty() {
            Score::single(Grooph::default_measure())
        } else {
            self.score
        };
        // Saved before a drum was struck by one voice at a time.
        let undoubled = score.undouble_instruments();
        RhythmDoc {
            dirty: self.dirty || undoubled,
            active_pattern_id: self.active_pattern_id.filter(|id| library.contains(*id)),
            ..RhythmDoc::new(score, self.cursor)
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PersistedState {
    version: u8,
    /// The score tab's document.
    score_doc: SavedDoc,
    /// Practice's document with its generated rhythm; a state saved before
    /// the two documents has none, and Practice starts with a new one.
    practice_doc: Option<SavedDoc>,
    /// The document shown, so the More screen opens again on the one it was
    /// opened from.
    document: Document,
    bpm: u32,
    audio_settings: AudioSettings,
    audio_latency_enabled: bool,
    audio_offset: f32,
    counting: CountingSettings,
    flash_enabled: bool,
    /// See [`UiState::show_score_tab`].
    show_score_tab: bool,
    guide: Guide,
    midi_selected_port_id: Option<String>,
    accuracy_enabled: bool,
    library: PatternLibrary,
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
    tab: Tab,
    /// The page's editor was open, reopened with the tab.
    editing: bool,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            version: PERSISTED_STATE_VERSION,
            score_doc: SavedDoc::default(),
            practice_doc: None,
            document: Document::Score,
            bpm: grooph_link::DEFAULT_BPM,
            audio_settings: AudioSettings::default(),
            audio_latency_enabled: true,
            audio_offset: 0.0,
            counting: CountingSettings::default(),
            flash_enabled: false,
            show_score_tab: false,
            guide: Guide::default(),
            midi_selected_port_id: None,
            accuracy_enabled: false,
            library: PatternLibrary::default(),
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
            tab: Tab::Metronome,
            editing: false,
        }
    }
}

impl PersistedState {
    fn from_app(app: &Grooph) -> Self {
        let generator = &app.editor.generator;
        let docs = &app.editor.docs;
        Self {
            version: PERSISTED_STATE_VERSION,
            score_doc: SavedDoc::of(&docs.score),
            practice_doc: Some(SavedDoc::of(&docs.practice)),
            document: app.document(),
            bpm: app.playback_ctl.bpm,
            audio_settings: app.playback_ctl.audio_cfg.settings,
            audio_latency_enabled: app.playback_ctl.audio_cfg.latency_enabled,
            audio_offset: app.playback_ctl.audio_cfg.offset,
            counting: app.ui.counting,
            flash_enabled: app.ui.flash_enabled,
            show_score_tab: app.ui.show_score_tab,
            guide: app.ui.guide,
            midi_selected_port_id: app.playback_ctl.midi.selected_port_id.clone(),
            accuracy_enabled: app.playback_ctl.accuracy.enabled,
            library: app.editor.library.clone(),
            generator_settings: generator.settings,
            endless: generator.endless,
            ghost_notes: generator.ghost_notes,
            count_in: app.playback_ctl.audio_cfg.count_in,
            swing: app.playback_ctl.audio_cfg.swing,
            play_along: app.playback_ctl.play_along.settings,
            gap_click: app.playback_ctl.audio_cfg.gap,
            tempo_ramp: app.playback_ctl.audio_cfg.ramp,
            presets: app.editor.presets.clone(),
            click_bars: app.click_bars(),
            counter_bars: app.counter_bars(),
            counter_names: app.counter_bars().names().to_vec(),
            metronome_meter: app.metronome_meter(),
            tab: app.tab(),
            editing: app.ui.mode == Mode::Edit,
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
    const ALL: [Self; 5] = [Self::Off, Self::Primary, Self::Ands, Self::Sixteenth, Self::Triplet];

    /// The subdivision counted in, `None` for `Off`.
    fn subdivision(self) -> Option<Subdivision> {
        match self {
            Self::Off => None,
            Self::Primary => Some(Subdivision::Beats),
            Self::Ands => Some(Subdivision::Halves),
            Self::Sixteenth => Some(Subdivision::Quarters),
            Self::Triplet => Some(Subdivision::Triplets),
        }
    }

    /// The name in the count menu: the start of the count in `system`, in
    /// the first bar of `score` it fits, else in 4/4 ("1 2 3 4 ...",
    /// "1 & 2 & ...", "ta ki da ...").
    fn label(self, system: CountingSystem, score: &Score) -> String {
        let Some(subdivision) = self.subdivision() else {
            return "Off".to_string();
        };
        let ts = score
            .measures()
            .iter()
            .map(|m| m.time_signature())
            .find(|ts| subdivision.fits(ts))
            .unwrap_or(TimeSignature::FOUR_FOUR);
        let config = CountConfig { subdivision, system };
        let slots = build_count_slots(&Measure::new(ts), &config);
        let take = if self == Self::Triplet { 3 } else { 4 };
        let labels: Vec<_> = slots.iter().filter_map(|s| s.label.as_deref()).take(take).collect();
        format!("{} ...", labels.join(" "))
    }

    /// Whether the subdivision counts sensibly in some measure of `score`.
    fn fits(self, score: &Score) -> bool {
        self.subdivision()
            .is_none_or(|s| score.measures().iter().any(|m| s.fits(&m.time_signature())))
    }
}

/// The counting overlay: a subdivision to count in (or `Off`, which hides
/// the overlay), in numbers or syllables.
#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(from = "StoredCountingSettings")]
struct CountingSettings {
    base: CountingBase,
    system: CountingSystem,
}

impl Default for CountingSettings {
    fn default() -> Self {
        Self { base: CountingBase::Off, system: CountingSystem::Numbers }
    }
}

/// Saved form, still reading the `enabled` and `show_labels` switches that
/// older versions stored next to `base`: either one off hid the overlay. Their
/// `show_tuplets` switch is ignored.
#[derive(Deserialize)]
struct StoredCountingSettings {
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default = "yes")]
    show_labels: bool,
    base: CountingBase,
    #[serde(default)]
    system: CountingSystem,
}

fn yes() -> bool {
    true
}

impl From<StoredCountingSettings> for CountingSettings {
    fn from(s: StoredCountingSettings) -> Self {
        if s.enabled && s.show_labels {
            Self { base: s.base, system: s.system }
        } else {
            Self { system: s.system, ..Self::default() }
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
        self.ui.narrow_screen = ui.available_width() < shell::NARROW_MAX_WIDTH;
        let embedded = self.embedded();
        // The root Ui spans the whole view; keep the app inside the safe
        // area: what the system bars (Android, see grooph-android) or the
        // chat host (embedded, see `raw_input_hook`) leave free. What they
        // cover keeps the app's background. Without insets it is all of it.
        let content = ui.ctx().content_rect();
        if content != ui.max_rect() {
            ui.painter().rect_filled(ui.max_rect(), 0, ui.visuals().panel_fill);
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            if embedded { self.embed_ui(ui) } else { self.shell_ui(ui) }
        });
        self.update_metronome_trainer();
        self.update_endless();
        if !embedded {
            self.run_summary_window(ui);
            self.calibration_window(ui);
        }

        if self.ui.pending_load.is_some() && !embedded {
            self.load_confirm_dialog(ui);
        }

        if self.ui.link_error.is_some() {
            self.link_error_dialog(ui);
        }

        if !embedded {
            self.update_notice(ui, frame);
        }

        self.handle_keyboard_input(ui);
        if !embedded {
            self.handle_play_along_input(ui);
        }

        for ev in self.ui.platform.take_visibility_events() {
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
        let bpm = self.played_bpm();
        let now = ui.input(|i| i.time);
        let mut output_lost = false;
        // The engine leaves `self` while it reads the played score, which
        // borrows `self`: no copy of the score per frame.
        let mut engine = self.playback_ctl.audio.take();
        if let Some(audio) = &mut engine {
            audio.set_audio_settings(self.playback_ctl.audio_cfg.settings);
            audio.set_playback_options(playback_options);
            if let Some(after) = audio.update(&audio_state, bpm, self.played_score(), now) {
                ui.ctx().request_repaint_after(after);
            }
            output_lost = audio.take_output_lost();
        }
        self.playback_ctl.audio = engine;
        // The output went away (e.g. a Bluetooth device): stop, as the next
        // play opens a new one.
        if output_lost {
            self.stop_transport();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Each embed starts from its link alone (see `embed`).
        if self.embedded() {
            return;
        }
        let state = PersistedState::from_app(self);
        eframe::set_value(storage, APP_STATE_KEY, &state);
        if let Some(backup) = self.editor.state_backup.take() {
            backup.write(storage);
        }
    }

    fn persist_egui_memory(&self) -> bool {
        !self.embedded()
    }

    fn raw_input_hook(&mut self, ctx: &Context, raw_input: &mut egui::RawInput) {
        if self.embedded() {
            embed::cancel_scrolled_touch(ctx, raw_input);
            // What the chat covers at the edges, e.g. its composer over the
            // bottom in fullscreen: egui lays everything out inside the rest.
            if let Some(insets) = embed::host_safe_area(ctx) {
                raw_input.safe_area_insets = Some(insets);
            }
        }
    }
}

/// eframe's options for the web app. Embedded in a chat (see [`embed`]),
/// pointer and touch events are left to the page as well, so a vertical drag
/// that starts on the view scrolls the conversation.
#[cfg(target_arch = "wasm32")]
pub fn web_options() -> eframe::WebOptions {
    let mut options = eframe::WebOptions::default();
    if platform::link_query().as_deref().and_then(embed_params).is_some() {
        options.should_prevent_default = Box::new(|event| !embed::chat_takes(event));
    }
    options
}

impl Grooph {
    fn playback_options(&self) -> PlaybackOptions {
        // A calibration clicks every beat once counted in, whatever the
        // user's options.
        if self.calibrating() {
            return PlaybackOptions { count_in: true, ..Default::default() };
        }
        let generator = &self.editor.generator;
        PlaybackOptions {
            count_in: self.playback_ctl.audio_cfg.count_in,
            // On the generator's grid: Practice's rhythm only.
            ghost_notes: (generator.ghost_notes && self.document() == Document::Practice)
                .then_some(generator.settings.subdivision),
            swing: self.playback_ctl.audio_cfg.swing,
            // Editing always sounds every bar, so changes can be heard.
            gap: GapClick {
                enabled: self.playback_ctl.audio_cfg.gap.enabled && !self.editing(),
                ..self.playback_ctl.audio_cfg.gap
            },
            ramp: self.played_ramp(),
            click: self.click_bars(),
            // The metronome plays its click bar and counter rhythm alone.
            played: if self.metronome_mode() { Played::Clicks } else { Played::Score },
        }
    }

    /// Whether the score or the metronome's clicks are being edited: a page's
    /// editor, or "Edit clicks" in the Metronome tab.
    pub(crate) fn editing(&self) -> bool {
        self.ui.mode == Mode::Edit
    }

    /// Tempo backbone of the working score, including swing. Must agree with
    /// the timing the audio engine builds from [`Self::playback_options`],
    /// including the tempo ramp of the bar being played.
    pub(crate) fn score_timing(&self) -> ScoreTiming {
        // A calibration plays without swing, as in `playback_options`.
        let swing =
            if self.calibrating() { Swing::default() } else { self.playback_ctl.audio_cfg.swing };
        ScoreTiming::from_score(self.played_score(), self.current_bpm()).with_swing(swing)
    }

    /// Tempo being played: the ramp tempo while a ramp runs, else the BPM setting.
    pub(crate) fn current_bpm(&self) -> u32 {
        self.playback_ctl.playback.ramp_bpm.unwrap_or(self.played_bpm())
    }

    fn clear_accuracy_for_edit(&mut self) {
        self.playback_ctl.accuracy.tracker.clear_for_edit();
    }

    pub(crate) fn current_measure(&self) -> &Measure {
        self.editor.doc().score.current(self.editor.doc().cursor.measure_idx)
    }

    pub(crate) fn current_measure_mut(&mut self) -> &mut Measure {
        let doc = self.editor.doc_mut();
        doc.score.current_mut(doc.cursor.measure_idx)
    }

    /// The staff the document is written on.
    pub(crate) fn staff_model(&self) -> &'static Staff {
        Staff::of(self.editor.doc().score.staff)
    }

    /// The instrument the tools write and change: the chosen one on a staff
    /// that writes it, else the staff's own (the one line's rhythm).
    pub(crate) fn edit_instrument(&self) -> Instrument {
        self.staff_model().instrument_or(self.editor.instrument)
    }

    /// The voice the cursor is in, whose beats the tools edit.
    pub(crate) fn current_voice(&self) -> &VoiceBar {
        self.editor
            .doc()
            .score
            .voice_at(self.editor.doc().score.clamp_cursor(self.editor.doc().cursor))
    }

    pub(crate) fn current_voice_mut(&mut self) -> &mut VoiceBar {
        let cursor = self.editor.doc().score.clamp_cursor(self.editor.doc().cursor);
        self.editor.doc_mut().score.voice_at_mut(cursor)
    }

    fn current_snapshot(&self) -> EditorSnapshot {
        EditorSnapshot {
            score: self.editor.doc().score.clone(),
            cursor: self.editor.doc().cursor,
            counter_name: self
                .metronome_mode()
                .then(|| self.playback_ctl.audio_cfg.counter.name(self.metronome_meter())),
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.editor.doc().history.can_undo()
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.editor.doc().history.can_redo()
    }

    /// Snapshot the current state, run `op`, and commit only if it reports a change.
    /// On commit, clears redo and accuracy edit state. On no-op, the snapshot is discarded.
    pub(crate) fn with_undo_snapshot<F>(&mut self, op: F) -> bool
    where
        F: FnOnce(&mut Self) -> bool,
    {
        let snap = self.current_snapshot();
        if op(self) {
            self.editor.doc_mut().history.push(snap);
            self.clear_accuracy_for_edit();
            self.editor.doc_mut().dirty = true;
            true
        } else {
            false
        }
    }

    /// Bring back `snap`'s score and cursor. In the metronome tab the
    /// meter may change with it: the current one keeps its bars.
    fn restore_snapshot(&mut self, snap: EditorSnapshot) {
        if self.metronome_mode() {
            self.store_metronome_bars();
        }
        let doc = self.editor.doc_mut();
        doc.score = snap.score;
        doc.cursor = snap.cursor;
        // The counter rhythm's bar is the document's, its name the settings'.
        if let Some(name) = snap.counter_name {
            self.playback_ctl.audio_cfg.counter.set_ratio(self.metronome_meter(), name);
        }
    }

    fn undo(&mut self) {
        let current = self.current_snapshot();
        if let Some(prev) = self.editor.doc_mut().history.pop_undo(current) {
            self.restore_snapshot(prev);
            self.clear_accuracy_for_edit();
            self.editor.doc_mut().dirty = true;
        }
    }

    fn redo(&mut self) {
        let current = self.current_snapshot();
        if let Some(next) = self.editor.doc_mut().history.pop_redo(current) {
            self.restore_snapshot(next);
            self.clear_accuracy_for_edit();
            self.editor.doc_mut().dirty = true;
        }
    }

    fn set_beat(
        &mut self,
        idx: usize,
        base: NoteValue,
        beat_kind: Option<BeatKind>,
    ) -> Option<Modification> {
        let pen = self.edit_instrument();
        let result = self.current_voice_mut().modify_beat(idx, base, beat_kind, pen);
        if result.is_some() {
            let new_len = self.current_voice().beats().len();
            if new_len > 0 {
                let last = new_len - 1;
                if self.editor.doc().cursor.beat_idx < last {
                    self.editor.doc_mut().cursor.beat_idx += 1;
                }
            }
        }

        result
    }

    fn set_tuplet(&mut self, idx: usize, tuplet_spec: Option<TupletSpec>) -> Option<Modification> {
        let pen = self.edit_instrument();
        let result = self.current_voice_mut().set_tuplet(idx, tuplet_spec, true, pen);
        match &result {
            Some(Modification::DissolveTuplet(tuplet_idx, _)) => {
                let new_len = self.current_voice().beats().len();
                if new_len > 0 {
                    self.editor.doc_mut().cursor.beat_idx = tuplet_idx.start_idx.min(new_len - 1);
                } else {
                    self.editor.doc_mut().cursor.beat_idx = 0;
                }
            }
            Some(Modification::SetTuplet(group_span, ..)) => {
                self.editor.doc_mut().cursor.beat_idx =
                    (group_span.end_idx + 1).min(self.current_voice().beats().len() - 1);
            }
            _ => {}
        }
        result
    }

    /// Add an empty bar after the last one, like it, and move the cursor
    /// there (the editor's ghost bar). Not in the metronome's one bar.
    pub(crate) fn push_measure(&mut self) {
        if self.metronome_mode() {
            return;
        }
        self.with_undo_snapshot(|app| {
            let doc = app.editor.doc_mut();
            let last = &doc.score.measures()[doc.score.len() - 1];
            let new_measure = last.empty_like(last.time_signature());
            doc.score.push(new_measure);
            let cursor = Cursor { measure_idx: doc.score.len() - 1, beat_idx: 0, ..doc.cursor };
            doc.cursor = doc.score.clamp_cursor(cursor);
            true
        });
    }

    /// Put a copy of the cursor's bar right after it and move the cursor
    /// there, to the same beat (the bar menu). Not in the metronome's one bar.
    pub(crate) fn duplicate_current_measure(&mut self) {
        if self.metronome_mode() {
            return;
        }
        self.with_undo_snapshot(|app| {
            let doc = app.editor.doc_mut();
            let cursor = doc.score.clamp_cursor(doc.cursor);
            let copy = doc.score.measures()[cursor.measure_idx].clone();
            doc.score.insert(cursor.measure_idx + 1, copy);
            doc.cursor = Cursor { measure_idx: cursor.measure_idx + 1, ..cursor };
            true
        });
    }

    /// Empty the cursor's bar, every voice of it (the bar menu). In the
    /// metronome only the click bar or counter rhythm being edited.
    pub(crate) fn clear_current_measure(&mut self) {
        if self.metronome_mode() {
            self.apply_tool(tool_palette::tool("meta.reset_measure"));
            return;
        }
        self.with_undo_snapshot(|app| {
            let doc = app.editor.doc_mut();
            let cursor = doc.score.clamp_cursor(doc.cursor);
            let measure = &doc.score.measures()[cursor.measure_idx];
            let empty = measure.empty_like(measure.time_signature());
            doc.score.measures_mut()[cursor.measure_idx] = empty;
            doc.cursor = Cursor { beat_idx: 0, ..cursor };
            true
        });
    }

    /// Respell the cursor's bar for reading, every voice (the bar menu's
    /// "Clean up bar", see [`grooph_measure::Measure::clean_up`]): every
    /// note stays where it starts, with its heads; only the written lengths
    /// and the rests change. The cursor stays on the beat sounding at its
    /// time. Not in the metronome.
    pub(crate) fn clean_up_current_measure(&mut self) {
        if self.metronome_mode() {
            return;
        }
        self.with_undo_snapshot(|app| {
            let doc = app.editor.doc_mut();
            let cursor = doc.score.clamp_cursor(doc.cursor);
            let onsets = |score: &Score| {
                grooph_measure::grid::DEFAULT_GRID
                    .compute_onset_ticks(score.voice_at(cursor).beats())
            };
            let tick = onsets(&doc.score).get(cursor.beat_idx).copied().unwrap_or(0);
            if !doc.score.measures_mut()[cursor.measure_idx].clean_up() {
                return false;
            }
            let beat_idx =
                onsets(&doc.score).iter().take_while(|&&t| t <= tick).count().saturating_sub(1);
            doc.cursor = Cursor { beat_idx, ..cursor };
            true
        });
    }

    /// Whether "Clean up bar" would change the cursor's bar.
    pub(crate) fn can_clean_up_current_measure(&self) -> bool {
        let doc = self.editor.doc();
        let cursor = doc.score.clamp_cursor(doc.cursor);
        doc.score.measures()[cursor.measure_idx].needs_clean_up()
    }

    /// Remove the currently active measure. Does nothing if only one measure
    /// remains (Score invariant: at least one measure).
    pub(crate) fn remove_current_measure(&mut self) {
        if self.editor.doc().score.len() <= 1 {
            return;
        }
        self.with_undo_snapshot(|app| {
            let idx = app.editor.doc().cursor.measure_idx;
            app.editor.doc_mut().score.remove(idx);
            let doc = app.editor.doc_mut();
            doc.cursor = doc.score.clamp_cursor(doc.cursor);
            true
        });
    }

    /// "Save": if a pattern is active, overwrite its score + tempo in place
    /// (keeping id and name); otherwise save a new one (see
    /// [`Self::save_pattern_as`]). Returns the pattern's id.
    /// The library keeps the score tab's score, also when it is saved from
    /// another page.
    pub(crate) fn save_active_pattern(&mut self) -> u64 {
        let bpm = self.playback_ctl.bpm;
        let doc = &mut self.editor.docs.score;
        match doc.active_pattern_id {
            Some(id) if self.editor.library.contains(id) => {
                self.editor.library.update(id, doc.score.clone(), bpm);
                doc.dirty = false;
                id
            }
            _ => self.save_pattern_as(),
        }
    }

    /// "Save as new": always create a new library entry under the next free
    /// "Pattern N" and make it the active pattern.
    pub(crate) fn save_pattern_as(&mut self) -> u64 {
        let doc = &mut self.editor.docs.score;
        let bpm = self.playback_ctl.bpm;
        let names = self.editor.library.patterns.iter().map(|p| p.name.as_str());
        let name = library::next_default_name("Pattern", names);
        let id = self.editor.library.add(name, doc.score.clone(), bpm);
        doc.active_pattern_id = Some(id);
        doc.dirty = false;
        id
    }

    /// Replace the current score + tempo with the saved pattern `id`. The score
    /// swap is undoable; the tempo change is applied directly (tempo is not part
    /// of the undo snapshot). Transport is stopped so playback restarts cleanly.
    pub(crate) fn load_pattern(&mut self, id: u64) {
        let Some(pattern) = self.editor.library.get(id) else {
            return;
        };
        let mut score = pattern.score.clone();
        // A drum struck by two voices at once is a change to save.
        let undoubled = score.undouble_instruments();
        let bpm = pattern.bpm;
        // A rhythm of the one line, saved before the score tab was for
        // drum grooves, opens in Practice.
        if score.staff == grooph_measure::StaffKind::Line {
            let undoubled = self.replace_practice_from_link(score);
            self.editor.docs.practice.dirty = undoubled;
            self.playback_ctl.bpm = bpm;
            self.playback_ctl.playback.reset();
            self.ui.picker = None;
            self.select_tab(Tab::Practice);
            return;
        }

        // The score tab's score is replaced.
        self.stop_transport();
        self.show_document(Document::Score);
        self.with_undo_snapshot(|app| {
            app.editor.doc_mut().score = score;
            app.editor.doc_mut().cursor = Cursor::start();
            true
        });
        self.playback_ctl.bpm = bpm;
        self.playback_ctl.playback.reset();
        self.editor.doc_mut().active_pattern_id = Some(id);
        self.editor.doc_mut().dirty = undoubled;
        self.show_loaded_pattern();
    }

    /// A loaded pattern or score link opens on the score tab. The library
    /// closes; the editor stays open if it was.
    fn show_loaded_pattern(&mut self) {
        self.ui.picker = None;
        if self.tab() != Tab::Score {
            self.select_tab(Tab::Score);
        }
    }

    /// Entry point for the load button: if the working score has unsaved changes,
    /// defer to the confirmation dialog; otherwise load immediately.
    pub(crate) fn request_load_pattern(&mut self, id: u64) {
        // A rhythm of the one line goes to Practice, which a load replaces
        // without asking: the score tab's changes are not at stake.
        if self
            .editor
            .library
            .get(id)
            .is_some_and(|p| p.score.staff == grooph_measure::StaffKind::Line)
        {
            self.load_pattern(id);
            return;
        }
        let dirty = self.editor.docs.score.dirty;
        if self.editor.docs.score.active_pattern_id == Some(id) && !dirty {
            // Already loaded and unchanged.
            self.show_document(Document::Score);
            self.show_loaded_pattern();
            return;
        }
        // Asking first leaves the metronome playing, in case of a cancel.
        if dirty {
            self.ui.pending_load = Some(PendingLoad::Pattern(id));
        } else {
            self.load_pattern(id);
        }
    }

    pub(crate) fn delete_pattern(&mut self, id: u64) {
        self.editor.library.remove(id);
        self.forget_pattern(id);
        if matches!(self.ui.pending_load, Some(PendingLoad::Pattern(p)) if p == id) {
            self.ui.pending_load = None;
        }
    }

    /// Open a shared link. A score link replaces the score tab's score and
    /// asks first if it has unsaved changes; a generator link replaces
    /// Practice's rhythm, a metronome or tempo-only link applies right away.
    pub(crate) fn open_link(&mut self, link: SharedLink) {
        let replaces_score =
            matches!(link.content, Some(LinkContent::Score(_))) && !self.opens_on_practice(&link);
        if replaces_score && self.editor.docs.score.dirty {
            self.ui.pending_load = Some(PendingLoad::Link(Box::new(link)));
        } else {
            self.apply_link(link);
        }
    }

    pub(crate) fn apply_link(&mut self, link: SharedLink) {
        // Each content opens on its own page: a score link on the score tab
        // (on Practice if shared from there, or while the score tab is
        // hidden), a generator link on Practice,
        // a metronome link on the metronome (below, with its meter and
        // click bar).
        let metronome = matches!(link.content, Some(LinkContent::Metronome(_)));
        let replaces_rhythm =
            matches!(link.content, Some(LinkContent::Score(_) | LinkContent::Generator { .. }));
        let mut undoubled = false;
        match link.content {
            Some(LinkContent::Score(score)) if self.opens_on_practice(&link) => {
                undoubled = self.replace_practice_from_link(score);
            }
            Some(LinkContent::Score(score)) => undoubled = self.replace_score_from_link(score),
            Some(LinkContent::Metronome(ts)) => {
                self.stop_transport();
                // The meter plays the link's counter rhythm, or none; a
                // `poly` link's as picked in the app.
                self.set_metronome_clicks(ts, link.poly, link.counter.clone());
            }
            Some(LinkContent::Generator { settings, seed }) => {
                self.stop_transport();
                self.show_document(Document::Practice);
                self.editor.generator.settings = GeneratorSettings {
                    custom_patterns: self.editor.generator.settings.custom_patterns,
                    ..settings
                };
                self.generate_new_score_with_seed(seed);
                self.ui.mode = Mode::Playback;
            }
            None => {}
        }
        if let Some(bpm) = link.bpm
            && bpm != self.playback_ctl.bpm
        {
            self.playback_ctl.bpm = bpm;
            // A rhythm from the link is clean below; otherwise the tempo
            // is a change of the saved score, as in the BPM picker.
            if !replaces_rhythm {
                self.mark_tempo_unsaved();
            }
            self.handle_bpm_change();
        }
        if let Some(swing) = link.swing {
            self.playback_ctl.audio_cfg.swing = swing;
        }
        if let Some(bar) = link.click {
            self.set_meter_click_bar(bar);
        }
        self.playback_ctl.playback.reset();
        if metronome {
            self.show_document(Document::Metronome);
            self.ui.mode = Mode::Playback;
        } else if replaces_rhythm {
            // The content is reproducible from the link, so it is not
            // "unsaved", unless a drum struck by two voices at once was
            // taken from one of them.
            self.editor.doc_mut().dirty = undoubled;
        }
    }

    /// Whether `link`'s rhythm opens on the practice page: a rhythm of the
    /// one line (the score tab is for drum grooves), or while the score tab
    /// is hidden (a developer setting), so it does not open on a page the
    /// user does not have, nor ask about that page's unsaved changes.
    fn opens_on_practice(&self, link: &SharedLink) -> bool {
        !self.ui.show_score_tab
            || matches!(&link.content, Some(LinkContent::Score(s)) if s.staff == grooph_measure::StaffKind::Line)
    }

    /// A rhythm shared from the practice page replaces its rhythm, as a new
    /// one from the generator would. Reports as
    /// [`Self::replace_score_from_link`].
    fn replace_practice_from_link(&mut self, mut score: Score) -> bool {
        let undoubled = score.undouble_instruments();
        self.stop_transport();
        self.show_document(Document::Practice);
        self.with_undo_snapshot(|app| {
            app.editor.doc_mut().score = score;
            app.editor.doc_mut().cursor = Cursor::start();
            true
        });
        self.ui.picker = None;
        self.ui.mode = Mode::Playback;
        undoubled
    }

    /// Reports whether a drum struck by two voices at once was taken from
    /// one of them (see [`Score::undouble_instruments`]).
    fn replace_score_from_link(&mut self, mut score: Score) -> bool {
        let undoubled = score.undouble_instruments();
        self.stop_transport();
        self.show_document(Document::Score);
        self.with_undo_snapshot(|app| {
            app.editor.doc_mut().score = score;
            app.editor.doc_mut().cursor = Cursor::start();
            true
        });
        self.editor.doc_mut().active_pattern_id = None;
        self.show_loaded_pattern();
        undoubled
    }

    /// Run a pending load after the unsaved-changes dialog.
    pub(crate) fn finish_pending_load(&mut self, pending: PendingLoad) {
        match pending {
            PendingLoad::Pattern(id) => self.load_pattern(id),
            PendingLoad::Link(link) => self.apply_link(*link),
            PendingLoad::NewKitScore => self.new_kit_score(),
        }
    }

    /// The measure a tool button shows: one beat of `template`, or a whole
    /// group for a tuplet, in a time signature it fills exactly.
    pub(crate) fn build_button_measure(template: BeatTemplate) -> Measure {
        let denominator = template.duration.base_note().denominator();
        let time_signature = match template.duration {
            Duration::Tuplet(TupletSpec { m, .. }) => TimeSignature::new(m, denominator),
            // A dotted value fills (2^(dots+1) - 1) of its smallest part.
            Duration::Dotted { dots, .. } => {
                TimeSignature::new((1 << (dots + 1)) - 1, denominator << dots)
            }
            Duration::Simple(..) => TimeSignature::new(1, denominator),
        };
        let mut measure = Measure::new_init(time_signature, template.kind);

        match template.duration {
            Duration::Tuplet(TupletSpec { n, .. }) => {
                for i in 0..n {
                    measure
                        .first_voice_mut()
                        .set_beat(i as BeatIdx, Beat::note(template.duration))
                        .unwrap();
                }
            }
            Duration::Dotted { .. } => {
                let beat = match template.kind {
                    BeatKind::Note => Beat::note(template.duration),
                    BeatKind::Rest => Beat::rest(template.duration),
                };
                measure.first_voice_mut().set_beat(0, beat).unwrap();
            }
            Duration::Simple(..) => {}
        }
        if !matches!(template.duration, Duration::Tuplet(..)) {
            if template.accented {
                measure.first_voice_mut().toggle_accent(0, Instrument::Rhythm);
            }
            if template.soft {
                measure.first_voice_mut().toggle_soft(0, Instrument::Rhythm);
            }
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
                // Cancel.
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
        // Practice's and the score tab's options sheet.
        let practice_page = matches!(self.tab(), Tab::Practice | Tab::Score);
        let ui = &mut self.ui;
        if ui.link_error.take().is_some()
            || ui.pending_load.take().is_some()
            || ui.renaming_preset.take().is_some()
            || ui.renaming_pattern.take().is_some()
        {
            return true;
        }
        if ui.picker.take().is_some() {
            // As when the picker closes itself.
            ui.picker = ui.picker_return.take();
            ui.tap_tempo.reset();
            return true;
        }
        ui.mode = match ui.mode {
            // A page's editor ("Done"), back to the page itself.
            Mode::Edit => {
                ui.shell.options_collapsed = true;
                Mode::Playback
            }
            // The options sheet, when unfolded.
            _ if practice_page && !ui.shell.options_collapsed => {
                ui.shell.options_collapsed = true;
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

    /// Esc: into or out of the page's editor.
    fn toggle_edit(&mut self) {
        match self.ui.mode {
            Mode::Edit => self.leave_editor(),
            Mode::Playback => self.ui.mode = Mode::Edit,
            _ => {}
        }
    }

    /// Done: back from a page's editor to the page itself, its options sheet
    /// folded away, so the card, ring or staff shows.
    pub(crate) fn leave_editor(&mut self) {
        self.ui.mode = Mode::Playback;
        self.ui.shell.options_collapsed = true;
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
        // Each run of endless mode is a new one, from two fresh bars.
        if self.endless_runs() {
            self.generate_new_score();
        }
        self.playback_ctl.transport_state = TransportState::Playing;
        if let Some(audio) = &mut self.playback_ctl.audio {
            audio.restart();
        }
        let midi_connected =
            self.playback_ctl.midi.input.as_ref().is_some_and(|input| input.is_connected());
        let accuracy_start = midi_connected.then(clock::now_seconds);
        self.playback_ctl.play_along.engaged = false;
        self.playback_ctl.accuracy.tracker.dismiss_summary();
        // The metronome plays no rhythm to play along with.
        if self.accuracy_active() {
            self.playback_ctl.accuracy.tracker.on_playback_start(accuracy_start);
        } else {
            self.playback_ctl.accuracy.tracker.on_playback_stop();
        }
        self.playback_ctl.playback.reset();
        // Mark the count-in as pending until the UI clock is known (first
        // frame of the measure panel fills in `since`).
        if self.playback_options().count_in {
            self.playback_ctl.playback.count_in = CountInState::Waiting { since: f64::NAN };
        }

        // The view in a chat has no permission for a wake lock.
        if !self.embedded() {
            self.ui.platform.acquire_wake_lock();
        }
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
            self.mark_tempo_unsaved();
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

    /// Whether timing feedback is at work: switched on, and in Practice, the
    /// rhythm to play along with (not the score's, not the metronome's).
    pub(crate) fn accuracy_active(&self) -> bool {
        self.playback_ctl.accuracy.enabled && !self.metronome_mode() && self.tab() != Tab::Score
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
        m.first_voice_mut().set_beat(0, Beat::note(q())).unwrap();
        m.first_voice_mut().set_beat(1, Beat::note(q())).unwrap();
        m.first_voice_mut().set_beat(2, Beat::note(q())).unwrap();
        m.first_voice_mut().set_beat(3, Beat::note(q())).unwrap();
        m
    }

    fn build_count_config(&self) -> Option<CountConfig> {
        let counting = self.ui.counting;
        let subdivision = counting.base.subdivision()?;
        Some(CountConfig { subdivision, system: counting.system })
    }

    pub fn new(cc: &CreationContext) -> Self {
        // Read before the link is opened, which clears the query.
        let query = platform::link_query();
        let embed = query.as_deref().and_then(embed_params);
        Self::new_with(cc, embed, query)
    }

    /// [`Self::new`] with the page's query and whether it embeds the app.
    pub(crate) fn new_with(
        cc: &CreationContext,
        embed: Option<EmbedParams>,
        query: Option<String>,
    ) -> Self {
        add_font(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let ff = FontFamily::Name("music".into());

        // Embedded, the user's own settings and library stay out of the chat.
        let saved = cc
            .storage
            .filter(|_| embed.is_none())
            .and_then(|storage| storage.get_string(APP_STATE_KEY));
        let (mut state, state_backup) = persist::restore(saved.as_deref());

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

        let midi = if embed.is_some() { None } else { Some(MidiInput::new()) };
        let (mut midi_input, midi_input_ports) = match midi.transpose() {
            Ok(None) => (None, Vec::new()),
            Ok(Some(mut input)) => {
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
        if embed.is_none() {
            update_check::start(cc.egui_ctx.clone());
        }

        let mut generator =
            GeneratorState::new(state.generator_settings, state.endless, state.ghost_notes);
        // The score tab edits a drum groove: a rhythm of the one line saved
        // there before starts it afresh (rhythms are Practice's).
        let score_doc = state.score_doc.into_doc(&state.library);
        let score_doc = if score_doc.score.staff == grooph_measure::StaffKind::Kit {
            score_doc
        } else {
            let ts = score_doc.score.measures()[0].time_signature();
            RhythmDoc::new(
                kit_palette::empty_groove(ts, grooph_measure::Voicing::default()),
                Cursor::start(),
            )
        };
        // Saved before Practice had its own: Practice starts with a new rhythm.
        let practice_doc = match state.practice_doc {
            Some(doc) => doc.into_doc(&state.library),
            None => RhythmDoc::new(
                generator.generate().unwrap_or_else(|| Score::single(Self::default_measure())),
                Cursor::start(),
            ),
        };
        let mut app = Self {
            editor: EditorState {
                document: Document::Score,
                docs: Documents {
                    // The click bar and counter rhythm of its meter: from
                    // here on they are kept in it alone.
                    metronome: RhythmDoc::new(
                        Score::single(metronome_bar(
                            state.click_bars.for_meter(state.metronome_meter),
                            state.counter_bars.for_meter(state.metronome_meter),
                        )),
                        Cursor::start(),
                    ),
                    practice: practice_doc,
                    score: score_doc,
                },
                button_measures,
                instrument: Instrument::Snare,
                library: state.library,
                presets: state.presets,
                active_preset: None,
                state_backup,
                metronome: MetronomeState::new(),
                generator,
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
                show_score_tab: state.show_score_tab,
                guide: state.guide,
                platform,
                renaming_preset: None,
                renaming_pattern: None,
                rename_typed: String::new(),
                pending_load: None,
                link_error: None,
                link_copied_at: None,
                library_saved: None,
                scroll_to_library_item: None,
                library_tab: Default::default(),
                narrow_screen: false,
                staff_em: 40.0,
                followed_cursor: None,
                followed_view_height: 0.0,
                touch_seen: platform::finger_is_the_pointer(),
                touching: false,
                shell: ShellState::default(),
                picker: None,
                picker_return: None,
                tap_tempo: Default::default(),
                host: None,
                embed: embed.map(|params| Embed::new(&cc.egui_ctx, params)),
            },
        };
        app.editor.active_preset = app.matching_preset();
        // The page the More screen was opened from, then the saved tab;
        // Practice for the score tab while it is hidden.
        let shown = |tab| match tab {
            Tab::Score if !state.show_score_tab => Tab::Practice,
            tab => tab,
        };
        app.select_tab(shown(state.document.tab()));
        app.select_tab(shown(state.tab));
        // The editor of that page, if it was open.
        if state.editing && app.ui.mode == Mode::Playback && !app.embedded() {
            app.ui.mode = Mode::Edit;
        }
        // A link opens after the saved tab, so its tab wins.
        if let Some(query) = query {
            app.open_link_from_url(&query);
        }
        // Embedded, only the metronome is shown, whatever the link opened.
        if app.embedded() && !app.metronome_mode() {
            app.select_tab(Tab::Metronome);
        }
        app
    }

    /// Open the link the app was started with (web only) and drop its
    /// parameters from the address bar, so a reload keeps later edits.
    fn open_link_from_url(&mut self, query: &str) {
        match grooph_link::parse_query(query) {
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
    use crate::shell::Tab;
    use grooph_measure::counting::CountingSystem;
    use grooph_measure::{Measure, Score, TimeSignature};

    #[test]
    fn button_measures_fill_their_time_signature() {
        use super::Grooph;
        use crate::tools::{ToolKind, all_tools};
        use grooph_measure::grid::DEFAULT_GRID;

        let templates = all_tools().iter().filter_map(|t| match t.kind {
            ToolKind::InsertBeat(template) => Some(template),
            _ => None,
        });
        let dotted = super::BeatTemplate {
            kind: grooph_measure::BeatKind::Note,
            duration: grooph_measure::duration::Duration::Dotted {
                dots: 1,
                base: grooph_measure::duration::NoteValue::Quarter,
            },
            accented: false,
            soft: false,
        };
        let quarter = super::BeatTemplate {
            duration: grooph_measure::duration::Duration::Simple(
                grooph_measure::duration::NoteValue::Quarter,
            ),
            ..dotted
        };
        let accented = super::BeatTemplate { accented: true, ..quarter };
        let soft = super::BeatTemplate { soft: true, ..quarter };
        let mut count = 0;
        for template in templates.chain([dotted, accented, soft]) {
            let measure = Grooph::build_button_measure(template);
            let ticks: u32 = measure
                .first_voice()
                .beats()
                .iter()
                .map(|b| DEFAULT_GRID.ticks_of(&b.duration).unwrap())
                .sum();
            let ts = measure.time_signature();
            assert_eq!(ticks, DEFAULT_GRID.ticks_per_measure(&ts), "{:?}", template.duration);
            let first = measure.first_voice().beats()[0];
            assert_eq!((first.kind(), first.duration), (template.kind, template.duration));
            if !template.duration.is_tuplet() {
                assert_eq!(measure.first_voice().beats().len(), 1, "{:?}", template.duration);
                assert_eq!((first.accented(), first.soft()), (template.accented, template.soft));
            }
            count += 1;
        }
        assert!(count > 10);
    }

    #[test]
    fn back_closes_one_layer_at_a_time() {
        use super::{Grooph, Mode};
        use crate::pickers::Picker;

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::More);
        app.ui.picker = Some(Picker::Settings);
        assert!(app.back());
        assert_eq!(app.ui.picker, None);
        assert_eq!(app.ui.mode, Mode::MoreMenu);
        // The MIDI input picker opened from Play along goes back to it.
        app.ui.picker = Some(Picker::MidiInput);
        app.ui.picker_return = Some(Picker::PlayAlong);
        assert!(app.back());
        assert_eq!(app.ui.picker, Some(Picker::PlayAlong));
        assert!(app.back());
        assert_eq!(app.ui.picker, None);
        // Nothing left to close: the host leaves the app. The practice
        // options sheet is not on this page, so it does not count.
        app.ui.shell.options_collapsed = false;
        assert!(!app.back());
        assert_eq!(app.ui.mode, Mode::MoreMenu);
        // On the practice page, back folds the sheet first.
        app.select_tab(Tab::Practice);
        assert!(app.back());
        assert!(app.ui.shell.options_collapsed);
        assert!(!app.back());
        // Leaving an editor, by back, Esc or Done, shows the page itself:
        // its sheet folds away too.
        for tab in [Tab::Practice, Tab::Metronome] {
            app.select_tab(tab);
            for leave in [
                Grooph::back as fn(&mut Grooph) -> bool,
                |app| {
                    app.toggle_edit();
                    true
                },
                |app| {
                    app.leave_editor();
                    true
                },
            ] {
                app.ui.shell.options_collapsed = false;
                app.edit_page();
                assert!(leave(&mut app));
                assert_eq!(app.ui.mode, Mode::Playback, "{tab:?}");
                assert!(app.ui.shell.options_collapsed, "{tab:?}");
            }
        }
    }

    #[test]
    fn an_editor_folds_the_options_sheet_as_it_opens() {
        use super::Grooph;

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        for tab in [Tab::Practice, Tab::Score, Tab::Metronome] {
            app.select_tab(tab);
            app.ui.shell.options_collapsed = false;
            shell_frame(&mut app, &ctx, 0.0, vec![]);
            app.edit_page();
            shell_frame(&mut app, &ctx, 0.1, vec![]);
            // Folded already, so Done shows no sheet sliding shut.
            assert!(app.ui.shell.options_collapsed, "{tab:?}");
            app.leave_editor();
        }
    }

    #[test]
    fn more_leaves_the_playing_metronome_alone() {
        use super::{Grooph, Mode, TransportState};

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Metronome);
        app.start_playback();
        app.select_tab(Tab::More);
        assert_eq!(app.tab(), Tab::More);
        assert!(app.metronome_mode());
        assert_eq!(app.playback_ctl.transport_state, TransportState::Playing);
        app.select_tab(Tab::Metronome);
        assert_eq!(app.ui.mode, Mode::Playback);
        assert_eq!(app.playback_ctl.transport_state, TransportState::Playing);
        // From More to a rhythm page the metronome closes, as from its tab.
        app.select_tab(Tab::More);
        app.select_tab(Tab::Practice);
        assert!(!app.metronome_mode());
        assert_eq!(app.playback_ctl.transport_state, crate::TransportState::Stopped);
    }

    #[test]
    fn library_holds_the_score_and_loads_it_into_the_score_tab() {
        use super::{Grooph, Mode, TransportState};
        use crate::pickers::Picker;

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.edit_page();
        app.editor.doc_mut().score = Score::single(Measure::new(TimeSignature::new(3, 4)))
            .with_staff(grooph_measure::StaffKind::Kit);
        let rhythm = app.editor.doc().score.clone();
        app.save_pattern_as();
        let id = app.editor.doc().active_pattern_id.unwrap();

        // Through the metronome to More: the library leaves it open.
        app.select_tab(Tab::Metronome);
        app.start_playback();
        app.select_tab(Tab::More);
        app.ui.picker = Some(Picker::Library);
        assert!(app.metronome_mode());
        // Saving keeps the score tab's score, not the click bar, and the
        // click plays on.
        app.save_pattern_as();
        assert!(app.metronome_mode());
        assert_eq!(app.playback_ctl.transport_state, TransportState::Playing);
        assert_eq!(format!("{:?}", app.editor.docs.score.score), format!("{rhythm:?}"));
        let saved = app.editor.docs.score.active_pattern_id.unwrap();
        let saved = app.editor.library.get(saved).unwrap();
        assert_eq!(format!("{:?}", saved.score), format!("{rhythm:?}"));
        // So does loading.
        app.select_tab(Tab::Metronome);
        app.select_tab(Tab::Score);
        app.edit_page();
        app.select_tab(Tab::Metronome);
        app.select_tab(Tab::More);

        // Loading closes the dialog and opens the score tab.
        app.request_load_pattern(id);
        assert_eq!(app.tab(), Tab::Score);
        assert_eq!(app.ui.picker, None);
        app.editor.doc_mut().active_pattern_id = None;
        app.editor.doc_mut().dirty = true;
        // From the practice page too, which keeps its rhythm.
        app.select_tab(Tab::Practice);
        let practice = format!("{:?}", app.editor.doc().score);
        app.select_tab(Tab::More);
        app.ui.picker = Some(Picker::Library);
        app.request_load_pattern(id);
        // The score tab's unsaved changes ask first, above the library.
        assert_eq!(app.tab(), Tab::More);
        assert_eq!(app.ui.picker, Some(Picker::Library));
        let pending = app.ui.pending_load.take().unwrap();
        app.finish_pending_load(pending);
        assert_eq!(app.tab(), Tab::Score);
        assert_eq!(app.ui.picker, None);
        assert_eq!(app.editor.doc().active_pattern_id, Some(id));
        app.select_tab(Tab::Practice);
        assert_eq!(format!("{:?}", app.editor.doc().score), practice);
        // Loading in the editor stays there.
        app.select_tab(Tab::Score);
        app.edit_page();
        app.editor.doc_mut().active_pattern_id = None;
        app.request_load_pattern(id);
        assert_eq!(app.ui.mode, Mode::Edit);
    }

    #[test]
    fn mixer_dialog_fits_its_sliders_where_the_screen_allows() {
        use super::Grooph;
        use crate::mixer_panel::mixer_width;
        use crate::pickers::Picker;
        use eframe::egui::{Pos2, RawInput, Rect, vec2};

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.ui.picker = Some(Picker::Mixer);
        let mut frame = |width: f32| {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 800.0))),
                ..Default::default()
            };
            let mut measured = None;
            let mut output = ctx.run_ui(input, |ui| {
                app.apply_style(ui);
                app.settings_picker(ui);
                measured = mixer_width(ui);
            });
            output.textures_delta.clear();
            measured.unwrap()
        };
        let mut sizes = Vec::new();
        for _ in 0..4 {
            sizes.push(frame(1200.0));
        }
        // Wider than the other dialogs, measured once and then stable.
        let wide = *sizes.last().unwrap();
        assert!(wide > 380.0 && wide < 1100.0, "{sizes:?}");
        assert_eq!(sizes[2], wide, "{sizes:?}");
    }

    #[test]
    fn library_previews_render_at_phone_width() {
        use super::Grooph;
        use crate::pickers::Picker;
        use eframe::egui::{Pos2, RawInput, Rect, vec2};
        use grooph_measure::Beat;
        use grooph_measure::duration::{e, q, s, t8};

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.edit_page();
        // From sparse to too dense for the row, which then clips.
        for d in [q(), e(), s(), t8()] {
            let mut m = Measure::new(TimeSignature::FOUR_FOUR);
            let mut i = 0;
            while i < m.first_voice().beats().len() {
                m.first_voice_mut().set_beat(i, Beat::note(d)).unwrap();
                i += 1;
            }
            app.editor.doc_mut().score = Score::new(vec![m.clone(), m]);
            app.save_pattern_as();
        }
        app.select_tab(Tab::More);
        app.ui.picker = Some(Picker::Library);
        for _ in 0..3 {
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(390.0, 844.0));
            let input = RawInput { screen_rect: Some(screen), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                app.apply_style(ui);
                app.settings_picker(ui);
            });
            output.textures_delta.clear();
        }
        assert_eq!(app.ui.picker, Some(Picker::Library));
    }

    #[test]
    fn a_preset_from_the_library_opens_the_practice_page() {
        use super::{Grooph, Mode};
        use crate::pickers::Picker;
        use crate::presets::{PresetId, builtin_presets};

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        // Through the metronome: the click bar stays the metronome's.
        app.select_tab(Tab::Metronome);
        let click_bar = format!("{:?}", app.editor.doc().score);
        app.select_tab(Tab::More);
        app.ui.picker = Some(Picker::Library);
        let last = builtin_presets().len() - 1;
        app.open_preset(PresetId::BuiltIn(last));
        assert_eq!(app.tab(), Tab::Practice);
        assert_eq!(app.ui.mode, Mode::Playback);
        assert_eq!(app.ui.picker, None);
        assert_eq!(app.editor.active_preset, Some(PresetId::BuiltIn(last)));
        assert_eq!(app.editor.generator.settings, builtin_presets()[last].1.generator);
        app.select_tab(Tab::Metronome);
        assert_eq!(format!("{:?}", app.editor.doc().score), click_bar);
    }

    #[test]
    fn the_score_tabs_library_holds_rhythms_only() {
        use super::Grooph;
        use crate::library_panel::LibraryTab;
        use crate::pickers::Picker;
        use eframe::egui::{Pos2, RawInput, Rect, Shape, vec2};

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.ui.library_tab = LibraryTab::Presets;
        let texts = |app: &mut Grooph| {
            let mut texts = Vec::new();
            for _ in 0..2 {
                let screen = Rect::from_min_size(Pos2::ZERO, vec2(390.0, 844.0));
                let input = RawInput { screen_rect: Some(screen), ..Default::default() };
                let mut output = ctx.run_ui(input, |ui| {
                    app.apply_style(ui);
                    app.settings_picker(ui);
                });
                output.textures_delta.clear();
                texts = output
                    .shapes
                    .iter()
                    .filter_map(|clipped| match &clipped.shape {
                        Shape::Text(text) => Some(text.galley.text().to_owned()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
            }
            texts
        };
        // From More: both halves.
        app.select_tab(Tab::More);
        app.ui.picker = Some(Picker::Library);
        assert!(texts(&mut app).iter().any(|t| t == "Presets"));
        // From the score tab's card: its rhythms, no presets.
        app.select_tab(Tab::Score);
        app.ui.picker = Some(Picker::Library);
        let shown = texts(&mut app);
        assert!(!shown.iter().any(|t| t == "Presets"), "{shown:?}");
        assert!(shown.iter().any(|t| t.contains("Save as new")), "{shown:?}");
    }

    #[test]
    fn library_and_preset_picker_render_at_phone_width() {
        use super::Grooph;
        use crate::library_panel::LibraryTab;
        use crate::pickers::Picker;
        use eframe::egui::{Pos2, RawInput, Rect, vec2};

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Practice);
        app.save_preset_as("Mine");
        app.select_tab(Tab::More);
        app.ui.picker = Some(Picker::Library);
        for tab in [LibraryTab::Rhythms, LibraryTab::Presets] {
            app.ui.library_tab = tab;
            for _ in 0..2 {
                let screen = Rect::from_min_size(Pos2::ZERO, vec2(390.0, 844.0));
                let input = RawInput { screen_rect: Some(screen), ..Default::default() };
                let mut output = ctx.run_ui(input, |ui| {
                    app.apply_style(ui);
                    app.settings_picker(ui);
                });
                output.textures_delta.clear();
            }
        }
        assert_eq!(app.ui.picker, Some(Picker::Library));
        // The Preset picker on the practice page shows the same cards.
        app.select_tab(Tab::Practice);
        app.ui.picker = Some(Picker::Preset);
        for _ in 0..2 {
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(390.0, 844.0));
            let input = RawInput { screen_rect: Some(screen), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                app.apply_style(ui);
                app.settings_picker(ui);
            });
            output.textures_delta.clear();
        }
        assert_eq!(app.ui.picker, Some(Picker::Preset));
    }

    /// Runs frames of the shell at phone size, `time` seconds into the run,
    /// with `events` in the first.
    fn shell_frame(
        app: &mut super::Grooph,
        ctx: &eframe::egui::Context,
        time: f64,
        events: Vec<eframe::egui::Event>,
    ) {
        use eframe::egui::{Pos2, RawInput, Rect, vec2};
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(390.0, 844.0));
        let input =
            RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            app.apply_style(ui);
            app.shell_ui(ui);
            app.update_endless();
            app.handle_keyboard_input(ui);
        });
        output.textures_delta.clear();
    }

    #[test]
    fn endless_mode_drops_played_bars_behind_the_more_screen() {
        use super::Grooph;

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Practice);
        app.editor.generator.endless = true;
        app.playback_ctl.bpm = 240;
        app.start_playback();
        app.select_tab(Tab::More);
        // A minute at 240 BPM: sixty bars of 4/4 go by.
        let mut time = 0.0;
        while time < 60.0 {
            shell_frame(&mut app, &ctx, time, vec![]);
            time += 0.1;
        }
        assert!(app.editor.doc().score.len() <= 4, "{} bars", app.editor.doc().score.len());
    }

    #[test]
    fn endless_mode_leaves_the_edited_score_alone() {
        use super::Grooph;

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Practice);
        app.editor.generator.endless = true;
        app.select_tab(Tab::Score);
        app.edit_page();
        let score = format!("{:?}", app.editor.doc().score);
        app.playback_ctl.bpm = 240;
        app.start_playback();
        // Also past the last bar and behind the More screen opened from Edit.
        let mut time = 0.0;
        while time < 20.0 {
            if time == 10.0 {
                app.select_tab(Tab::More);
            }
            shell_frame(&mut app, &ctx, time, vec![]);
            time += 0.5;
        }
        assert_eq!(format!("{:?}", app.editor.doc().score), score);
    }

    #[test]
    fn esc_on_the_more_screen_closes_the_dialog_first() {
        use super::Grooph;
        use crate::pickers::Picker;
        use eframe::egui::{Event, Key, Modifiers};

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::More);
        app.ui.picker = Some(Picker::Mixer);
        shell_frame(&mut app, &ctx, 0.0, vec![]);
        let esc = |pressed| Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        shell_frame(&mut app, &ctx, 0.1, vec![esc(true), esc(false)]);
        assert_eq!(app.ui.picker, None);
        assert_eq!(app.tab(), Tab::More);
        // Without a dialog, Esc stays on More, also with the metronome
        // behind it: its click bar editor does not open.
        app.select_tab(Tab::Metronome);
        app.select_tab(Tab::More);
        shell_frame(&mut app, &ctx, 0.2, vec![esc(true), esc(false)]);
        assert_eq!(app.tab(), Tab::More);
        assert!(app.metronome_mode());
        app.select_tab(Tab::Metronome);
        assert_eq!(app.ui.mode, super::Mode::Playback);
    }

    #[test]
    fn save_is_off_while_there_is_nothing_new_to_save() {
        use super::Grooph;
        use crate::presets::{PresetId, builtin_presets};

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.edit_page();
        // Rhythms: a new one while none is active, then the active one only
        // with changes.
        app.editor.doc_mut().active_pattern_id = None;
        let target = app.rhythm_save_target();
        assert_eq!((target.overwrites, target.enabled), (None, true));
        app.save_pattern_as();
        let target = app.rhythm_save_target();
        assert!(target.overwrites.is_some() && !target.enabled);
        app.editor.doc_mut().dirty = true;
        assert!(app.rhythm_save_target().enabled);

        // Presets: an unchanged built-in has nothing to save, a changed one
        // saves a new preset.
        app.select_tab(Tab::Practice);
        app.apply_preset(PresetId::BuiltIn(0));
        assert_eq!(app.editor.generator.settings, builtin_presets()[0].1.generator);
        let target = app.preset_save_target();
        assert_eq!((target.overwrites, target.enabled), (None, false));
        app.playback_ctl.bpm += 1;
        let target = app.preset_save_target();
        assert_eq!((target.overwrites, target.enabled), (None, true));
        let id = app.save_preset_as("");
        let target = app.preset_save_target();
        assert_eq!(target.overwrites, app.editor.presets.get(id).map(|p| p.name.clone()));
        assert!(!target.enabled);
        app.playback_ctl.bpm += 1;
        assert!(app.preset_save_target().enabled);
        app.editor.active_preset = None;
        assert!(app.preset_save_target().enabled);
    }

    #[test]
    fn a_load_that_asks_first_leaves_the_metronome_playing() {
        use super::{Grooph, TransportState};

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.edit_page();
        let id = app.save_pattern_as();
        app.editor.doc_mut().dirty = true;
        app.select_tab(Tab::Metronome);
        app.start_playback();
        app.select_tab(Tab::More);
        app.editor.doc_mut().active_pattern_id = None;
        app.request_load_pattern(id);
        assert!(app.ui.pending_load.is_some());
        assert!(app.metronome_mode());
        assert_eq!(app.playback_ctl.transport_state, TransportState::Playing);
        // Loading after all puts the metronome away.
        let pending = app.ui.pending_load.take().unwrap();
        app.finish_pending_load(pending);
        assert!(!app.metronome_mode());
        assert_eq!(app.editor.doc().active_pattern_id, Some(id));
    }

    #[test]
    fn library_dialog_stays_within_a_phone_screen() {
        use super::Grooph;
        use crate::pickers::Picker;
        use eframe::egui::Id;

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.edit_page();
        let first = app.save_pattern_as();
        app.save_pattern_as();
        app.save_pattern_as();
        app.editor.library.patterns[1].name = "A name far too long for a phone's dialog".into();
        app.select_tab(Tab::More);
        app.ui.picker = Some(Picker::Library);
        let width = |app: &mut Grooph| {
            for i in 0..3 {
                shell_frame(app, &ctx, f64::from(i) * 0.1, vec![]);
            }
            ctx.memory(|m| m.area_rect(Id::unique("settings_picker"))).unwrap().width()
        };
        let closed = width(&mut app);
        assert!(closed <= 390.0, "{closed}");
        // Renaming opens a field and two buttons under the card.
        app.ui.renaming_pattern = Some(first);
        assert_eq!(width(&mut app), closed);
    }

    #[test]
    fn a_link_asks_first_for_a_rhythm_put_aside_by_the_metronome() {
        use super::Grooph;
        use grooph_link::{LinkContent, SharedLink};

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.ui.show_score_tab = true;
        app.select_tab(Tab::Score);
        app.edit_page();
        app.editor.doc_mut().dirty = true;
        app.select_tab(Tab::Metronome);
        app.select_tab(Tab::More);
        // The click bar has no changes; the rhythm behind it has.
        app.editor.doc_mut().dirty = false;
        let score = Score::single(Measure::new(TimeSignature::new(3, 4)))
            .with_staff(grooph_measure::StaffKind::Kit);
        app.open_link(SharedLink {
            content: Some(LinkContent::Score(score)),
            ..Default::default()
        });
        assert!(app.ui.pending_load.is_some());
        assert!(app.metronome_mode());
    }

    #[test]
    fn a_link_with_a_doubled_drum_opens_with_it_once() {
        use super::Grooph;
        use grooph_measure::notation::format_score;

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        let groove = "4/4 S-H:q H:q H:h ; >S:q K:q K:h";
        let query = format!("v=2&r={}", groove.replace(' ', "+").replace(';', "%3B"));
        app.open_link(grooph_link::parse_query(&query).unwrap().unwrap());
        let score = &app.editor.doc().score;
        assert_eq!(format_score(score).unwrap(), "4/4 >S-H:q H:q H:h ; r:q K:q K:h");
        // No longer the link's groove: a change to save.
        assert!(app.editor.doc().dirty);
    }

    #[test]
    fn a_shared_rhythm_opens_on_the_page_it_was_shared_from() {
        use super::Grooph;
        use grooph_measure::notation::{format_score, parse_score};

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut from = Grooph::new(&cc);
        for (tab, rhythm) in
            [(Tab::Practice, "4/4 q e e q q"), (Tab::Score, "3/4 H:q S-H:q H:q ; K:q r:q r:q")]
        {
            from.select_tab(tab);
            from.editor.doc_mut().score = parse_score(rhythm).unwrap();
            if tab == Tab::Score {
                from.editor.doc_mut().score.staff = grooph_measure::StaffKind::Kit;
            }
            let url = from.share_link_url().unwrap();
            let query = url.split_once('?').unwrap().1;
            let mut to = Grooph::new(&cc);
            to.ui.show_score_tab = true;
            let practice_before = format_score(&to.editor.docs.practice.score).unwrap();
            to.open_link(grooph_link::parse_query(query).unwrap().unwrap());
            assert_eq!(to.tab(), tab, "{url}");
            assert_eq!(format_score(&to.editor.doc().score).unwrap(), rhythm);
            // The other page keeps its rhythm.
            if tab == Tab::Score {
                assert_eq!(format_score(&to.editor.docs.practice.score).unwrap(), practice_before);
            }
        }
        // With the score tab hidden, a score's link opens on Practice, without
        // asking about the hidden score's changes.
        let url = from.share_link_url().unwrap();
        let mut to = Grooph::new(&cc);
        assert!(!to.ui.show_score_tab);
        to.editor.docs.score.dirty = true;
        to.open_link(grooph_link::parse_query(url.split_once('?').unwrap().1).unwrap().unwrap());
        assert!(to.ui.pending_load.is_none());
        assert_eq!(to.tab(), Tab::Practice);
        assert_eq!(
            format_score(&to.editor.doc().score).unwrap(),
            "3/4 H:q S-H:q H:q ; K:q r:q r:q"
        );
    }

    #[test]
    fn a_rhythm_of_the_one_line_opens_on_practice_never_in_the_score_tab() {
        use super::Grooph;
        use grooph_measure::notation::{format_score, parse_score};

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.ui.show_score_tab = true;
        app.select_tab(Tab::Score);
        // The score tab starts with a groove, not a rhythm.
        assert!(app.editor.doc().score.staff == grooph_measure::StaffKind::Kit);
        let url = {
            let mut from = Grooph::new(&cc);
            from.select_tab(Tab::Practice);
            from.editor.doc_mut().score = parse_score("3/4 q q e e").unwrap();
            from.share_link_url().unwrap()
        };
        app.open_link(grooph_link::parse_query(url.split_once('?').unwrap().1).unwrap().unwrap());
        assert_eq!(app.tab(), Tab::Practice);
        assert_eq!(format_score(&app.editor.doc().score).unwrap(), "3/4 q q e e");
        assert!(app.editor.docs.score.score.staff == grooph_measure::StaffKind::Kit);
        // So does one from the library, as the pattern loaded and unchanged.
        let id = app.editor.library.add("Old".into(), parse_score("4/4 h h").unwrap(), 90);
        app.select_tab(Tab::Score);
        // Without asking about the score tab's changes, which it leaves alone.
        app.editor.docs.score.dirty = true;
        app.request_load_pattern(id);
        assert!(app.ui.pending_load.is_none());
        assert!(app.editor.docs.score.dirty);
        assert_eq!(app.tab(), Tab::Practice);
        assert_eq!(format_score(&app.editor.doc().score).unwrap(), "4/4 h h");
        assert!(!app.editor.docs.practice.dirty);
    }

    #[test]
    fn the_bar_meter_picker_swallows_the_shortcuts() {
        use super::{Grooph, TransportState};
        use eframe::egui::{Event, Key, Modifiers};

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.edit_page();
        app.apply_tool(crate::tool_palette::tool("meta.change_time_signature"));
        let space = Event::Key {
            key: Key::Space,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        shell_frame(&mut app, &ctx, 0.0, vec![space]);
        assert_eq!(app.playback_ctl.transport_state, TransportState::Stopped);
        assert_eq!(app.ui.mode, super::Mode::Edit);
    }

    #[test]
    fn the_editor_draws_timing_marks_beside_its_ghost_bar() {
        use super::Grooph;

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Practice);
        app.playback_ctl.accuracy.enabled = true;
        app.edit_page();
        // A run's marks are on the staff while the editor shows the ghost bar.
        app.playback_ctl.accuracy.tracker.on_playback_start(Some(0.0));
        assert!(app.playback_ctl.accuracy.tracker.has_marks());
        shell_frame(&mut app, &ctx, 0.0, vec![]);
        shell_frame(&mut app, &ctx, 0.1, vec![]);
    }

    #[test]
    fn after_a_finger_the_staff_leaves_its_drags_to_the_scroll_area() {
        use super::Grooph;
        use eframe::egui::{Event, TouchDeviceId, TouchId, TouchPhase, pos2};

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.edit_page();
        let touch = |phase| Event::Touch {
            device_id: TouchDeviceId(0),
            id: TouchId(0),
            phase,
            pos: pos2(100.0, 100.0),
            force: None,
        };
        shell_frame(&mut app, &ctx, 0.0, vec![]);
        assert!(!app.ui.touch_seen);
        // The finger is down: not yet, a gesture keeps its sense.
        shell_frame(&mut app, &ctx, 0.1, vec![touch(TouchPhase::Start)]);
        assert!(!app.ui.touch_seen);
        shell_frame(&mut app, &ctx, 0.2, vec![touch(TouchPhase::End)]);
        shell_frame(&mut app, &ctx, 0.3, vec![]);
        assert!(app.ui.touch_seen);
    }

    #[test]
    fn the_score_tab_always_shows_its_meter() {
        use super::Grooph;
        use eframe::egui::{Pos2, RawInput, Rect, vec2};

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.ui.show_score_tab = true;
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(390.0, 300.0));
        let first = |app: &mut Grooph, tab| {
            app.select_tab(tab);
            let mut shown = false;
            let input = RawInput { screen_rect: Some(rect), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                shown = app.staff_opts(ui, rect).time_signature_first;
            });
            output.textures_delta.clear();
            shown
        };
        assert!(first(&mut app, Tab::Score));
        assert!(!first(&mut app, Tab::Practice));
    }

    #[test]
    fn a_kit_link_opens_and_shares_the_groove() {
        use super::Grooph;
        use grooph_measure::StaffKind;

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.ui.show_score_tab = true;
        let groove = "4/4 H:e S-H:e H:e H:e H:e S-H:e Ho:e Hc:e ; K:q r:q K:q r:q";
        let query = format!("v=2&bpm=96&r={}", groove.replace(' ', "+").replace(';', "%3B"));
        app.open_link(grooph_link::parse_query(&query).unwrap().unwrap());
        assert_eq!(app.editor.doc().score.staff, StaffKind::Kit);
        assert_eq!(app.editor.doc().score.measures()[0].voices().len(), 2);
        // It opens in the score tab; Practice keeps its rhythm.
        assert_eq!(app.tab(), Tab::Score);
        for (i, tab) in [Tab::Score, Tab::Practice, Tab::Score].into_iter().enumerate() {
            app.select_tab(tab);
            shell_frame(&mut app, &ctx, i as f64 * 0.1, vec![]);
            assert_eq!(app.editor.doc().score.staff == StaffKind::Kit, tab == Tab::Score);
        }
        // Sharing writes the groove back.
        let url = app.share_link_url().unwrap();
        let shared = grooph_link::parse_query(url.split_once('?').unwrap().1).unwrap().unwrap();
        let Some(grooph_link::LinkContent::Score(score)) = shared.content else { panic!("{url}") };
        assert_eq!(grooph_measure::notation::format_score(&score).unwrap(), groove);
    }

    #[test]
    fn a_tap_below_the_line_edits_the_lower_voice() {
        use super::{Grooph, Mode};
        use eframe::egui::{Event, Modifiers, PointerButton, pos2};
        use grooph_measure::Cursor;
        use grooph_measure::notation::parse_score;

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.edit_page();
        app.editor.doc_mut().score =
            parse_score("4/4 H:e H:e H:e H:e H:e H:e H:e H:e ; K:q K:q K:q K:q").unwrap();
        app.ui.mode = Mode::Edit;
        let mut time = 0.0;
        let mut tap = |app: &mut Grooph, y: f32| {
            let pos = pos2(215.0, y);
            for pressed in [true, false] {
                let event = Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                };
                time += 0.5;
                shell_frame(app, &ctx, time, vec![Event::PointerMoved(pos), event]);
            }
            app.editor.doc().cursor
        };
        shell_frame(&mut app, &ctx, 0.0, vec![]);
        // Down from the top of the staff: the hands first, then the kick.
        // Down the middle of the staff: the hands' fifth eighth, then the
        // kick's third quarter.
        let cursors: Vec<Cursor> = (0..100).step_by(4).map(|y| tap(&mut app, y as f32)).collect();
        let kick = cursors.iter().position(|c| c.voice == 1).expect("a tap on the kick");
        assert!(cursors[..kick].contains(&Cursor::in_voice(0, 0, 4)), "{cursors:?}");
        assert_eq!(cursors[kick], Cursor::in_voice(0, 1, 2));
    }

    #[test]
    fn deleting_the_rhythm_behind_the_metronome_forgets_it() {
        use super::Grooph;

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.edit_page();
        let id = app.save_pattern_as();
        app.select_tab(Tab::Metronome);
        app.select_tab(Tab::More);
        app.delete_pattern(id);
        app.select_tab(Tab::Score);
        app.edit_page();
        assert_eq!(app.editor.doc().active_pattern_id, None);
    }

    #[test]
    fn keys_on_the_more_screen_leave_the_hidden_score_alone() {
        use super::{Grooph, TransportState};
        use crate::tools::all_tools;
        use eframe::egui::{Event, Key, Modifiers};

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.edit_page();
        app.select_tab(Tab::More);
        let score = format!("{:?}", app.editor.doc().score);
        let key = |key, pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        // Every plain tool key: none edits the score behind More (tools run
        // in edit mode only).
        let mut time = 0.0;
        for tool in all_tools() {
            for shortcut in tool.shortcuts.iter().filter(|s| !s.shift && !s.command) {
                if shortcut.key == Key::Space || shortcut.key == Key::Escape {
                    continue;
                }
                time += 0.1;
                shell_frame(
                    &mut app,
                    &ctx,
                    time,
                    vec![key(shortcut.key, true), key(shortcut.key, false)],
                );
            }
        }
        assert_eq!(format!("{:?}", app.editor.doc().score), score);
        assert_eq!(app.tab(), Tab::More);
        // Space still plays.
        shell_frame(
            &mut app,
            &ctx,
            time + 0.1,
            vec![key(Key::Space, true), key(Key::Space, false)],
        );
        assert_eq!(app.playback_ctl.transport_state, TransportState::Playing);
    }

    #[test]
    fn save_as_new_opens_the_name_and_says_saved() {
        use super::Grooph;
        use crate::library_panel::{LibraryItem, SaveAs};

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.edit_page();
        let id = app.save_rhythm(SaveAs::New, 1.0);
        assert_eq!(app.ui.renaming_pattern, Some(id));
        assert_eq!(app.ui.rename_typed, app.editor.library.get(id).unwrap().name);
        assert_eq!(app.ui.library_saved, Some((LibraryItem::Pattern(id), 1.0)));
        assert_eq!(app.ui.scroll_to_library_item, Some(LibraryItem::Pattern(id)));
        // Overwriting names no new pattern and opens no name.
        app.ui.renaming_pattern = None;
        app.editor.doc_mut().dirty = true;
        assert_eq!(app.save_rhythm(SaveAs::Update, 2.0), id);
        assert_eq!(app.editor.library.patterns.len(), 1);
        assert_eq!(app.ui.renaming_pattern, None);

        app.select_tab(Tab::Practice);
        let preset = app.save_preset(SaveAs::New, 3.0);
        assert_eq!(app.ui.renaming_preset, Some(preset));
        assert_eq!(app.ui.library_saved, Some((LibraryItem::Preset(preset), 3.0)));
        app.playback_ctl.bpm += 1;
        assert_eq!(app.save_preset(SaveAs::Update, 4.0), preset);
        assert_eq!(app.editor.presets.get(preset).unwrap().settings, app.preset_snapshot());
    }

    #[test]
    fn more_dialogs_stay_within_a_phone_screen() {
        use super::Grooph;
        use crate::pickers::Picker;
        use eframe::egui::Id;

        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        let mut time = 0.0;
        for picker in [Picker::Mixer, Picker::Settings, Picker::Help, Picker::Preset] {
            app.select_tab(if picker == Picker::Preset { Tab::Practice } else { Tab::More });
            app.ui.picker = Some(picker);
            for _ in 0..3 {
                time += 0.1;
                shell_frame(&mut app, &ctx, time, vec![]);
            }
            let rect = ctx.memory(|m| m.area_rect(Id::unique("settings_picker"))).unwrap();
            assert!(rect.width() <= 390.0 && rect.height() <= 844.0, "{picker:?}: {rect:?}");
        }
    }

    /// Everything a calibration run must leave alone, as text.
    fn user_state(app: &super::Grooph) -> String {
        let docs = &app.editor.docs;
        let audio = &app.playback_ctl.audio_cfg;
        format!(
            "{:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {} {} {} {} {} {} | {} {:?} {} {:?} {:?} {} {}",
            docs.metronome.score,
            docs.practice.score,
            docs.score.score,
            docs.metronome.cursor,
            docs.practice.cursor,
            docs.score.cursor,
            docs.metronome.active_pattern_id,
            docs.practice.active_pattern_id,
            docs.score.active_pattern_id,
            docs.metronome.dirty,
            docs.practice.dirty,
            docs.score.dirty,
            docs.metronome.history.can_undo(),
            docs.practice.history.can_undo(),
            docs.score.history.can_undo(),
            app.playback_ctl.bpm,
            audio.swing,
            audio.count_in,
            audio.gap,
            audio.ramp,
            app.editor.generator.endless,
            app.editor.generator.ghost_notes,
        )
    }

    #[test]
    fn a_calibration_run_touches_no_page_and_no_setting() {
        use super::Grooph;
        use crate::play_along::HitSource;
        use grooph_measure::swing::Swing;

        for page in [Tab::Metronome, Tab::Practice, Tab::Score] {
            let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
            let mut app = Grooph::new(&cc);
            app.playback_ctl.bpm = 133;
            app.playback_ctl.audio_cfg.swing =
                Swing { unit: grooph_measure::swing::SwingUnit::Eighths, percent: 66 };
            app.editor.generator.endless = true;
            app.editor.generator.ghost_notes = true;
            app.playback_ctl.audio_cfg.gap.enabled = true;
            app.select_tab(page);
            let document = app.document();
            app.select_tab(Tab::More);
            let before = user_state(&app);
            app.start_calibration(HitSource::Tap);
            // It plays its own click bar at its own tempo, plainly.
            assert!(app.calibrating());
            assert_eq!(app.played_bpm(), crate::calibration::CALIBRATION_BPM, "{page:?}");
            let bar = format!("{:?}", Score::single(Grooph::default_measure()));
            assert_eq!(format!("{:?}", *app.played_score()), bar, "{page:?}");
            // Hits are measured against that bar, and taps count on every
            // page, the metronome's too.
            assert!(app.tap_capture_active(), "{page:?}");
            // Counted in, and none of the user's options: no ghost notes,
            // swing, gap, ramp, click pattern or counter rhythm.
            assert_eq!(
                app.playback_options(),
                grooph_audio::PlaybackOptions { count_in: true, ..Default::default() },
                "{page:?}"
            );
            assert_eq!(app.score_timing().swing(), Swing::default());
            assert!(!app.endless_runs());
            // The page and everything of the user's stay as they are,
            // also for a save during the run.
            assert_eq!(app.tab(), Tab::More);
            assert_eq!(app.document(), document, "{page:?}");
            assert_eq!(user_state(&app), before, "{page:?}");
            let saved = PersistedState::from_app(&app);
            assert_eq!(saved.bpm, 133);
            assert_eq!(
                saved.swing,
                Swing { unit: grooph_measure::swing::SwingUnit::Eighths, percent: 66 }
            );
            assert_eq!(
                format!("{:?}", saved.score_doc.score),
                format!("{:?}", app.editor.docs.score.score)
            );
            // Cancelled: nothing to put back.
            app.stop_transport();
            assert!(!app.calibrating());
            assert_eq!(user_state(&app), before, "{page:?}");
            assert_eq!(app.played_bpm(), 133);
        }
    }

    #[test]
    fn a_calibration_applies_the_offset_it_finds() {
        use super::Grooph;
        use crate::calibration::{CALIBRATION_HITS, CALIBRATION_WARMUP};
        use crate::play_along::HitSource;

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::More);
        let before = user_state(&app);
        app.start_calibration(HitSource::Tap);
        let cal = app.playback_ctl.play_along.calibration.as_mut().unwrap();
        // The warm-up hits are ignored, the median of the rest is the offset.
        for _ in 0..CALIBRATION_WARMUP {
            cal.push(-200.0);
        }
        for i in 0..CALIBRATION_HITS {
            cal.push(if i % 2 == 0 { 24.0 } else { 26.0 });
        }
        app.advance_calibration();
        assert_eq!(app.playback_ctl.play_along.settings.offset_ms(HitSource::Tap), 25.0);
        assert!(!app.calibrating());
        assert_eq!(app.playback_ctl.transport_state, crate::TransportState::Stopped);
        assert_eq!(user_state(&app), before);
    }

    #[test]
    fn the_metronome_measures_hits_against_its_clicks() {
        use super::Grooph;

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Metronome);
        // It plays its own document as clicks, and hits count against it.
        assert_eq!(
            format!("{:?}", app.played_score()),
            format!("{:?}", app.editor.docs.metronome.score)
        );
        assert!(app.played_score().measures()[0].first_voice().beats().iter().any(|b| b.is_note()));
        assert_eq!(app.playback_options().played, grooph_audio::Played::Clicks);
        app.select_tab(Tab::Practice);
        assert_eq!(app.playback_options().played, grooph_audio::Played::Score);
    }

    #[test]
    fn polyrhythm_switch_brings_back_the_last_ratio() {
        use super::Grooph;
        use grooph_measure::counter::Ratio;

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Metronome);
        let first = app.counter_choices().first().copied();
        assert!(first.is_some());
        assert_eq!(app.counter_to_restore(), first);
        let last = *app.counter_choices().last().unwrap();
        app.ui.shell.last_counter = Some(last);
        assert_eq!(app.counter_to_restore(), Some(last));
        // A ratio the meter has no counter bar for falls back to the first.
        app.ui.shell.last_counter = Some(Ratio { n: 97, span: 1 });
        assert_eq!(app.counter_to_restore(), first);
    }

    #[test]
    fn silent_bars_row_names_the_pattern() {
        use crate::pickers::silent_bars_label;
        use grooph_audio::{GapClick, GapMode};

        let mut gap =
            GapClick { on_bars: 3, off_bars: 1, random_percent: 25, ..GapClick::default() };
        // Off shows what switching on brings back.
        assert_eq!(silent_bars_label(&gap), "3 on · 1 off");
        gap.enabled = true;
        assert_eq!(silent_bars_label(&gap), "3 on · 1 off");
        gap.mode = GapMode::Random;
        assert_eq!(silent_bars_label(&gap), "Random 25%");
    }

    #[test]
    fn muted_bars_sound_while_editing() {
        use super::Grooph;

        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.playback_ctl.audio_cfg.gap.enabled = true;
        for tab in [Tab::Practice, Tab::Metronome] {
            app.select_tab(tab);
            assert!(app.playback_options().gap.enabled, "{tab:?}");
        }
        app.select_tab(Tab::Score);
        app.edit_page();
        assert!(!app.playback_options().gap.enabled);
        // Editing the metronome's clicks too.
        app.select_tab(Tab::Metronome);
        app.ui.mode = super::Mode::Edit;
        assert!(!app.playback_options().gap.enabled);
        app.select_tab(Tab::Score);
        app.edit_page();
        // The setting itself stays on, and bars are muted again after editing.
        assert!(app.playback_ctl.audio_cfg.gap.enabled);
        app.select_tab(Tab::Practice);
        assert!(app.playback_options().gap.enabled);
    }

    fn load(saved: &str) -> CountingSettings {
        ron::from_str(saved).unwrap()
    }

    #[test]
    fn counting_overlay_is_off_by_default() {
        let c = CountingSettings::default();
        assert!(c.base == CountingBase::Off);
    }

    #[test]
    fn fresh_practice_has_no_space_and_no_timing_feedback() {
        let s = PersistedState::default();
        assert_eq!(s.generator_settings.space, 0.0);
        assert!(!s.accuracy_enabled);
    }

    #[test]
    fn saved_tab_round_trips_and_first_start_opens_metronome() {
        let s = PersistedState { tab: Tab::Score, ..Default::default() };
        let back: PersistedState = ron::from_str(&ron::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.tab, Tab::Score);
        assert_eq!(PersistedState::default().tab, Tab::Metronome);
        let older: PersistedState = ron::from_str("(version: 2)").unwrap();
        assert_eq!(older.tab, Tab::Metronome);
        assert!(!older.editing);
    }

    #[test]
    fn saved_sounds_still_load() {
        // The rhythm, metronome and counter sounds were separate enums with
        // these variant names; they all load as the one `Sound`.
        use grooph_audio::{AudioSettings, Sound};
        for (saved, sound) in [
            ("Click", Sound::Click),
            ("Claves", Sound::Claves),
            ("Snare", Sound::Snare),
            ("Xylophone", Sound::Xylophone),
        ] {
            assert_eq!(ron::from_str::<Sound>(saved).unwrap(), sound);
        }
        // Settings saved before a voice had its sound get that voice's default.
        let saved = ron::to_string(&AudioSettings::default()).unwrap();
        let older = saved.replace("counter_sound:Xylophone,", "");
        assert_ne!(older, saved);
        let back: AudioSettings = ron::from_str(&older).unwrap();
        assert_eq!(back.counter_sound, Sound::COUNTER_DEFAULT);
    }

    #[test]
    fn settings_saved_before_the_kit_get_its_volumes() {
        use grooph_audio::{AudioSettings, KitVolumes};
        let mut settings = AudioSettings { kit_muted: true, ..AudioSettings::default() };
        settings.kit.toms = 0.25;
        let saved = ron::to_string(&settings).unwrap();
        assert_eq!(ron::from_str::<AudioSettings>(&saved).unwrap(), settings);
        let kit = saved.find(",kit:").expect("kit saved");
        let older = format!("{})", &saved[..kit]);
        let back: AudioSettings = ron::from_str(&older).unwrap();
        assert_eq!(back.kit, KitVolumes::default());
        assert!(!back.kit_muted);
    }

    #[test]
    fn counting_settings_round_trip() {
        let c =
            CountingSettings { base: CountingBase::Sixteenth, system: CountingSystem::Takadimi };
        let back = load(&ron::to_string(&c).unwrap());
        assert!(back.base == CountingBase::Sixteenth);
        assert_eq!(back.system, CountingSystem::Takadimi);
    }

    #[test]
    fn the_count_menu_shows_each_system() {
        use CountingSystem::*;
        let four_four = Score::single(Measure::new(TimeSignature::FOUR_FOUR));
        let labels = |system| CountingBase::ALL.map(|b| b.label(system, &four_four));
        assert_eq!(
            labels(Numbers),
            ["Off", "1 2 3 4 ...", "1 & 2 & ...", "1 e & a ...", "1 trip let ..."]
        );
        assert_eq!(
            labels(Takadimi),
            ["Off", "ta ta ta ta ...", "ta di ta di ...", "ta ka di mi ...", "ta ki da ..."]
        );
        assert_eq!(
            labels(Gordon),
            ["Off", "du du du du ...", "du de du de ...", "du ta de ta ...", "du da di ..."]
        );
    }

    #[test]
    fn the_count_menu_spells_the_count_in_the_meter() {
        use CountingSystem::*;
        let six_eight = Score::single(Measure::new(TimeSignature::new(6, 8)));
        let labels = |system| {
            [CountingBase::Primary, CountingBase::Ands, CountingBase::Triplet]
                .map(|b| b.label(system, &six_eight))
        };
        assert_eq!(labels(Numbers), ["1 2 ...", "1 ta la ta ...", "1 la li ..."]);
        assert_eq!(labels(Takadimi), ["ta ta ...", "ta va ki di ...", "ta ki da ..."]);
        assert_eq!(labels(Gordon), ["du du ...", "du ta da ta ...", "du da di ..."]);
    }

    #[test]
    fn the_count_menu_spells_a_count_in_the_first_bar_it_fits() {
        let score = Score::new(vec![
            Measure::new(TimeSignature::new(6, 16)),
            Measure::new(TimeSignature::new(6, 8)),
        ]);
        assert_eq!(CountingBase::Primary.label(CountingSystem::Numbers, &score), "1 2 ...");
        assert_eq!(CountingBase::Ands.label(CountingSystem::Numbers, &score), "1 ta la ta ...");
        let lone = Score::single(Measure::new(TimeSignature::new(6, 16)));
        assert_eq!(CountingBase::Ands.label(CountingSystem::Numbers, &lone), "1 & 2 & ...");
    }

    #[test]
    fn older_counting_switches_still_hide_the_overlay() {
        let on = load("(enabled: true, show_labels: true, base: Ands, show_tuplets: true)");
        assert!(on.base == CountingBase::Ands);
        assert_eq!(on.system, CountingSystem::Numbers);
        for hidden in [
            "(enabled: false, show_labels: true, base: Ands, show_tuplets: true)",
            "(enabled: true, show_labels: false, base: Ands, show_tuplets: true)",
        ] {
            let c = load(hidden);
            assert!(c.base == CountingBase::Off, "{hidden}");
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
