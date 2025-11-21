use crate::measure::duration::{Duration, NoteValue};
use crate::measure::{Measure, TimeSignature};

use crate::layout::pixel_layout::{NoteLayout, build_measure_layout};
use crate::measure::duration::NoteValue::*;
use crate::measure::duration::human_readable;
use crate::measure::{Beat, BeatKind};
use crate::render::beat::draw_beat;
use crate::render::glyphs::{
    GLYPH_LEFT_TUPLET_BRACKET, GLYPH_NOTE_32ND, GLYPH_NOTE_EIGHTH, GLYPH_NOTE_HALF,
    GLYPH_NOTE_QUARTER, GLYPH_NOTE_SIXTEENTH, GLYPH_NOTE_WHOLE, GLYPH_REST_32ND, GLYPH_REST_EIGHTH,
    GLYPH_REST_HALF, GLYPH_REST_QUARTER, GLYPH_REST_WHOLE, GLYPH_RIGHT_TUPLET_BRACKET,
    TUPLET_DIGITS,
};
use crate::render::measure::draw_measure;
use crate::tools::{EditOp, Modifier, Tool, ToolGroup, ToolKind, all_tools};
use eframe::egui::scroll_area::{ScrollBarVisibility, ScrollSource};
use eframe::egui::{
    Align, Atom, Context, Direction, Id, Key, Label, Layout, Vec2, global_theme_preference_switch,
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
                    let is_note = matches!(beat.kind, BeatKind::Note);
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
                Duration::Tuplet { n, .. } => {
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

        if self.show_info {
            egui::TopBottomPanel::top("info").show(ctx, |ui| {
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
                        BeatKind::Note => "note",
                        BeatKind::Rest => "rest",
                    };
                    label = format!("Beat: {}, {} {}", beat_text, desc, kind);
                }
                ui.add(Label::new(label));
            });
        }

        if self.show_settings {
            // egui::TopBottomPanel::top("settings").show(ctx, |ui| {
            //     let mut style = ui.ctx().style().spacing.scroll;
            //     style.ui(ui);
            //
            //     ui.ctx().all_styles_mut(|s| s.spacing.scroll = style);
            // });
        }

        // egui::TopBottomPanel::bottom("tool_palette")
        //     .frame(Frame::group(&ctx.style()).fill(ctx.style().visuals.panel_fill))
        //     .resizable(false)
        //     .show(ctx, |ui| {
        //         let tools = all_tools();
        //         let groups = [ToolGroup::Notes, ToolGroup::Rests, ToolGroup::Tuplets];
        //
        //         egui::ScrollArea::horizontal()
        //             .scroll_source(ScrollSource::ALL)
        //             .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        //             .show(ui, |ui| {
        //                 let layout = Layout::from_main_dir_and_cross_align(
        //                     Direction::LeftToRight,
        //                     Align::Center,
        //                 );
        //                 ui.with_layout(layout, |ui| {
        //                     for g in groups {
        //                         let group_tools: Vec<_> =
        //                             tools.iter().filter(|t| t.group == g).collect();
        //                         if group_tools.is_empty() {
        //                             continue;
        //                         }
        //
        //                         for t in group_tools {
        //                             match t.kind {
        //                                 ToolKind::InsertBeat(template) => {
        //                                     let symbol_id = Id::new(t.id);
        //                                     let symbol = Atom::custom(symbol_id, Vec2::splat(80.0));
        //                                     let button = egui::Button::new(symbol)
        //                                         .corner_radius(10)
        //                                         .atom_ui(ui);
        //
        //                                     if let Some(rect) = button.rect(symbol_id) {
        //                                         let measure = Measure::new_init(
        //                                             TimeSignature::ONE_FOUR,
        //                                             template.kind,
        //                                         );
        //                                         let measure_layout = build_measure_layout_px(
        //                                             &measure,
        //                                             rect,
        //                                             &self.font_id,
        //                                             ui.ctx().pixels_per_point(),
        //                                         );
        //                                         let painter = &ui.painter_at(rect);
        //                                         for note in &measure_layout.notes {
        //                                             draw_beat(
        //                                                 painter,
        //                                                 note,
        //                                                 &self.font_id,
        //                                                 ui.style().visuals.text_color(),
        //                                             );
        //                                         }
        //                                     }
        //                                 }
        //                                 _ => {}
        //                             }
        //                             // let resp =
        //                             //     ui.add_sized([tile, tile], button).on_hover_text(t.label);
        //                             // if resp.clicked() {
        //                             //     self.apply_tool(t);
        //                             // }
        //                         }
        //                     }
        //                 })
        //             });
        //     });

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
                            let idx = if target_x <= layout.content.left() {
                                0
                            } else if target_x >= layout.content.right() {
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

                // Edits apply only when cursor is on a committed beat
                let idx = self.cursor_idx.min(beats_len.saturating_sub(1));
                if i.key_pressed(Key::Delete) {
                    // Remove beat at cursor
                    self.measure.remove(idx);
                    // Move cursor right
                    let new_pos = (self.measure.beats().len() - 1).min(self.cursor_idx + 1);
                    self.cursor_idx = new_pos;
                }
                if i.key_pressed(Key::Backspace) {
                    // Remove beat at cursor
                    self.measure.remove(idx);
                    // Move cursor left
                    let new_len = self.measure.beats().len();
                    let new_pos = self.cursor_idx.saturating_sub(1).min(new_len - 1);
                    self.cursor_idx = new_pos;
                }
                if i.key_pressed(Key::Space) {
                    // Toggle between note and rest at cursor (preserve duration)
                    self.measure.toggle_beat_kind(idx);
                }
                if i.key_pressed(Key::Num1) {
                    self.set_beat(idx, Quarter, false, None);
                }
                if i.key_pressed(Key::Num2) {
                    self.set_beat(idx, Eighth, true, None);
                }
                if i.key_pressed(Key::Num3) {
                    self.set_beat(idx, Sixteenth, true, None);
                }
                if i.key_pressed(Key::Num4) {
                    self.set_beat(idx, ThirtySecond, true, None);
                }
                if i.key_pressed(Key::Period) {
                    // Toggle dotted (1 dot) for the current beat. If it cannot be changed (would overflow or unfillable), ignore.
                    let _ = self.measure.toggle_dotted_at(idx);
                }
                if i.key_pressed(Key::A) {
                    // Toggle user accent on the current beat
                    self.measure.toggle_accent_at(idx);
                }
                if i.key_pressed(Key::T) {
                    // Cycle tuplets with stable group start targeting:
                    // Non-tuplet -> 1/8 Triplet -> 1/16 Quintuplet -> 1/16 Sextuplet -> 1/16 Septuplet -> 1/16 Nonuplet -> Dissolve
                    // If the cursor is inside an existing tuplet group, always operate at the group's start index
                    let beats = self.measure.beats();
                    let gid_at_cursor = beats[idx].tuplet_group_id;
                    let start_idx = if let Some(gid) = gid_at_cursor {
                        // scan left to find the first index with the same group id
                        let mut sidx = idx;
                        while sidx > 0
                            && self.measure.beats()[sidx - 1].tuplet_group_id == Some(gid)
                        {
                            sidx -= 1;
                        }
                        sidx
                    } else {
                        idx
                    };

                    // Determine current state using the duration at the group start (or cursor if non-tuplet)
                    let cur_beat = self.measure.beats()[start_idx];
                    let mut did_dissolve = false;
                    let mut did_recreate = false;
                    // Falls wir uns in einer Tuplet‑Gruppe befinden und in die nächste Gruppe wechseln,
                    // erfassen wir vorher die relative Notenbelegung, um sie nach der Rekreation
                    // bestmöglich auf die neue Grid zu projizieren.
                    let mut captured_offsets: Option<Vec<(u32, bool)>> = None;
                    let changed = match cur_beat.duration {
                        Duration::Tuplet { n, m, base } => {
                            let next_target = match (n, m, base) {
                                (3, 2, Eighth) => Some((5, 4, Sixteenth)),
                                (5, 4, Sixteenth) => Some((6, 4, Sixteenth)),
                                (6, 4, Sixteenth) => Some((7, 4, Sixteenth)),
                                (7, 4, Sixteenth) => Some((9, 8, Sixteenth)),
                                _ => None,
                            };

                            // Dissolve current group from its start
                            // Vor dem Auflösen ggf. Noten‑Offsets erfassen, nur wenn wir ein nächstes Ziel haben
                            if next_target.is_some() {
                                captured_offsets =
                                    self.measure.tuplet_group_note_offsets(start_idx);
                            }
                            if self.measure.dissolve_tuplet_group_at(start_idx) {
                                did_dissolve = true;
                                // Try to convert to next target if defined, also at group start
                                if let Some((tn, tm, tbase)) = next_target {
                                    if self
                                        .measure
                                        .convert_to_tuplet_at(start_idx, tn, tm, tbase, false)
                                    {
                                        did_recreate = true;
                                        // Nach erfolgreicher Rekreation ggf. Projektion anwenden
                                        if let Some(ref src) = captured_offsets {
                                            let _ = self
                                                .measure
                                                .apply_tuplet_projection_at(start_idx, src);
                                        }
                                    }
                                }
                                true
                            } else {
                                false
                            }
                        }
                        _ => {
                            let ok =
                                self.measure.convert_to_tuplet_at(start_idx, 3, 2, Eighth, false);
                            if ok {
                                did_recreate = true;
                            }
                            ok
                        }
                    };

                    // Cursor nur verschieben, wenn wir ausschließlich aufgelöst haben (kein direktes Re‑Create)
                    if changed && did_dissolve && !did_recreate {
                        let new_len = self.measure.beats().len();
                        if new_len > 0 {
                            self.cursor_idx = start_idx.min(new_len - 1);
                        } else {
                            self.cursor_idx = 0;
                        }
                    }
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
        allow_on_tuplet: bool,
        beat_kind: Option<BeatKind>,
    ) -> bool {
        let cur = self.measure.beats()[idx];
        let new_dur_opt = match cur.duration {
            Duration::Tuplet { n, m, base: _ } => {
                if allow_on_tuplet {
                    Some(Duration::Tuplet { n, m, base })
                } else {
                    None
                }
            }
            _ => Some(Duration::Simple(base)),
        };

        let ok = if let Some(new_dur) = new_dur_opt {
            let kind = if let Some(override_kind) = beat_kind { override_kind } else { cur.kind };
            let new_beat = match kind {
                BeatKind::Note => Beat::note(new_dur),
                BeatKind::Rest => Beat::rest(new_dur),
            };
            self.measure.set_beat_at(idx, new_beat).is_ok()
        } else {
            false
        };

        if ok {
            let new_len = self.measure.beats().len();
            if new_len > 0 {
                let last = new_len - 1;
                if self.cursor_idx < last {
                    self.cursor_idx += 1;
                }
            }
        }

        ok
    }

    fn apply_tool(&mut self, tool: &Tool) {
        match tool.kind {
            ToolKind::InsertBeat(template) => {
                let beats_len = self.measure.beats().len();
                if beats_len == 0 {
                    return;
                }
                let idx = self.cursor_idx.min(beats_len - 1);
                self.set_beat(idx, template.duration.base_note(), true, Some(template.kind));
            }
            _ => {
                // Andere Toolarten (Modifier/Edit/Meta) werden in einem späteren Schritt verdrahtet.
            }
        }
    }

    pub fn new(cc: &CreationContext) -> Self {
        add_font(&cc.egui_ctx);
        let ff = FontFamily::Name("music".into());
        let m = Measure::new(TimeSignature::SEVEN_EIGHT);
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
