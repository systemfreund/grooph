use crate::Grooph;
use crate::pickers::Picker;
use crate::tools::{DeleteOp, MetaOp, Modifier, NavOp, Tool, ToolGroup, ToolKind, all_tools};
use crate::{Mode, tools};
use eframe::egui;
use eframe::egui::scroll_area::{ScrollBarVisibility, ScrollSource};
use eframe::egui::{
    Align, Atom, Button, Color32, Direction, FontFamily, FontId, IdSalt, Layout, Rect, Response,
    RichText, Ui, Vec2,
};
use grooph_layout::pixel_layout::{
    LayoutOpts, MeasureLayout, build_measure_layout, build_time_sig_layout, compute_em,
};
use grooph_measure::BeatKind::{Note, Rest};
use grooph_measure::duration::{Duration, TupletKind};
use grooph_measure::{Cursor, Measure, StaffKind};
use grooph_render::measure::{draw_notes, draw_time_signature};
use log::info;
use tools::EditOp;

pub(crate) const TOOL_PALETTE_BUTTON_SIZE: f32 = 70.0;
const TOOL_PALETTE_BUTTON_CORNER_RADIUS: f32 = 2.0;

enum ButtonKind {
    /// Renders a miniature notation using `note_button`.
    Note,
    /// Renders a stacked time-signature glyph.
    TimeSignature,
    /// Plain text/symbol button with optional disabled state.
    Plain,
}

fn button_kind(kind: &ToolKind) -> ButtonKind {
    match kind {
        ToolKind::InsertBeat(..)
        | ToolKind::Modify(Modifier::ToggleDotted { .. })
        | ToolKind::Modify(Modifier::ToggleAccent | Modifier::ToggleSoft) => ButtonKind::Note,
        ToolKind::Meta(MetaOp::ChangeTimeSignature) => ButtonKind::TimeSignature,
        _ => ButtonKind::Plain,
    }
}

impl Grooph {
    pub(super) fn tool_palette_panel(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("tool_palette")
            .show_separator_line(false)
            .resizable(false)
            .show_collapsible(ui, &mut (self.ui.mode == Mode::Edit), |ui| {
                self.kit_palette(ui);
                let tools = all_tools().iter().filter(|t| t.show_in_palette).collect::<Vec<_>>();
                let groups = [
                    ToolGroup::Edit,
                    ToolGroup::Meta,
                    ToolGroup::Notes,
                    ToolGroup::Modifiers,
                    ToolGroup::Rests,
                    ToolGroup::Tuplets,
                ];

                egui::ScrollArea::horizontal()
                    .scroll_source(ScrollSource::ALL)
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        let layout = Layout::from_main_dir_and_cross_align(
                            Direction::LeftToRight,
                            Align::Center,
                        )
                        .with_cross_justify(true);

                        ui.with_layout(layout, |ui| {
                            self.tool_palette(tools.to_vec(), groups.as_slice(), ui);
                        })
                    });
            });
    }

    fn tool_palette(&mut self, tools: Vec<&Tool>, groups: &[ToolGroup], ui: &mut Ui) {
        for g in groups {
            for t in tools.iter().filter(|t| &t.group == g) {
                let response = match button_kind(&t.kind) {
                    ButtonKind::Note => self.note_button(ui, t.id),
                    ButtonKind::TimeSignature => self.time_signature_button(ui, t.id),
                    ButtonKind::Plain => self.plain_tool_button(ui, t),
                };
                if response.clicked() {
                    self.apply_tool(t);
                }
            }
        }
    }

    fn plain_tool_button(&self, ui: &mut Ui, tool: &Tool) -> Response {
        let enabled = match tool.kind {
            ToolKind::Edit(EditOp::Undo) => self.can_undo(),
            ToolKind::Edit(EditOp::Redo) => self.can_redo(),
            _ => true,
        };
        ui.add_enabled_ui(enabled, |ui| {
            ui.add_sized(
                Vec2::splat(TOOL_PALETTE_BUTTON_SIZE),
                Button::new(RichText::new(tool.label).size(24.0))
                    .corner_radius(TOOL_PALETTE_BUTTON_CORNER_RADIUS),
            )
        })
        .inner
    }

    pub(super) fn apply_tool(&mut self, tool: &Tool) {
        if !self.tool_applicable(tool) {
            return;
        }

        // Non-mutating tools have dedicated paths that manage their own state.
        match tool.kind {
            ToolKind::Edit(EditOp::Undo) => {
                self.undo();
                return;
            }
            ToolKind::Edit(EditOp::Redo) => {
                self.redo();
                return;
            }
            // The click bar's meter is the metronome's: pick it there.
            ToolKind::Meta(MetaOp::ChangeTimeSignature) if self.metronome_mode() => {
                self.ui.picker = Some(Picker::MetronomeMeter);
                return;
            }
            ToolKind::Meta(MetaOp::ChangeTimeSignature) => {
                self.ui.mode = Mode::TimeSignature(self.current_measure().time_signature());
                return;
            }
            ToolKind::Navigate(op) => {
                self.execute_navigation(op);
                return;
            }
            _ => {}
        }

        debug_assert!(tool.kind.is_mutating());

        let committed = self.with_undo_snapshot(|g| {
            // A drum is struck by one voice at a time: an edit that has two
            // voices strike it together is refused. One there already (from
            // a link, say) leaves the other edits of its measure alone.
            let kit = !g.staff_model().one_line();
            let before = kit.then(|| (g.editor.doc().cursor, g.current_measure().clone()));
            let changed = g.execute_mutating_tool(tool);
            if let Some((cursor, measure)) = before
                && changed
                && g.current_measure()
                    .doubled_instruments()
                    .iter()
                    .any(|d| !measure.doubled_instruments().contains(d))
            {
                let doc = g.editor.doc_mut();
                doc.score.measures_mut()[cursor.measure_idx] = measure;
                doc.cursor = cursor;
                return false;
            }
            changed
        });
        if committed {
            info!("applied tool: {}", tool.id);
        }
    }

    /// Gate for any tool dispatch — single source of truth, regardless of trigger (palette / keyboard / panel).
    fn tool_applicable(&self, tool: &Tool) -> bool {
        // The heads of one instrument and its techniques are the kit's: on
        // the one line a note is its one head.
        let kit_only =
            matches!(tool.kind, ToolKind::Modify(Modifier::ToggleHead | Modifier::CycleTechnique));
        // Tools only run in edit mode, and not while a modal dialog is open.
        // Mode::TimeSignature is a sub-state of editing but blocks tool actions.
        matches!(self.ui.mode, Mode::Edit) && !(kit_only && self.staff_model().one_line())
    }

    /// Moves the cursor according to `op`. Non-mutating; never produces an undo snapshot.
    /// Left/Right cross measure boundaries; Home/End jump to the very first/last beat in the score.
    /// The cursor stays in its voice, or the last one of a measure with fewer.
    fn execute_navigation(&mut self, op: NavOp) {
        let measure_count = self.editor.doc().score.len();
        let cursor = self.editor.doc().cursor;
        let to = |measure_idx, beat_idx| Cursor { measure_idx, beat_idx, ..cursor };
        let moved = match op {
            NavOp::Left if cursor.beat_idx > 0 => to(cursor.measure_idx, cursor.beat_idx - 1),
            // The clamp puts it on the last beat.
            NavOp::Left if cursor.measure_idx > 0 => to(cursor.measure_idx - 1, usize::MAX),
            NavOp::Left => cursor,
            NavOp::Right if cursor.beat_idx + 1 < self.current_voice().beats().len() => {
                to(cursor.measure_idx, cursor.beat_idx + 1)
            }
            NavOp::Right if cursor.measure_idx + 1 < measure_count => to(cursor.measure_idx + 1, 0),
            NavOp::Right => cursor,
            NavOp::Start => to(0, 0),
            NavOp::End => to(measure_count.saturating_sub(1), usize::MAX),
        };
        let doc = self.editor.doc_mut();
        doc.cursor = doc.score.clamp_cursor(moved);
    }

    /// Executes a tool that is known to be mutating. Returns whether a change was committed.
    /// Must only be called from within `with_undo_snapshot`.
    fn execute_mutating_tool(&mut self, tool: &Tool) -> bool {
        let beats_len = self.current_voice().beats().len();
        match tool.kind {
            ToolKind::InsertBeat(template) => {
                if beats_len == 0 {
                    return false;
                }
                let idx = self.editor.doc().cursor.beat_idx.min(beats_len - 1);
                let result = match template.duration {
                    Duration::Simple(_) => {
                        self.set_beat(idx, template.duration.base_note(), Some(template.kind))
                    }
                    Duration::Tuplet(spec) => self.set_tuplet(idx, Some(spec)),
                    _ => None,
                };
                result.is_some()
            }
            ToolKind::Meta(MetaOp::ResetMeasure) => {
                // The voice being edited; the others stay.
                let cursor = self.editor.doc().score.clamp_cursor(self.editor.doc().cursor);
                self.editor.doc_mut().score.measures_mut()[cursor.measure_idx]
                    .clear_voice(cursor.voice, Rest);
                self.editor.doc_mut().cursor = Cursor { beat_idx: 0, ..cursor };
                true
            }
            ToolKind::Modify(modifier) => {
                if beats_len == 0 {
                    return false;
                }
                let idx = self.editor.doc().cursor.beat_idx.min(beats_len - 1);
                let pen = self.edit_instrument();
                let result = match modifier {
                    Modifier::ToggleDotted { dots: _ } => {
                        self.current_voice_mut().toggle_dotted(idx)
                    }
                    Modifier::ToggleAccent => self.current_voice_mut().toggle_accent(idx, pen),
                    Modifier::ToggleSoft => self.current_voice_mut().toggle_soft(idx, pen),
                    Modifier::ToggleRestNote => self.current_voice_mut().toggle_beat_kind(idx, pen),
                    Modifier::CycleTuplet => self.set_tuplet(idx, None),
                    Modifier::ToggleHead => self.current_voice_mut().toggle_head(idx, pen),
                    Modifier::CycleTechnique => self.current_voice_mut().cycle_technique(idx, pen),
                };
                result.is_some()
            }
            ToolKind::Delete(op) => {
                if beats_len == 0 {
                    return false;
                }
                let idx = self.editor.doc().cursor.beat_idx.min(beats_len - 1);
                self.current_voice_mut().remove(idx);
                let new_len = self.current_voice().beats().len();
                self.editor.doc_mut().cursor.beat_idx = if new_len == 0 {
                    0
                } else {
                    let last = new_len - 1;
                    match op {
                        DeleteOp::Forward => (self.editor.doc().cursor.beat_idx + 1).min(last),
                        DeleteOp::Backward => {
                            self.editor.doc().cursor.beat_idx.saturating_sub(1).min(last)
                        }
                    }
                };
                true
            }
            ToolKind::Edit(_)
            | ToolKind::Meta(MetaOp::ChangeTimeSignature)
            | ToolKind::Navigate(_) => {
                debug_assert!(false, "non-mutating tool reached execute_mutating_tool");
                false
            }
        }
    }

    fn time_signature_button(&self, ui: &mut Ui, id: &str) -> Response {
        let symbol_id = IdSalt::new(id);
        let symbol = Atom::custom(symbol_id, Vec2::splat(TOOL_PALETTE_BUTTON_SIZE));
        let button =
            Button::new(symbol).corner_radius(TOOL_PALETTE_BUTTON_CORNER_RADIUS).atom_ui(ui);

        if let Some(rect) = button.rect(symbol_id) {
            // Render a stacked 4/4 symbol using Bravura, similar to measure rendering
            let painter = &ui.painter_at(rect);
            let em = compute_em(&rect, 0.7, ui);
            let font_id = FontId::new(em, self.ui.music_font_id.family.clone());

            // Build a minimal layout area for the time signature only
            let opts = LayoutOpts {
                rect,
                font_id: font_id.clone(),
                pixels_per_point: ui.ctx().pixels_per_point(),
                em,
                layout_clef: false,
                layout_time_signature: true,
                y_offset: 0.0,
                stem_length_factor: 0.9,
                stem_thickness_factor: 0.03,
                accent_displacement: 0.0,
                stem_down: false,
                shared_line: false,
                staff: StaffKind::Line,
            };

            // Use a temporary measure just for layout width and positions
            let ts = self.current_measure().time_signature();
            let layout = build_time_sig_layout(&ts, rect.center().x, &opts);
            draw_time_signature(painter, &layout, ts, &font_id, ui.style().visuals.text_color());
        }

        button.response
    }

    fn note_button(&self, ui: &mut Ui, id: &str) -> Response {
        let measure = self.editor.button_measures.get(id).unwrap();
        let family = self.ui.music_font_id.family.clone();
        notation_button(ui, IdSalt::new(id), measure, family, false, TOOL_PALETTE_BUTTON_SIZE)
    }
}

/// Padding (px) between a [`notation_button`]'s frame and its notes.
const NOTATION_PADDING: f32 = 4.0;
/// Height of a quarter note's ink (stem and head) in em, as drawn here.
const QUARTER_INK_EM: f32 = 0.85;
/// Margin around the notes inside their tile, as a share of its height.
const NOTATION_MARGIN: f32 = 0.08;

/// A button `size` high showing `measure` as miniature notation (tool
/// palette, grouping picker). `selected` renders it in the highlighted state.
pub(crate) fn notation_button(
    ui: &mut Ui,
    symbol_id: IdSalt,
    measure: &Measure,
    family: FontFamily,
    selected: bool,
    size: f32,
) -> Response {
    // The notes take most of the button's padding, the button keeps its
    // size.
    let padding = ui.spacing().button_padding;
    let room = 2.0 * (padding - Vec2::splat(NOTATION_PADDING)).max(Vec2::ZERO);
    let symbol = Atom::custom(symbol_id, notation_size(measure, size) + room);
    ui.spacing_mut().button_padding = padding.min(Vec2::splat(NOTATION_PADDING));
    let button = Button::new(symbol)
        .corner_radius(TOOL_PALETTE_BUTTON_CORNER_RADIUS)
        .selected(selected)
        .atom_ui(ui);
    ui.spacing_mut().button_padding = padding;
    if let Some(rect) = button.rect(symbol_id) {
        let color = ui.style().visuals.text_color();
        paint_notation(ui, rect, measure, family, color);
    }
    button.response
}

/// A button like a [`notation_button`] showing "none": a circle struck
/// through, `size` high.
pub(crate) fn none_button(ui: &mut Ui, symbol_id: IdSalt, selected: bool, size: f32) -> Response {
    let padding = ui.spacing().button_padding;
    let room = 2.0 * (padding - Vec2::splat(NOTATION_PADDING)).max(Vec2::ZERO);
    let symbol = Atom::custom(symbol_id, Vec2::splat(size) + room);
    ui.spacing_mut().button_padding = padding.min(Vec2::splat(NOTATION_PADDING));
    let button = Button::new(symbol)
        .corner_radius(TOOL_PALETTE_BUTTON_CORNER_RADIUS)
        .selected(selected)
        .atom_ui(ui);
    ui.spacing_mut().button_padding = padding;
    if let Some(rect) = button.rect(symbol_id) {
        paint_none(ui.painter(), rect, ui.style().visuals.text_color());
    }
    button.response
}

/// "None" in `rect`: a circle struck through, as on a [`none_button`].
pub(crate) fn paint_none(painter: &egui::Painter, rect: Rect, color: Color32) {
    let radius = 0.3 * rect.width().min(rect.height());
    let stroke = egui::Stroke::new((0.12 * radius).max(1.5), color);
    let center = rect.center();
    painter.circle_stroke(center, radius, stroke);
    // Lower left to upper right.
    let d = radius * std::f32::consts::FRAC_1_SQRT_2 * Vec2::new(1.0, -1.0);
    painter.line_segment([center - d, center + d], stroke);
}

/// Size of a [`notation_button`]'s notes `size` high: tuplets and long rows
/// of short notes get a wider tile.
pub(crate) fn notation_size(measure: &Measure, size: f32) -> Vec2 {
    let wide = measure.first_voice().beats().len() > 6
        || measure
            .first_voice()
            .beats()
            .iter()
            .any(|b| b.is_note() && b.duration.as_tuplet_spec().is_some());
    Vec2::new(if wide { 1.5 * size } else { size }, size)
}

/// Draw `measure` as miniature notation fitted into `rect`, as on a
/// [`notation_button`].
pub(crate) fn paint_notation(
    ui: &Ui,
    rect: Rect,
    measure: &Measure,
    family: FontFamily,
    color: Color32,
) {
    // Size the glyphs by the first note; rest-only measures use rest sizing.
    let Some(first) = measure.first_voice().beats().first() else {
        return;
    };
    let template = measure.first_voice().beats().iter().find(|b| b.is_note()).unwrap_or(first);
    {
        let cap_factor = match (template.kind(), template.duration) {
            (Rest, _) => 0.8,
            (Note, Duration::Tuplet(spec)) if matches!(spec.kind(), TupletKind::Nonuplet) => 0.35,
            (Note, Duration::Tuplet(..)) => 0.4,
            _ => 0.7,
        };

        let em = compute_em(&rect, cap_factor, ui);

        let y_offset = match (template.kind(), template.duration) {
            (Rest, _) => 2.0,
            (Note, Duration::Tuplet(..)) => 22.0,
            _ => 20.0,
        };

        let make_opts = |em: f32, rect: Rect| -> LayoutOpts {
            LayoutOpts {
                rect,
                font_id: FontId::new(em, family.clone()),
                pixels_per_point: ui.ctx().pixels_per_point(),
                em,
                layout_clef: false,
                layout_time_signature: false,
                y_offset,
                stem_length_factor: 0.8,
                stem_thickness_factor: 0.06,
                accent_displacement: 0.1,
                stem_down: false,
                shared_line: false,
                staff: StaffKind::Line,
            }
        };

        // Probe pass: lay the measure out at the size chosen from the
        // template beat, then take the ink of what it draws (note glyphs
        // incl. flags, and tuplet brackets/numbers) instead of
        // trusting the fixed factors above. Scale it to fill the tile
        // along whichever axis it reaches first, and center it: a
        // trailing unbeamed flag or a tuplet number stays inside, a single
        // note grows to the tile's height.
        let content = notes_ink(&build_measure_layout(measure.first_voice(), &make_opts(em, rect)));
        let avail = rect.shrink(NOTATION_MARGIN * rect.height());
        let content_size = Vec2::new((content.width()).max(1.0), (content.height()).max(1.0));
        let scale = (avail.width() / content_size.x).min(avail.height() / content_size.y);
        // No glyph larger than a quarter note filling the tile would be:
        // rests and flags keep their size next to the notes.
        let fitted_em = (em * scale).min(avail.height() / QUARTER_INK_EM);
        // Re-measure at the new size (glyph widths don't scale perfectly
        // linearly) and center the content in the tile.
        let fitted =
            notes_ink(&build_measure_layout(measure.first_voice(), &make_opts(fitted_em, rect)));
        let (final_em, final_rect) = (fitted_em, rect.translate(rect.center() - fitted.center()));

        let opts = make_opts(final_em, final_rect);
        let measure_layout = build_measure_layout(measure.first_voice(), &opts);
        let painter = &ui.painter_at(rect);
        draw_notes(painter, &measure_layout, color, None, &opts);
    }
}

/// Bounding rect (in the same pixel space as `LayoutOpts::rect`) of the ink
/// of everything a measure draws. Used to detect when the fixed sizing
/// factors in [`notation_button`] under- or over-shoot the tile, e.g. a
/// trailing unbeamed flag overhanging the right edge, or a tuplet number
/// close enough to the top of the tile to be clipped.
fn notes_ink(layout: &MeasureLayout) -> Rect {
    layout.notes_ink().unwrap_or(Rect::ZERO)
}

#[cfg(test)]
mod tests {
    use crate::shell::Tab;
    use crate::tools::all_tools;
    use crate::{Grooph, Mode};
    use grooph_measure::Cursor;
    use grooph_measure::notation::{format_score, parse_score};

    fn editing(score: &str) -> Grooph {
        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Score);
        app.editor.doc_mut().score = parse_score(score).unwrap();
        app.ui.mode = Mode::Edit;
        app
    }

    fn apply(app: &mut Grooph, id: &str) {
        let tool = all_tools().iter().find(|t| t.id == id).unwrap();
        app.apply_tool(tool);
    }

    fn text(app: &Grooph) -> String {
        format_score(&app.editor.doc().score).unwrap()
    }

    #[test]
    fn the_tools_edit_the_voice_of_the_cursor() {
        let mut app = editing("4/4 H:q H:q H:q H:q ; K:h K:h");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        apply(&mut app, "insert.note.q");
        // The kick keeps its head in a new duration; the hands stay.
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:h K:q r:q");
        apply(&mut app, "modify.toggle.rest_note");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 2));
        assert!(app.editor.doc().score.measures()[0].voice(1).unwrap().beats()[2].is_note());
        apply(&mut app, "edit.undo");
        apply(&mut app, "meta.reset_measure");
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; r:q r:q r:q r:q");
        apply(&mut app, "edit.undo");
        assert_eq!(text(&app), "4/4 H:q H:q H:q H:q ; K:h K:q r:q");
    }

    #[test]
    fn the_cursor_keeps_its_voice_from_measure_to_measure() {
        let mut app = editing("4/4 H:h H:h ; K:q K:q K:q K:q | H:h H:h ; K:h K:h | q q q q");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 3);
        apply(&mut app, "nav.right");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(1, 1, 0));
        apply(&mut app, "nav.left");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 3));
        apply(&mut app, "nav.start");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 0));
        // The last measure has one voice.
        apply(&mut app, "nav.end");
        assert_eq!(app.editor.doc().cursor, Cursor::at(2, 3));
        // Into a last measure with two, End keeps the voice.
        let mut two = editing("4/4 H:h H:h ; K:q K:q K:q K:q | H:h H:h ; K:h K:h");
        two.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        apply(&mut two, "nav.end");
        assert_eq!(two.editor.doc().cursor, Cursor::in_voice(1, 1, 1));
        // A new measure has the voices of the one before.
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 0);
        app.append_measure();
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(1, 1, 0));
        assert_eq!(app.editor.doc().score.measures()[1].voices().len(), 2);
        app.remove_current_measure();
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(1, 1, 0));
    }

    #[test]
    fn a_line_score_edits_as_before() {
        let mut app = editing("4/4 >q (q) q q");
        app.editor.doc_mut().cursor = Cursor::at(0, 1);
        apply(&mut app, "insert.note.e");
        assert_eq!(text(&app), "4/4 >q e r:e q q");
        assert_eq!(app.editor.doc().cursor, Cursor::at(0, 2));
        apply(&mut app, "meta.reset_measure");
        assert_eq!(text(&app), "4/4 r:q r:q r:q r:q");
    }

    #[test]
    fn the_rock_groove_can_be_entered_on_the_kit() {
        use grooph_measure::Instrument::{HiHat, Kick, Snare};
        let mut app = editing("4/4 q q q q");
        app.new_kit_score();
        assert_eq!(text(&app), "kit 4/4 r:q r:q r:q r:q ; r:q r:q r:q r:q");
        // The hi-hat's eighths, step by step.
        app.pick_instrument(HiHat);
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 0, 0));
        for _ in 0..8 {
            apply(&mut app, "insert.note.e");
        }
        // The snare on two and four, on the hi-hat's stem.
        app.pick_instrument(Snare);
        for beat in [1, 5] {
            app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, beat);
            apply(&mut app, "modify.toggle.head");
        }
        // The hi-hat opens on the last but one eighth and closes on the last.
        app.pick_instrument(HiHat);
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 6);
        apply(&mut app, "modify.cycle_technique");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 7);
        for _ in 0..3 {
            apply(&mut app, "modify.cycle_technique");
        }
        // The kick goes below, to the feet's voice, on the beat sounding.
        app.pick_instrument(Kick);
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 3));
        for beat in [0, 2] {
            app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, beat);
            apply(&mut app, "modify.toggle.rest_note");
        }
        assert_eq!(text(&app), "4/4 H:e S-H:e H:e H:e H:e S-H:e Ho:e Hc:e ; K:q r:q K:q r:q");
        // Accent and ghost change the picked instrument's head alone.
        app.pick_instrument(Snare);
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        apply(&mut app, "modify.toggle.accent");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 5);
        apply(&mut app, "modify.toggle.soft");
        assert!(text(&app).starts_with("4/4 H:e >S-H:e H:e H:e H:e (S)-H:e"), "{}", text(&app));
        // Taking the snare away leaves the hi-hat; the hi-hat's head away
        // on a beat of its own leaves a rest.
        apply(&mut app, "modify.toggle.head");
        app.pick_instrument(HiHat);
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 2);
        apply(&mut app, "modify.toggle.head");
        assert!(text(&app).starts_with("4/4 H:e >S-H:e r:e H:e H:e H:e"), "{}", text(&app));
        // Every step undoes, back to the empty kit bar.
        while app.can_undo() && app.editor.doc().score.staff == grooph_measure::StaffKind::Kit {
            apply(&mut app, "edit.undo");
        }
        assert_eq!(text(&app), "4/4 q q q q");
    }

    #[test]
    fn a_drum_is_not_struck_by_both_voices_at_once() {
        use grooph_measure::Instrument::Snare;
        let mut app = editing("4/4 S-H:q H:q H:q H:q ; K:q r:q K:q r:q");
        app.editor.instrument = Snare;
        // The snare is above on the first beat: not below as well.
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 0);
        apply(&mut app, "modify.toggle.head");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        apply(&mut app, "modify.toggle.rest_note");
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q H:q ; K:q S:q K:q r:q");
        // Below on the second beat, so not above there.
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        apply(&mut app, "modify.toggle.head");
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q H:q ; K:q S:q K:q r:q");
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 0, 1));
        // A refused edit leaves nothing to undo.
        apply(&mut app, "edit.undo");
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q H:q ; K:q r:q K:q r:q");

        // A snare in both voices already, from a link: the other edits of
        // the measure go through, a second doubling does not.
        let mut app = editing("4/4 S-H:q H:q H:q H:q ; S:q r:q K:h");
        app.editor.instrument = Snare;
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 3);
        apply(&mut app, "modify.toggle.accent");
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q H:q ; S:q r:q K:h");
        apply(&mut app, "modify.toggle.head");
        assert_eq!(text(&app), "4/4 S-H:q H:q H:q S-H:q ; S:q r:q K:h");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 2);
        apply(&mut app, "modify.toggle.head");
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 1);
        apply(&mut app, "modify.toggle.head");
        assert_eq!(text(&app), "4/4 S-H:q H:q S-H:q S-H:q ; S:q S:q K:h");
    }

    #[test]
    fn the_one_line_has_no_instruments_to_pick() {
        let mut app = editing("4/4 q (q) r:q q");
        app.pick_instrument(grooph_measure::Instrument::Kick);
        assert_eq!(app.editor.doc().cursor, Cursor::at(0, 0));
        apply(&mut app, "modify.toggle.head");
        apply(&mut app, "modify.cycle_technique");
        assert_eq!(text(&app), "4/4 q (q) r:q q");
        // The tools write the line's rhythm, as before.
        app.editor.doc_mut().cursor = Cursor::at(0, 2);
        apply(&mut app, "modify.toggle.rest_note");
        apply(&mut app, "modify.toggle.accent");
        assert_eq!(text(&app), "4/4 q (q) >q q");
    }
}
