//! Onset detection for sharp, percussive hits (sticks, claps, a practice
//! pad): a hit starts where the signal first crosses the threshold, and the
//! ringing that follows is ignored for a short while.

/// After a hit, further crossings are ignored for this long (s): one stroke
/// rings and bounces for a few tens of milliseconds. Shorter than a 32nd note
/// at 200 BPM (75 ms).
pub const REFRACTORY_S: f64 = 0.04;

/// Default threshold (dBFS). A stick hit near a phone peaks around 0 dBFS,
/// the phone's own click picked up by its microphone around -30 dBFS.
pub const DEFAULT_THRESHOLD_DB: f32 = -15.0;

pub fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn linear_to_db(x: f32) -> f32 {
    if x > 0.0 { 20.0 * x.log10() } else { f32::NEG_INFINITY }
}

pub struct OnsetDetector {
    sample_rate: f64,
    threshold: f32,
    refractory: u64,
    /// Samples left until the next hit may start.
    hold: u64,
}

impl OnsetDetector {
    pub fn new(sample_rate: u32, threshold_db: f32) -> Self {
        Self {
            sample_rate: sample_rate as f64,
            threshold: db_to_linear(threshold_db),
            refractory: (REFRACTORY_S * sample_rate as f64).round() as u64,
            hold: 0,
        }
    }

    pub fn set_threshold_db(&mut self, db: f32) {
        self.threshold = db_to_linear(db);
    }

    /// Scan mono `samples`, the first captured at `start` seconds. Pushes the
    /// time of each hit onto `onsets` and returns the block's peak level
    /// (linear).
    pub fn process(
        &mut self,
        samples: impl IntoIterator<Item = f32>,
        start: f64,
        onsets: &mut Vec<f64>,
    ) -> f32 {
        let mut peak = 0f32;
        for (i, x) in samples.into_iter().enumerate() {
            let x = x.abs();
            peak = peak.max(x);
            if self.hold > 0 {
                self.hold -= 1;
            } else if x >= self.threshold {
                onsets.push(start + i as f64 / self.sample_rate);
                self.hold = self.refractory;
            }
        }
        peak
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 1000;

    fn signal(len: usize, hits: &[(usize, f32)]) -> Vec<f32> {
        let mut s = vec![0.001; len];
        for &(at, level) in hits {
            // A hit: a sharp peak that rings down over 30 ms.
            for k in 0..30 {
                if let Some(x) = s.get_mut(at + k) {
                    *x = level * (1.0 - k as f32 / 30.0) * if k % 2 == 0 { 1.0 } else { -1.0 };
                }
            }
        }
        s
    }

    #[test]
    fn finds_each_hit_once_at_its_start() {
        let mut d = OnsetDetector::new(RATE, -15.0);
        let mut onsets = Vec::new();
        d.process(signal(1000, &[(100, 0.9), (400, 0.5)]), 2.0, &mut onsets);
        assert_eq!(onsets, vec![2.1, 2.4]);
    }

    #[test]
    fn ignores_signal_below_threshold() {
        let mut d = OnsetDetector::new(RATE, -15.0);
        let mut onsets = Vec::new();
        // -30 dBFS: the click bleeding into the microphone.
        d.process(signal(500, &[(100, db_to_linear(-30.0))]), 0.0, &mut onsets);
        assert!(onsets.is_empty());
    }

    #[test]
    fn refractory_spans_block_boundaries() {
        let mut d = OnsetDetector::new(RATE, -15.0);
        let mut onsets = Vec::new();
        let s = signal(200, &[(95, 0.9), (150, 0.9)]);
        d.process(s[..100].iter().copied(), 0.0, &mut onsets);
        d.process(s[100..].iter().copied(), 0.1, &mut onsets);
        assert_eq!(onsets.len(), 2);
        assert!((onsets[0] - 0.095).abs() < 1e-9);
        assert!((onsets[1] - 0.15).abs() < 1e-9);
    }

    #[test]
    fn reports_block_peak() {
        let mut d = OnsetDetector::new(RATE, 0.0);
        let peak = d.process([0.1, -0.5, 0.2], 0.0, &mut Vec::new());
        assert_eq!(peak, 0.5);
        assert!((linear_to_db(0.5) + 6.02).abs() < 0.01);
    }
}
