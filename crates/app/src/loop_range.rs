//! The loop range of Practice's and the score tab's documents: the bars
//! `a..=b` that play and loop (see [`grooph_measure::BarRange`]).
//!
//! The document keeps the range ([`RhythmDoc::range`]); the audio engine
//! gets it with the playback options and takes a new one over at the next
//! barline. Until then the bars played still go by the old range, so the
//! app keeps that one ([`LoopPlayback::held`]) for its timing (the cursor,
//! play along, the trainer) and lets go of it when the cursor crosses a
//! barline ([`Grooph::cross_barline`]). Edits, undo and opening another
//! score take the document's range over at once.
//!
//! [`RhythmDoc::range`]: crate::documents::RhythmDoc::range

use crate::documents::Document;
use crate::score_change::ScoreChange;
use crate::{Grooph, TransportState};
use eframe::egui;
use eframe::egui::{Color32, Painter, Pos2, Rect, Sense, Stroke, Vec2, pos2};
use grooph_layout::staff_layout::{PlacedMeasure, StaffLayout};
use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{BarRange, MeasureIdx};

/// The range playback still goes by after the document's changed.
#[derive(Default)]
pub(crate) struct LoopPlayback {
    /// `Some(range)` from a change of the document's range while playing
    /// until the cursor crosses the next barline: the range (`None`: the
    /// whole score) of the bars played until then.
    pub(crate) held: Option<Option<BarRange>>,
}

impl Grooph {
    /// The loop range of the page shown (or behind the More screen); never
    /// in the metronome.
    pub(crate) fn loop_range(&self) -> Option<BarRange> {
        self.editor.docs.rhythm(self.document())?.range()
    }

    /// Whether the page shown has the loop button and the flags: Practice
    /// and the score tab, but not Practice in endless mode, and never
    /// embedded in a chat (only the metronome is there).
    pub(crate) fn loop_offered(&self) -> bool {
        match self.document() {
            Document::Metronome => false,
            Document::Practice => !self.editor.generator.config.endless && !self.embedded(),
            Document::Score => !self.embedded(),
        }
    }

    /// The user sets, moves or switches off the loop range of the page
    /// shown: no undo step and nothing unsaved. While playing it counts from
    /// the next barline on (see the module's description).
    pub(crate) fn set_loop_range(&mut self, range: Option<BarRange>) {
        let Some(doc) = self.editor.rhythm_doc_mut() else {
            return;
        };
        let old = doc.range();
        doc.set_range(range);
        let new = doc.range();
        if new == old {
            return;
        }
        let playing = self.playback_ctl.transport_state == TransportState::Playing
            && !self.playback_ctl.playback.counting_in();
        let held = &mut self.playback_ctl.loop_play.held;
        if !playing {
            *held = None;
        } else if held.is_none() {
            *held = Some(old);
        } else if *held == Some(new) {
            // Back to the range still played: nothing changes at the barline.
            *held = None;
        }
    }

    /// Endless mode is switched on: it never loops, so Practice's loop range
    /// goes, and no undo brings it back.
    pub(crate) fn switch_endless_on(&mut self) {
        self.editor.generator.config.endless = true;
        self.editor.docs.practice.set_range(None);
    }

    /// The loop button: on with every bar, or off.
    pub(crate) fn toggle_loop(&mut self) {
        let range = match self.loop_range() {
            Some(_) => None,
            None => Some(BarRange::all(self.editor.doc().score().len())),
        };
        self.set_loop_range(range);
    }

    /// The range the bars played go by now: the one held after a change
    /// until the next barline, else the document's; none while calibrating
    /// (see [`crate::calibration::played_range`]).
    pub(crate) fn played_range(&self) -> Option<BarRange> {
        if self.calibrating() {
            return None;
        }
        self.playback_ctl.loop_play.held.unwrap_or_else(|| self.audio_range())
    }

    /// The range the audio engine gets: the document's, which it takes over
    /// at the next barline; none while calibrating or in endless mode.
    pub(crate) fn audio_range(&self) -> Option<BarRange> {
        if self.endless_runs() {
            return None;
        }
        crate::calibration::played_range(
            &self.editor.docs,
            self.editor.document,
            &self.playback_ctl.play_along,
        )
    }

    /// The bars the bar after a barline comes from (see
    /// [`BarRange::next_bar`]): the audio engine's range, all bars of
    /// `timing` without one.
    pub(crate) fn next_loop(&self, timing: &ScoreTiming) -> BarRange {
        self.audio_range()
            .map_or(BarRange::all(timing.measure_count()), |r| r.clamped(timing.measure_count()))
    }

    /// The cursor moved from `before` to `after`: past a barline, a range
    /// held after a change gives way to the document's
    /// ([`ScoreChange::Range`]). Returns whether it did, so the timing is
    /// built again.
    pub(crate) fn cross_barline(&mut self, before: f64, after: f64, timing: &ScoreTiming) -> bool {
        if self.playback_ctl.loop_play.held.is_none() || timing.measure_count() == 0 {
            return false;
        }
        let crossed = after < before
            || timing.measure_at_global_tick(before) != timing.measure_at_global_tick(after);
        if crossed {
            self.playback_ctl.loop_play.held = None;
            self.score_changed(ScoreChange::Range);
        }
        crossed
    }
}

/// Which end of the loop range a flag marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum LoopEnd {
    /// The barline before the range's first bar.
    First,
    /// The barline after its last bar.
    Last,
}

/// A flag being dragged: its end and the bar it is on now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LoopDrag {
    pub(crate) end: LoopEnd,
    pub(crate) bar: MeasureIdx,
}

/// The finger-sized area a flag is grabbed by (points), over the top of
/// its row, larger than the flag drawn.
pub(crate) const FLAG_GRAB: f32 = 44.0;

/// Whether a flag of `range` stands on the barline after bar `idx` (at its
/// top right corner, where the bar menu's chip is).
pub(crate) fn flag_after(range: BarRange, idx: MeasureIdx) -> bool {
    range.last() == idx || range.first() == idx + 1
}
/// The drawn flag's width and height (points).
const FLAG_WIDTH: f32 = 18.0;
const FLAG_HEIGHT: f32 = 14.0;
/// The line on the barline (points).
const FLAG_LINE: f32 = 4.0;

impl LoopEnd {
    /// The bar of `range` the flag stands at.
    fn bar(self, range: BarRange) -> MeasureIdx {
        match self {
            LoopEnd::First => range.first(),
            LoopEnd::Last => range.last(),
        }
    }

    /// Its barline's x at the bar `placed`: before it or after it.
    fn barline_x(self, placed: &PlacedMeasure) -> f32 {
        match self {
            LoopEnd::First => placed.rect.left(),
            LoopEnd::Last => placed.rect.right(),
        }
    }

    /// Which way the flag points: into the range.
    fn into_range(self) -> f32 {
        match self {
            LoopEnd::First => 1.0,
            LoopEnd::Last => -1.0,
        }
    }
}

impl LoopDrag {
    /// `range` with the dragged end where the drag has it, stopping one
    /// bar before the other end.
    pub(crate) fn applied(self, range: BarRange) -> BarRange {
        match self.end {
            LoopEnd::First => range.with_first(self.bar),
            LoopEnd::Last => range.with_last(self.bar),
        }
    }
}

/// Where the flag of `end` grabs on `staff` for `range`: around the top of
/// its barline.
pub(crate) fn grab_rect(staff: &StaffLayout, range: BarRange, end: LoopEnd) -> Option<Rect> {
    let placed = staff.placed(end.bar(range))?;
    let x = end.barline_x(placed);
    Some(Rect::from_center_size(
        pos2(x, placed.rect.top() + FLAG_GRAB / 2.0 - FLAG_HEIGHT),
        Vec2::splat(FLAG_GRAB),
    ))
}

/// The bar of `bars` whose barline on the side of `end` lies nearest
/// `pos`: in the row `pos` is in (or the nearest row), then the closest
/// barline in it. So a flag snaps to barlines and can go to another row.
pub(crate) fn nearest_bar(
    staff: &StaffLayout,
    pos: Pos2,
    end: LoopEnd,
    bars: std::ops::RangeInclusive<MeasureIdx>,
) -> Option<MeasureIdx> {
    let distance = |placed: &PlacedMeasure| {
        let r = placed.rect;
        let dy = if pos.y < r.top() {
            r.top() - pos.y
        } else if pos.y > r.bottom() {
            pos.y - r.bottom()
        } else {
            0.0
        };
        (dy, (pos.x - end.barline_x(placed)).abs())
    };
    bars.filter_map(|i| staff.placed(i).map(|p| (i, distance(p))))
        .min_by(|(_, a), (_, b)| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)))
        .map(|(i, _)| i)
}

/// Draw the flag of `end` of `range` on `staff` in `color`: a line on its
/// barline from the staff's bottom line over the space above the row, with
/// a small flag at its top pointing into the range. `em` is the staff's.
fn draw_flag(
    painter: &Painter,
    staff: &StaffLayout,
    range: BarRange,
    end: LoopEnd,
    em: f32,
    color: Color32,
) {
    let Some(placed) = staff.placed(end.bar(range)) else {
        return;
    };
    let Some(system) = staff.systems.iter().find(|s| s.rect.contains(placed.rect.center())) else {
        return;
    };
    let x = end.barline_x(placed);
    let top = placed.rect.top() + 2.0;
    let bottom = system.y_baseline + 0.5 * em;
    painter.line_segment([pos2(x, top), pos2(x, bottom)], Stroke::new(FLAG_LINE, color));
    let dir = end.into_range();
    painter.add(egui::Shape::convex_polygon(
        vec![
            pos2(x, top),
            pos2(x + dir * FLAG_WIDTH, top + FLAG_HEIGHT / 2.0),
            pos2(x, top + FLAG_HEIGHT),
        ],
        color,
        Stroke::NONE,
    ));
}

/// Veil a bar outside the loop range, as the gap click veils a silent bar
/// (without its 🔇).
pub(crate) fn draw_outside_bar(ui: &egui::Ui, rect: Rect) {
    let bg = ui.visuals().panel_fill;
    let veil = Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), 150);
    ui.painter().rect_filled(rect, 4.0, veil);
}

impl Grooph {
    /// The loop range the staff shows: the page's, with a flag being
    /// dragged where it is; none where the page has no loop or while a
    /// calibration plays.
    pub(crate) fn shown_loop(&self) -> Option<BarRange> {
        if !self.loop_offered() || self.calibrating() {
            return None;
        }
        let range = self.loop_range()?;
        Some(self.ui.staff.loop_drag.map_or(range, |d| d.applied(range)))
    }

    /// The flags on `staff` (laid out from the document's score, maybe with
    /// the ghost bar), drawn at `em`, and their drags: a flag moves its end
    /// from barline to barline while dragged and sets the range when let go.
    /// Returns whether the pointer is the flags' this frame, so the staff
    /// does not take the press for a tap or an edit.
    pub(crate) fn loop_flags(&mut self, ui: &egui::Ui, staff: &StaffLayout, em: f32) -> bool {
        let Some(range) = self.loop_range().filter(|_| self.shown_loop().is_some()) else {
            self.ui.staff.loop_drag = None;
            return false;
        };
        let bars = self.editor.doc().score().len();
        let shown = self.ui.staff.loop_drag.map_or(range, |d| d.applied(range));
        let mut taken = self.ui.staff.loop_drag.is_some();
        let press = ui.input(|i| i.pointer.press_origin());
        for end in [LoopEnd::First, LoopEnd::Last] {
            let Some(rect) = grab_rect(staff, shown, end) else {
                continue;
            };
            taken |= press.is_some_and(|p| rect.contains(p));
            let resp = ui
                .interact(rect, egui::Id::unique("loop_flag").with(end), Sense::drag())
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            if resp.drag_started() {
                self.ui.staff.loop_drag = Some(LoopDrag { end, bar: end.bar(shown) });
            }
            if resp.dragged()
                && let Some(pos) = resp.interact_pointer_pos()
                && let Some(drag) = &mut self.ui.staff.loop_drag
                && drag.end == end
            {
                // One bar before the other flag at the most.
                let allowed = match end {
                    LoopEnd::First => 0..=range.last(),
                    LoopEnd::Last => range.first()..=bars - 1,
                };
                if let Some(bar) = nearest_bar(staff, pos, end, allowed) {
                    drag.bar = bar;
                }
            }
            if resp.drag_stopped()
                && let Some(drag) = self.ui.staff.loop_drag.take()
            {
                self.set_loop_range(Some(drag.applied(range)));
            }
        }
        let shown = self.ui.staff.loop_drag.map_or(range, |d| d.applied(range));
        let color = ui.visuals().selection.bg_fill;
        for end in [LoopEnd::First, LoopEnd::Last] {
            draw_flag(ui.painter(), staff, shown, end, em, color);
        }
        taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SavedDoc;
    use crate::documents::RhythmDoc;
    use crate::library::PatternLibrary;
    use crate::shell::Tab;
    use crate::{Mode, TransportState};
    use grooph_measure::notation::parse_score;
    use grooph_measure::{Cursor, Score, StaffKind};

    fn app() -> Grooph {
        let cc = crate::test_support::creation_context(egui::Context::default());
        Grooph::new(&cc)
    }

    /// Bars `a..=b`, counted from 1.
    fn bars(a: usize, b: usize) -> Option<BarRange> {
        BarRange::new(a - 1, b - 1)
    }

    /// The score tab with `n` bars of quarters, its editor open, the bars
    /// `a..=b` looping and nothing to undo.
    fn score_tab(n: usize, a: usize, b: usize) -> Grooph {
        let mut app = app();
        app.select_tab(Tab::Score);
        app.edit_page();
        let bar = parse_score("4/4 q q q q").unwrap().measures()[0].clone();
        let doc = &mut app.editor.docs.score;
        doc.score = Score::new(vec![bar; n]);
        doc.dirty = false;
        app.set_loop_range(bars(a, b));
        app
    }

    /// The score tab's document as saved.
    fn saved_doc_of(doc: &RhythmDoc) -> String {
        ron::to_string(&SavedDoc::of(doc)).unwrap()
    }

    /// A saved document read back as the app restores it, its fields that
    /// do not read falling back alone.
    fn saved_doc_ron(saved: &str, library: &PatternLibrary) -> Option<RhythmDoc> {
        let raw = ron::value::RawValue::from_ron(saved).ok()?;
        let repaired = crate::persist::repair_field("score_doc", raw)?.0;
        Some(repaired.into_rust::<SavedDoc>().ok()?.into_doc(library))
    }

    fn range(app: &Grooph) -> Option<BarRange> {
        app.loop_range()
    }

    /// Put the edit cursor on bar `bar` (from 1).
    fn at_bar(app: &mut Grooph, bar: usize) {
        *app.editor.doc_mut().cursor_mut() = Cursor::at(bar - 1, 0);
    }

    #[test]
    fn the_loop_button_switches_every_bar_on_and_off() {
        let mut app = score_tab(3, 1, 1);
        app.set_loop_range(None);
        app.toggle_loop();
        assert_eq!(range(&app), bars(1, 3));
        app.toggle_loop();
        assert_eq!(range(&app), None);
        // A score of one bar: the flags frame it.
        let mut app = score_tab(1, 1, 1);
        app.set_loop_range(None);
        app.toggle_loop();
        assert_eq!(range(&app), bars(1, 1));
    }

    #[test]
    fn the_range_follows_the_bars_it_covers_through_edits() {
        let check = |bar: usize, edit: fn(&mut Grooph), want: Option<BarRange>| {
            let mut app = score_tab(8, 3, 5);
            at_bar(&mut app, bar);
            edit(&mut app);
            assert_eq!(range(&app), want, "bar {bar}");
        };
        check(1, Grooph::duplicate_current_measure, bars(4, 6));
        check(4, Grooph::duplicate_current_measure, bars(3, 6));
        // The copy of its last bar lies after it, outside.
        check(5, Grooph::duplicate_current_measure, bars(3, 5));
        check(1, Grooph::remove_current_measure, bars(2, 4));
        check(4, Grooph::remove_current_measure, bars(3, 4));
        check(1, Grooph::push_measure, bars(3, 5));
        // The only bar of a range deleted: no range.
        let mut app = score_tab(8, 4, 4);
        at_bar(&mut app, 4);
        app.remove_current_measure();
        assert_eq!(range(&app), None);
        // All bars, and one appended: it lies outside.
        let mut app = score_tab(4, 1, 4);
        app.push_measure();
        assert_eq!(app.editor.doc().score().len(), 5);
        assert_eq!(range(&app), bars(1, 4));
    }

    #[test]
    fn undo_brings_the_range_back_with_its_bars() {
        let mut app = score_tab(9, 5, 8);
        at_bar(&mut app, 2);
        app.remove_current_measure();
        assert_eq!(range(&app), bars(4, 7));
        app.undo();
        assert_eq!(range(&app), bars(5, 8));
        app.redo();
        assert_eq!(range(&app), bars(4, 7));
    }

    #[test]
    fn undo_leaves_a_range_the_user_changed_since() {
        let mut app = score_tab(9, 5, 8);
        at_bar(&mut app, 9);
        app.remove_current_measure();
        app.set_loop_range(bars(6, 8));
        // Undo adds bar 9 back; the user's range stays.
        app.undo();
        assert_eq!(range(&app), bars(6, 8));
        app.redo();
        assert_eq!(range(&app), bars(6, 8));
        // Cut to the score when it gets shorter.
        let mut app = score_tab(4, 1, 1);
        app.push_measure();
        app.set_loop_range(bars(2, 5));
        app.undo();
        assert_eq!(range(&app), bars(2, 4));
        // A range switched off never comes back.
        let mut app = score_tab(4, 2, 3);
        app.push_measure();
        app.set_loop_range(None);
        app.undo();
        assert_eq!(range(&app), None);
        app.redo();
        assert_eq!(range(&app), None);
    }

    #[test]
    fn setting_the_range_is_no_edit() {
        let mut app = score_tab(4, 1, 1);
        assert!(!app.can_undo());
        app.set_loop_range(bars(2, 3));
        app.toggle_loop();
        app.toggle_loop();
        assert!(!app.can_undo());
        assert!(!app.editor.docs.score.dirty);
    }

    #[test]
    fn another_score_opens_without_a_range_and_undo_brings_it_back() {
        // "New drum groove", then undo.
        let mut app = score_tab(4, 2, 3);
        app.new_score(StaffKind::Kit);
        assert_eq!(range(&app), None);
        app.undo();
        assert_eq!(app.editor.doc().score().len(), 4);
        assert_eq!(range(&app), bars(2, 3));
        app.redo();
        assert_eq!(range(&app), None);
        // A library pattern.
        let mut app = score_tab(4, 2, 3);
        let id = app.save_pattern_as();
        app.load_pattern(id);
        assert_eq!(range(&app), None);
        // A shared link.
        let mut app = score_tab(4, 2, 3);
        app.replace_score_from_link(parse_score("4/4 q q q q | q q q q").unwrap());
        assert_eq!(range(&app), None);
        // On Practice: 🎲 (and so a generator setting).
        let mut app = self::app();
        app.select_tab(Tab::Practice);
        app.toggle_loop();
        assert!(range(&app).is_some());
        app.generate_new_score();
        assert_eq!(range(&app), None);
        app.undo();
        assert!(range(&app).is_some());
    }

    #[test]
    fn endless_mode_deletes_practices_range() {
        let mut app = app();
        app.select_tab(Tab::Practice);
        app.toggle_loop();
        assert!(app.loop_offered());
        app.switch_endless_on();
        assert_eq!(range(&app), None);
        assert!(!app.loop_offered());
        app.editor.generator.config.endless = false;
        assert_eq!(range(&app), None);
    }

    #[test]
    fn the_range_is_saved_with_its_document_but_not_in_the_library() {
        let app = score_tab(4, 2, 3);
        let saved = saved_doc_of(&app.editor.docs.score);
        let doc = saved_doc_ron(&saved, &app.editor.library).unwrap();
        assert_eq!(doc.range(), bars(2, 3));
        // Without the field, or with one that does not fit: none.
        let without = saved.replace(",range:Some((first:1,last:2))", "");
        assert_ne!(without, saved);
        assert_eq!(saved_doc_ron(&without, &app.editor.library).unwrap().range(), None);
        let outside = saved.replace("last:2", "last:7");
        assert_eq!(saved_doc_ron(&outside, &app.editor.library).unwrap().range(), None);
        let reversed = saved.replace("first:1,last:2", "first:2,last:1");
        assert_eq!(saved_doc_ron(&reversed, &app.editor.library).unwrap().range(), None);
        // The library keeps the score alone.
        let mut app = app;
        let id = app.save_pattern_as();
        app.load_pattern(id);
        assert_eq!(range(&app), None);
    }

    #[test]
    fn each_document_has_its_own_range() {
        let mut app = score_tab(4, 2, 3);
        app.select_tab(Tab::Practice);
        assert_eq!(range(&app), None);
        app.toggle_loop();
        app.select_tab(Tab::Score);
        assert_eq!(range(&app), bars(2, 3));
        // The metronome plays without one.
        app.select_tab(Tab::Metronome);
        assert_eq!(app.playback_options().range, None);
        assert!(!app.loop_offered());
        // So does a calibration, on any page.
        app.select_tab(Tab::Score);
        assert_eq!(app.playback_options().range, bars(2, 3));
        app.start_calibration(crate::play_along::HitSource::Tap);
        assert_eq!(app.playback_options().range, None);
        assert_eq!(app.score_timing().loop_bars(), BarRange::all(1));
    }

    #[test]
    fn a_range_changed_while_playing_counts_from_the_next_barline() {
        let mut app = score_tab(8, 3, 6);
        app.ui.mode = Mode::Playback;
        app.playback_ctl.transport_state = TransportState::Playing;
        let timing = app.score_timing();
        // In bar 5.
        let in_bar_5 = timing.measure_start_tick(4) as f64 + 10.0;
        app.playback_ctl.playback.smooth_tick = in_bar_5;
        app.set_loop_range(bars(3, 8));
        // The engine gets the new range at once, the bars played still go
        // by the old one.
        assert_eq!(app.playback_options().range, bars(3, 8));
        assert_eq!(app.score_timing().loop_bars(), bars(3, 6).unwrap());
        assert_eq!(app.next_loop(&timing), bars(3, 8).unwrap());
        // Within bar 5 nothing changes, also with a new tempo or a ramp's step.
        assert!(!app.cross_barline(in_bar_5, in_bar_5 + 100.0, &timing));
        app.score_changed(ScoreChange::Tempo);
        app.score_changed(ScoreChange::Timing);
        assert_eq!(app.score_timing().loop_bars(), bars(3, 6).unwrap());
        // At the barline the new one is played.
        let in_bar_6 = timing.measure_start_tick(5) as f64 + 1.0;
        assert!(app.cross_barline(in_bar_5, in_bar_6, &timing));
        assert_eq!(app.score_timing().loop_bars(), bars(3, 8).unwrap());
        // Changed and changed back before the barline: nothing waits.
        app.set_loop_range(bars(4, 8));
        app.set_loop_range(bars(3, 8));
        assert!(app.playback_ctl.loop_play.held.is_none());
        // Stopped, a change counts at once.
        app.playback_ctl.transport_state = TransportState::Stopped;
        app.set_loop_range(bars(2, 2));
        assert_eq!(app.score_timing().loop_bars(), bars(2, 2).unwrap());
    }
}
