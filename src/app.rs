use crate::measure::duration::{Duration, NoteValue, TupletSpec};
use crate::measure::{BeatIdx, Measure, TimeSignature};

use crate::layout::pixel_layout::{LayoutOpts, build_measure_layout};
use crate::measure::BeatKind::Note;
use crate::measure::duration::NoteValue::*;
use crate::measure::duration::human_readable;
use crate::measure::editing::Modification;
use crate::measure::{Beat, BeatKind};
use crate::render::measure::{compute_em, draw_measure, draw_notes};
use crate::tools::{Tool, ToolKind};
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
use std::collections::HashMap;

pub struct Grooph<'a> {
    font_family: FontFamily,
    font_id: FontId,
    measure: Measure<'a>,
    cursor_idx: BeatIdx,
    show_info: bool,
    show_settings: bool,
    // Prebuilt measures for note/rest/tuplet tool buttons to avoid per-frame reconstruction
    button_measures: HashMap<&'static str, Measure<'static>>,
    undo_stack: Vec<(Measure<'a>, BeatIdx)>,
    redo_stack: Vec<(Measure<'a>, BeatIdx)>,
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

impl App for Grooph<'_> {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        ctx.style_mut(|style| {
            // style
        });
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
            egui::TopBottomPanel::top("settings").show(ctx, |ui| {
                let mut visuals = ui.ctx().style().visuals.clone();
                visuals.ui(ui);

                // let mut debug = ui.ctx().style().interaction;
                // debug.ui(ui);

                ui.ctx().style_mut(|s| s.visuals = visuals.clone());
            });
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
            // Undo / Redo shortcuts: Ctrl/Cmd+Z (undo), Ctrl/Cmd+Shift+Z or Ctrl/Cmd+Y (redo)
            let mut consumed_undo_redo = false;
            let undo_combo = i.key_pressed(Key::Z) && (i.modifiers.command || i.modifiers.ctrl);
            let redo_combo_z = i.key_pressed(Key::Z)
                && (i.modifiers.command || i.modifiers.ctrl)
                && i.modifiers.shift;
            let redo_combo_y = i.key_pressed(Key::Y) && (i.modifiers.command || i.modifiers.ctrl);
            if undo_combo && !i.modifiers.shift {
                self.undo();
                consumed_undo_redo = true;
            } else if redo_combo_z || redo_combo_y {
                self.redo();
                consumed_undo_redo = true;
            }

            let beats_len = self.measure.beats().len();
            let total_len = beats_len;
            if total_len > 0 && !consumed_undo_redo {
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
                    self.push_undo();
                    self.measure.remove(idx);
                    // Move cursor right
                    let new_pos = (self.measure.beats().len() - 1).min(self.cursor_idx + 1);
                    self.cursor_idx = new_pos;
                    self.clear_redo();
                }
                if i.key_pressed(Key::Backspace) {
                    // Remove beat at the cursor
                    self.push_undo();
                    self.measure.remove(idx);
                    // Move cursor left
                    let new_len = self.measure.beats().len();
                    let new_pos = self.cursor_idx.saturating_sub(1).min(new_len - 1);
                    self.cursor_idx = new_pos;
                    self.clear_redo();
                }
                // Keyboard input routed through tool shortcuts
                for t in all_tools().iter().filter(|t| t.shortcut.is_some()) {
                    let sc = t.shortcut.unwrap();
                    if let Some(key) = Self::char_to_key(sc.key) {
                        // Match exact shift requirement
                        if i.key_pressed(key) && i.modifiers.shift == sc.with_shift {
                            self.apply_tool(t);
                        }
                    }
                }
                if i.key_pressed(Key::T) {
                    // Snapshot before attempting tuplet cycle via hotkey
                    self.push_undo();
                    let res = self.set_tuplet(idx, None);
                    if res.is_some() {
                        self.clear_redo();
                    } else {
                        let _ = self.undo_stack.pop();
                    }
                }
            }
        });
    }
}

impl Grooph<'_> {
    fn push_undo(&mut self) {
        self.undo_stack.push((self.measure.clone(), self.cursor_idx));
    }

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

    fn build_button_measure(template: BeatTemplate) -> Measure<'static> {
        let beat_count = if let Duration::Tuplet(TupletSpec { m, .. }) = template.duration {
            m
        } else {
            1
        };

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

    fn apply_tool(&mut self, tool: &Tool) {
        // Take a snapshot; if nothing changes, we'll drop it.
        self.push_undo();
        let result = match tool.kind {
            ToolKind::InsertBeat(template) => {
                let beats_len = self.measure.beats().len();
                if beats_len == 0 {
                    // No state change, drop the snapshot and return
                    let _ = self.undo_stack.pop();
                    return;
                }
                let idx = self.cursor_idx.min(beats_len - 1);

                match template.duration {
                    Duration::Simple(_) => {
                        self.set_beat(idx, template.duration.base_note(), Some(template.kind))
                    }
                    Duration::Tuplet(spec) => self.set_tuplet(idx, Some(spec)),
                    _ => None,
                }
            }
            ToolKind::Modify(modifier) => {
                let beats_len = self.measure.beats().len();
                if beats_len == 0 {
                    let _ = self.undo_stack.pop();
                    return;
                }
                let idx = self.cursor_idx.min(beats_len - 1);
                match modifier {
                    crate::tools::Modifier::ToggleDotted { dots: _ } => {
                        self.measure.toggle_dotted(idx)
                    }
                    crate::tools::Modifier::ToggleAccent => {
                        self.measure.toggle_accent(idx)
                    }
                    crate::tools::Modifier::ToggleRestNote => {
                        self.measure.toggle_beat_kind(idx)
                    }
                }
            }
            _ => {
                // Andere Toolarten (Modifier/Edit/Meta) werden in einem späteren Schritt verdrahtet.
                None
            }
        };

        // Clear redo for real changes; otherwise drop the snapshot we took before
        if result.is_some() {
            self.clear_redo();
        } else {
            let _ = self.undo_stack.pop();
        }

        println!("{:?}", result);
    }

    fn char_to_key(c: char) -> Option<Key> {
        use Key::*;
        Some(match c {
            '1' => Num1,
            '2' => Num2,
            '3' => Num3,
            '4' => Num4,
            '5' => Num5,
            '6' => Num6,
            '7' => Num7,
            '8' => Num8,
            '9' => Num9,
            '0' => Num0,
            '.' => Period,
            ' ' => Space,
            'a' | 'A' => A,
            't' | 'T' => T,
            _ => return None,
        })
    }

    fn note_button(&self, ui: &mut Ui, template: BeatTemplate, id: &str) -> Response {
        let tile = 90.0;
        let symbol_id = Id::new(id);
        let symbol = Atom::custom(symbol_id, Vec2::splat(tile));
        let button = egui::Button::new(symbol).corner_radius(10).atom_ui(ui);

        if let Some(rect) = button.rect(symbol_id) {
            // Use a prebuilt measure for this button
            let measure = self
                .button_measures
                .get(id)
                .unwrap();

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
            let measure_layout = build_measure_layout(measure, &opts);
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
        // Precompute button measures for all insert-beat tools
        let mut button_measures: HashMap<&'static str, Measure<'static>> = HashMap::new();
        for t in all_tools() {
            if let ToolKind::InsertBeat(template) = t.kind {
                button_measures.insert(t.id, Self::build_button_measure(template));
            }
        }

        Self {
            font_family: ff.clone(),
            font_id: FontId::new(16.0, ff),
            measure: m,
            cursor_idx: 0,
            show_info: false,
            show_settings: false,
            button_measures,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }
}
