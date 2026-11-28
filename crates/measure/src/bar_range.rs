//! The loop range: the bars of a score that play, `first..=last`.
//!
//! Measure indices stay the score's: a range only says which of its bars
//! play and where playback wraps. At least one bar is always in it.

use crate::MeasureIdx;
use serde::{Deserialize, Serialize};

/// Bars `first..=last` of a score (0-based measure indices), `first <= last`
/// (also when read: a saved `first > last` does not read).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawBarRange")]
pub struct BarRange {
    first: MeasureIdx,
    last: MeasureIdx,
}

/// A [`BarRange`] as saved, before it is checked.
#[derive(Deserialize)]
struct RawBarRange {
    first: MeasureIdx,
    last: MeasureIdx,
}

impl TryFrom<RawBarRange> for BarRange {
    type Error = &'static str;

    fn try_from(raw: RawBarRange) -> Result<Self, Self::Error> {
        Self::new(raw.first, raw.last).ok_or("a loop range ends before it starts")
    }
}

impl BarRange {
    /// Bars `first..=last`; `None` when `first > last`.
    pub fn new(first: MeasureIdx, last: MeasureIdx) -> Option<Self> {
        (first <= last).then_some(Self { first, last })
    }

    /// Every bar of a score of `len` bars (at least one).
    pub fn all(len: usize) -> Self {
        Self { first: 0, last: len.max(1) - 1 }
    }

    pub fn first(&self) -> MeasureIdx {
        self.first
    }

    pub fn last(&self) -> MeasureIdx {
        self.last
    }

    /// Number of bars in the range.
    pub fn len(&self) -> usize {
        self.last - self.first + 1
    }

    /// Never empty: kept for clippy's `len_without_is_empty`.
    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn contains(&self, idx: MeasureIdx) -> bool {
        (self.first..=self.last).contains(&idx)
    }

    /// The bars as a range, for loops and slices.
    pub fn bars(&self) -> std::ops::Range<MeasureIdx> {
        self.first..self.last + 1
    }

    /// Whether the range lies inside a score of `len` bars.
    pub fn fits(&self, len: usize) -> bool {
        self.last < len
    }

    /// The range cut to a score of `len` bars: its end, and if need be its
    /// start, move back to the score's last bar.
    pub fn clamped(self, len: usize) -> Self {
        let max = len.max(1) - 1;
        Self { first: self.first.min(max), last: self.last.min(max) }
    }

    /// The range with `first` moved to `first`, at most onto its last bar.
    pub fn with_first(self, first: MeasureIdx) -> Self {
        Self { first: first.min(self.last), ..self }
    }

    /// The range with `last` moved to `last`, at least onto its first bar.
    pub fn with_last(self, last: MeasureIdx) -> Self {
        Self { last: last.max(self.first), ..self }
    }

    /// The bar that plays after `current` in a score of `len` bars, with
    /// this range: the next bar (bar 0 after the score's last) if it is in
    /// the range, else the range's first bar. So the range loops, and after
    /// the range changed playback goes on into the new range where it can
    /// and jumps to its start where it cannot.
    pub fn next_bar(&self, current: MeasureIdx, len: usize) -> MeasureIdx {
        let next = if current + 1 >= len { 0 } else { current + 1 };
        if self.contains(next) { next } else { self.first }
    }

    /// The range after a bar was inserted at `idx` (the bars from `idx` on
    /// move back by one): before the range it moves along, after its first
    /// bar and up to its last it grows, after its last it stays. A copy of
    /// the range's last bar (inserted after it) and a bar appended to the
    /// score lie outside.
    pub fn after_insert(self, idx: MeasureIdx) -> Self {
        if idx <= self.first {
            Self { first: self.first + 1, last: self.last + 1 }
        } else if idx <= self.last {
            Self { last: self.last + 1, ..self }
        } else {
            self
        }
    }

    /// The range after the bar `idx` was removed: before the range it moves
    /// forward, inside it shrinks; `None` when its only bar went.
    pub fn after_remove(self, idx: MeasureIdx) -> Option<Self> {
        if idx < self.first {
            Some(Self { first: self.first - 1, last: self.last - 1 })
        } else if idx <= self.last {
            (self.first < self.last).then(|| Self { last: self.last - 1, ..self })
        } else {
            Some(self)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bars `a..=b` counted from 1, as the user sees them.
    fn bars(a: usize, b: usize) -> BarRange {
        BarRange::new(a - 1, b - 1).unwrap()
    }

    #[test]
    fn a_range_has_at_least_one_bar() {
        assert_eq!(BarRange::new(3, 2), None);
        assert_eq!(BarRange::all(0), BarRange::all(1));
        assert_eq!(bars(2, 5).with_first(7), bars(5, 5));
        assert_eq!(bars(2, 5).with_last(0), bars(2, 2));
        assert_eq!(bars(3, 9).clamped(4), bars(3, 4));
        assert_eq!(bars(6, 9).clamped(4), bars(4, 4));
    }

    #[test]
    fn the_next_bar_loops_the_range_and_follows_a_new_one() {
        // Bar 5 of 3..=6 in 8 bars, counted from 1.
        let next = |r: BarRange, current: usize, len| r.next_bar(current - 1, len) + 1;
        assert_eq!(next(bars(3, 6), 5, 8), 6);
        assert_eq!(next(bars(3, 6), 6, 8), 3);
        // The end dragged to bar 8: 5 → 6 → 7 → 8 → 3.
        let wider = bars(3, 8);
        assert_eq!([5, 6, 7, 8].map(|b| next(wider, b, 8)), [6, 7, 8, 3]);
        // The end dragged to bar 5: back to 3 after bar 5.
        assert_eq!(next(bars(3, 5), 5, 8), 3);
        // Loop off: on to the end, then bar 1.
        let all = BarRange::all(8);
        assert_eq!([5, 7, 8].map(|b| next(all, b, 8)), [6, 8, 1]);
        // A range ahead: jump to its start.
        assert_eq!(next(bars(7, 8), 5, 8), 7);
    }

    #[test]
    fn edits_move_the_range_with_its_bars() {
        // 3..=5 of 8 bars; a duplicate is inserted after its bar.
        let r = bars(3, 5);
        let duplicate = |bar: usize| r.after_insert(bar);
        assert_eq!(duplicate(1), bars(4, 6));
        assert_eq!(duplicate(4), bars(3, 6));
        assert_eq!(duplicate(3), bars(3, 6));
        assert_eq!(duplicate(5), bars(3, 5));
        assert_eq!(r.after_insert(8), bars(3, 5), "appended");
        let delete = |bar: usize| r.after_remove(bar - 1);
        assert_eq!(delete(1), Some(bars(2, 4)));
        assert_eq!(delete(4), Some(bars(3, 4)));
        assert_eq!(delete(6), Some(bars(3, 5)));
        assert_eq!(bars(4, 4).after_remove(3), None);
        // All bars: an appended bar stays outside.
        assert_eq!(BarRange::all(4).after_insert(4), BarRange::all(4));
    }
}
