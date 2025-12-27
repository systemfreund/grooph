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
use crate::measure::duration::{Duration, NoteValue, TupletSpec, q};
use crate::measure::{BeatIdx, Measure, TimeSignature};

use crate::audio::{AudioSettings, PlayerState};
use crate::measure::duration::NoteValue::*;
use crate::measure::editing::Modification;
use crate::measure::{Beat, BeatKind};
use crate::app::tools::ToolKind;
use crate::app::tools::{BeatTemplate, Modifier, all_tools};
use eframe::egui::{Context, TextStyle, Widget};
use eframe::epaint::text::{FontInsert, InsertFontFamily};
use eframe::epaint::{FontFamily, FontId};
use eframe::{App, CreationContext, egui};
use log::{debug, info};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(PartialEq, Eq)]
enum Mode {
    Edit,
    Playback,
    Mixer,
    Settings,
    Help,
    TimeSignature { beats: u8, unit: u8 },
}

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
    player_state: PlayerState,
    bpm: u32,
    audio: Option<crate::audio::Audio>,
    // Mixer volumes [0.0, 1.0]
    audio_settings: AudioSettings,
    // Playback smoothing state
    playback_smooth_tick: f64,
    playback_last_update: Option<f64>,
    playback_total_ticks: u32,

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

        #[cfg(target_arch = "wasm32")]
        self.handle_visibility_change();

        // If we should be playing but have no audio engine, try to create it (works after user gesture on iOS)
        if self.player_state == PlayerState::Playing && self.audio.is_none() {
            debug!("Creating audio engine.");
            self.audio = crate::audio::Audio::new(self.bpm);
        }

        if let Some(audio) = &mut self.audio {
            audio.set_audio_settings(self.audio_settings);
            if audio.update(&self.player_state, self.bpm, &self.measure) {
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

    pub fn toggle_playback(&mut self) {
        let old_state = self.player_state.clone();
        self.player_state = if old_state == PlayerState::Playing {
            PlayerState::Stopped
        } else {
            PlayerState::Playing
        };
        info!("Toggle playback: {:?} -> {:?}", old_state, self.player_state);

        #[cfg(target_arch = "wasm32")]
        if self.player_state == PlayerState::Playing {
            self.acquire_wake_lock();
        } else {
            self.release_wake_lock();
        }
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

        let this = Self {
            mode: Mode::Playback,
            music_font_id: FontId::new(16.0, ff),
            measure: m,
            cursor_idx: 0,
            button_measures,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            font_bump: 8.0,
            baseline_dark: None,
            baseline_light: None,
            player_state: PlayerState::Stopped,
            bpm: 120,
            audio: None,
            audio_settings: AudioSettings::default(),
            playback_smooth_tick: 0.0,
            playback_last_update: None,
            playback_total_ticks: 0,
            flash_intensity: 0.0,
            last_primary_beat: None,
            #[cfg(target_arch = "wasm32")]
            wake_lock: Rc::new(RefCell::new(None)),
            layout_width_cap_factor: 0.1,
            layout_accent_below: true,
            layout_proportional_spacing: true,
            layout_stem_length_factor: 0.9,
            layout_debug_bbox: false,
            audio_offset: 0.0,
            audio_latency_enabled: true,
        };

        // WASM: install visibilitychange/pageshow listeners once
        #[cfg(target_arch = "wasm32")]
        {
            web::install_visibility_listeners(cc.egui_ctx.clone());
        }

        this
    }
}
