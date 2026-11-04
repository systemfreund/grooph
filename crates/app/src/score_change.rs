//! The one place that brings the app in line after the score or its timing
//! changed: the accuracy session, the playback state, the cursor and the
//! document's flags (`dirty`, `active_pattern_id`). Call sites make the
//! change themselves, then report what kind it was to
//! [`Grooph::score_changed`].
//!
//! The audio plan, the staff layout and the onsets are pulled from the score
//! every frame and need nothing from here, except Endless dropping a bar
//! (the audio engine's position moves back with it).

use crate::documents::Document;
use crate::{Grooph, TransportState, clock};
use grooph_measure::{Cursor, Score};

/// What changed about the score of the document shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScoreChange {
    /// The notes changed in place: an edit (tools, bar actions, voices,
    /// pickers, the metronome's clicks), undo or redo. The cursor is the
    /// caller's.
    Edited,
    /// A new score took the document's place; it starts at its first beat.
    Replaced(Origin),
    /// Endless mode added a bar at the end.
    Appended,
    /// Endless mode dropped the first bar, `ticks` long.
    DroppedFront { ticks: u64 },
    /// The BPM setting changed, the notes did not. A saved score keeps its
    /// tempo, so this can be an unsaved change of the score tab's score.
    Tempo,
    /// The tempo played changed, nothing to save: a ramp's step, or the
    /// tempo of a link that brought its own rhythm.
    Timing,
    /// Another document is shown: what plays is another score.
    Switched,
}

/// Where a [`ScoreChange::Replaced`] score came from, which says whether it
/// is a change to save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The saved pattern `id`, unchanged unless a drum struck by two voices
    /// at once was taken from one of them (`undoubled`).
    Pattern { id: u64, undoubled: bool },
    /// A shared link, or a pattern of the one line opened on Practice: it can
    /// be opened again, so it is not unsaved, unless `undoubled` (as above).
    Link { undoubled: bool },
    /// A new rhythm of the generator or a new drum groove: unsaved, of no
    /// pattern.
    New,
}

impl Grooph {
    /// Bring accuracy, playback state, cursor and document flags in line
    /// with `change`, which the caller has just made.
    pub(crate) fn score_changed(&mut self, change: ScoreChange) {
        match change {
            ScoreChange::Edited => {
                self.playback_ctl.accuracy.tracker.clear_for_edit();
                // The bar's length may have changed while playing.
                self.realign_accuracy_clock();
                self.editor.doc_mut().dirty = true;
            }
            ScoreChange::Replaced(origin) => {
                self.playback_ctl.accuracy.tracker.clear_for_edit();
                if self.playback_ctl.transport_state == TransportState::Stopped {
                    self.playback_ctl.playback.reset();
                }
                // A new rhythm of the generator while playing plays on.
                self.realign_accuracy_clock();
                let (pattern, dirty) = match origin {
                    Origin::Pattern { id, undoubled } => (Some(id), undoubled),
                    Origin::Link { undoubled } => (None, undoubled),
                    Origin::New => (None, true),
                };
                let doc = self.editor.doc_mut();
                doc.cursor = Cursor::start();
                doc.active_pattern_id = pattern;
                doc.dirty = dirty;
            }
            ScoreChange::Appended => self.endless_changed(),
            ScoreChange::DroppedFront { ticks } => {
                self.shift_playback_back(ticks);
                self.endless_changed();
            }
            ScoreChange::Tempo => {
                self.mark_tempo_unsaved();
                self.realign_accuracy_clock();
            }
            ScoreChange::Timing => self.realign_accuracy_clock(),
            ScoreChange::Switched => {
                self.playback_ctl.playback.reset();
                self.playback_ctl.accuracy.tracker.clear_for_edit();
            }
        }
    }

    /// Replace the document's score with `score` from `origin`, as one
    /// undoable step.
    pub(crate) fn replace_score(&mut self, score: Score, origin: Origin) {
        let snap = self.current_snapshot();
        self.editor.doc_mut().history.push(snap);
        self.editor.doc_mut().score = score;
        self.score_changed(ScoreChange::Replaced(origin));
    }

    /// A tempo change: a saved score keeps its tempo, so that is an unsaved
    /// change of the score tab's score when it is shown or is a saved
    /// pattern, whichever page the tempo was changed on.
    fn mark_tempo_unsaved(&mut self) {
        let doc = &mut self.editor.docs.score;
        if self.editor.document == Document::Score || doc.active_pattern_id.is_some() {
            doc.dirty = true;
        }
    }

    /// The running score of endless mode is no saved one.
    fn endless_changed(&mut self) {
        // The loop got longer or shorter: anchor the clock again.
        self.realign_accuracy_clock();
        self.editor.doc_mut().active_pattern_id = None;
        self.editor.doc_mut().dirty = true;
    }

    /// The score lost its first bar, `ticks` long: everything moves back by
    /// it. The cursors by its ticks, the view by one row, so nothing on
    /// screen jumps.
    fn shift_playback_back(&mut self, ticks: u64) {
        self.playback_ctl.accuracy.tracker.drop_leading_ticks(ticks);
        if let Some(audio) = self.playback_ctl.audio.as_mut() {
            audio.drop_leading_ticks(ticks);
        }
        let playback = &mut self.playback_ctl.playback;
        playback.smooth_tick = (playback.smooth_tick - ticks as f64).max(0.0);
        playback.heard_tick = (playback.heard_tick - ticks as f64).max(0.0);
        playback.scroll.shift(-playback.row_pitch);
        playback.rows_removed = true;
        playback.rows_above_view = playback.rows_above_view.saturating_sub(1);
        if let Some((m_idx, _)) = &mut playback.last_primary_beat {
            *m_idx = m_idx.saturating_sub(1);
        }
        if let Some((m_idx, _)) = &mut playback.guide_bar {
            *m_idx = m_idx.saturating_sub(1);
        }
        if let Some(anchor) = &mut playback.anchor {
            anchor.measure_idx = anchor.measure_idx.saturating_sub(1);
        }
        // In the same voice; on the dropped measure, from the start of the
        // one that took its place.
        let cursor = self.editor.doc().cursor;
        let moved = match cursor.measure_idx {
            0 => Cursor { beat_idx: 0, ..cursor },
            m => Cursor { measure_idx: m - 1, ..cursor },
        };
        let doc = self.editor.doc_mut();
        doc.cursor = doc.score.clamp_cursor(moved);
    }

    /// Re-anchor the accuracy session to the current playback position: its
    /// timeline assumes the timing it started with. Called for a
    /// [`ScoreChange`] and when the count-in bar ends.
    pub(crate) fn realign_accuracy_clock(&mut self) {
        if self.playback_ctl.transport_state != TransportState::Playing {
            return;
        }
        if !self.playback_ctl.accuracy.tracker.has_start_time() {
            return;
        }

        let timing = self.score_timing();
        if timing.total_loop_seconds() <= 0.0 {
            return;
        }

        // On the loop as it is now: a change that shortened it can leave
        // the cursor past its end until the next frame wraps it, which the
        // session would take for a wrap, judging the loop so far as missed.
        let tick =
            self.playback_ctl.playback.smooth_tick.rem_euclid(timing.total_loop_ticks() as f64);
        let start_time = clock::now_seconds() - timing.global_tick_to_seconds(tick);
        self.playback_ctl.accuracy.tracker.realign_start_time(start_time, tick);
    }
}

#[cfg(test)]
mod tests {
    use super::{Origin, ScoreChange};
    use crate::guide::AnchorFlash;
    use crate::shell::Tab;
    use crate::{Grooph, TransportState, clock};
    use eframe::egui;
    use grooph_measure::duration::q;
    use grooph_measure::swing::Swing;
    use grooph_measure::tempo::ScoreTiming;
    use grooph_measure::{Beat, BeatKind, Cursor, Measure, Score, TimeSignature};

    fn quarters(bars: &[TimeSignature]) -> Score {
        let mut score = Score::single(Measure::new_init(bars[0], BeatKind::Note));
        for &ts in &bars[1..] {
            score.push(Measure::new_init(ts, BeatKind::Note));
        }
        score
    }

    /// Practice with two bars of quarters, timing feedback on, a run's mark
    /// on its first note and the cursor on the second bar.
    fn practice_app() -> Grooph {
        let cc = crate::test_support::creation_context(egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Practice);
        app.playback_ctl.accuracy.enabled = true;
        let ts = TimeSignature::FOUR_FOUR;
        let doc = app.editor.doc_mut();
        doc.score = quarters(&[ts, ts]);
        doc.cursor = Cursor::at(1, 2);
        doc.dirty = false;
        doc.active_pattern_id = Some(7);
        let tracker = &mut app.playback_ctl.accuracy.tracker;
        tracker.on_playback_start_at(0.0, 0.0);
        let timing = app.score_timing();
        let score = app.played_score().clone();
        app.playback_ctl.accuracy.tracker.record_hit(0.0, &timing, &score);
        assert!(app.playback_ctl.accuracy.tracker.mark_for_onset(0).is_some());
        app
    }

    /// Playing, the cursor at `tick`.
    fn play_at(app: &mut Grooph, tick: f64) {
        app.playback_ctl.transport_state = TransportState::Playing;
        app.playback_ctl.playback.smooth_tick = tick;
    }

    /// The accuracy clock is anchored at the cursor, on the timing now.
    fn assert_anchored_at(app: &Grooph, tick: f64, before: f64) {
        let (start, last_tick) = app.playback_ctl.accuracy.tracker.anchor().unwrap();
        assert_eq!(last_tick, Some(tick));
        let seconds = app.score_timing().global_tick_to_seconds(tick);
        assert!(start >= before - seconds && start <= clock::now_seconds() - seconds, "{start}");
    }

    fn marks_cleared(app: &Grooph) -> bool {
        app.playback_ctl.accuracy.tracker.mark_for_onset(0).is_none()
            && app.playback_ctl.accuracy.tracker.live_summary().is_none()
    }

    #[test]
    fn an_edit_clears_the_marks_and_marks_the_score_unsaved() {
        let mut app = practice_app();
        app.with_undo_snapshot(|app| app.current_voice_mut().set_beat(0, Beat::rest(q())).is_ok());
        assert!(marks_cleared(&app));
        let doc = app.editor.doc();
        assert!(doc.dirty);
        assert_eq!(doc.active_pattern_id, Some(7));
        assert_eq!(doc.cursor, Cursor::at(1, 2));
        // Stopped: the clock is left as it is.
        assert_eq!(app.playback_ctl.accuracy.tracker.anchor().unwrap().1, None);
    }

    /// Fix: an edit while playing that changes the bar's length, e.g. a new
    /// time signature, re-anchors the accuracy clock, as a tempo change
    /// does. It kept the old one, and judged hits on the old bar lengths.
    #[test]
    fn an_edit_while_playing_anchors_the_accuracy_clock_again() {
        let mut app = practice_app();
        let tick = app.score_timing().measure_start_tick(1) as f64 + 10.0;
        play_at(&mut app, tick);
        let before = clock::now_seconds();
        app.with_undo_snapshot(|app| {
            app.editor.doc_mut().score =
                quarters(&[TimeSignature::new(3, 4), TimeSignature::new(3, 4)]);
            true
        });
        assert!(marks_cleared(&app));
        assert_anchored_at(&app, tick, before);
        // Undo is an edit, too.
        let before = clock::now_seconds();
        app.undo();
        assert_eq!(app.current_measure().time_signature(), TimeSignature::FOUR_FOUR);
        assert_anchored_at(&app, tick, before);
    }

    /// An edit that shortens the loop past the cursor anchors the clock on
    /// the cursor's place in the new loop, so the next frame is no wrap
    /// that judges the notes before it as missed.
    #[test]
    fn an_edit_that_shortens_the_loop_anchors_the_clock_inside_it() {
        let mut app = practice_app();
        let bar = app.score_timing().measure_start_tick(1) as f64;
        play_at(&mut app, bar + 10.0);
        let before = clock::now_seconds();
        app.with_undo_snapshot(|app| app.editor.doc_mut().score.remove(1).is_some());
        assert_anchored_at(&app, 10.0, before);
        let timing = app.score_timing();
        let score = app.played_score().clone();
        let tracker = &mut app.playback_ctl.accuracy.tracker;
        tracker.update_progress(clock::now_seconds(), &timing, &score);
        assert_eq!(tracker.live_summary().map_or(0, |s| s.misses), 0);
    }

    #[test]
    fn a_replaced_score_starts_at_its_first_beat_with_its_origins_flags() {
        let ts = TimeSignature::FOUR_FOUR;
        for (origin, pattern, dirty) in [
            (Origin::Pattern { id: 3, undoubled: false }, Some(3), false),
            (Origin::Pattern { id: 3, undoubled: true }, Some(3), true),
            (Origin::Link { undoubled: false }, None, false),
            (Origin::Link { undoubled: true }, None, true),
            (Origin::New, None, true),
        ] {
            let mut app = practice_app();
            app.playback_ctl.playback.smooth_tick = 12.0;
            app.playback_ctl.playback.rows_above_view = 2;
            app.replace_score(quarters(&[ts]), origin);
            let doc = app.editor.doc();
            assert_eq!(doc.score.len(), 1);
            assert_eq!(doc.cursor, Cursor::start());
            assert_eq!((doc.active_pattern_id, doc.dirty), (pattern, dirty), "{origin:?}");
            assert!(marks_cleared(&app));
            assert!(app.can_undo());
            // Stopped: the playback state starts over.
            assert_eq!(app.playback_ctl.playback.smooth_tick, 0.0);
            assert_eq!(app.playback_ctl.playback.rows_above_view, 0);
        }
    }

    /// Fix: a new rhythm of the generator while playing (a changed setting,
    /// a preset) plays on from the cursor, so the accuracy clock is anchored
    /// there again on the new rhythm's timing.
    #[test]
    fn a_new_rhythm_while_playing_plays_on_and_anchors_the_clock_again() {
        let mut app = practice_app();
        let tick = app.score_timing().measure_start_tick(1) as f64 + 10.0;
        play_at(&mut app, tick);
        let before = clock::now_seconds();
        app.replace_score(quarters(&[TimeSignature::new(5, 4)]), Origin::New);
        assert_eq!(app.playback_ctl.playback.smooth_tick, tick);
        assert!(marks_cleared(&app));
        assert_anchored_at(&app, tick, before);
    }

    #[test]
    fn an_appended_bar_anchors_the_clock_and_is_no_saved_pattern() {
        let mut app = practice_app();
        play_at(&mut app, 30.0);
        let before = clock::now_seconds();
        app.editor
            .doc_mut()
            .score
            .push(Measure::new_init(TimeSignature::FOUR_FOUR, BeatKind::Note));
        app.score_changed(ScoreChange::Appended);
        assert_anchored_at(&app, 30.0, before);
        // The run's marks stay.
        assert!(app.playback_ctl.accuracy.tracker.mark_for_onset(0).is_some());
        let doc = app.editor.doc();
        assert_eq!((doc.active_pattern_id, doc.dirty), (None, true));
        assert_eq!(doc.cursor, Cursor::at(1, 2));
    }

    #[test]
    fn a_dropped_first_bar_moves_everything_back_by_its_ticks() {
        let mut app = practice_app();
        let ticks = app.score_timing().measure_start_tick(1);
        // A mark on the second bar's first note, which becomes the first.
        let timing = app.score_timing();
        let score = app.played_score().clone();
        let at = timing.global_tick_to_seconds(ticks as f64);
        app.playback_ctl.accuracy.tracker.record_hit(at, &timing, &score);
        let tick = ticks as f64 + 10.0;
        play_at(&mut app, tick);
        let playback = &mut app.playback_ctl.playback;
        playback.heard_tick = tick;
        playback.rows_above_view = 1;
        playback.row_pitch = 50.0;
        playback.scroll.update(80.0, 0.0);
        playback.last_primary_beat = Some((1, 0));
        playback.guide_bar = Some((1, 10.0));
        playback.anchor = Some(AnchorFlash { bar: 4, measure_idx: 1, tick: 0, at: 0.0 });

        app.editor.doc_mut().score.remove(0);
        let before = clock::now_seconds();
        app.score_changed(ScoreChange::DroppedFront { ticks });

        let playback = &app.playback_ctl.playback;
        assert_eq!(playback.smooth_tick, 10.0);
        assert_eq!(playback.heard_tick, 10.0);
        assert_eq!(playback.scroll.value_at(0.0), Some(30.0));
        assert!(playback.rows_removed);
        assert_eq!(playback.rows_above_view, 0);
        assert_eq!(playback.last_primary_beat, Some((0, 0)));
        assert_eq!(playback.guide_bar, Some((0, 10.0)));
        assert_eq!(playback.anchor.map(|a| a.measure_idx), Some(0));
        let tracker = &app.playback_ctl.accuracy.tracker;
        assert!(tracker.mark_for_onset(0).is_some());
        assert!(tracker.mark_for_onset(ticks).is_none());
        assert_anchored_at(&app, 10.0, before);
        let doc = app.editor.doc();
        assert_eq!(doc.cursor, Cursor::at(0, 2));
        assert_eq!((doc.active_pattern_id, doc.dirty), (None, true));
    }

    #[test]
    fn a_tempo_change_anchors_the_clock_and_marks_the_shown_score_unsaved() {
        let mut app = practice_app();
        play_at(&mut app, 30.0);
        app.playback_ctl.bpm += 10;
        let before = clock::now_seconds();
        app.score_changed(ScoreChange::Tempo);
        assert_anchored_at(&app, 30.0, before);
        // The marks stay; Practice's rhythm keeps no tempo.
        assert!(app.playback_ctl.accuracy.tracker.mark_for_onset(0).is_some());
        assert!(!app.editor.doc().dirty);
        // The score tab's score does.
        app.select_tab(Tab::Score);
        app.editor.docs.score.dirty = false;
        app.score_changed(ScoreChange::Tempo);
        assert!(app.editor.docs.score.dirty);
    }

    #[test]
    fn a_timing_change_anchors_the_clock_and_saves_nothing() {
        let mut app = practice_app();
        play_at(&mut app, 30.0);
        app.playback_ctl.playback.ramp_bpm = Some(150);
        let before = clock::now_seconds();
        app.score_changed(ScoreChange::Timing);
        assert_anchored_at(&app, 30.0, before);
        assert!(app.playback_ctl.accuracy.tracker.mark_for_onset(0).is_some());
        assert!(!app.editor.doc().dirty);
        assert!(!app.editor.docs.score.dirty);
    }

    #[test]
    fn a_switched_document_starts_playback_and_marks_over() {
        let mut app = practice_app();
        app.playback_ctl.playback.smooth_tick = 30.0;
        app.select_tab(Tab::Metronome);
        assert_eq!(app.playback_ctl.playback.smooth_tick, 0.0);
        assert!(marks_cleared(&app));
    }

    /// Checked, no fix needed: swing moves notes inside their beat pair but
    /// leaves the clock's seconds and ticks as they are, so a swing change
    /// while playing needs no new anchor. (A ramp's step does, see
    /// `update_metronome_trainer`, which reports it as
    /// [`ScoreChange::Timing`].)
    #[test]
    fn swing_leaves_the_accuracy_clock_as_it_is() {
        let score = quarters(&[TimeSignature::FOUR_FOUR, TimeSignature::new(7, 8)]);
        let straight = ScoreTiming::from_score(&score, 120);
        let swung = straight.clone().with_swing(Swing { percent: 66, ..Swing::default() });
        assert!(!swung.swing().is_straight());
        for seconds in [0.0, 0.3, 1.25, 2.9, 3.7] {
            assert_eq!(
                straight.seconds_to_global_tick(seconds),
                swung.seconds_to_global_tick(seconds)
            );
        }
        assert_eq!(straight.total_loop_seconds(), swung.total_loop_seconds());
    }
}
