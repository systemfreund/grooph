//! A run by position in the beat, for the strip under the drift chart:
//! whether the offbeats, the "e"s or the "la"s sit apart from the rest.

use grooph_measure::counting::CountPos;

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

/// What a run collects per position while it plays.
#[derive(Default)]
pub(super) struct PositionLog {
    tallies: Vec<(CountPos, AccuracyStats, u64)>,
}

impl PositionLog {
    fn tally(&mut self, pos: CountPos) -> &mut (CountPos, AccuracyStats, u64) {
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
    /// notes fell on at least two positions: one has nothing to compare.
    pub(super) fn finish(&self, overall_ms: f64) -> Option<BeatPositions> {
        if self.tallies.len() < 2 {
            return None;
        }
        let mut positions: Vec<PositionTiming> = self
            .tallies
            .iter()
            .map(|(pos, stats, misses)| {
                let shown = stats.count >= MIN_POSITION_HITS;
                PositionTiming {
                    pos: *pos,
                    hits: stats.count,
                    misses: *misses,
                    mean_ms: stats.mean_ms().filter(|_| shown),
                    spread_ms: stats.spread_ms().filter(|_| shown),
                }
            })
            .collect();
        // Through the beat; the parts of a short beat before a long one's.
        positions.sort_by(|a, b| {
            let at = |p: &CountPos| p.part as f64 / p.parts as f64;
            at(&a.pos).total_cmp(&at(&b.pos)).then(a.pos.units.cmp(&b.pos.units))
        });
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PositionTiming {
    /// Beat 1 stands for every beat.
    pub pos: CountPos,
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

    fn pos(units: u8, beat: u32, part: u8, parts: u8) -> CountPos {
        CountPos { units, uneven: false, beat, part, parts }
    }

    #[test]
    fn every_beat_start_is_one_position() {
        let mut log = PositionLog::default();
        log.push_hit(pos(1, 1, 0, 1), 0.0);
        log.push_hit(pos(1, 3, 0, 1), 0.0);
        log.push_hit(pos(3, 2, 0, 1), 0.0);
        log.push_miss(pos(1, 2, 1, 2));
        let positions = log.finish(0.0).unwrap();
        assert_eq!(positions.positions.len(), 2);
        assert_eq!(positions.positions[0].hits, 3);
        assert_eq!(positions.positions[1].misses, 1);
        // Too few hits for an average.
        assert_eq!(positions.positions[0].mean_ms, None);
    }

    #[test]
    fn sorted_through_the_beat_and_the_farthest_stands_out() {
        let mut log = PositionLog::default();
        for _ in 0..8 {
            log.push_hit(pos(1, 2, 1, 2), 30.0);
            log.push_hit(pos(1, 1, 3, 4), 12.0);
            log.push_hit(pos(1, 1, 0, 1), -20.0);
        }
        let positions = log.finish(7.0).unwrap();
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
        assert_eq!(log.finish(0.0).unwrap().notable, None);
    }

    #[test]
    fn one_position_has_nothing_to_compare() {
        let mut log = PositionLog::default();
        log.push_hit(pos(1, 1, 0, 1), 5.0);
        log.push_hit(pos(1, 2, 0, 1), 5.0);
        assert!(log.finish(5.0).is_none());
    }
}
