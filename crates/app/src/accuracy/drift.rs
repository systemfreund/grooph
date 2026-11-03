//! A run over time, for the drift chart under "Your run": each hit's
//! deviation at its note's time, the missed notes and the silent bars.

use super::session::HitGrade;

/// A hit on the chart: `t` seconds after the run's first note.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DriftHit {
    pub t: f64,
    pub delta_ms: f64,
    pub grade: HitGrade,
}

/// What a run collects for its chart while it plays, in seconds on the
/// session's clock.
#[derive(Default)]
pub(super) struct DriftLog {
    /// Each hit and the length of its note's bar.
    hits: Vec<(DriftHit, f64)>,
    misses: Vec<f64>,
}

impl DriftLog {
    pub(super) fn push_hit(&mut self, note_s: f64, delta_ms: f64, grade: HitGrade, bar_s: f64) {
        self.hits.push((DriftHit { t: note_s, delta_ms, grade }, bar_s));
    }

    pub(super) fn push_miss(&mut self, note_s: f64) {
        self.misses.push(note_s);
    }

    /// The finished run, `silent` its silent stretches (start, end). `None`
    /// with fewer than two hits: no course to draw.
    pub(super) fn finish(&self, silent: impl Iterator<Item = (f64, f64)>) -> Option<Drift> {
        if self.hits.len() < 2 {
            return None;
        }
        let mut hits = self.hits.clone();
        hits.sort_by(|a, b| a.0.t.total_cmp(&b.0.t));
        let mut misses = self.misses.clone();
        misses.sort_by(f64::total_cmp);
        let origin = hits[0].0.t.min(misses.first().copied().unwrap_or(f64::INFINITY));
        let end = hits[hits.len() - 1].0.t.max(misses.last().copied().unwrap_or(f64::NEG_INFINITY));
        let length_s = (end - origin).max(f64::EPSILON);

        // The average over about a bar centred on each hit: the drift,
        // without the scatter of single hits.
        let times: Vec<f64> = hits.iter().map(|(h, _)| h.t).collect();
        // A hit without a bar length (no tempo) has no bar to average over.
        let trend = hits
            .iter()
            .filter(|&&(_, bar_s)| bar_s > 0.0)
            .map(|&(hit, bar_s)| {
                let from = times.partition_point(|&t| t < hit.t - bar_s / 2.0);
                let to = times.partition_point(|&t| t <= hit.t + bar_s / 2.0);
                let window = &hits[from..to];
                let mean =
                    window.iter().map(|(h, _)| h.delta_ms).sum::<f64>() / window.len() as f64;
                (hit.t - origin, mean)
            })
            .collect();

        let mut stretches: Vec<(f64, f64)> = Vec::new();
        for (start, end) in silent {
            let (start, end) = ((start - origin).max(0.0), (end - origin).min(length_s));
            if end <= start {
                continue;
            }
            // Silent bars in a row make one stretch.
            match stretches.last_mut() {
                Some(last) if start - last.1 < 1e-3 => last.1 = end,
                _ => stretches.push((start, end)),
            }
        }

        Some(Drift {
            hits: hits.into_iter().map(|(h, _)| DriftHit { t: h.t - origin, ..h }).collect(),
            trend,
            misses: misses.into_iter().map(|t| t - origin).collect(),
            silent: stretches,
            length_s,
        })
    }
}

/// A finished run over time, in seconds from its first note.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Drift {
    /// In time order.
    pub hits: Vec<DriftHit>,
    /// The average deviation over about a bar around each hit: (t, ms).
    /// Hits without a bar length are left out.
    pub trend: Vec<(f64, f64)>,
    /// When the missed notes were due.
    pub misses: Vec<f64>,
    /// Stretches of silent bars (start, end).
    pub silent: Vec<(f64, f64)>,
    /// From the first note to the last.
    pub length_s: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_starts_at_the_first_note_and_averages_about_a_bar() {
        let mut log = DriftLog::default();
        // Bars of 2 s from 10 s on: on time, then 40 ms early.
        for (t, d) in [(10.0, 0.0), (11.0, 0.0), (12.0, -40.0), (13.0, -40.0)] {
            log.push_hit(t, d, HitGrade::Perfect, 2.0);
        }
        log.push_miss(14.0);
        let drift = log.finish([(12.0, 14.0), (14.0, 16.0)].into_iter()).unwrap();
        assert_eq!(drift.hits[0].t, 0.0);
        assert_eq!(drift.length_s, 4.0);
        assert_eq!(drift.misses, vec![4.0]);
        // Within ±1 s of each hit.
        let trend: Vec<f64> = drift.trend.iter().map(|&(_, ms)| ms).collect();
        assert_eq!(trend, vec![0.0, -40.0 / 3.0, -80.0 / 3.0, -40.0]);
        // Joined, and cut at the run's end.
        assert_eq!(drift.silent, vec![(2.0, 4.0)]);
    }

    #[test]
    fn hits_without_a_bar_length_stay_out_of_the_trend() {
        let mut log = DriftLog::default();
        log.push_hit(0.0, 10.0, HitGrade::Perfect, 0.0);
        log.push_hit(1.0, -10.0, HitGrade::Perfect, 2.0);
        let drift = log.finish(std::iter::empty()).unwrap();
        assert_eq!(drift.hits.len(), 2);
        assert_eq!(drift.trend, vec![(1.0, 0.0)]);
    }

    #[test]
    fn finish_needs_two_hits() {
        let mut log = DriftLog::default();
        log.push_hit(1.0, 5.0, HitGrade::Perfect, 2.0);
        log.push_miss(2.0);
        assert!(log.finish(std::iter::empty()).is_none());
    }
}
