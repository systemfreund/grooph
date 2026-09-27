//! Sample-accurate playback cursor over the score loop.
//!
//! A [`TickSource`] advances a `cursor` (measured in global ticks across the
//! whole score loop) by one audio sample at a time and collects any
//! [`SoundType`] triggers that the cursor crossed during that step. It owns
//! *only* the timing state — no voices, no synthesis, no rodio types — and
//! pulls per-measure tempo from a [`ScoreTiming`] so per-measure tempo
//! changes work without drift accumulation.

use crate::schedule::{Schedule, SoundType};
use grooph_measure::tempo::ScoreTiming;

/// One bar of clicks played before the score starts. Ticks are in the first
/// measure's tick scale and run at its tempo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CountInPlan {
    pub(crate) ticks: u32,
    /// Click positions within `0..ticks` (downbeat plus primary beats).
    pub(crate) clicks: Vec<u32>,
}

struct CountInProgress {
    plan: CountInPlan,
    pos: f64,
}

pub(crate) struct TickSource {
    cursor: f64,
    sample_rate: u32,
    count_in: Option<CountInProgress>,
}

impl TickSource {
    pub(crate) fn new(sample_rate: u32) -> Self {
        Self { cursor: 0.0, sample_rate, count_in: None }
    }

    /// Cursor for display: global score tick, or a negative tick counting up
    /// to 0 while the count-in bar plays.
    pub(crate) fn cursor(&self) -> f64 {
        match &self.count_in {
            Some(ci) => ci.pos - ci.plan.ticks as f64,
            None => self.cursor,
        }
    }

    /// Start playback from the top of the score, optionally after a count-in.
    pub(crate) fn restart(&mut self, count_in: Option<&CountInPlan>) {
        self.cursor = 0.0;
        self.count_in = count_in
            .filter(|p| p.ticks > 0)
            .map(|plan| CountInProgress { plan: plan.clone(), pos: 0.0 });
    }

    /// Advance the cursor by one audio sample using the per-measure tempo at
    /// the current cursor position, and append every schedule trigger
    /// crossed during this step to `out`.
    ///
    /// Wraps the cursor at the end of the score loop so the metronome loops
    /// indefinitely.
    pub(crate) fn advance_one_sample(
        &mut self,
        timing: &ScoreTiming,
        schedule: &Schedule,
        out: &mut Vec<SoundType>,
    ) {
        let total_ticks = timing.total_loop_ticks() as f64;
        if total_ticks <= 0.0 {
            return;
        }

        if let Some(ci) = &mut self.count_in {
            let step = timing.ticks_per_sec_in_measure(0) / self.sample_rate as f64;
            let old_pos = ci.pos;
            let new_pos = old_pos + step;
            let end = ci.plan.ticks as f64;
            for &c in &ci.plan.clicks {
                let c = c as f64;
                if c >= old_pos.ceil() && c < new_pos.min(end) {
                    out.push(SoundType::CountIn);
                }
            }
            if new_pos < end {
                ci.pos = new_pos;
                return;
            }
            // Count-in finished within this sample: carry the remainder into
            // the score so the first downbeat lands exactly one bar later.
            self.count_in = None;
            self.cursor = 0.0;
            let leftover = new_pos - end;
            schedule.collect_in_range(0, leftover, out);
            self.cursor = leftover;
            return;
        }

        let old_cursor = self.cursor;
        // Pick the per-measure tempo at the current cursor position. Within one
        // sample, the rate of the old measure is reused; the next sample picks
        // up the new measure's rate. Max drift at a TS boundary is ~20 µs at
        // 48 kHz and does not accumulate — schedule triggers are exact integer
        // ticks and are picked up by `collect_in_range` precisely.
        let measure_idx = timing.measure_at_global_tick(old_cursor);
        let tps = timing.ticks_per_sec_in_measure(measure_idx);
        let ticks_per_sample = tps / self.sample_rate as f64;

        let mut new_cursor = old_cursor + ticks_per_sample;

        if new_cursor >= total_ticks {
            // 1. [old_cursor, total_ticks)
            schedule.collect_in_range(old_cursor.ceil() as u64, total_ticks, out);

            // Wrap at score end.
            new_cursor -= total_ticks;

            // 2. [0.0, new_cursor)
            schedule.collect_in_range(0, new_cursor, out);
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
    use grooph_measure::duration::q;
    use grooph_measure::{Beat, Measure, Score, TimeSignature};

    fn score_4_4_quarters() -> Score {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        for i in 0..4 {
            m.set_beat(i, Beat::note(q())).unwrap();
        }
        Score::single(m)
    }

    #[test]
    fn empty_timing_yields_no_triggers_and_no_advance() {
        let timing = ScoreTiming::default();
        let schedule = Schedule::default();
        let mut src = TickSource::new(48000);
        let mut out = Vec::new();
        src.advance_one_sample(&timing, &schedule, &mut out);
        assert!(out.is_empty());
        assert_eq!(src.cursor(), 0.0);
    }

    #[test]
    fn first_sample_picks_up_downbeat() {
        let score = score_4_4_quarters();
        let timing = ScoreTiming::from_score(&score, 120);
        let schedule = Schedule::build(&score, &timing, None);
        let mut src = TickSource::new(48000);
        let mut out = Vec::new();
        src.advance_one_sample(&timing, &schedule, &mut out);
        // Downbeat at tick 0 lies in [ceil(0), new_cursor) once new_cursor > 0.
        assert!(out.contains(&SoundType::Downbeat));
        assert!(src.cursor() > 0.0);
    }

    #[test]
    fn cursor_stays_within_loop_and_score_repeats() {
        let score = score_4_4_quarters();
        let timing = ScoreTiming::from_score(&score, 120);
        let schedule = Schedule::build(&score, &timing, None);
        let total = timing.total_loop_ticks() as f64;
        let mut src = TickSource::new(48000);
        let mut out = Vec::new();
        let mut downbeats = 0usize;
        // 3 s of playback at 120 bpm in 4/4 → score loop (2 s) wraps once.
        for _ in 0..(48_000 * 3) {
            src.advance_one_sample(&timing, &schedule, &mut out);
            downbeats += out.iter().filter(|s| **s == SoundType::Downbeat).count();
            out.clear();
            assert!(src.cursor() < total, "cursor escaped loop: {}", src.cursor());
        }
        assert!(downbeats >= 2, "expected score to loop; got {downbeats} downbeats");
    }

    fn count_in_4_4() -> CountInPlan {
        let tpb = grooph_measure::grid::DEFAULT_GRID.ticks_per_beat(&TimeSignature::FOUR_FOUR);
        CountInPlan { ticks: 4 * tpb, clicks: vec![0, tpb, 2 * tpb, 3 * tpb] }
    }

    #[test]
    fn count_in_plays_one_bar_before_the_first_downbeat() {
        let score = score_4_4_quarters();
        let timing = ScoreTiming::from_score(&score, 120);
        let schedule = Schedule::build(&score, &timing, None);
        let mut src = TickSource::new(48000);
        src.restart(Some(&count_in_4_4()));
        let mut out = Vec::new();
        let mut count_in_clicks = Vec::new();
        let mut first_downbeat = None;
        // 120 bpm 4/4: count-in bar lasts 2 s; run 2.5 s.
        for sample in 0..(48_000 * 5 / 2) {
            src.advance_one_sample(&timing, &schedule, &mut out);
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
        let schedule = Schedule::build(&score, &timing, None);
        let mut src = TickSource::new(48000);
        let mut out = Vec::new();
        for _ in 0..10_000 {
            src.advance_one_sample(&timing, &schedule, &mut out);
        }
        src.restart(None);
        assert_eq!(src.cursor(), 0.0);
    }
}
