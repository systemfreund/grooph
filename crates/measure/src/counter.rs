//! Counter rhythms: a second click bar the metronome plays against its click
//! bar, for polyrhythms like three against four.
//!
//! A counter rhythm is [`Ratio`] `n` even clicks in the time of `span` beat
//! units, repeated until the bar is full, written as a measure of the meter
//! like a click bar (see [`crate::click`]): its notes click at their level,
//! rests are silent. The clicks take the simplest note value that is exactly
//! as long; where there is none (three against four has no half-note
//! triplet), a shorter value with rests up to the next click. [`CounterBars`]
//! keeps the counter rhythm of each meter that has one.

use crate::click::{ClickLevel, same_bar};
use crate::grid::DEFAULT_GRID;
use crate::math::gcd;
use crate::notation::{code_of, parse_score};
use crate::{BeatKind, Measure, TimeSignature};
use serde::{Deserialize, Serialize};

/// Most clicks a counter rhythm offers per span.
const MAX_CLICKS: u8 = 9;
/// Most notes and rests written for one click, so a click is never spelled
/// as a long row of tiny rests.
const MAX_SLOTS_PER_CLICK: u32 = 4;

/// `n` even clicks in the time of `span` beat units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ratio {
    pub n: u8,
    pub span: u8,
}

impl std::fmt::Display for Ratio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.n, self.span)
    }
}

/// The counter bar of `ratio` in `ts`: its first click medium, the others
/// soft (the click bar already clicks the downbeat loud). `None` if the
/// span does not divide the bar or the clicks fall off the tick grid.
pub fn counter_bar(ts: TimeSignature, ratio: Ratio) -> Option<Measure> {
    let Ratio { n, span } = ratio;
    if n < 2 || span == 0 || !ts.beats.is_multiple_of(span) {
        return None;
    }
    let span_ticks = span as u32 * DEFAULT_GRID.ticks_per_beat(&ts);
    if !span_ticks.is_multiple_of(n as u32) {
        return None;
    }
    let click_ticks = span_ticks / n as u32;
    let mut durations: Vec<_> = DEFAULT_GRID
        .durations
        .iter()
        .filter_map(|d| Some((DEFAULT_GRID.ticks_of(d)?, *d)))
        .filter(|&(t, _)| click_ticks.is_multiple_of(t) && click_ticks / t <= MAX_SLOTS_PER_CLICK)
        .collect();
    durations.sort_by_key(|&(t, _)| std::cmp::Reverse(t));
    let repeats = ts.beats / span;
    durations.into_iter().find_map(|(ticks, d)| {
        let code = code_of(&d)?;
        let slots = click_ticks / ticks;
        let mut tokens = Vec::new();
        for click in 0..(repeats as u32 * n as u32) {
            tokens.push(if click == 0 { code.to_string() } else { format!("({code})") });
            tokens.extend((1..slots).map(|_| format!("r:{code}")));
        }
        parse_score(&format!("{ts} {}", tokens.join(" "))).ok()?.measures.into_iter().next()
    })
}

/// The ratios offered in `ts`, the ones over the whole bar first: spans of
/// two or more beat units that divide the bar, with more than half and
/// fewer than twice as many clicks as the span has beats, and no common
/// factor (6:4 is 3:2 twice). Only ratios with a counter bar.
pub fn counter_ratios(ts: TimeSignature) -> Vec<Ratio> {
    let mut ratios = Vec::new();
    for span in (2..=ts.beats).rev().filter(|&s| ts.beats.is_multiple_of(s)) {
        for n in 2..=MAX_CLICKS {
            let ratio = Ratio { n, span };
            if 2 * n > span
                && n < 2 * span
                && gcd(n as u32, span as u32) == 1
                && counter_bar(ts, ratio).is_some()
            {
                ratios.push(ratio);
            }
        }
    }
    ratios
}

/// The ratio whose counter bar has the same notes and rests as `bar`,
/// whatever their levels, if any.
pub fn ratio_of(bar: &Measure) -> Option<Ratio> {
    let ts = bar.time_signature();
    let shape = |m: &Measure| m.beats().iter().map(|b| b.duration).collect::<Vec<_>>();
    counter_ratios(ts)
        .into_iter()
        .find(|&r| counter_bar(ts, r).is_some_and(|m| shape(&m) == shape(bar)))
}

/// Index of each click of `bar` (its first beat) and the beats it spans:
/// a click and the rests up to the next one. A rest a tap has made of a
/// click stays its own click, so only the rests the counter bar was
/// written with are folded in.
pub fn counter_clicks(bar: &Measure) -> Vec<std::ops::Range<usize>> {
    let ts = bar.time_signature();
    let slots = ratio_of(bar)
        .and_then(|r| counter_bar(ts, r))
        .map(|m| m.beats().iter().map(|b| b.kind == BeatKind::Note).collect::<Vec<_>>());
    let starts: Vec<usize> = match slots {
        Some(is_click) => (0..bar.beats().len()).filter(|&i| is_click[i]).collect(),
        None => (0..bar.beats().len()).collect(),
    };
    let mut ends: Vec<usize> = starts.iter().skip(1).copied().collect();
    ends.push(bar.beats().len());
    starts.into_iter().zip(ends).map(|(s, e)| s..e).collect()
}

/// The level of the click starting at `idx` of a counter bar.
pub fn click_level(bar: &Measure, idx: usize) -> ClickLevel {
    bar.beats().get(idx).map_or(ClickLevel::Off, ClickLevel::of)
}

/// The counter bar of each meter that has one.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CounterBars {
    bars: Vec<Measure>,
}

impl std::fmt::Debug for CounterBars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.bars.iter().map(|m| m.time_signature())).finish()
    }
}

impl PartialEq for CounterBars {
    fn eq(&self, other: &Self) -> bool {
        self.bars.len() == other.bars.len()
            && self.bars.iter().zip(&other.bars).all(|(a, b)| same_bar(a, b))
    }
}

impl Eq for CounterBars {}

impl CounterBars {
    /// The counter bar of `ts`, `None` if it has none.
    pub fn for_meter(&self, ts: TimeSignature) -> Option<&Measure> {
        self.bars.iter().find(|m| m.time_signature() == ts)
    }

    /// Give `ts` the counter bar `bar` (of that meter), or none.
    pub fn set(&mut self, ts: TimeSignature, bar: Option<Measure>) {
        self.bars.retain(|m| m.time_signature() != ts);
        self.bars.extend(bar.filter(|m| m.time_signature() == ts));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notation::format_score;
    use crate::score::Score;

    fn text(m: &Measure) -> String {
        format_score(&Score::single(m.clone())).unwrap()
    }

    fn bar(ts: &str, n: u8, span: u8) -> String {
        text(&counter_bar(ts.parse().unwrap(), Ratio { n, span }).unwrap())
    }

    #[test]
    fn clicks_take_the_simplest_value() {
        assert_eq!(bar("3/4", 2, 3), "3/4 q. (q.)");
        assert_eq!(bar("3/4", 4, 3), "3/4 e. (e.) (e.) (e.)");
        assert_eq!(bar("6/8", 4, 6), "6/8 e. (e.) (e.) (e.)");
        assert_eq!(bar("2/4", 3, 2), "2/4 t4 (t4) (t4)");
        assert_eq!(bar("4/4", 5, 4), "4/4 qt4 (qt4) (qt4) (qt4) (qt4)");
        assert_eq!(bar("4/4", 7, 4), "4/4 spt4 (spt4) (spt4) (spt4) (spt4) (spt4) (spt4)");
    }

    #[test]
    fn spans_repeat_until_the_bar_is_full() {
        assert_eq!(bar("4/4", 3, 2), "4/4 t4 (t4) (t4) (t4) (t4) (t4)");
    }

    #[test]
    fn clicks_without_a_value_get_rests_up_to_the_next() {
        // No half-note triplet: quarter triplets, every other one a rest.
        assert_eq!(bar("4/4", 3, 4), "4/4 t4 r:t4 (t4) r:t4 (t4) r:t4");
        assert_eq!(bar("4/4", 2, 4), "4/4 q r:q (q) r:q");
    }

    #[test]
    fn ratios_of_common_meters() {
        let ratios = |ts: &str| {
            counter_ratios(ts.parse().unwrap()).iter().map(|r| r.to_string()).collect::<Vec<_>>()
        };
        assert_eq!(ratios("4/4"), ["3:4", "5:4", "7:4", "3:2"]);
        assert_eq!(ratios("3/4"), ["2:3", "4:3", "5:3"]);
        assert_eq!(ratios("6/8"), ["5:6", "7:6", "2:3", "4:3", "3:2"]);
        assert!(counter_bar(TimeSignature::SEVEN_EIGHT, Ratio { n: 8, span: 7 }).is_none());
    }

    #[test]
    fn ratio_of_ignores_levels() {
        let ts = TimeSignature::FOUR_FOUR;
        let mut m = counter_bar(ts, Ratio { n: 3, span: 4 }).unwrap();
        m.set_click_level(2, ClickLevel::Off);
        assert_eq!(ratio_of(&m), Some(Ratio { n: 3, span: 4 }));
        assert_eq!(ratio_of(&crate::click::default_click_bar(ts)), None);
    }

    #[test]
    fn clicks_fold_in_their_rests() {
        let ts = TimeSignature::FOUR_FOUR;
        let mut m = counter_bar(ts, Ratio { n: 3, span: 4 }).unwrap();
        assert_eq!(counter_clicks(&m), [0..2, 2..4, 4..6]);
        // A click tapped off stays a click.
        m.set_click_level(2, ClickLevel::Off);
        assert_eq!(counter_clicks(&m), [0..2, 2..4, 4..6]);
        assert_eq!(click_level(&m, 2), ClickLevel::Off);
    }

    #[test]
    fn bars_are_kept_per_meter() {
        let mut bars = CounterBars::default();
        let four = TimeSignature::FOUR_FOUR;
        bars.set(four, counter_bar(four, Ratio { n: 3, span: 4 }));
        assert!(bars.for_meter(four).is_some());
        assert!(bars.for_meter(TimeSignature::THREE_FOUR).is_none());
        // A bar of another meter is not stored.
        bars.set(TimeSignature::THREE_FOUR, counter_bar(four, Ratio { n: 5, span: 4 }));
        assert!(bars.for_meter(TimeSignature::THREE_FOUR).is_none());
        bars.set(four, None);
        assert_eq!(bars, CounterBars::default());
    }
}
