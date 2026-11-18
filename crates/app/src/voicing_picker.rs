//! The score's voicing rule in the score tab's options: the "Voicing" row,
//! its picker of preset tiles and custom switches, and the question whether
//! the bars already written follow a changed rule too.
//!
//! The picker edits a pending rule; the score changes only when it closes,
//! and every change of one visit is one rule change (one undo step). Where
//! revoicing by the new rule would change a bar, "Revoice existing measures
//! too?" asks first: Yes sets the rule and revoices every bar, No sets the
//! rule only, Cancel (or dismissing the question) keeps the old rule.

use crate::Grooph;
use crate::library_panel::{PreviewLook, choice_buttons, dialog_width, pattern_preview};
use crate::pickers::{Picker, ROW_HEIGHT};
use crate::tool_palette::KIT_ORDER;
use eframe::egui::{
    self, Button, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, WidgetInfo, vec2,
};
use grooph_measure::notation::parse_score;
use grooph_measure::{Measure, Score, StaffKind, Voicing};

/// The bar the tiles show where the cursor's bar has no notes: a groove in
/// Hands/Feet, revoiced for each tile.
const EXAMPLE_BAR: &str = "2/4 H:e H:e S-H:e H:e ; K:e K:e r:q";

/// The voicing picker's rule while it is open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VoicingEdit {
    /// The score's rule when the picker opened.
    from: Voicing,
    /// The rule picked so far.
    pending: Voicing,
    /// The custom switches are shown.
    custom_open: bool,
}

/// The name of `voicing`: its preset's, or "Custom".
pub(crate) fn voicing_label(voicing: Voicing) -> &'static str {
    match voicing {
        Voicing::HANDS_FEET => "Hands/Feet",
        Voicing::ONE_VOICE => "One voice",
        Voicing::CYMBALS_ABOVE => "Cymbals above",
        _ => "Custom",
    }
}

/// What the tile of `preset` shows for `bar`: the bar revoiced by it, and
/// whether it stays as it is because it cannot be revoiced so. A bar
/// without notes shows a two-beat groove instead.
pub(crate) fn voicing_preview(bar: &Measure, preset: Voicing) -> (Measure, bool) {
    let example;
    let bar = match bar.head_onsets().is_empty() {
        true => {
            example = parse_score(EXAMPLE_BAR).expect("the example parses").measures()[0].clone();
            &example
        }
        false => bar,
    };
    match bar.revoiced(&preset) {
        Some(revoiced) => (revoiced, false),
        None => (bar.clone(), true),
    }
}

impl Grooph {
    /// Open the voicing picker on the score's rule.
    pub(crate) fn open_voicing_picker(&mut self) {
        let from = self.editor.doc().score().voicing;
        let custom_open = !Voicing::PRESETS.contains(&from);
        self.ui.dialogs.voicing_edit = Some(VoicingEdit { from, pending: from, custom_open });
        self.ui.dialogs.picker = Some(Picker::Voicing);
    }

    /// Pick `voicing` in the open voicing picker; the score changes when it
    /// closes.
    pub(crate) fn pick_voicing(&mut self, voicing: Voicing) {
        if let Some(edit) = &mut self.ui.dialogs.voicing_edit {
            edit.pending = voicing;
        }
    }

    /// The rule the open voicing picker shows.
    pub(crate) fn pending_voicing(&self) -> Option<Voicing> {
        self.ui.dialogs.voicing_edit.map(|e| e.pending)
    }

    /// The voicing picker closed: a changed rule is set where no bar would
    /// change by it, else the question comes up.
    pub(crate) fn close_voicing_picker(&mut self) {
        let Some(edit) = self.ui.dialogs.voicing_edit.take() else {
            return;
        };
        if edit.pending == edit.from {
            return;
        }
        if self.revoicing_changes_a_bar(edit.pending) {
            self.ui.dialogs.voicing_prompt = Some(edit.pending);
        } else {
            // Only bars without notes can change: their voices of rests.
            self.set_voicing(edit.pending, true);
        }
    }

    /// The answer to "Revoice existing measures too?": `Some(true)` sets the
    /// rule and revoices every bar, `Some(false)` sets the rule only (each
    /// one undo step), `None` (Cancel) keeps the old rule.
    pub(crate) fn answer_voicing_prompt(&mut self, revoice: Option<bool>) {
        let Some(voicing) = self.ui.dialogs.voicing_prompt.take() else {
            return;
        };
        let Some(revoice) = revoice else {
            return;
        };
        match self.set_voicing(voicing, revoice) {
            0 => {}
            1 => self.notify("1 bar stayed as it was"),
            stuck => self.notify(format!("{stuck} bars stayed as they were")),
        }
    }

    /// "Revoice existing measures too?", while a rule awaits its answer.
    pub(crate) fn voicing_prompt_dialog(&mut self, ui: &mut Ui) {
        if self.ui.dialogs.voicing_prompt.is_none() {
            return;
        }
        let width = dialog_width(ui.ctx());
        let modal = egui::Modal::new(egui::Id::unique("voicing_prompt")).show(ui.ctx(), |ui| {
            ui.set_max_width(width);
            ui.heading("Revoice existing measures too?");
            ui.add_space(8.0);
            ui.label("Yes sorts the notes of every bar by the new rule; it sounds the same. No keeps the bars as they are; only new notes follow it.");
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);
            choice_buttons(ui, width, &["Yes", "No", "Cancel"])
        });
        match modal.inner {
            Some(0) => self.answer_voicing_prompt(Some(true)),
            Some(1) => self.answer_voicing_prompt(Some(false)),
            Some(_) => self.answer_voicing_prompt(None),
            None if modal.should_close() => self.answer_voicing_prompt(None),
            None => {}
        }
    }

    /// The voicing picker: a tile per preset showing the cursor's bar in
    /// it, then "Custom" with a switch per instrument. Stays open: Done
    /// closes it, and the score changes then.
    pub(crate) fn voicing_choices(&mut self, ui: &mut Ui) -> bool {
        let Some(pending) = self.pending_voicing() else {
            return true;
        };
        let mut custom_open = self.ui.dialogs.voicing_edit.is_some_and(|e| e.custom_open);
        let mut picked = None;
        ui.weak("Which instruments are written in the upper voice, stems up, and which in the lower one, stems down, as they are entered.");
        ui.add_space(8.0);
        let look = PreviewLook {
            family: self.ui.fonts.music_font_id.family.clone(),
            stem_length_factor: self.ui.layout.stem_length_factor,
        };
        let bar = self.current_measure().clone();
        let room = (ui.ctx().content_rect().height() - 260.0).max(160.0);
        egui::ScrollArea::vertical().max_height(room).show(ui, |ui| {
            for preset in Voicing::PRESETS {
                let (preview, stays) = voicing_preview(&bar, preset);
                let selected = pending == preset;
                if voicing_tile(ui, voicing_label(preset), &preview, stays, selected, &look)
                    .clicked()
                {
                    picked = Some(preset);
                }
                ui.add_space(4.0);
            }
            let custom = !Voicing::PRESETS.contains(&pending);
            let arrow = if custom_open { "⏷" } else { "⏵" };
            let row = Button::new(RichText::new(format!("{arrow}  Custom")).size(18.0))
                .selected(custom)
                .min_size(vec2(ui.available_width(), ROW_HEIGHT));
            if ui.add(row).clicked() {
                custom_open = !custom_open;
            }
            if custom_open {
                for instrument in KIT_ORDER {
                    if let Some(lower) =
                        voice_switch(ui, instrument.label(), pending.is_lower(instrument))
                    {
                        picked = Some(pending.with_lower(instrument, lower));
                    }
                }
            }
        });
        if let Some(edit) = &mut self.ui.dialogs.voicing_edit {
            edit.custom_open = custom_open;
        }
        if let Some(voicing) = picked {
            self.pick_voicing(voicing);
        }
        false
    }
}

/// A preset's tile: its name over `bar`, and the hint where the bar stays
/// as it is. Marked while `selected`.
fn voicing_tile(
    ui: &mut Ui,
    name: &str,
    bar: &Measure,
    stays: bool,
    selected: bool,
    look: &PreviewLook,
) -> egui::Response {
    let visuals = ui.visuals();
    let stroke = match selected {
        true => Stroke::new(2.0, visuals.selection.bg_fill),
        false => Stroke::new(1.0, visuals.widgets.inactive.bg_stroke.color),
    };
    let frame = Frame::new()
        .fill(visuals.extreme_bg_color)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(8));
    let shown = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        let label = RichText::new(name).size(18.0);
        ui.label(if selected { label.strong() } else { label });
        let score = Score::single(bar.clone()).with_staff(StaffKind::Kit);
        pattern_preview(ui, &score, ui.available_width(), look);
        if stays {
            ui.weak("This bar stays as it is");
        }
    });
    let response = ui.interact(
        shown.response.rect,
        ui.make_persistent_id(("voicing_tile", name)),
        Sense::click(),
    );
    response.widget_info(|| WidgetInfo::selected(egui::Role::Button, true, selected, name));
    response
}

/// A row with `name` and an Up / Down switch, `lower` showing which; the
/// side tapped, if it is the other.
fn voice_switch(ui: &mut Ui, name: &str, lower: bool) -> Option<bool> {
    let mut picked = None;
    ui.horizontal(|ui| {
        let spacing = ui.spacing().item_spacing.x;
        let button = 72.0;
        let label = ui.available_width() - 2.0 * (button + spacing);
        let (rect, _) = ui.allocate_exact_size(vec2(label, ROW_HEIGHT), Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::proportional(17.0),
            ui.visuals().text_color(),
        );
        for (down, text) in [(false, "▲ Up"), (true, "▼ Down")] {
            let side = Button::new(RichText::new(text).size(16.0))
                .selected(lower == down)
                .min_size(vec2(button, ROW_HEIGHT));
            if ui.add(side).on_hover_text(format!("{name}: {text}")).clicked() && lower != down {
                picked = Some(down);
            }
        }
    });
    picked
}

#[cfg(test)]
mod tests {
    use super::{voicing_label, voicing_preview};
    use crate::pickers::Picker;
    use crate::shell::Tab;
    use crate::tools::ToolId;
    use crate::{Grooph, Mode};
    use grooph_measure::Instrument::{HiHatFoot, Kick, Snare};
    use grooph_measure::notation::{format_score, parse_score};
    use grooph_measure::{Cursor, StaffKind, Voicing};

    const HANDS_FEET: &str = "4/4 H:e H:e S-H:e H:e H:e H:e S-H:e H:e ; K:q r:q K:e K:e r:q";
    const CYMBALS_ABOVE: &str = "4/4 H:e H:e H:e H:e H:e H:e H:e H:e ; K:q S:q K:e K:e S:q";
    /// Snare triplets above, kick quintuplets below: the snare cannot go
    /// down (see `Measure::revoiced`).
    const STUCK: &str = "4/4 S:t8 S:t8 S:t8 r:q r:h ; K:qt16 K:qt16 K:qt16 K:qt16 K:qt16 r:q r:h";

    fn editing(score: &str) -> Grooph {
        let cc = crate::test_support::creation_context(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.editor.doc_mut().edit_score(|s| *s = parse_score(score).unwrap());
        app.ui.mode = Mode::Edit;
        app
    }

    fn text(app: &Grooph) -> String {
        format_score(app.editor.doc().score()).unwrap()
    }

    fn rule(app: &Grooph) -> Voicing {
        app.editor.doc().score().voicing
    }

    fn steps(app: &Grooph) -> u64 {
        app.editor.doc().history().pushed()
    }

    fn undo(app: &mut Grooph) {
        app.apply_tool(ToolId::EditUndo.tool());
    }

    /// Opens the picker, picks `voicing` and closes it.
    fn change_rule(app: &mut Grooph, voicing: Voicing) {
        app.open_voicing_picker();
        assert_eq!(app.ui.dialogs.picker, Some(Picker::Voicing));
        app.pick_voicing(voicing);
        app.close_voicing_picker();
    }

    #[test]
    fn a_changed_rule_asks_whether_the_bars_follow() {
        let both = format!("{HANDS_FEET} | {}", &HANDS_FEET[4..]);
        let cymbals = format!("{CYMBALS_ABOVE} | {}", &CYMBALS_ABOVE[4..]);
        // Yes: the rule and every bar, one step.
        let mut app = editing(&both);
        change_rule(&mut app, Voicing::CYMBALS_ABOVE);
        assert_eq!(app.ui.dialogs.voicing_prompt, Some(Voicing::CYMBALS_ABOVE));
        // Nothing changed while the question is open.
        assert_eq!((text(&app), rule(&app)), (both.clone(), Voicing::HANDS_FEET));
        app.answer_voicing_prompt(Some(true));
        assert_eq!((text(&app), rule(&app)), (cymbals.clone(), Voicing::CYMBALS_ABOVE));
        assert_eq!(app.ui.dialogs.notice, None);
        undo(&mut app);
        assert_eq!((text(&app), rule(&app)), (both.clone(), Voicing::HANDS_FEET));
        // No: the rule only, one step.
        let mut app = editing(&both);
        change_rule(&mut app, Voicing::CYMBALS_ABOVE);
        app.answer_voicing_prompt(Some(false));
        assert_eq!((text(&app), rule(&app)), (both.clone(), Voicing::CYMBALS_ABOVE));
        undo(&mut app);
        assert_eq!((text(&app), rule(&app)), (both.clone(), Voicing::HANDS_FEET));
        // Cancel, as dismissing the question: nothing, and no step.
        let mut app = editing(&both);
        let before = steps(&app);
        change_rule(&mut app, Voicing::CYMBALS_ABOVE);
        app.answer_voicing_prompt(None);
        assert_eq!(app.ui.dialogs.voicing_prompt, None);
        assert_eq!((text(&app), rule(&app)), (both, Voicing::HANDS_FEET));
        assert_eq!(steps(&app), before);
    }

    #[test]
    fn a_rule_that_changes_no_bar_is_set_without_asking() {
        let mut app = editing(HANDS_FEET);
        app.new_score(StaffKind::Kit);
        let empty = text(&app);
        let before = steps(&app);
        change_rule(&mut app, Voicing::CYMBALS_ABOVE);
        assert_eq!(app.ui.dialogs.voicing_prompt, None);
        assert_eq!((text(&app), rule(&app)), (empty.clone(), Voicing::CYMBALS_ABOVE));
        assert_eq!(steps(&app), before + 1);
        undo(&mut app);
        assert_eq!((text(&app), rule(&app)), (empty, Voicing::HANDS_FEET));
        // The same rule again is no change.
        change_rule(&mut app, Voicing::HANDS_FEET);
        assert_eq!(steps(&app), before + 1);
    }

    #[test]
    fn the_changes_of_one_visit_are_one_step() {
        let mut app = editing(HANDS_FEET);
        app.new_score(StaffKind::Kit);
        let before = steps(&app);
        app.open_voicing_picker();
        // The custom switches open closed on a preset.
        assert!(!app.ui.dialogs.voicing_edit.unwrap().custom_open);
        let mut pending = rule(&app);
        for (instrument, lower) in [(Snare, true), (Kick, false), (Snare, false)] {
            pending = pending.with_lower(instrument, lower);
            app.pick_voicing(pending);
        }
        assert_eq!(voicing_label(app.pending_voicing().unwrap()), "Custom");
        // Not in the score until the picker closes.
        assert_eq!(rule(&app), Voicing::HANDS_FEET);
        app.close_voicing_picker();
        assert_eq!(rule(&app), Voicing::from_lower([HiHatFoot]));
        assert_eq!(voicing_label(rule(&app)), "Custom");
        assert_eq!(steps(&app), before + 1);
        // A custom rule opens with the switches shown and Custom marked.
        app.open_voicing_picker();
        assert!(app.ui.dialogs.voicing_edit.unwrap().custom_open);
        // Every foot up is One voice.
        app.pick_voicing(app.pending_voicing().unwrap().with_lower(HiHatFoot, false));
        assert_eq!(app.pending_voicing(), Some(Voicing::ONE_VOICE));
        assert_eq!(voicing_label(app.pending_voicing().unwrap()), "One voice");
        app.close_voicing_picker();
        assert_eq!(rule(&app), Voicing::ONE_VOICE);
        undo(&mut app);
        assert_eq!(rule(&app), Voicing::from_lower([HiHatFoot]));
        undo(&mut app);
        assert_eq!(rule(&app), Voicing::HANDS_FEET);
    }

    #[test]
    fn a_bar_that_cannot_be_revoiced_stays_and_says_so() {
        let mut app = editing(&format!("{STUCK} | {}", &HANDS_FEET[4..]));
        change_rule(&mut app, Voicing::CYMBALS_ABOVE);
        app.answer_voicing_prompt(Some(true));
        assert_eq!(text(&app), format!("{STUCK} | {}", &CYMBALS_ABOVE[4..]));
        assert_eq!(rule(&app), Voicing::CYMBALS_ABOVE);
        let notice = app.ui.dialogs.notice.as_ref().map(|n| n.text.as_str());
        assert_eq!(notice, Some("1 bar stayed as it was"));
        // "Revoice bar…" on it.
        app.ui.dialogs.notice = None;
        *app.editor.doc_mut().cursor_mut() = Cursor::at(0, 0);
        let before = steps(&app);
        app.revoice_current_measure(Voicing::CYMBALS_ABOVE);
        assert_eq!(text(&app), format!("{STUCK} | {}", &CYMBALS_ABOVE[4..]));
        assert_eq!(steps(&app), before);
        let notice = app.ui.dialogs.notice.as_ref().map(|n| n.text.as_str());
        assert_eq!(notice, Some("This bar stayed as it was"));
    }

    #[test]
    fn the_tiles_show_the_bar_in_each_preset() {
        let bar = |s: &str| parse_score(s).unwrap().measures()[0].clone();
        let shown = |m| format_score(&grooph_measure::Score::single(m)).unwrap();
        // An empty bar shows the example in each style.
        let empty = bar("4/4 r:h r:h ; r:w");
        let tiles = Voicing::PRESETS.map(|v| {
            let (m, stays) = voicing_preview(&empty, v);
            assert!(!stays);
            shown(m)
        });
        assert_eq!(
            tiles,
            [
                "2/4 H:e H:e S-H:e H:e ; K:e K:e r:q",
                "2/4 K-H:e K-H:e S-H:e H:e",
                "2/4 H:e H:e H:e H:e ; K:e K:e S:q",
            ]
        );
        // The cursor's bar, revoiced; one that cannot be stays, with the hint.
        assert_eq!(
            voicing_preview(&bar(HANDS_FEET), Voicing::CYMBALS_ABOVE),
            (bar(CYMBALS_ABOVE), false)
        );
        assert_eq!(voicing_preview(&bar(STUCK), Voicing::CYMBALS_ABOVE), (bar(STUCK), true));
    }

    #[test]
    fn new_grooves_are_written_as_the_setting_says() {
        let mut app = editing(HANDS_FEET);
        app.editor.settings.new_groove_voicing = Voicing::CYMBALS_ABOVE;
        app.new_score(StaffKind::Kit);
        assert_eq!(rule(&app), Voicing::CYMBALS_ABOVE);
        // "Use this score's" copies a custom rule as it is now.
        let custom = Voicing::from_lower([Kick, Snare]);
        app.editor.doc_mut().edit_score(|s| s.voicing = custom);
        app.use_score_voicing_for_new_grooves();
        assert_eq!(app.editor.settings.new_groove_voicing, custom);
        app.editor.doc_mut().edit_score(|s| s.voicing = Voicing::ONE_VOICE);
        assert_eq!(app.editor.settings.new_groove_voicing, custom);
        app.new_score(StaffKind::Kit);
        assert_eq!(rule(&app), custom);
    }

    /// Runs the dialogs and the notice for a frame on a phone's screen.
    fn dialog_frame(app: &mut Grooph, ctx: &eframe::egui::Context, time: f64) {
        use eframe::egui::{Pos2, RawInput, Rect, vec2};
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(390.0, 844.0))),
            time: Some(time),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            app.apply_style(ui);
            app.settings_picker(ui);
            app.notice_toast(ui);
        });
        output.textures_delta.clear();
    }

    #[test]
    fn the_picker_the_question_and_the_notice_show_and_go() {
        let ctx = eframe::egui::Context::default();
        let cc = crate::test_support::creation_context(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.editor.doc_mut().edit_score(|s| *s = parse_score(STUCK).unwrap());
        app.editor.doc_mut().edit_score(|s| s.voicing = Voicing::from_lower([Kick]));
        // Custom opens with its switches.
        app.open_voicing_picker();
        dialog_frame(&mut app, &ctx, 0.0);
        assert_eq!(app.ui.dialogs.picker, Some(Picker::Voicing));
        app.close_voicing_picker();
        app.ui.dialogs.voicing_prompt = Some(Voicing::CYMBALS_ABOVE);
        app.ui.dialogs.picker = None;
        dialog_frame(&mut app, &ctx, 0.1);
        assert_eq!(app.ui.dialogs.voicing_prompt, Some(Voicing::CYMBALS_ABOVE));
        // The notice goes after its time, counted from when it first shows.
        app.notify("This bar stayed as it was");
        dialog_frame(&mut app, &ctx, 10.0);
        dialog_frame(&mut app, &ctx, 12.5);
        assert!(app.ui.dialogs.notice.is_some());
        dialog_frame(&mut app, &ctx, 13.1);
        assert_eq!(app.ui.dialogs.notice, None);
    }
}
