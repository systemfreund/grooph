//! Metronome trainer options decided per bar while playing: muted bars
//! ("gap click") and a stepwise tempo ramp ("speed trainer").
//!
//! Both are functions of the number of bars played since the score started,
//! so the audio thread and the UI agree on them without rebuilding the
//! schedule, and they keep working across loops and in endless mode.

use grooph_measure::tempo::{MAX_BPM, MIN_BPM};
use serde::{Deserialize, Serialize};

/// What a muted bar silences.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GapMute {
    /// Only downbeat and primary-beat clicks; notes still sound.
    #[default]
    Metronome,
    /// Clicks, notes and ghost notes.
    Everything,
}

/// How muted bars are chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GapMode {
    /// `on_bars` audible, then `off_bars` muted, repeating.
    #[default]
    Pattern,
    /// Each bar after the first is muted with `random_percent` probability.
    Random,
}

/// Regularly muted bars to practise inner time. The count-in is never muted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GapClick {
    pub enabled: bool,
    pub mode: GapMode,
    pub on_bars: u32,
    pub off_bars: u32,
    pub random_percent: u8,
    pub mute: GapMute,
}

impl Default for GapClick {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: GapMode::Pattern,
            on_bars: 2,
            off_bars: 2,
            random_percent: 30,
            mute: GapMute::Metronome,
        }
    }
}

impl GapClick {
    /// Whether bar `bar` (0 = first bar of the score) is muted in pattern mode.
    /// Random mode is decided by the audio thread, see [`GapDecider`].
    pub fn pattern_muted(&self, bar: u64) -> bool {
        let on = self.on_bars.max(1) as u64;
        let period = on + self.off_bars as u64;
        self.enabled && bar % period >= on
    }
}

/// Decides per bar whether it is muted; keeps the random state for
/// [`GapMode::Random`].
pub(crate) struct GapDecider {
    rng: u64,
    bar: Option<u64>,
    muted: bool,
}

impl GapDecider {
    pub(crate) fn new(seed: u64) -> Self {
        Self { rng: seed | 1, bar: None, muted: false }
    }

    /// Forget the current bar and reseed for a new run.
    pub(crate) fn reset(&mut self, seed: u64) {
        self.rng = seed | 1;
        self.bar = None;
        self.muted = false;
    }

    /// Mute state of `bar`. The decision is taken once when the bar starts.
    pub(crate) fn muted(&mut self, gap: &GapClick, bar: u64) -> bool {
        if !gap.enabled {
            return false;
        }
        if self.bar != Some(bar) {
            self.bar = Some(bar);
            self.muted = match gap.mode {
                GapMode::Pattern => gap.pattern_muted(bar),
                // The first bar always sounds, so the pulse is set.
                GapMode::Random => bar > 0 && self.next_percent() < gap.random_percent as u64,
            };
        }
        self.muted
    }

    /// xorshift64*: 0..100.
    fn next_percent(&mut self) -> u64 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) % 100
    }
}

/// What the ramp counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RampUnit {
    #[default]
    Bars,
    /// Complete passes through the score.
    Loops,
}

/// What happens once the target tempo is reached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RampEnd {
    /// Stay at the target.
    #[default]
    Hold,
    /// Jump back to the start tempo and ramp again.
    Restart,
    /// Ramp back to the start tempo, then up again.
    Cycle,
}

/// Raise (or lower) the tempo by `step_bpm` every `every` bars or loops,
/// from the score tempo up to `target_bpm`. Changes land on bar boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TempoRamp {
    pub enabled: bool,
    pub target_bpm: u32,
    pub step_bpm: u32,
    pub every: u32,
    pub unit: RampUnit,
    pub end: RampEnd,
    /// After stopping, keep the tempo reached instead of the start tempo.
    pub keep_reached: bool,
}

impl Default for TempoRamp {
    fn default() -> Self {
        Self {
            enabled: false,
            target_bpm: 140,
            step_bpm: 5,
            every: 4,
            unit: RampUnit::Bars,
            end: RampEnd::Hold,
            keep_reached: false,
        }
    }
}

impl TempoRamp {
    /// Tempo of bar `bar` (0 = first bar of the score) for a ramp starting
    /// at `start_bpm` over a score of `bars_per_loop` measures.
    pub fn bpm_for_bar(&self, start_bpm: u32, bar: u64, bars_per_loop: usize) -> u32 {
        let target = self.target_bpm.clamp(MIN_BPM, MAX_BPM);
        if !self.enabled || target == start_bpm || self.step_bpm == 0 {
            return start_bpm;
        }
        let bars_per_step = self.every.max(1) as u64
            * match self.unit {
                RampUnit::Bars => 1,
                RampUnit::Loops => bars_per_loop.max(1) as u64,
            };
        let done = bar / bars_per_step;
        let span = start_bpm.abs_diff(target) as u64;
        let step = self.step_bpm as u64;
        // Steps to reach the target; the last one may be shorter.
        let steps = span.div_ceil(step);
        let k = match self.end {
            RampEnd::Hold => done.min(steps),
            RampEnd::Restart => done % (steps + 1),
            RampEnd::Cycle => {
                let p = done % (2 * steps);
                if p <= steps { p } else { 2 * steps - p }
            }
        };
        let delta = (k * step).min(span) as u32;
        if target > start_bpm { start_bpm + delta } else { start_bpm - delta }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(target: u32, step: u32, every: u32, end: RampEnd) -> TempoRamp {
        TempoRamp {
            enabled: true,
            target_bpm: target,
            step_bpm: step,
            every,
            unit: RampUnit::Bars,
            end,
            keep_reached: false,
        }
    }

    #[test]
    fn ramp_steps_every_n_bars_and_holds_at_target() {
        let r = ramp(110, 5, 2, RampEnd::Hold);
        let bpms: Vec<u32> = (0..8).map(|b| r.bpm_for_bar(100, b, 1)).collect();
        assert_eq!(bpms, [100, 100, 105, 105, 110, 110, 110, 110]);
    }

    #[test]
    fn ramp_last_step_is_capped_at_target() {
        let r = ramp(108, 5, 1, RampEnd::Hold);
        assert_eq!(r.bpm_for_bar(100, 1, 1), 105);
        assert_eq!(r.bpm_for_bar(100, 2, 1), 108);
        assert_eq!(r.bpm_for_bar(100, 9, 1), 108);
    }

    #[test]
    fn ramp_restart_and_cycle() {
        let restart = ramp(110, 5, 1, RampEnd::Restart);
        let r: Vec<u32> = (0..6).map(|b| restart.bpm_for_bar(100, b, 1)).collect();
        assert_eq!(r, [100, 105, 110, 100, 105, 110]);
        let cycle = ramp(110, 5, 1, RampEnd::Cycle);
        let c: Vec<u32> = (0..7).map(|b| cycle.bpm_for_bar(100, b, 1)).collect();
        assert_eq!(c, [100, 105, 110, 105, 100, 105, 110]);
    }

    #[test]
    fn ramp_can_slow_down_and_count_loops() {
        let mut r = ramp(90, 5, 1, RampEnd::Hold);
        r.unit = RampUnit::Loops;
        // Two-bar score: one step per loop = every two bars.
        let bpms: Vec<u32> = (0..5).map(|b| r.bpm_for_bar(100, b, 2)).collect();
        assert_eq!(bpms, [100, 100, 95, 95, 90]);
    }

    #[test]
    fn disabled_ramp_keeps_start_tempo() {
        let r = TempoRamp::default();
        assert_eq!(r.bpm_for_bar(100, 50, 1), 100);
    }

    #[test]
    fn gap_pattern_mutes_off_bars() {
        let gap = GapClick { enabled: true, on_bars: 2, off_bars: 1, ..Default::default() };
        let muted: Vec<bool> = (0..6).map(|b| gap.pattern_muted(b)).collect();
        assert_eq!(muted, [false, false, true, false, false, true]);
        let off = GapClick { enabled: false, ..gap };
        assert!(!off.pattern_muted(2));
    }

    #[test]
    fn gap_random_never_mutes_first_bar_and_is_stable_within_a_bar() {
        let gap = GapClick {
            enabled: true,
            mode: GapMode::Random,
            random_percent: 100,
            ..Default::default()
        };
        let mut d = GapDecider::new(42);
        assert!(!d.muted(&gap, 0));
        assert!(d.muted(&gap, 1));
        let gap_never = GapClick { random_percent: 0, ..gap };
        let mut d = GapDecider::new(42);
        assert!((0..20).all(|b| !d.muted(&gap_never, b)));
    }

    #[test]
    fn gap_random_mutes_roughly_the_given_share() {
        let gap = GapClick {
            enabled: true,
            mode: GapMode::Random,
            random_percent: 30,
            ..Default::default()
        };
        let mut d = GapDecider::new(7);
        let muted = (1..=1000).filter(|&b| d.muted(&gap, b)).count();
        assert!((200..400).contains(&muted), "muted {muted} of 1000");
    }
}
