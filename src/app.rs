mod help;
mod keyboard_input;
mod main_menu;
mod measure_panel;
mod mixer_panel;
mod settings_panel;
mod style;
mod time_signature_dialog;
mod tool_palette;

use crate::measure::duration::{Duration, NoteValue, TupletSpec, q};
use crate::measure::{BeatIdx, Measure, TimeSignature};

use crate::audio::MixerVolumes;
use crate::measure::duration::NoteValue::*;
use crate::measure::editing::Modification;
use crate::measure::{Beat, BeatKind};
use crate::tools::ToolKind;
use crate::{BeatTemplate, all_tools};
use eframe::egui::{Context, TextStyle, Widget};
use eframe::epaint::text::{FontInsert, InsertFontFamily};
use eframe::epaint::{FontFamily, FontId};
use eframe::{App, CreationContext, egui};
use std::collections::HashMap;

pub struct Grooph {
    music_font_id: FontId,
    measure: Measure,
    cursor_idx: BeatIdx,
    edit_mode_enabled: bool,
    show_info: bool,
    show_settings: bool,
    show_mixer: bool,
    // Time signature dialog state
    show_ts_dialog: bool,
    ts_beats: u8,
    ts_unit: u8,
    // Prebuilt measures for note/rest/tuplet tool buttons to avoid per-frame reconstruction
    button_measures: HashMap<&'static str, Measure>,
    undo_stack: Vec<(Measure, BeatIdx)>,
    redo_stack: Vec<(Measure, BeatIdx)>,
    // Global UI font bump configuration and per-theme baselines (so bump applies to dark & light)
    font_bump: f32,
    // Store baselines as small vectors to avoid trait bounds on TextStyle (Ord/Hash)
    baseline_dark: Option<Vec<(TextStyle, f32)>>,
    baseline_light: Option<Vec<(TextStyle, f32)>>,
    player_state: PlayerState,
    bpm: u32,
    audio: Option<crate::audio::Audio>,
    // Mixer volumes [0.0, 1.0]
    mixer: MixerVolumes,
    // Playback smoothing state
    playback_smooth_tick: f64,
    playback_last_update: Option<f64>,
    playback_total_ticks: u32,
    #[cfg(target_arch = "wasm32")]
    web_last_visible: bool,
    #[cfg(target_arch = "wasm32")]
    web_was_playing_before_hide: bool,
}

#[derive(Clone, PartialEq, Eq)]
enum PlayerState {
    Playing,
    Paused,
    Stopped,
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

        if self.show_ts_dialog {
            self.time_signature_dialog(ctx);
        }

        self.handle_keyboard_input(ctx);

        // WASM: handle page visibility transitions to keep audio engine healthy
        #[cfg(target_arch = "wasm32")]
        {
            // Use re-exports from web_sys to avoid adding direct deps in Cargo.toml
            use web_sys::js_sys::Reflect;
            use web_sys::wasm_bindgen::JsCast as _;
            use web_sys::wasm_bindgen::JsValue;
            if let Some(win) = web_sys::window()
                && let Some(doc) = win.document()
            {
                // Use JS interop to read document.visibilityState for broader web-sys compatibility
                let doc_js: &JsValue = doc.as_ref();
                let visible_now = Reflect::get(doc_js, &JsValue::from_str("visibilityState"))
                    .ok()
                    .and_then(|v: JsValue| v.as_string())
                    .map(|s| s == "visible")
                    .unwrap_or(true);
                if !visible_now && self.web_last_visible {
                    // going hidden
                    self.web_was_playing_before_hide = self.player_state == PlayerState::Playing;
                    // Pause player state and stop audio cleanly
                    self.player_state = PlayerState::Paused;
                    if let Some(audio) = &mut self.audio {
                        audio.stop();
                    }
                    self.web_last_visible = false;
                } else if visible_now && !self.web_last_visible {
                    // returning visible: drop audio to force clean re-init
                    self.audio = None;
                    if self.web_was_playing_before_hide {
                        // restore playing state; creation may fail until next user gesture, we'll retry below
                        self.player_state = PlayerState::Playing;
                    }
                    self.web_last_visible = true;
                }
            }
        }

        // If we should be playing but have no audio engine, try to create it (works after user gesture on iOS)
        if self.player_state == PlayerState::Playing && self.audio.is_none() {
            self.audio = crate::audio::Audio::new(self.bpm);
        }

        if let Some(audio) = &mut self.audio {
            audio.set_mixer(self.mixer);
            if audio.update(self.player_state == PlayerState::Playing, self.bpm, &self.measure) {
                ctx.request_repaint();
            }
        }
    }
}

impl Grooph {
    fn push_undo(&mut self) { self.undo_stack.push((self.measure.clone(), self.cursor_idx)); }

    fn clear_redo(&mut self) { self.redo_stack.clear(); }

    fn undo(&mut self) {
        if let Some((m, c)) = self.undo_stack.pop() {
            // Move the current state to redo, replace it with the popped snapshot
            let current = (std::mem::replace(&mut self.measure, m), self.cursor_idx);
            self.cursor_idx = c;
            self.redo_stack.push(current);
        }
    }

    fn redo(&mut self) {
        if let Some((m, c)) = self.redo_stack.pop() {
            let current = (std::mem::replace(&mut self.measure, m), self.cursor_idx);
            self.cursor_idx = c;
            self.undo_stack.push(current);
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
            if let Duration::Tuplet(TupletSpec { m, .. }) = template.duration { m } else { 1 };

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

        measure
    }

    pub fn new(cc: &CreationContext) -> Self {
        add_font(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let ff = FontFamily::Name("music".into());

        // Default measure
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.set_beat(0, Beat::note(q())).unwrap();
        m.set_beat(1, Beat::note(q())).unwrap();
        m.set_beat(2, Beat::note(q())).unwrap();
        m.set_beat(3, Beat::note(q())).unwrap();

        // Precompute button measures for all insert-beat tools
        let mut button_measures: HashMap<&'static str, Measure> = HashMap::new();
        for t in all_tools() {
            if let ToolKind::InsertBeat(template) = t.kind {
                button_measures.insert(t.id, Self::build_button_measure(template));
            }
        }

        Self {
            music_font_id: FontId::new(16.0, ff),
            measure: m,
            cursor_idx: 0,
            edit_mode_enabled: false,
            show_info: false,
            show_settings: false,
            show_mixer: false,
            show_ts_dialog: false,
            ts_beats: TimeSignature::FOUR_FOUR.beats,
            ts_unit: TimeSignature::FOUR_FOUR.beat_unit,
            button_measures,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            font_bump: 8.0,
            baseline_dark: None,
            baseline_light: None,
            player_state: PlayerState::Stopped,
            bpm: 120,
            audio: None,
            mixer: MixerVolumes::default(),
            playback_smooth_tick: 0.0,
            playback_last_update: None,
            playback_total_ticks: 0,
            #[cfg(target_arch = "wasm32")]
            web_last_visible: true,
            #[cfg(target_arch = "wasm32")]
            web_was_playing_before_hide: false,
        }
    }
}
