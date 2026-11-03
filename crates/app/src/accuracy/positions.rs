//! A run by position in the beat, for the strip under the drift chart:
//! whether the offbeats, the "e"s or the "la"s sit apart from the rest.

use grooph_measure::counting::{CountPos, CountingSystem};

use super::session::AccuracyStats;

/// A position shows its average from this many hits on.
pub(crate) const MIN_POSITION_HITS: u64 = 4;
/// A position stands out from this many hits on...
const NOTABLE_HITS: u64 = 8;
/// ...when its average lies more than this far from the run's.
const NOTABLE_MS: f64 = 10.0;

/// The key of a position: its place in a beat of its size, whichever beat
/// of the measure. Every beat's start is one position, whatever its size.
pub(super) fn position_key(pos: CountPos) -> CountPos {
    if pos.parts <= 1 {
        CountPos { units: 1, uneven: false, beat: 1, part: 0, parts: 1 }
    } else {
        CountPos { beat: 1, ..pos }
    }
}

/// The label `system` gives `pos`, its place as a fraction where it says
/// nothing.
pub(crate) fn position_label(pos: &CountPos, system: CountingSystem) -> String {
    system.label(pos).unwrap_or_else(|| format!("{}/{}", pos.part, pos.parts))
}

type Tally = (CountPos, AccuracyStats, u64);

/// What a run collects per position while it plays.
#[derive(Default)]
pub(super) struct PositionLog {
    tallies: Vec<Tally>,
}

impl PositionLog {
    fn tally(&mut self, pos: CountPos) -> &mut Tally {
        let key = position_key(pos);
        let i = match self.tallies.iter().position(|(p, ..)| *p == key) {
            Some(i) => i,
            None => {
                self.tallies.push((key, AccuracyStats::default(), 0));
                self.tallies.len() - 1
            }
        };
        &mut self.tallies[i]
    }

    pub(super) fn push_hit(&mut self, pos: CountPos, delta_ms: f64) {
        self.tally(pos).1.push(delta_ms);
    }

    pub(super) fn push_miss(&mut self, pos: CountPos) {
        self.tally(pos).2 += 1;
    }

    /// The finished run, `overall_ms` its average deviation. `None` unless
    /// notes fell on at least two positions.
    pub(super) fn finish(&self, overall_ms: f64) -> Option<PositionTallies> {
        (self.tallies.len() >= 2)
            .then(|| PositionTallies { tallies: self.tallies.clone(), overall_ms })
    }
}

/// A finished run's tallies per position, to be shown in a counting system.
#[derive(Clone)]
pub(crate) struct PositionTallies {
    tallies: Vec<Tally>,
    overall_ms: f64,
}

impl PositionTallies {
    /// The run by position as `system` names them: positions it names the
    /// same (the "&" of a 4/4 beat and of a 7/8 pair) are one. `None` unless
    /// two of them have enough hits to compare.
    pub(crate) fn in_system(&self, system: CountingSystem) -> Option<BeatPositions> {
        let mut sorted = self.tallies.clone();
        // Through the beat; the parts of a short beat before a long one's.
        sorted.sort_by(|a, b| {
            let at = |p: &CountPos| p.part as f64 / p.parts as f64;
            at(&a.0).total_cmp(&at(&b.0)).then(a.0.units.cmp(&b.0.units))
        });
        let mut merged: Vec<(String, Tally)> = Vec::new();
        for (pos, stats, misses) in sorted {
            let label = position_label(&pos, system);
            match merged.iter_mut().find(|(l, _)| *l == label) {
                Some((_, (_, s, m))) => {
                    s.merge(&stats);
                    *m += misses;
                }
                None => merged.push((label, (pos, stats, misses))),
            }
        }
        let positions: Vec<PositionTiming> = merged
            .into_iter()
            .map(|(label, (pos, stats, misses))| {
                let shown = stats.count >= MIN_POSITION_HITS;
                PositionTiming {
                    pos,
                    label,
                    hits: stats.count,
                    misses,
                    mean_ms: stats.mean_ms().filter(|_| shown),
                    spread_ms: stats.spread_ms().filter(|_| shown),
                }
            })
            .collect();
        if positions.iter().filter(|p| p.mean_ms.is_some()).count() < 2 {
            return None;
        }
        let overall_ms = self.overall_ms;
        let notable = positions
            .iter()
            .enumerate()
            .filter(|(_, p)| p.hits >= NOTABLE_HITS)
            .filter_map(|(i, p)| Some((i, (p.mean_ms? - overall_ms).abs())))
            .filter(|&(_, off)| off > NOTABLE_MS)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i);
        Some(BeatPositions { positions, overall_ms, notable })
    }
}

/// One position of a finished run.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PositionTiming {
    /// Beat 1 stands for every beat.
    pub pos: CountPos,
    /// As the counting system names it.
    pub label: String,
    pub hits: u64,
    pub misses: u64,
    /// `None` below [`MIN_POSITION_HITS`] hits.
    pub mean_ms: Option<f64>,
    pub spread_ms: Option<f64>,
}

/// A finished run by position in the beat.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BeatPositions {
    /// Through the beat, its start first.
    pub positions: Vec<PositionTiming>,
    /// The run's average deviation.
    pub overall_ms: f64,
    /// The position whose average lies farthest from the run's, if one
    /// stands out.
    pub notable: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const NUMBERS: CountingSystem = CountingSystem::Numbers;

    fn pos(units: u8, beat: u32, part: u8, parts: u8) -> CountPos {
        CountPos { units, uneven: false, beat, part, parts }
    }

    fn finish(log: &PositionLog, overall_ms: f64) -> Option<BeatPositions> {
        log.finish(overall_ms)?.in_system(NUMBERS)
    }

    #[test]
    fn every_beat_start_is_one_position() {
        let mut log = PositionLog::default();
        for _ in 0..2 {
            log.push_hit(pos(1, 1, 0, 1), 0.0);
            log.push_hit(pos(1, 3, 0, 1), 0.0);
            log.push_hit(pos(3, 2, 0, 1), 0.0);
        }
        for _ in 0..4 {
            log.push_hit(pos(1, 2, 1, 2), 0.0);
        }
        log.push_miss(pos(1, 2, 1, 2));
        log.push_miss(pos(1, 1, 1, 4));
        let positions = finish(&log, 0.0).unwrap();
        let rows: Vec<(&str, u64, u64)> =
            positions.positions.iter().map(|p| (p.label.as_str(), p.hits, p.misses)).collect();
        assert_eq!(rows, [("1", 6, 0), ("e", 0, 1), ("&", 4, 1)]);
        // Too few hits for an average.
        assert_eq!(positions.positions[1].mean_ms, None);
    }

    #[test]
    fn sorted_through_the_beat_and_the_farthest_stands_out() {
        let mut log = PositionLog::default();
        for _ in 0..8 {
            log.push_hit(pos(1, 2, 1, 2), 30.0);
            log.push_hit(pos(1, 1, 3, 4), 12.0);
            log.push_hit(pos(1, 1, 0, 1), -20.0);
        }
        let positions = finish(&log, 7.0).unwrap();
        let parts: Vec<(u8, u8)> =
            positions.positions.iter().map(|p| (p.pos.part, p.pos.parts)).collect();
        assert_eq!(parts, [(0, 1), (1, 2), (3, 4)]);
        // 27 ms below the run's average beats 23 above.
        assert_eq!(positions.notable, Some(0));
        assert_eq!(positions.positions[1].mean_ms, Some(30.0));
    }

    #[test]
    fn nothing_stands_out_close_to_the_average_or_on_few_hits() {
        let mut log = PositionLog::default();
        for _ in 0..8 {
            log.push_hit(pos(1, 1, 0, 1), 5.0);
            log.push_hit(pos(1, 1, 1, 2), -5.0);
        }
        // Far off, but on too few hits.
        for _ in 0..7 {
            log.push_hit(pos(1, 1, 1, 4), 80.0);
        }
        assert_eq!(finish(&log, 0.0).unwrap().notable, None);
    }

    #[test]
    fn positions_named_the_same_are_one() {
        // The "&" of a 4/4 beat and of a 7/8 pair.
        let pair = CountPos { units: 2, uneven: true, beat: 2, part: 1, parts: 2 };
        let mut log = PositionLog::default();
        for _ in 0..2 {
            log.push_hit(pos(1, 1, 1, 2), 10.0);
            log.push_hit(pair, 20.0);
        }
        for _ in 0..4 {
            log.push_hit(pos(1, 1, 0, 1), 0.0);
        }
        let positions = finish(&log, 0.0).unwrap();
        assert_eq!(positions.positions.len(), 2);
        assert_eq!(positions.positions[1].hits, 4);
        assert_eq!(positions.positions[1].mean_ms, Some(15.0));
        // Gordon names them apart ("de", "be"): too few hits each.
        let tallies = log.finish(0.0).unwrap();
        assert!(tallies.in_system(CountingSystem::Gordon).is_none());
    }

    #[test]
    fn nothing_to_compare_on_one_position_or_without_two_averages() {
        let mut log = PositionLog::default();
        log.push_hit(pos(1, 1, 0, 1), 5.0);
        log.push_hit(pos(1, 2, 0, 1), 5.0);
        assert!(log.finish(5.0).is_none());
        // Two positions, but only one with enough hits.
        log.push_hit(pos(1, 1, 1, 2), 5.0);
        log.push_hit(pos(1, 1, 0, 1), 5.0);
        log.push_hit(pos(1, 1, 0, 1), 5.0);
        assert!(finish(&log, 5.0).is_none());
        for _ in 0..3 {
            log.push_hit(pos(1, 1, 1, 2), 5.0);
        }
        assert!(finish(&log, 5.0).is_some());
    }
}
