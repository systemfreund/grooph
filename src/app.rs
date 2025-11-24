use crate::measure::duration::{Duration, NoteValue, TupletSpec};
use crate::measure::{Measure, TimeSignature};

use crate::layout::pixel_layout::{LayoutOpts, build_measure_layout};
use crate::measure::BeatKind::Note;
use crate::measure::duration::NoteValue::*;
use crate::measure::duration::human_readable;
use crate::measure::editing::Modification;
use crate::measure::{Beat, BeatKind};
use crate::render::glyphs::{
    GLYPH_LEFT_TUPLET_BRACKET, GLYPH_NOTE_32ND, GLYPH_NOTE_EIGHTH, GLYPH_NOTE_HALF,
    GLYPH_NOTE_QUARTER, GLYPH_NOTE_SIXTEENTH, GLYPH_NOTE_WHOLE, GLYPH_REST_32ND, GLYPH_REST_EIGHTH,
    GLYPH_REST_HALF, GLYPH_REST_QUARTER, GLYPH_REST_WHOLE, GLYPH_RIGHT_TUPLET_BRACKET,
    TUPLET_DIGITS,
};
use crate::render::measure::{compute_em, draw_measure, draw_notes};
use crate::tools::{EditOp, Modifier, Tool, ToolKind};
use crate::{BeatTemplate, ToolGroup, all_tools};
use BeatKind::Rest;
use eframe::egui::scroll_area::{ScrollBarVisibility, ScrollSource};
use eframe::egui::{
    Align, Atom, Context, Direction, Id, Key, Label, Layout, Response, Ui, Vec2,
    global_theme_preference_switch,
};
use eframe::epaint::text::{FontInsert, InsertFontFamily};
use eframe::epaint::{FontFamily, FontId};
use eframe::{App, CreationContext, egui};
use egui::containers::Frame;

pub struct Grooph<'a> {
    font_family: FontFamily,
    font_id: FontId,
    measure: Measure<'a>,
    cursor_idx: usize,
    show_info: bool,
    show_settings: bool,
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

    ctx.add_font(FontInsert::new(
        "Bravura Text",
        egui::FontData::from_static(include_bytes!("../assets/fonts/BravuraText.otf")),
        vec![InsertFontFamily {
            family: FontFamily::Name("music-text".into()),
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    ))
}

/// Liefert ein symbolisches Icon pro Tool als Text sowie true,
/// wenn das Tool eine Noten darstellt, ansonsten false..
fn tool_icon_glyph(t: &Tool) -> (String, bool) {
    match t.kind {
        ToolKind::InsertBeat(beat) => {
            match beat.duration {
                Duration::Simple(base) => {
                    let is_note = matches!(beat.kind, Note);
                    let s = if is_note {
                        match base {
                            Quarter => GLYPH_NOTE_QUARTER,
                            Eighth => GLYPH_NOTE_EIGHTH,
                            Sixteenth => GLYPH_NOTE_SIXTEENTH,
                            ThirtySecond => GLYPH_NOTE_32ND,
                            Half => GLYPH_NOTE_HALF,
                            Whole => GLYPH_NOTE_WHOLE,
                        }
                    } else {
                        match base {
                            Quarter => GLYPH_REST_QUARTER,
                            Eighth => GLYPH_REST_EIGHTH,
                            Sixteenth => GLYPH_REST_EIGHTH,
                            ThirtySecond => GLYPH_REST_32ND,
                            Half => GLYPH_REST_HALF,
                            Whole => GLYPH_REST_WHOLE,
                        }
                    };

                    (s.to_string(), is_note)
                }
                Duration::Tuplet(TupletSpec { n, .. }) => {
                    // Tuplets: zeige nur die Zählzahl (3,5,6,7,9)
                    (
                        format!(
                            "{}{}{}",
                            GLYPH_LEFT_TUPLET_BRACKET,
                            TUPLET_DIGITS[n as usize],
                            GLYPH_RIGHT_TUPLET_BRACKET
                        ),
                        false,
                    )
                }
                Duration::Dotted { .. } => {
                    // In der Palette aktuell nicht als Insert vorgesehen
                    ("⋯".to_string(), false)
                }
            }
        }
        ToolKind::Modify(m) => match m {
            Modifier::ToggleDotted { .. } => ("·".to_string(), false), // Punktierung
            Modifier::ToggleAccent => (">".to_string(), false),
            Modifier::ToggleRestNote => ("↔".to_string(), false),
        },
        ToolKind::Edit(op) => match op {
            EditOp::Erase => ("⌫".to_string(), false),
            EditOp::ReplaceOnApply => ("⇄".to_string(), false),
            EditOp::FillToBoundary => ("⇥".to_string(), false),
        },
        ToolKind::Meta(_m) => ("TS".to_string(), false),
    }
}

impl App for Grooph<'_> {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            ui.horizontal(|ui| {
                global_theme_preference_switch(ui);
                ui.separator();
                ui.toggle_value(&mut self.show_info, "?");
                ui.toggle_value(&mut self.show_settings, "⚙");
            });
        });

        egui::TopBottomPanel::top("info").show_animated(ctx, self.show_info, |ui| {
            ui.label(
                "Keybindings: \n\
                    Arrow keys: Move cursor\n\
                    Del/Backspace: Remove note\n\
                    Space: Toggle between note and rest\n\
                    A: Set/unset accent\n\
                    1-4: Set duration (1=1/4, 2=1/8, 3=1/16, 4=1/32)\n\
                    Period: Toggle dotted\n\
                    T: Cycle tuplet (Tri -> Quint -> Sext -> Sept -> Non -> Dissolve)\n",
            );

            // Label showing absolute beat position at the cursor and human-readable duration/kind
            let mut beat_text = String::from("-");
            let idx = self.cursor_idx;
            let positions = self.measure.beat_positions();
            if idx < positions.len() {
                let v = positions[idx];
                let mut s = format!("{:.3}", v);
                // Trim trailing zeros and optional dot for a cleaner look
                while s.ends_with('0') {
                    s.pop();
                }
                if s.ends_with('.') {
                    s.pop();
                }
                beat_text = s;
            }
            let mut label = format!("Beat: {}", beat_text);
            if idx < self.measure.beats().len() {
                let b = self.measure.beats()[idx];
                let desc = human_readable(&b.duration);
                let kind = match b.kind {
                    Note => "note",
                    Rest => "rest",
                };
                label = format!("Beat: {}, {} {}", beat_text, desc, kind);
            }
            ui.add(Label::new(label));
        });

        if self.show_settings {
            // egui::TopBottomPanel::top("settings").show(ctx, |ui| {
            //     let mut style = ui.ctx().style().spacing.scroll;
            //     style.ui(ui);
            //
            //     ui.ctx().all_styles_mut(|s| s.spacing.scroll = style);
            // });
        }

        egui::TopBottomPanel::bottom("tool_palette")
            .frame(Frame::group(&ctx.style()).fill(ctx.style().visuals.panel_fill))
            .resizable(false)
            .show(ctx, |ui| {
                let tools = all_tools();
                let groups = [ToolGroup::Notes, ToolGroup::Tuplets, ToolGroup::Rests];

                egui::ScrollArea::horizontal()
                    .scroll_source(ScrollSource::ALL)
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        let layout = Layout::from_main_dir_and_cross_align(
                            Direction::LeftToRight,
                            Align::Center,
                        );
                        ui.with_layout(layout, |ui| {
                            self.tool_palette(tools, groups.as_slice(), ui);
                        })
                    });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            Frame::canvas(ui.style())
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::NONE)
                .show(ui, |ui| {
                    let size = ui.available_size();
                    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());

                    let layout =
                        draw_measure(ui, &self.font_id, &self.measure, rect, Some(self.cursor_idx));

                    if (resp.clicked() || resp.dragged())
                        && let Some(pos) = resp.interact_pointer_pos()
                    {
                        // Falls keine Beats vorhanden sind, nichts tun
                        if !layout.notes.is_empty() {
                            // Außerhalb des Inhalts: zum nächstliegenden Rand clampen
                            let target_x = pos.x;
                            let idx = if target_x <= rect.left() {
                                0
                            } else if target_x >= rect.right() {
                                layout.notes.len() - 1
                            } else {
                                // Innerhalb: Index des nächstgelegenen x-Centers suchen
                                let mut best_i = 0usize;
                                let mut best_d = f32::MAX;
                                for (i, nl) in layout.notes.iter().enumerate() {
                                    let d = (nl.center.x - target_x).abs();
                                    if d < best_d {
                                        best_d = d;
                                        best_i = i;
                                    }
                                }
                                best_i
                            };
                            self.cursor_idx = idx;
                        }
                    }
                });
        });

        ctx.input(|i| {
            let beats_len = self.measure.beats().len();
            let total_len = beats_len;
            if total_len > 0 {
                // Navigation over committed beats only
                let mut pos = self.cursor_idx;
                if i.key_pressed(Key::ArrowLeft) {
                    pos = pos.saturating_sub(1);
                }
                if i.key_pressed(Key::ArrowRight) {
                    let max_idx = total_len.saturating_sub(1);
                    if pos < max_idx {
                        pos += 1;
                    }
                }
                if i.key_pressed(Key::Home) {
                    pos = 0;
                }
                if i.key_pressed(Key::End) {
                    pos = total_len.saturating_sub(1);
                }
                self.cursor_idx = pos;

                // Edits apply only when the cursor is on a committed beat
                let idx = self.cursor_idx.min(beats_len.saturating_sub(1));
                if i.key_pressed(Key::Delete) {
                    // Remove beat at the cursor
                    self.measure.remove(idx);
                    // Move cursor right
                    let new_pos = (self.measure.beats().len() - 1).min(self.cursor_idx + 1);
                    self.cursor_idx = new_pos;
                }
                if i.key_pressed(Key::Backspace) {
                    // Remove beat at the cursor
                    self.measure.remove(idx);
                    // Move cursor left
                    let new_len = self.measure.beats().len();
                    let new_pos = self.cursor_idx.saturating_sub(1).min(new_len - 1);
                    self.cursor_idx = new_pos;
                }
                if i.key_pressed(Key::Space) {
                    self.measure.toggle_beat_kind(idx);
                }
                if i.key_pressed(Key::Num1) {
                    self.set_beat(idx, Quarter, None);
                }
                if i.key_pressed(Key::Num2) {
                    self.set_beat(idx, Eighth, None);
                }
                if i.key_pressed(Key::Num3) {
                    self.set_beat(idx, Sixteenth, None);
                }
                if i.key_pressed(Key::Num4) {
                    self.set_beat(idx, ThirtySecond, None);
                }
                if i.key_pressed(Key::Period) {
                    self.measure.toggle_dotted(idx);
                }
                if i.key_pressed(Key::A) {
                    self.measure.toggle_accent(idx);
                }
                if i.key_pressed(Key::T) {
                    self.set_tuplet(idx, None);
                }
            }
        });
    }
}

impl Grooph<'_> {
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
            Some(Modification::DissolveTuplet(tuplet_idx)) => {
                let new_len = self.measure.beats().len();
                if new_len > 0 {
                    self.cursor_idx = *tuplet_idx.min(&(new_len - 1));
                } else {
                    self.cursor_idx = 0;
                }
            }
            Some(Modification::SetTuplet(group_span)) => {
                self.cursor_idx = (group_span.end_idx + 1).min(self.measure.beats().len() - 1);
            }
            _ => {}
        }
        result
    }

    fn apply_tool(&mut self, tool: &Tool) {
        match tool.kind {
            ToolKind::InsertBeat(template) => {
                let beats_len = self.measure.beats().len();
                if beats_len == 0 {
                    return;
                }
                let idx = self.cursor_idx.min(beats_len - 1);

                let result = match template.duration {
                    Duration::Simple(_) => {
                        self.set_beat(idx, template.duration.base_note(), Some(template.kind))
                    }
                    Duration::Tuplet(spec) => self.set_tuplet(idx, Some(spec)),
                    _ => None,
                };

                println!("{:?}", result);
            }
            _ => {
                // Andere Toolarten (Modifier/Edit/Meta) werden in einem späteren Schritt verdrahtet.
            }
        }
    }

    fn note_button(&self, ui: &mut Ui, template: BeatTemplate, id: &str) -> Response {
        let tile = 90.0;
        let symbol_id = Id::new(id);
        let symbol = Atom::custom(symbol_id, Vec2::splat(tile));
        let button = egui::Button::new(symbol).corner_radius(10).atom_ui(ui);

        if let Some(rect) = button.rect(symbol_id) {
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
                    measure.set_beat(i as usize, Beat::note(template.duration)).unwrap();
                }
            }

            let em = compute_em(&rect, 0.4, ui);
            let opts = LayoutOpts {
                rect,
                font_id: FontId::new(em, self.font_id.family.clone()),
                em,
                layout_clef: false,
                layout_time_signature: false,
                y_offset: if template.kind == Note { 18.0 } else { 5.0 },
                stem_length_factor: 0.9,
                stem_thickness_factor: 0.03,
            };
            let measure_layout = build_measure_layout(&measure, &opts);
            let painter = &ui.painter_at(rect);
            draw_notes(painter, &measure_layout, ui.style().visuals.text_color(), &opts);
        }

        button.response
    }

    fn tool_palette(&mut self, tools: &[Tool], groups: &[ToolGroup], ui: &mut Ui) {
        for g in groups {
            let group_tools: Vec<_> = tools.iter().filter(|t| &t.group == g).collect();
            if group_tools.is_empty() {
                continue;
            }

            for t in group_tools {
                if let ToolKind::InsertBeat(template) = t.kind {
                    let button = self.note_button(ui, template, t.id);
                    if button.clicked() {
                        self.apply_tool(t);
                    }
                }
            }
        }
    }

    pub fn new(cc: &CreationContext) -> Self {
        add_font(&cc.egui_ctx);
        let ff = FontFamily::Name("music".into());
        let m = Measure::new(TimeSignature::FOUR_FOUR);
        Self {
            font_family: ff.clone(),
            font_id: FontId::new(16.0, ff),
            measure: m,
            cursor_idx: 0,
            show_info: false,
            show_settings: false,
        }
    }
}
