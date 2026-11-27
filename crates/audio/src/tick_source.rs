//! Sample-accurate playback cursor over the score loop.
//!
//! A [`TickSource`] advances a `cursor` (measured in global ticks of the
//! whole score) by one audio sample at a time and collects any
//! [`SoundType`] triggers that the cursor crossed during that step. It owns
//! *only* the timing state — no voices, no synthesis, no rodio types — and
//! pulls per-measure tempo from a [`ScoreTiming`] so per-measure tempo
//! changes work without drift accumulation.

use crate::schedule::{Schedule, SoundType};
use grooph_measure::tempo::ScoreTiming;
use std::sync::Arc;

/// One bar of clicks played before the score starts. Ticks are in the tick
/// scale of the loop's first measure and run at its tempo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CountInPlan {
    pub(crate) ticks: u32,
    /// Click positions within `0..ticks` (every beat unit) and
    /// their sound ([`SoundType::CountIn`] or [`SoundType::CountInSoft`]).
    pub(crate) clicks: Vec<(u32, SoundType)>,
}

struct CountInProgress {
    /// Shared with the playback plan, so starting a count-in on the audio
    /// thread does not copy it.
    plan: Arc<CountInPlan>,
    pos: f64,
}

pub(crate) struct TickSource {
    cursor: f64,
    sample_rate: u32,
    count_in: Option<CountInProgress>,
    /// Bars started since the loop's first downbeat, across loops.
    bars: u64,
}

impl TickSource {
    pub(crate) fn new(sample_rate: u32) -> Self {
        Self { cursor: 0.0, sample_rate, count_in: None, bars: 0 }
    }

    /// Cursor for display: global score tick, or a negative tick counting up
    /// to 0 while the count-in bar plays.
    pub(crate) fn cursor(&self) -> f64 {
        match &self.count_in {
            Some(ci) => ci.pos - ci.plan.ticks as f64,
            None => self.cursor,
        }
    }

    /// Bars started since playback started at the loop's first downbeat
    /// (0 = first bar). Keeps counting across loops: the bars played.
    pub(crate) fn bars(&self) -> u64 {
        self.bars
    }

    pub(crate) fn in_count_in(&self) -> bool {
        self.count_in.is_some()
    }

    /// Move the cursor back by `ticks` after as many were dropped from the
    /// front of the score. The bar count keeps going.
    pub(crate) fn shift_back(&mut self, ticks: f64) {
        self.cursor = (self.cursor - ticks).max(0.0);
    }

    /// Start playback from the top of the loop (`timing`'s
    /// [`loop_start_tick`](ScoreTiming::loop_start_tick)), optionally after
    /// a count-in.
    pub(crate) fn restart(&mut self, timing: &ScoreTiming, count_in: Option<&Arc<CountInPlan>>) {
        self.cursor = Self::loop_start(timing);
        self.bars = 0;
        self.count_in = count_in
            .filter(|p| p.ticks > 0)
            .map(|plan| CountInProgress { plan: Arc::clone(plan), pos: 0.0 });
    }

    /// The loop's first tick, or 0 for an empty timing.
    fn loop_start(timing: &ScoreTiming) -> f64 {
        if timing.measure_count() == 0 { 0.0 } else { timing.loop_start_tick() as f64 }
    }

    /// Advance the cursor by one audio sample using the per-measure tempo at
    /// the current cursor position, and append every schedule trigger
    /// crossed during this step to `out`.
    ///
    /// At each barline the next bar is the one after (bar 0 after the
    /// score's last) if it is in `timing`'s loop, else the loop's first
    /// ([`BarRange::next_bar`](grooph_measure::BarRange::next_bar)), so the
    /// loop repeats indefinitely and a new loop takes over at the next
    /// barline. `tempo_scale` speeds the score up (> 1) or slows it down
    /// relative to `timing`; it does not apply to the count-in.
    pub(crate) fn advance_one_sample(
        &mut self,
        timing: &ScoreTiming,
        schedule: &Schedule,
        tempo_scale: f64,
        out: &mut Vec<SoundType>,
    ) {
        if timing.score_ticks() == 0 {
            return;
        }
        let loop_start = Self::loop_start(timing);

        if let Some(ci) = &mut self.count_in {
            let first = timing.loop_bars().first();
            let step = timing.ticks_per_sec_in_measure(first) / self.sample_rate as f64;
            let old_pos = ci.pos;
            let new_pos = old_pos + step;
            let end = ci.plan.ticks as f64;
            for &(c, sound) in &ci.plan.clicks {
                let c = c as f64;
                if c >= old_pos.ceil() && c < new_pos.min(end) {
                    out.push(sound);
                }
            }
            if new_pos < end {
                ci.pos = new_pos;
                return;
            }
            // Count-in finished within this sample: carry the remainder into
            // the score so the loop's first downbeat lands exactly one bar
            // later.
            self.count_in = None;
            let leftover = new_pos - end;
            schedule.collect_in_range(loop_start as u64, loop_start + leftover, out);
            self.cursor = loop_start + leftover;
            return;
        }

        let old_cursor = self.cursor;
        // Pick the per-measure tempo at the current cursor position. Within one
        // sample, the rate of the old measure is reused; the next sample picks
        // up the new measure's rate. Max drift at a TS boundary is ~20 µs at
        // 48 kHz and does not accumulate — schedule triggers are exact integer
        // ticks and are picked up by `collect_in_range` precisely.
        let measure_idx = timing.measure_at_global_tick(old_cursor);
        let tps = timing.ticks_per_sec_in_measure(measure_idx) * tempo_scale;
        let ticks_per_sample = tps / self.sample_rate as f64;

        let mut new_cursor = old_cursor + ticks_per_sample;
        let bar_end = timing.measure_start_tick(measure_idx + 1) as f64;

        if new_cursor >= bar_end {
            // 1. [old_cursor, bar_end)
            schedule.collect_in_range(old_cursor.ceil() as u64, bar_end, out);
            // On into the next bar of the loop, which may lie elsewhere.
            let next = timing.loop_bars().next_bar(measure_idx, timing.measure_count());
            let next_start = timing.measure_start_tick(next) as f64;
            new_cursor = next_start + (new_cursor - bar_end);
            // 2. [next_start, new_cursor)
            schedule.collect_in_range(next_start as u64, new_cursor, out);
            self.bars += 1;
        } else {
            // [old_cursor, new_cursor)
            schedule.collect_in_range(old_cursor.ceil() as u64, new_cursor, out);
        }
        self.cursor = new_cursor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::click::ClickBars;
    use grooph_measure::duration::q;
    use grooph_measure::{BarRange, Beat, Measure, Score, TimeSignature};

    fn score_4_4_quarters() -> Score {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        for i in 0..4 {
            m.first_voice_mut().set_beat(i, Beat::note(q())).unwrap();
        }
        Score::single(m)
    }

    #[test]
    fn empty_timing_yields_no_triggers_and_no_advance() {
        let timing = ScoreTiming::default();
        let schedule = Schedule::default();
        let mut src = TickSource::new(48000);
        let mut out = Vec::new();
        src.advance_one_sample(&timing, &schedule, 1.0, &mut out);
        assert!(out.is_empty());
        assert_eq!(src.cursor(), 0.0);
    }

    #[test]
    fn first_sample_picks_up_downbeat() {
        let score = score_4_4_quarters();
        let timing = ScoreTiming::from_score(&score, 120);
        let schedule = Schedule::build(&score, &timing, None, Some(&ClickBars::default()));
        let mut src = TickSource::new(48000);
        let mut out = Vec::new();
        src.advance_one_sample(&timing, &schedule, 1.0, &mut out);
        // Downbeat at tick 0 lies in [ceil(0), new_cursor) once new_cursor > 0.
        assert!(out.contains(&SoundType::Downbeat));
        assert!(src.cursor() > 0.0);
    }

    #[test]
    fn cursor_stays_within_loop_and_score_repeats() {
        let score = score_4_4_quarters();
        let timing = ScoreTiming::from_score(&score, 120);
        let schedule = Schedule::build(&score, &timing, None, Some(&ClickBars::default()));
        let total = timing.total_loop_ticks() as f64;
        let mut src = TickSource::new(48000);
        let mut out = Vec::new();
        let mut downbeats = 0usize;
        // 3 s of playback at 120 bpm in 4/4 → score loop (2 s) wraps once.
        for _ in 0..(48_000 * 3) {
            src.advance_one_sample(&timing, &schedule, 1.0, &mut out);
            downbeats += out.iter().filter(|s| **s == SoundType::Downbeat).count();
            out.clear();
            assert!(src.cursor() < total, "cursor escaped loop: {}", src.cursor());
        }
        assert!(downbeats >= 2, "expected score to loop; got {downbeats} downbeats");
    }

    fn count_in_4_4() -> Arc<CountInPlan> {
        let tpb = grooph_measure::grid::DEFAULT_GRID.ticks_per_beat(&TimeSignature::FOUR_FOUR);
        let clicks = (0..4).map(|i| (i * tpb, SoundType::CountIn)).collect();
        Arc::new(CountInPlan { ticks: 4 * tpb, clicks })
    }

    #[test]
    fn count_in_plays_one_bar_before_the_first_downbeat() {
        let score = score_4_4_quarters();
        let timing = ScoreTiming::from_score(&score, 120);
        let schedule = Schedule::build(&score, &timing, None, Some(&ClickBars::default()));
        let mut src = TickSource::new(48000);
        src.restart(&timing, Some(&count_in_4_4()));
        let mut out = Vec::new();
        let mut count_in_clicks = Vec::new();
        let mut first_downbeat = None;
        // 120 bpm 4/4: count-in bar lasts 2 s; run 2.5 s.
        for sample in 0..(48_000 * 5 / 2) {
            src.advance_one_sample(&timing, &schedule, 1.0, &mut out);
            for s in out.drain(..) {
                match s {
                    SoundType::CountIn => count_in_clicks.push(sample),
                    SoundType::Downbeat if first_downbeat.is_none() => {
                        first_downbeat = Some(sample)
                    }
                    _ => {}
                }
            }
            if sample < 48_000 * 2 - 1 {
                assert!(src.cursor() < 0.0, "cursor must be negative during count-in");
            }
        }
        assert_eq!(count_in_clicks.len(), 4);
        // Clicks every half second, downbeat after two seconds (±1 sample).
        for (i, s) in count_in_clicks.iter().enumerate() {
            assert!((*s as i64 - i as i64 * 24_000).abs() <= 1, "click {i} at sample {s}");
        }
        let db = first_downbeat.expect("score must start after count-in") as i64;
        assert!((db - 96_000).abs() <= 1, "first downbeat at sample {db}");
    }

    #[test]
    fn restart_without_count_in_starts_at_zero() {
        let score = score_4_4_quarters();
        let timing = ScoreTiming::from_score(&score, 120);
        let schedule = Schedule::build(&score, &timing, None, Some(&ClickBars::default()));
        let mut src = TickSource::new(48000);
        let mut out = Vec::new();
        for _ in 0..10_000 {
            src.advance_one_sample(&timing, &schedule, 1.0, &mut out);
        }
        src.restart(&timing, None);
        assert_eq!(src.cursor(), 0.0);
    }

    #[test]
    fn bars_count_measures_across_loops() {
        let mut m2 = Measure::new(TimeSignature::FOUR_FOUR);
        m2.first_voice_mut().set_beat(0, Beat::note(q())).unwrap();
        let mut score = score_4_4_quarters();
        score.push(m2);
        let timing = ScoreTiming::from_score(&score, 120);
        let schedule = Schedule::build(&score, &timing, None, Some(&ClickBars::default()));
        let mut src = TickSource::new(48000);
        let mut out = Vec::new();
        // 120 bpm 4/4: a bar lasts 2 s. After 5 s the third bar (second loop) plays.
        for _ in 0..(48_000 * 5) {
            src.advance_one_sample(&timing, &schedule, 1.0, &mut out);
        }
        assert_eq!(src.bars(), 2);
        src.restart(&timing, None);
        assert_eq!(src.bars(), 0);
    }

    #[test]
    fn shift_back_keeps_the_position_in_a_shortened_score() {
        let mut score = score_4_4_quarters();
        score.push(score.measures()[0].clone());
        let timing = ScoreTiming::from_score(&score, 120);
        let schedule = Schedule::build(&score, &timing, None, Some(&ClickBars::default()));
        let bar = timing.ticks_per_measure(0) as f64;
        let mut src = TickSource::new(48000);
        let mut out = Vec::new();
        // 3 s in: halfway through the second bar.
        for _ in 0..(48_000 * 3) {
            src.advance_one_sample(&timing, &schedule, 1.0, &mut out);
        }
        let before = src.cursor();
        src.shift_back(bar);
        assert!((src.cursor() - (before - bar)).abs() < 1e-9);
        assert_eq!(src.bars(), 1);
    }

    #[test]
    fn tempo_scale_speeds_up_the_cursor() {
        let score = score_4_4_quarters();
        let timing = ScoreTiming::from_score(&score, 120);
        let schedule = Schedule::build(&score, &timing, None, Some(&ClickBars::default()));
        let mut normal = TickSource::new(48000);
        let mut fast = TickSource::new(48000);
        let mut out = Vec::new();
        for _ in 0..24_000 {
            normal.advance_one_sample(&timing, &schedule, 1.0, &mut out);
            fast.advance_one_sample(&timing, &schedule, 1.5, &mut out);
        }
        assert!((fast.cursor() / normal.cursor() - 1.5).abs() < 1e-6);
    }

    /// `n` bars of 4/4 quarters.
    fn bars_of_quarters(n: usize) -> Score {
        let bar = score_4_4_quarters().measures()[0].clone();
        Score::new(vec![bar; n])
    }

    /// The bars (counted from 1) `src` plays over `seconds` at 120 BPM,
    /// with `timing`, each once as it starts; the first is the bar it is in.
    fn bars_played(src: &mut TickSource, timing: &ScoreTiming, seconds: f64) -> Vec<usize> {
        let schedule = Schedule::default();
        let mut out = Vec::new();
        let mut bars = vec![timing.measure_at_global_tick(src.cursor()) + 1];
        for _ in 0..(48_000.0 * seconds) as usize {
            src.advance_one_sample(timing, &schedule, 1.0, &mut out);
            let bar = timing.measure_at_global_tick(src.cursor()) + 1;
            if src.cursor() >= 0.0 && bars.last() != Some(&bar) {
                bars.push(bar);
            }
            assert!(timing.loop_bars().contains(bar - 1) || bars.len() <= 2, "{bars:?}");
        }
        bars
    }

    /// Bars `a..=b` counted from 1.
    fn range(a: usize, b: usize) -> Option<BarRange> {
        BarRange::new(a - 1, b - 1)
    }

    #[test]
    fn a_loop_range_starts_at_its_first_bar_and_wraps_after_its_last() {
        let score = bars_of_quarters(6);
        let timing = ScoreTiming::from_score(&score, 120).with_loop(range(3, 5));
        let mut src = TickSource::new(48000);
        src.restart(&timing, None);
        assert_eq!(src.cursor(), timing.measure_start_tick(2) as f64);
        // Bars of 2 s: 3 4 5 3 4 5 3 in 13 s, never outside the range.
        assert_eq!(bars_played(&mut src, &timing, 13.0), [3, 4, 5, 3, 4, 5, 3]);
        assert_eq!(src.bars(), 6);
    }

    #[test]
    fn a_count_in_leads_into_the_first_bar_of_the_range() {
        let score = bars_of_quarters(4);
        let timing = ScoreTiming::from_score(&score, 120).with_loop(range(2, 3));
        let schedule = Schedule::build(&score, &timing, None, Some(&ClickBars::default()));
        let mut src = TickSource::new(48000);
        src.restart(&timing, Some(&count_in_4_4()));
        let mut out = Vec::new();
        for _ in 0..(48_000 * 2 + 10) {
            src.advance_one_sample(&timing, &schedule, 1.0, &mut out);
        }
        assert!(!src.in_count_in());
        let start = timing.measure_start_tick(1) as f64;
        assert!(src.cursor() > start && src.cursor() < start + 100.0, "{}", src.cursor());
        assert_eq!(src.bars(), 0);
    }

    #[test]
    fn a_new_range_takes_over_at_the_next_barline() {
        let score = bars_of_quarters(8);
        let whole = ScoreTiming::from_score(&score, 120);
        let old = whole.clone().with_loop(range(3, 6));
        // Halfway through bar 5 of 3..=6.
        let in_bar_5 = || {
            let mut src = TickSource::new(48000);
            src.restart(&old, None);
            bars_played(&mut src, &old, 5.0);
            assert_eq!(old.measure_at_global_tick(src.cursor()), 4);
            src
        };
        let after = |new: &ScoreTiming| bars_played(&mut in_bar_5(), new, 8.0);
        // The end dragged to bar 8: on into it, then back to 3.
        assert_eq!(after(&whole.clone().with_loop(range(3, 8))), [5, 6, 7, 8, 3]);
        // The end dragged onto bar 5: back to 3 after it.
        assert_eq!(after(&whole.clone().with_loop(range(3, 5))), [5, 3, 4, 5, 3]);
        // Loop off: on to the score's end, then bar 1.
        assert_eq!(after(&whole), [5, 6, 7, 8, 1]);
        // A range ahead: the bar plays to its end, then its first bar.
        assert_eq!(after(&whole.clone().with_loop(range(7, 8))), [5, 7, 8, 7, 8]);
        // No count-in at the switch, and the bars keep counting.
        let mut src = in_bar_5();
        bars_played(&mut src, &whole, 2.0);
        assert!(!src.in_count_in());
        assert_eq!(src.bars(), 3);
    }
}
