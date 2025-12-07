use crate::measure::duration::{Duration, NoteValue, TupletSpec};
use crate::measure::{BeatIdx, Measure, TimeSignature};

use crate::layout::pixel_layout::{LayoutOpts, build_measure_layout, build_time_sig_layout};
use crate::measure::BeatKind::Note;
use crate::measure::duration::NoteValue::*;
use crate::measure::duration::human_readable;
use crate::measure::editing::Modification;
use crate::measure::{Beat, BeatKind};
use crate::render::glyphs;
use crate::render::measure::{compute_em, draw_measure, draw_notes};
use crate::tools::{MetaOp, Tool, ToolKind};
use crate::{BeatTemplate, ToolGroup, all_tools};
use BeatKind::Rest;
use eframe::egui::scroll_area::{ScrollBarVisibility, ScrollSource};
use eframe::egui::{Align, Align2, Atom, Button, Context, Direction, Id, Key, Label, Layout, Response, RichText, TextStyle, Ui, Vec2, Widget, Margin, global_theme_preference_buttons};
use eframe::epaint::text::{FontInsert, InsertFontFamily};
use eframe::epaint::{FontFamily, FontId};
use eframe::{App, CreationContext, egui};
use egui::containers::Frame;
use std::collections::HashMap;
use log::info;

pub struct Grooph {
    font_family: FontFamily,
    font_id: FontId,
    measure: Measure,
    cursor_idx: BeatIdx,
    show_info: bool,
    show_settings: bool,
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
    is_running: bool,
    bpm: u32,
    audio: Option<crate::audio::Audio>,
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
        // Ensure the font-size bump applies for both dark and light themes by reapplying
        // an idempotent adjustment relative to each theme's baseline sizes.
        let is_dark = ctx.style().visuals.dark_mode;
        ctx.style_mut(|style| {
            // Capture baseline for current theme if not yet recorded
            if is_dark {
                if self.baseline_dark.is_none() {
                    let mut v = Vec::new();
                    for (ts, font) in style.text_styles.iter() {
                        v.push((ts.clone(), font.size));
                    }
                    self.baseline_dark = Some(v);
                }
                if let Some(base) = &self.baseline_dark {
                    for (ts, font) in style.text_styles.iter_mut() {
                        if let Some((_, sz)) = base.iter().find(|(t, _)| t == ts) {
                            font.size = *sz + self.font_bump;
                        }
                    }
                }
            } else {
                if self.baseline_light.is_none() {
                    let mut v = Vec::new();
                    for (ts, font) in style.text_styles.iter() {
                        v.push((ts.clone(), font.size));
                    }
                    self.baseline_light = Some(v);
                }
                if let Some(base) = &self.baseline_light {
                    for (ts, font) in style.text_styles.iter_mut() {
                        if let Some((_, sz)) = base.iter().find(|(t, _)| t == ts) {
                            font.size = *sz + self.font_bump;
                        }
                    }
                }
            }
        });

        // Global UI tweaks: increase button paddings across the app
        ctx.style_mut(|style| {
            style.spacing.button_padding = Vec2::new(10.0, 10.0);
            style.spacing.window_margin = Margin::same(10);
        });

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.toggle_value(&mut self.show_info, "?");
                ui.toggle_value(&mut self.show_settings, "⚙");

                // Playback controls
                ui.label("BPM");
                ui.add(egui::DragValue::new(&mut self.bpm).range(20..=300).speed(0.03));
                ui.separator();
                if ui.button(if self.is_running { "⏸" } else { "⏵" }).clicked() {
                    let old_running = self.is_running;
                    self.is_running = !old_running;
                    if self.is_running && self.audio.is_none() {
                        self.audio = crate::audio::Audio::new(self.bpm);
                    }
                }
                if ui.button("⏹").clicked() {
                    self.is_running = false;
                    if let Some(audio) = &mut self.audio {
                        audio.stop();
                    }
                }
            });
        });
        
        if let Some(audio) = &mut self.audio {
            audio.update(self.is_running, self.bpm, &self.measure);
        }

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
                global_theme_preference_buttons(ui);
            });
        }

        egui::TopBottomPanel::bottom("tool_palette")
            .frame(Frame::group(&ctx.style()).fill(ctx.style().visuals.panel_fill))
            .resizable(false)
            .show(ctx, |ui| {
                let tools = all_tools();
                // Ensure Edit tools (Undo/Redo) are shown first in the palette
                let groups = [
                    ToolGroup::Edit,
                    ToolGroup::Meta,
                    ToolGroup::Notes,
                    ToolGroup::Tuplets,
                    ToolGroup::Rests,
                ];

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

                    // Query playback position from audio (ticks)
                    let playback_tick: Option<f64> = self
                        .audio
                        .as_ref()
                        .and_then(|a| a.playback_position().map(|(t, _)| t));

                    let layout = draw_measure(
                        ui,
                        &self.font_id,
                        &self.measure,
                        rect,
                        Some(self.cursor_idx),
                        playback_tick,
                    );

                    // Block canvas interactions while the time signature dialog is open
                    if !self.show_ts_dialog
                        && (resp.clicked() || resp.dragged())
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

        if self.show_ts_dialog {
            egui::Window::new("Change time signature")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    let layout = Layout::top_down(Align::Center).with_cross_align(Align::Center);
                    ui.with_layout(layout, |ui| {
                        let l2 = Layout::left_to_right(Align::Min);
                        ui.with_layout(l2, |ui| {
                            egui::ComboBox::from_id_salt("beats")
                                .selected_text(format!("{}", self.ts_beats))
                                .show_ui(ui, |ui| {
                                    for v in 1u8..=16u8 {
                                        ui.selectable_value(&mut self.ts_beats, v, format!("{}", v));
                                    }
                                });
                            ui.label(" / ");
                            egui::ComboBox::from_id_salt("beat_unit")
                                .selected_text(format!("{}", self.ts_unit))
                                .show_ui(ui, |ui| {
                                    for v in [4u8, 8, 16] {
                                        ui.selectable_value(&mut self.ts_unit, v, format!("{}", v));
                                    }
                                });
                        });

                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);

                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                self.show_ts_dialog = false;
                            }
                            if ui.button("Done").clicked() {
                                // Prevent no-op undo entries
                                let current = self.measure.time_signature();
                                let new_ts =
                                    TimeSignature { beats: self.ts_beats, beat_unit: self.ts_unit };
                                if new_ts == current {
                                    self.show_ts_dialog = false;
                                    return;
                                }

                                // Snapshot before change
                                self.push_undo();
                                let res = self.measure.set_time_signature(new_ts);
                                match res {
                                    Ok(_) => {
                                        self.clear_redo();
                                        // Clamp cursor within bounds
                                        let new_len = self.measure.beats().len();
                                        if new_len > 0 {
                                            self.cursor_idx = self.cursor_idx.min(new_len - 1);
                                        } else {
                                            self.cursor_idx = 0;
                                        }
                                        self.show_ts_dialog = false;
                                    }
                                    Err(_) => {
                                        // Roll back snapshot if failed
                                        let _ = self.undo_stack.pop();
                                    }
                                }
                            }
                        });
                    });
                });
        }

        ctx.input(|i| {
            // While the time signature dialog is open, ignore global keyboard shortcuts
            if self.show_ts_dialog {
                return;
            }
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
        // Block palette/tool actions while a modal dialog is open
        if self.show_ts_dialog {
            return;
        }
        // Take a snapshot for tools that change state; Meta tools like dialogs drop it.
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
            ToolKind::Meta(MetaOp::ChangeTimeSignature) => {
                // Opening a dialog is not a state mutation: drop snapshot, open dialog
                let _ = self.undo_stack.pop();
                let ts = self.measure.time_signature();
                self.ts_beats = ts.beats;
                self.ts_unit = ts.beat_unit;
                self.show_ts_dialog = true;
                return; // no further state change now
            }
            ToolKind::Meta(MetaOp::ResetMeasure) => {
                // Keep the snapshot we took before calling apply_tool
                let ts = self.measure.time_signature();
                self.measure = Measure::new_init(ts, Rest);
                self.cursor_idx = 0;
                Some(Modification::ChangeTimeSignature(ts, ts))
            }
            ToolKind::Edit(crate::tools::EditOp::Undo) => {
                // Undo should not create a new snapshot; drop the one we took and perform undo
                let _ = self.undo_stack.pop();
                self.undo();
                return;
            }
            ToolKind::Edit(crate::tools::EditOp::Redo) => {
                // Redo should not create a new snapshot; drop the one we took and perform redo
                let _ = self.undo_stack.pop();
                self.redo();
                return;
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
                    crate::tools::Modifier::ToggleAccent => self.measure.toggle_accent(idx),
                    crate::tools::Modifier::ToggleRestNote => self.measure.toggle_beat_kind(idx),
                }
            }
        };

        // Clear redo for real changes; otherwise drop the snapshot we took before
        if result.is_some() {
            self.clear_redo();
        } else {
            let _ = self.undo_stack.pop();
        }

        info!("edited: {:?}", result);
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
        let tile = 80.0;
        let symbol_id = Id::new(id);
        let symbol = Atom::custom(symbol_id, Vec2::splat(tile));
        let button = Button::new(symbol).corner_radius(10).atom_ui(ui);

        if let Some(rect) = button.rect(symbol_id) {
            // Use a prebuilt measure for this button
            let measure = self.button_measures.get(id).unwrap();

            let cap_factor = match template {
                BeatTemplate { kind: Note, duration: Duration::Simple(..) } => 0.6,
                BeatTemplate {
                    kind: Note,
                    duration: Duration::Tuplet(TupletSpec { n: 9, .. }),
                } => 0.4,
                BeatTemplate { kind: Rest, .. } => 0.6,
                _ => 0.4,
            };

            let em = compute_em(&rect, cap_factor, ui);

            let y_offset = match template {
                BeatTemplate { kind: Note, duration: Duration::Simple(..) } => 20.0,
                BeatTemplate { kind: Note, duration: Duration::Tuplet(..) } => 18.0,
                _ => 2.0,
            };

            let opts = LayoutOpts {
                rect,
                font_id: FontId::new(em, self.font_id.family.clone()),
                em,
                layout_clef: false,
                layout_time_signature: false,
                y_offset,
                stem_length_factor: 0.8,
                stem_thickness_factor: 0.03,
            };
            let measure_layout = build_measure_layout(measure, &opts);
            let painter = &ui.painter_at(rect);
            draw_notes(painter, &measure_layout, ui.style().visuals.text_color(), &opts);
        }

        button.response
    }

    fn time_signature_button(&self, ui: &mut Ui, id: &str) -> Response {
        let tile = 80.0;
        let symbol_id = Id::new(id);
        let symbol = Atom::custom(symbol_id, Vec2::splat(tile));
        let button = Button::new(symbol).corner_radius(10).atom_ui(ui);

        if let Some(rect) = button.rect(symbol_id) {
            // Render a stacked 4/4 symbol using Bravura, similar to measure rendering
            let painter = &ui.painter_at(rect);
            let em = compute_em(&rect, 0.5, ui);
            let font_id = FontId::new(em, self.font_id.family.clone());

            // Build a minimal layout area for the time signature only
            let opts = LayoutOpts {
                rect,
                font_id: font_id.clone(),
                em,
                layout_clef: false,
                layout_time_signature: true,
                y_offset: 0.0,
                stem_length_factor: 0.9,
                stem_thickness_factor: 0.03,
            };

            // Use a temporary measure just for layout width & positions
            let ts = self.measure.time_signature();
            let layout = build_time_sig_layout(&ts, rect.center().x, &opts);
            let top = glyphs::ts_glyphs(ts.beats);
            let bot = glyphs::ts_glyphs(ts.beat_unit);
            for (p, ch) in layout.beats.iter().zip(top.iter()) {
                painter.text(
                    *p,
                    Align2::CENTER_CENTER,
                    ch.to_string(),
                    font_id.clone(),
                    ui.style().visuals.text_color(),
                );
            }
            for (p, ch) in layout.beat_unit.iter().zip(bot.iter()) {
                painter.text(
                    *p,
                    Align2::CENTER_CENTER,
                    ch.to_string(),
                    font_id.clone(),
                    ui.style().visuals.text_color(),
                );
            }
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
                match t.kind {
                    ToolKind::InsertBeat(template) => {
                        let button = self.note_button(ui, template, t.id);
                        if button.clicked() {
                            self.apply_tool(t);
                        }
                    }
                    ToolKind::Meta(MetaOp::ChangeTimeSignature) => {
                        let button = self.time_signature_button(ui, t.id);
                        if button.clicked() {
                            self.apply_tool(t);
                        }
                    }
                    _ => {
                        // Generic button for non-insert tools (e.g., Edit: Undo/Redo)
                        let button = Button::new(RichText::new(t.label).size(24.0))
                            .corner_radius(10)
                            .min_size(Vec2::splat(80.0))
                            .ui(ui);
                        if button.clicked() {
                            self.apply_tool(t);
                        }
                    }
                }
            }
        }
    }

    pub fn new(cc: &CreationContext) -> Self {
        add_font(&cc.egui_ctx);
        let ff = FontFamily::Name("music".into());
        // Initialize per-theme baselines on first update; font bump applied idempotently there.
        let m = Measure::new(TimeSignature::FOUR_FOUR);

        // Precompute button measures for all insert-beat tools
        let mut button_measures: HashMap<&'static str, Measure> = HashMap::new();
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
            show_ts_dialog: false,
            ts_beats: TimeSignature::FOUR_FOUR.beats,
            ts_unit: TimeSignature::FOUR_FOUR.beat_unit,
            button_measures,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            font_bump: 4.0,
            baseline_dark: None,
            baseline_light: None,
            is_running: false,
            bpm: 120,
            audio: None,
        }
    }
}
