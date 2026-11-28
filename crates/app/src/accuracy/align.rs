//! Pairing hits with notes by an alignment over a short sliding stretch,
//! per instrument (on one line, one lane for every note).
//!
//! The notes of a lane follow each other on one continuous timeline across
//! the loop's end: occurrence `n` is the lane's note `n mod len` in loop
//! `n div len`, at `loop · total + performed tick`. A note's window reaches
//! up to its neighbouring notes (on a kit at most the catch window), and a
//! hit is a candidate for every note whose window holds it. Within the open
//! stretch, hits and notes are paired in order so that the sum of the pairs'
//! costs (their deviation in shares of the gap on the hit's side) plus
//! [`ALIGN_COST`] for every miss-and-extra pair is minimal. A note and what
//! it was paired with become final once the progress cursor reaches
//! [`Lane::final_at`]; only then are they recorded.

use std::collections::HashMap;

use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{Instrument, StaffKind};

use super::session::{GlobalBeatOnset, catch_window_ticks};

/// What one unpaired note plus one unpaired hit cost, in shares of a gap.
/// Only this sum matters: every pairing with one pair fewer has one miss and
/// one extra more. A late hit on a note whose neighbour gets its own hit
/// counts for it while it is less than this share of the gap late.
pub(super) const ALIGN_COST: f64 = 0.75;
/// A note becomes final at the latest this long after the next note of its
/// lane (ms).
pub(super) const LOOKAHEAD_CAP_MS: f64 = 500.0;

/// A hit not yet final: where it was played, on the session's continuous
/// timeline.
#[derive(Clone, Copy, Debug)]
pub(super) struct PendingHit {
    /// Ticks per second where and when it was played: its deviation is
    /// measured at that tempo.
    pub tps: f64,
    /// The loop it was played in ([`super::session::RecordingData::loops`]).
    pub loop_idx: i64,
    /// Where on the loop, as heard (global ticks).
    pub tick: f64,
    /// When, on the shared clock (s).
    pub timestamp: f64,
}

/// Which note a lane's frontier is: its loop, its written onset (unique in
/// its lane, and kept when swing changes) and where it sounded (global
/// ticks), for when the score changed and the onset is gone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct NoteAt {
    pub loop_idx: i64,
    pub onset: u64,
    pub performed: u64,
}

/// The open stretch of one instrument.
#[derive(Clone, Debug, Default)]
pub(super) struct LaneState {
    /// The first note not final yet. `None` until the lane starts at the
    /// session's origin.
    pub frontier: Option<NoteAt>,
    /// Where the last hit made final as a note's was played (loop, tick):
    /// a hit before it would cross that pair.
    pub floor: Option<(i64, f64)>,
    /// The hits not yet final, in the order played.
    pub hits: Vec<PendingHit>,
}

impl LaneState {
    /// Move the lane back by `ticks` with its score, which lost them from
    /// its front.
    pub(super) fn drop_leading_ticks(&mut self, ticks: u64) {
        if let Some(note) = &mut self.frontier {
            note.onset = note.onset.saturating_sub(ticks);
            note.performed = note.performed.saturating_sub(ticks);
        }
        if let Some((_, tick)) = &mut self.floor {
            *tick -= ticks as f64;
        }
        for h in &mut self.hits {
            h.tick -= ticks as f64;
        }
    }
}

/// The pending state of every lane.
pub(super) type Lanes = HashMap<Instrument, LaneState>;

/// The notes of one lane on the score as played now.
pub(super) struct Lane<'a> {
    pub beats: &'a [GlobalBeatOnset],
    /// Indices into `beats`, in the order they sound.
    pub idx: Vec<usize>,
    pub total: f64,
    pub timing: &'a ScoreTiming,
    pub staff: StaffKind,
}

/// What the alignment made of a lane's first open note.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Decision {
    /// Paired with the pending hit at this index.
    Hit(usize),
    Miss,
}

impl<'a> Lane<'a> {
    pub(super) fn new(
        beats: &'a [GlobalBeatOnset],
        instrument: Instrument,
        total: f64,
        timing: &'a ScoreTiming,
        staff: StaffKind,
    ) -> Self {
        let mut idx: Vec<usize> = (0..beats.len())
            .filter(|&i| beats[i].is_note && beats[i].instrument == instrument)
            .collect();
        // In the order they sound: swing can move a written note past one
        // of another voice (a swung "&" after a triplet's second note).
        idx.sort_by_key(|&i| (beats[i].performed_tick, beats[i].onset_tick));
        Self { beats, idx, total, timing, staff }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.idx.is_empty()
    }

    fn len(&self) -> i64 {
        self.idx.len() as i64
    }

    /// The entry in `beats` of occurrence `n`.
    pub(super) fn beat(&self, n: i64) -> usize {
        self.idx[n.rem_euclid(self.len()) as usize]
    }

    /// The loop of occurrence `n`.
    pub(super) fn loop_of(&self, n: i64) -> i64 {
        n.div_euclid(self.len())
    }

    /// Where occurrence `n` is heard, on the continuous timeline.
    pub(super) fn tick(&self, n: i64) -> f64 {
        self.loop_of(n) as f64 * self.total + self.beats[self.beat(n)].performed_tick as f64
    }

    /// Where a hit at `tick` of loop `loop_idx` lies on the timeline.
    pub(super) fn at(&self, loop_idx: i64, tick: f64) -> f64 {
        loop_idx as f64 * self.total + tick
    }

    /// Ticks per second at occurrence `n`.
    pub(super) fn tps(&self, n: i64) -> f64 {
        self.timing.ticks_per_sec_in_measure(self.beats[self.beat(n)].measure_idx)
    }

    /// The gaps from occurrence `n` back to its neighbouring note and on to
    /// the next: the whole loop for a lane of one note.
    pub(super) fn gaps(&self, n: i64) -> (f64, f64) {
        let t = self.tick(n);
        (t - self.tick(n - 1), self.tick(n + 1) - t)
    }

    /// How far the window of occurrence `n` reaches before and after it: up
    /// to the neighbouring notes, on a kit at most the catch window.
    pub(super) fn window(&self, n: i64) -> (f64, f64) {
        let catch =
            catch_window_ticks(self.staff, self.timing, self.beats[self.beat(n)].measure_idx);
        let (before, after) = self.gaps(n);
        (before.min(catch), after.min(catch))
    }

    /// Whether the window of occurrence `n` holds a hit at `x`.
    pub(super) fn holds(&self, n: i64, x: f64) -> bool {
        let d = x - self.tick(n);
        let (before, after) = self.window(n);
        -before <= d && d <= after
    }

    /// What pairing a hit at `x` with occurrence `n` costs: its deviation
    /// in shares of the gap on its side.
    fn pair_cost(&self, n: i64, x: f64) -> f64 {
        let d = x - self.tick(n);
        let (before, after) = self.gaps(n);
        let gap = if d < 0.0 { before } else { after };
        if gap > 0.0 { d.abs() / gap } else { 0.0 }
    }

    /// The last occurrence heard at or before `x`.
    pub(super) fn at_or_before(&self, x: f64) -> i64 {
        // A loop spans its range's ticks, from the range's first downbeat.
        let start = self.timing.loop_start_tick() as f64;
        let loop_idx = ((x - start) / self.total).floor() as i64;
        let r = x - loop_idx as f64 * self.total;
        let j = self.idx.partition_point(|&i| self.beats[i].performed_tick as f64 <= r) as i64;
        loop_idx * self.len() + j - 1
    }

    /// The occurrence `note` names: the lane's note written at its onset,
    /// or, if the score has none there any more, the first sounding at or
    /// after where it sounded.
    pub(super) fn find(&self, note: NoteAt) -> i64 {
        let written = self.idx.iter().position(|&i| self.beats[i].onset_tick == note.onset);
        let j = written.unwrap_or_else(|| {
            self.idx.partition_point(|&i| self.beats[i].performed_tick < note.performed)
        }) as i64;
        note.loop_idx * self.len() + j
    }

    /// The frontier's form of occurrence `n`.
    pub(super) fn key_of(&self, n: i64) -> NoteAt {
        let beat = &self.beats[self.beat(n)];
        NoteAt { loop_idx: self.loop_of(n), onset: beat.onset_tick, performed: beat.performed_tick }
    }

    /// Whether a hit at `x` is a candidate for any occurrence from `from` on.
    pub(super) fn has_candidate(&self, from: i64, x: f64) -> bool {
        let n = self.at_or_before(x);
        (n >= from && self.holds(n, x)) || (n + 1 >= from && self.holds(n + 1, x))
    }

    /// When on the timeline occurrence `n` becomes final: halfway between
    /// the next note and the one after it, at most [`LOOKAHEAD_CAP_MS`]
    /// after the next.
    pub(super) fn final_at(&self, n: i64) -> f64 {
        let b = self.tick(n + 1);
        let c = self.tick(n + 2);
        let tps = self.tps(n + 1).max(0.0);
        ((b + c) / 2.0).min(b + LOOKAHEAD_CAP_MS / 1000.0 * tps)
    }

    /// Align the `hits` (on the timeline, in order) with the occurrences
    /// from `from` on, and say what the first of them gets.
    pub(super) fn decide(&self, from: i64, hits: &[f64]) -> Decision {
        self.align(from, hits).first().copied().flatten().map_or(Decision::Miss, Decision::Hit)
    }

    /// The optimal alignment of `hits` (on the timeline, in order) with the
    /// occurrences from `from` on, as far as a hit can reach: for each of
    /// them in order, the index of its hit.
    pub(super) fn align(&self, from: i64, hits: &[f64]) -> Vec<Option<usize>> {
        let Some(&last) = hits.last() else {
            return Vec::new();
        };
        // Up to the first note after the last hit, whose window may hold it.
        let to = self.at_or_before(last) + 1;
        if to < from {
            return Vec::new();
        }
        let notes = (to - from + 1) as usize;
        let k = hits.len();
        // best[i][j]: the least cost of the first i notes and j hits.
        let mut best = vec![vec![0.0_f64; k + 1]; notes + 1];
        // 0: note i-1 unpaired, 1: hit j-1 unpaired, 2: both paired.
        let mut step = vec![vec![0u8; k + 1]; notes + 1];
        step[0].iter_mut().skip(1).for_each(|s| *s = 1);
        for i in 1..=notes {
            let n = from + i as i64 - 1;
            for j in 0..=k {
                let mut cost = best[i - 1][j];
                let mut how = 0;
                if j > 0 {
                    if best[i][j - 1] < cost {
                        cost = best[i][j - 1];
                        how = 1;
                    }
                    let x = hits[j - 1];
                    if self.holds(n, x) {
                        let paired = best[i - 1][j - 1] + self.pair_cost(n, x) - ALIGN_COST;
                        if paired < cost {
                            cost = paired;
                            how = 2;
                        }
                    }
                }
                best[i][j] = cost;
                step[i][j] = how;
            }
        }
        let mut out = vec![None; notes];
        let (mut i, mut j) = (notes, k);
        while i > 0 || j > 0 {
            match step[i][j] {
                0 => i -= 1,
                1 => j -= 1,
                _ => {
                    out[i - 1] = Some(j - 1);
                    i -= 1;
                    j -= 1;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::notation::parse_score;

    use super::super::session::compute_global_beat_onsets;

    /// Eighths at 120 BPM: a gap is 250 ms.
    fn eighths() -> (Vec<GlobalBeatOnset>, ScoreTiming) {
        let score = parse_score("4/4 e e e e e e e e").unwrap();
        let timing = ScoreTiming::from_score(&score, 120);
        (compute_global_beat_onsets(&score, &timing), timing)
    }

    #[test]
    fn a_lane_runs_on_across_the_loop_end() {
        let (beats, timing) = eighths();
        let total = timing.total_loop_ticks() as f64;
        let lane = Lane::new(&beats, Instrument::Rhythm, total, &timing, StaffKind::Line);
        let gap = total / 8.0;
        assert_eq!(lane.tick(8), total);
        assert_eq!(lane.tick(-1), -gap);
        assert_eq!(lane.at_or_before(total + 0.5 * gap), 8);
        assert_eq!(lane.find(lane.key_of(8)), 8);
        assert_eq!(lane.find(NoteAt { loop_idx: 1, onset: 1, performed: 1 }), 9);
        // Halfway between the next two notes.
        assert_eq!(lane.final_at(0), 1.5 * gap);
        // The windows reach up to the neighbours.
        assert!(lane.holds(0, gap) && lane.holds(1, 0.0) && !lane.holds(1, -1.0));
    }

    #[test]
    fn the_alignment_pairs_in_order() {
        let (beats, timing) = eighths();
        let total = timing.total_loop_ticks() as f64;
        let lane = Lane::new(&beats, Instrument::Rhythm, total, &timing, StaffKind::Line);
        let g = total / 8.0;
        // A (note 1) 0.6 late, B on time.
        assert_eq!(lane.align(1, &[1.6 * g, 2.0 * g]), [Some(0), Some(1), None]);
        // A 0.8 late, B on time: A missed, the late hit extra.
        assert_eq!(lane.align(1, &[1.8 * g, 2.0 * g]), [None, Some(1), None]);
    }
}
