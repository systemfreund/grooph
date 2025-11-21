use crate::measure::duration::{COMMON_DURATIONS, Duration, NoteValue, qt16, s, st16, t8, t16};
use crate::measure::{Measure, TimeSignature};

use crate::layout::pixel_layout::build_measure_layout_px;
use crate::measure::duration;
use crate::measure::duration::NoteValue::*;
use crate::measure::duration::human_readable;
use crate::measure::{Beat, BeatKind};
use crate::render::glyphs::{
    GLYPH_NOTE_32ND, GLYPH_NOTE_EIGHTH, GLYPH_NOTE_HALF, GLYPH_NOTE_QUARTER, GLYPH_NOTE_SIXTEENTH,
    GLYPH_NOTE_WHOLE, GLYPH_NOTEHEAD_BLACK, GLYPH_REST_32ND, GLYPH_REST_EIGHTH, GLYPH_REST_HALF,
    GLYPH_REST_QUARTER, GLYPH_REST_WHOLE,
};
use crate::render::measure::draw_measure;
use crate::tools::{EditOp, MetaOp, Modifier, Tool, ToolGroup, ToolKind, all_tools};
use eframe::egui::UiKind::ScrollArea;
use eframe::egui::scroll_area::{ScrollBarVisibility, ScrollSource};
use eframe::egui::{Context, Key, Label, TextStyle, global_theme_preference_switch};
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
}

/// Liefert ein symbolisches Icon pro Tool als Text sowie einen Hinweis,
/// ob bevorzugt die Musik-Schriftart genutzt werden sollte.
fn tool_icon_text(t: &Tool) -> (String, bool) {
    match t.kind {
        ToolKind::InsertBeat(beat) => {
            match beat.duration {
                Duration::Simple(base) => {
                    let is_note = matches!(beat.kind, BeatKind::Note);
                    if is_note {
                        let s = match base {
                            Quarter => GLYPH_NOTE_QUARTER,
                            Eighth => GLYPH_NOTE_EIGHTH,
                            Sixteenth => GLYPH_NOTE_SIXTEENTH,
                            ThirtySecond => GLYPH_NOTE_32ND,
                            Half => GLYPH_NOTE_HALF,
                            Whole => GLYPH_NOTE_WHOLE,
                        };
                        (s.to_string(), true)
                    } else {
                        // Pausen als symbolisches "R" mit Basis
                        let s = match base {
                            Quarter => GLYPH_REST_QUARTER,
                            Eighth => GLYPH_REST_EIGHTH,
                            Sixteenth => GLYPH_REST_EIGHTH,
                            ThirtySecond => GLYPH_REST_32ND,
                            Half => GLYPH_REST_HALF,
                            Whole => GLYPH_REST_WHOLE,
                        };
                        (s.to_string(), true)
                    }
                }
                Duration::Tuplet { n, .. } => {
                    // Tuplets: zeige nur die Zählzahl (3,5,6,7,9)
                    (format!("{}", n), false)
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
            egui::TopBottomPanel::top("settings").show(ctx, |ui| {
                let mut style = ui.ctx().style().spacing.scroll;
                style.ui(ui);

                ui.ctx().all_styles_mut(|s| s.spacing.scroll = style);
            });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            Frame::canvas(ui.style())
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::NONE)
                .show(ui, |ui| {
                    // Interaktives Zeichenfeld: vollständige verfügbare Fläche als klickbares Rect
                    let size = ui.available_size();
                    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());

                    // Render der Measure innerhalb des klickbaren Bereichs
                    draw_measure(ui, &self.font_id, &self.measure, rect, Some(self.cursor_idx));

                    // Maus-/Touch-Klick: Cursor auf den nächstgelegenen Beat setzen
                    if (resp.clicked() || resp.dragged())
                        && let Some(pos) = resp.interact_pointer_pos()
                    {
                        // Layout erneut berechnen (Positionsdaten für Hit-Testing)
                        let layout = build_measure_layout_px(
                            &self.measure,
                            rect,
                            &self.font_id,
                            ui.ctx().pixels_per_point(),
                        );

                        // Falls keine Beats vorhanden sind, nichts tun
                        if !layout.x_centers.is_empty() {
                            // Außerhalb des Inhalts: zum nächstliegenden Rand clampen
                            let target_x = pos.x;
                            let idx = if target_x <= layout.content_left {
                                0
                            } else if target_x >= layout.content_right {
                                layout.x_centers.len() - 1
                            } else {
                                // Innerhalb: Index des nächstgelegenen x-Centers suchen
                                let mut best_i = 0usize;
                                let mut best_d = f32::MAX;
                                for (i, &cx) in layout.x_centers.iter().enumerate() {
                                    let d = (cx - target_x).abs();
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

        egui::TopBottomPanel::bottom("tool_palette")
            .frame(
                Frame::NONE
                    .fill(egui::Color32::TRANSPARENT)
                    .stroke(egui::Stroke::NONE)
                    .inner_margin(egui::Vec2::splat(10.0)),
            )
            .resizable(false)
            .max_height(120.0)
            .min_height(120.0)
            .show(ctx, |ui| {
                let tools = all_tools();
                let groups = [
                    ToolGroup::Notes,
                    ToolGroup::Rests,
                    ToolGroup::Modifiers,
                    ToolGroup::Tuplets,
                    ToolGroup::Edit,
                    ToolGroup::Meta,
                ];

                egui::ScrollArea::horizontal()
                    .scroll_source(ScrollSource::ALL)
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for g in groups {
                                let group_tools: Vec<_> =
                                    tools.iter().filter(|t| t.group == g).collect();
                                if group_tools.is_empty() {
                                    continue;
                                }

                                // Quadrat-Kacheln nebeneinander, umbrechend
                                let tile = 88.0; // Seitenlänge der Tool-Kacheln
                                for t in group_tools {
                                    // Symbol + optional Shortcut im Tooltip
                                    let (icon_text, is_music_icon) = tool_icon_text(t);
                                    let mut rich = egui::RichText::new(icon_text)
                                        .text_style(TextStyle::Button)
                                        .size(20.0);
                                    if is_music_icon {
                                        // Versuche, die Musik-Schriftart zu verwenden (Bravura, falls verfügbar)
                                        rich = rich.family(self.font_family.clone());
                                    }
                                    let button = egui::Button::new(rich)
                                        .corner_radius(10)
                                        //.sense(egui::Sense::click_and_drag())
                                        .min_size(egui::vec2(tile, tile));
                                    let resp = ui.add_sized([tile, tile], button);
                                    // Noch kein Klick/Drag-Verhalten – nur Darstellung
                                }
                            }
                        })
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
                    self.apply_base_duration_key(idx, Quarter, false);
                }
                if i.key_pressed(Key::Num2) {
                    self.apply_base_duration_key(idx, Eighth, true);
                }
                if i.key_pressed(Key::Num3) {
                    self.apply_base_duration_key(idx, Sixteenth, true);
                }
                if i.key_pressed(Key::Num4) {
                    self.apply_base_duration_key(idx, ThirtySecond, true);
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
    /// Wendet eine Basis-Notenwert-Änderung (Num1–Num4) auf den Beat bei `idx` an.
    ///
    /// - `base` bestimmt den Ziel-Basiswert (Viertel, Achtel, Sechzehntel, Zweiunddreißigstel).
    /// - `allow_on_tuplet`: Wenn `true`, wird bei Tuplets nur die Basis geändert und (n,m) beibehalten.
    ///   Wenn `false`, werden Tuplets ignoriert (z. B. keine Viertel-Tuplets unterstützen).
    fn apply_base_duration_key(&mut self, idx: usize, base: NoteValue, allow_on_tuplet: bool) {
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
        if let Some(new_dur) = new_dur_opt {
            let new_beat = match cur.kind {
                BeatKind::Note => Beat::note(new_dur),
                BeatKind::Rest => Beat::rest(new_dur),
            };
            let _ = self.measure.set_beat_at(idx, new_beat);
        }
    }

    pub fn new(cc: &CreationContext) -> Self {
        add_font(&cc.egui_ctx);
        let ff = FontFamily::Name("music".into());
        let mut m = Measure::new(TimeSignature::SEVEN_EIGHT);
        m.set_beat_at(0, Beat::note(st16())).unwrap();
        m.set_beat_at(6, Beat::note(t8())).unwrap();
        // m.set_beat_at(6, Beat::note(qt16())).unwrap();
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
