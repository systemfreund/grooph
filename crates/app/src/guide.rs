//! What the practice staff shows while it plays: the moving playback line,
//! or only short flashes on some beats ("anchors"), so the player has to
//! read and count the rest themselves.

use crate::Grooph;
use grooph_measure::TimeSignature;
use grooph_measure::grid::DEFAULT_GRID;
use serde::{Deserialize, Serialize};

/// How long an anchor takes to fade out (s).
pub(crate) const ANCHOR_FADE_S: f64 = 0.2;

/// The support while playing, from the most (the line) to none at all.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Guide {
    Line,
    #[default]
    EveryBeat,
    EverySecondBeat,
    EveryBar,
    EverySecondBar,
    None,
}

impl Guide {
    pub(crate) const ALL: [Guide; 6] = [
        Self::Line,
        Self::EveryBeat,
        Self::EverySecondBeat,
        Self::EveryBar,
        Self::EverySecondBar,
        Self::None,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Line => "Line",
            Self::EveryBeat => "Every beat",
            Self::EverySecondBeat => "Every second beat",
            Self::EveryBar => "Every bar",
            Self::EverySecondBar => "Every second bar",
            Self::None => "None",
        }
    }

    /// The step as it works in `ts`: with only two beats a bar, every second
    /// beat is beat 1 of every bar.
    pub(crate) fn in_meter(self, ts: &TimeSignature) -> Guide {
        if self == Self::EverySecondBeat && beat_ticks(ts).len() <= 2 {
            Self::EveryBar
        } else {
            self
        }
    }

    /// Whether the step means something of its own in `ts`.
    pub(crate) fn offered_in(self, ts: &TimeSignature) -> bool {
        self.in_meter(ts) == self
    }

    /// Ticks of the anchors in the `bar`-th bar played (0 = the first), a
    /// bar of `ts`.
    pub(crate) fn anchor_ticks(self, ts: &TimeSignature, bar: u64) -> Vec<u32> {
        let beats = beat_ticks(ts);
        match self.in_meter(ts) {
            Self::Line | Self::None => Vec::new(),
            Self::EveryBeat => beats,
            Self::EverySecondBeat => beats.into_iter().step_by(2).collect(),
            Self::EveryBar => vec![0],
            Self::EverySecondBar if bar.is_multiple_of(2) => vec![0],
            Self::EverySecondBar => Vec::new(),
        }
    }

    /// The last anchor at or before `local_tick` in the `bar`-th bar played.
    pub(crate) fn anchor_at(self, ts: &TimeSignature, bar: u64, local_tick: f64) -> Option<u32> {
        self.anchor_ticks(ts, bar).into_iter().take_while(|&t| t as f64 <= local_tick).last()
    }
}

/// The anchor last reached while playing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AnchorFlash {
    /// Bars played before it (see [`crate::state::PlaybackState::bars_played`]).
    pub(crate) bar: u64,
    pub(crate) measure_idx: usize,
    /// Local tick of its beat.
    pub(crate) tick: u32,
    /// When it was reached (s).
    pub(crate) at: f64,
}

impl Grooph {
    /// Meter the guide picker offers its steps for: the first bar's.
    pub(crate) fn guide_meter(&self) -> TimeSignature {
        self.editor.score.measures.first().map_or(TimeSignature::FOUR_FOUR, |m| m.time_signature())
    }

    /// The guide in use: the metronome tab and calibration keep the line.
    pub(crate) fn active_guide(&self) -> Guide {
        if self.metronome_mode() || self.calibration_source().is_some() {
            Guide::Line
        } else {
            self.ui.guide
        }
    }

    /// Count the bars played and note the anchor `playing` (measure, local
    /// tick at its written position) has reached. Nothing during the
    /// count-in: the first anchor is the score's first downbeat.
    pub(crate) fn update_guide_anchor(&mut self, playing: Option<(usize, f64)>, now: f64) {
        let guide = self.active_guide();
        let playback = &mut self.playback_ctl.playback;
        let Some((measure_idx, tick)) = playing.filter(|_| !playback.counting_in()) else {
            return;
        };
        let Some(ts) = self.editor.score.measures.get(measure_idx).map(|m| m.time_signature())
        else {
            return;
        };
        // A new bar: another measure, or the same one again after a loop
        // (the tick jumps back; small nudges toward the audio don't count).
        let half_bar = f64::from(DEFAULT_GRID.ticks_per_measure(&ts)) / 2.0;
        match playback.guide_bar {
            Some((idx, last)) if idx == measure_idx && last - tick < half_bar => {}
            Some(_) => playback.bars_played += 1,
            None => {}
        }
        playback.guide_bar = Some((measure_idx, tick));

        let bar = playback.bars_played;
        if let Some(beat) = guide.anchor_at(&ts, bar, tick)
            && playback.anchor.is_none_or(|a| (a.bar, a.tick) != (bar, beat))
        {
            playback.anchor = Some(AnchorFlash { bar, measure_idx, tick: beat, at: now });
        }
    }
}

/// Starts of the primary beats of `ts`: the metronome's click groups, e.g.
/// the dotted quarters of 6/8.
fn beat_ticks(ts: &TimeSignature) -> Vec<u32> {
    let mut ticks = vec![0];
    ticks.extend(DEFAULT_GRID.primary_boundaries(ts));
    ticks
}

/// Brightness of an anchor `elapsed` seconds after its beat: full at once,
/// then fading out.
pub(crate) fn anchor_intensity(elapsed: f64) -> f32 {
    if (0.0..ANCHOR_FADE_S).contains(&elapsed) {
        (1.0 - elapsed / ANCHOR_FADE_S) as f32
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use grooph_measure::duration::{Duration, e, q};

    /// `ticks` counted in `unit`s from 1, e.g. the eighths 1 and 4 of 6/8.
    fn counted(ticks: Vec<u32>, unit: Duration) -> Vec<u32> {
        let unit = DEFAULT_GRID.ticks_of(&unit).unwrap();
        ticks.into_iter().map(|t| t / unit + 1).collect()
    }

    fn eighths(ticks: Vec<u32>) -> Vec<u32> {
        counted(ticks, e())
    }

    fn quarters(ticks: Vec<u32>) -> Vec<u32> {
        counted(ticks, q())
    }

    #[test]
    fn every_beat_marks_the_primary_beats() {
        let four = TimeSignature::FOUR_FOUR;
        assert_eq!(quarters(Guide::EveryBeat.anchor_ticks(&four, 0)), [1, 2, 3, 4]);
        let six = TimeSignature::SIX_EIGHT;
        assert_eq!(eighths(Guide::EveryBeat.anchor_ticks(&six, 0)), [1, 4]);
        let five = TimeSignature::new(5, 8);
        assert_eq!(eighths(Guide::EveryBeat.anchor_ticks(&five, 0)), [1, 4]);
        let five_two_three = five.with_grouping(1);
        assert_eq!(eighths(Guide::EveryBeat.anchor_ticks(&five_two_three, 0)), [1, 3]);
    }

    #[test]
    fn every_second_beat_starts_again_in_every_bar() {
        let three = TimeSignature::THREE_FOUR;
        for bar in 0..3 {
            assert_eq!(quarters(Guide::EverySecondBeat.anchor_ticks(&three, bar)), [1, 3]);
        }
        let four = TimeSignature::FOUR_FOUR;
        assert_eq!(quarters(Guide::EverySecondBeat.anchor_ticks(&four, 1)), [1, 3]);
    }

    #[test]
    fn every_second_beat_is_every_bar_with_two_beats() {
        for ts in [TimeSignature::new(2, 4), TimeSignature::SIX_EIGHT, TimeSignature::new(5, 8)] {
            assert_eq!(Guide::EverySecondBeat.in_meter(&ts), Guide::EveryBar);
            assert!(!Guide::EverySecondBeat.offered_in(&ts));
            assert_eq!(Guide::EverySecondBeat.anchor_ticks(&ts, 0), [0]);
        }
        assert!(Guide::EverySecondBeat.offered_in(&TimeSignature::THREE_FOUR));
    }

    #[test]
    fn bar_steps_mark_beat_one() {
        let four = TimeSignature::FOUR_FOUR;
        for bar in 0..4 {
            assert_eq!(Guide::EveryBar.anchor_ticks(&four, bar), [0]);
        }
        let second: Vec<_> =
            (0..5).map(|bar| Guide::EverySecondBar.anchor_ticks(&four, bar)).collect();
        assert_eq!(second, [vec![0], vec![], vec![0], vec![], vec![0]]);
    }

    #[test]
    fn line_and_none_have_no_anchors() {
        let four = TimeSignature::FOUR_FOUR;
        assert!(Guide::Line.anchor_ticks(&four, 0).is_empty());
        assert!(Guide::None.anchor_ticks(&four, 0).is_empty());
    }

    #[test]
    fn anchor_at_is_the_last_beat_reached() {
        let four = TimeSignature::FOUR_FOUR;
        let quarter = DEFAULT_GRID.ticks_of(&q()).unwrap() as f64;
        assert_eq!(Guide::EveryBeat.anchor_at(&four, 0, 0.0), Some(0));
        assert_eq!(Guide::EveryBeat.anchor_at(&four, 0, 2.5 * quarter), Some(2 * quarter as u32));
        assert_eq!(Guide::EveryBar.anchor_at(&four, 0, 3.9 * quarter), Some(0));
        assert_eq!(Guide::EverySecondBar.anchor_at(&four, 1, 0.0), None);
    }

    #[test]
    fn anchors_fade_out() {
        assert_eq!(anchor_intensity(0.0), 1.0);
        assert!(anchor_intensity(0.1) > 0.0 && anchor_intensity(0.1) < 1.0);
        assert_eq!(anchor_intensity(ANCHOR_FADE_S), 0.0);
        assert_eq!(anchor_intensity(-0.1), 0.0);
    }

    fn practice_app(bars: usize, guide: Guide) -> Grooph {
        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_compact_tab(crate::compact::CompactTab::Practice);
        let measure = Grooph::default_measure();
        app.editor.score.measures = vec![measure; bars];
        app.ui.guide = guide;
        app
    }

    fn quarter() -> f64 {
        DEFAULT_GRID.ticks_of(&q()).unwrap() as f64
    }

    fn anchor(app: &Grooph) -> Option<(u64, usize, u32)> {
        app.playback_ctl.playback.anchor.map(|a| (a.bar, a.measure_idx, a.tick))
    }

    #[test]
    fn every_second_bar_counts_the_bars_played_across_loops() {
        let mut app = practice_app(3, Guide::EverySecondBar);
        app.update_guide_anchor(Some((0, 0.0)), 0.0);
        assert_eq!(anchor(&app), Some((0, 0, 0)));
        app.update_guide_anchor(Some((1, 0.0)), 1.0);
        assert_eq!(anchor(&app), Some((0, 0, 0)));
        app.update_guide_anchor(Some((2, 0.0)), 2.0);
        assert_eq!(anchor(&app), Some((2, 2, 0)));
        // The loop starts over: bar 3 has no anchor, bar 4 has.
        app.update_guide_anchor(Some((0, 0.0)), 3.0);
        assert_eq!(anchor(&app), Some((2, 2, 0)));
        app.update_guide_anchor(Some((1, 0.0)), 4.0);
        assert_eq!(anchor(&app), Some((4, 1, 0)));
    }

    #[test]
    fn a_one_bar_loop_fires_its_anchor_every_time() {
        let mut app = practice_app(1, Guide::EveryBar);
        app.update_guide_anchor(Some((0, 0.0)), 0.0);
        app.update_guide_anchor(Some((0, 3.5 * quarter())), 1.0);
        assert_eq!(anchor(&app), Some((0, 0, 0)));
        app.update_guide_anchor(Some((0, 0.1)), 2.0);
        assert_eq!(app.playback_ctl.playback.anchor.map(|a| (a.bar, a.at)), Some((1, 2.0)));
    }

    #[test]
    fn a_small_step_back_is_not_a_new_bar() {
        let mut app = practice_app(1, Guide::EveryBeat);
        app.update_guide_anchor(Some((0, 2.0 * quarter())), 0.0);
        app.update_guide_anchor(Some((0, 1.9 * quarter())), 0.1);
        assert_eq!(app.playback_ctl.playback.bars_played, 0);
    }

    #[test]
    fn nothing_flashes_during_the_count_in() {
        let mut app = practice_app(1, Guide::EveryBeat);
        app.playback_ctl.playback.count_in = crate::state::CountInState::Active;
        app.update_guide_anchor(Some((0, 0.0)), 0.0);
        assert_eq!(anchor(&app), None);
        app.playback_ctl.playback.count_in = crate::state::CountInState::Off;
        app.update_guide_anchor(Some((0, 0.0)), 1.0);
        assert_eq!(anchor(&app), Some((0, 0, 0)));
    }

    #[test]
    fn the_metronome_keeps_the_line() {
        let mut app = practice_app(1, Guide::None);
        assert_eq!(app.active_guide(), Guide::None);
        app.open_metronome();
        assert_eq!(app.active_guide(), Guide::Line);
    }

    #[test]
    fn the_default_is_every_beat() {
        assert_eq!(Guide::default(), Guide::EveryBeat);
    }
}
