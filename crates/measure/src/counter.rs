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
use crate::duration::{Duration, TupletSpec, dt32, nt8_10, nt16_10, nt32};
use crate::grid::DEFAULT_GRID;
use crate::math::gcd;
use crate::notation::{code_of, parse_score};
use crate::{BeatKind, Measure, TimeSignature};
use serde::{Deserialize, Serialize};

/// Most clicks a counter rhythm offers per span.
const MAX_CLICKS: u8 = 9;
/// Most clicks per span of a ratio of one's own (see [`own_ratios`]).
const MAX_OWN_CLICKS: u8 = 16;
/// Most notes and rests written for one click, so a click is never spelled
/// as a long row of tiny rests. Seven lets a click span an odd number of
/// beats in 5/4 and 7/4: 2:5 is an eighth and four eighth rests.
const MAX_SLOTS_PER_CLICK: u32 = 7;

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

/// The counter bar of `ratio` in `ts`, every click medium (the click bar
/// already clicks the downbeat loud). `None` if the
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
    let beat_ticks = DEFAULT_GRID.ticks_per_beat(&ts);
    let mut durations: Vec<_> = DEFAULT_GRID
        .durations
        .iter()
        .filter_map(|d| Some((DEFAULT_GRID.ticks_of(d)?, *d)))
        .filter(|&(t, _)| click_ticks.is_multiple_of(t) && click_ticks / t <= MAX_SLOTS_PER_CLICK)
        .filter(|&(t, d)| fits_the_beats(d, t, beat_ticks))
        .collect();
    // Longest first, but the nonuplets and decuplets only where nothing else
    // fits: 3:5 stays triplets. Of equally long tuplets, the one whose group
    // fills a beat: sextuplets in 5/4, triplets in 7/8.
    let last_resort = |d: Duration| [nt8_10(), nt16_10(), nt32(), dt32()].contains(&d);
    let off_the_beat = |t: u32, d: Duration| match d {
        Duration::Tuplet(TupletSpec { n, .. }) => t * u32::from(n) != beat_ticks,
        _ => false,
    };
    durations.sort_by_key(|&(t, d)| (last_resort(d), std::cmp::Reverse(t), off_the_beat(t, d)));
    let repeats = ts.beats / span;
    durations.into_iter().find_map(|(ticks, d)| {
        let code = code_of(&d)?;
        let slots = click_ticks / ticks;
        let mut tokens = Vec::new();
        for _ in 0..(repeats as u32 * n as u32) {
            tokens.push(code.to_string());
            tokens.extend((1..slots).map(|_| format!("r:{code}")));
        }
        parse_score(&format!("{ts} {}", tokens.join(" "))).ok()?.measures.into_iter().next()
    })
}

/// Whether a group of tuplet `d` (each `ticks` long) lies on the beats:
/// whole beats, or a part of one. A quintuplet over a dotted quarter fits
/// 6/8 but would straddle the second beat of 3/4.
fn fits_the_beats(d: Duration, ticks: u32, beat_ticks: u32) -> bool {
    let Duration::Tuplet(TupletSpec { n, .. }) = d else {
        return true;
    };
    let group = ticks * u32::from(n);
    group.is_multiple_of(beat_ticks) || beat_ticks.is_multiple_of(group)
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

/// Every ratio of one's own in `ts`: any span of beat units that divides
/// the bar with 2 to 16 clicks, no common factor, and a counter bar. The
/// offered ones ([`counter_ratios`]) are among them. Ordered by span, then
/// clicks.
pub fn own_ratios(ts: TimeSignature) -> Vec<Ratio> {
    let mut ratios = Vec::new();
    for span in (1..=ts.beats).filter(|&s| ts.beats.is_multiple_of(s)) {
        for n in 2..=MAX_OWN_CLICKS {
            let ratio = Ratio { n, span };
            if gcd(n as u32, span as u32) == 1 && counter_bar(ts, ratio).is_some() {
                ratios.push(ratio);
            }
        }
    }
    ratios
}

/// The ratio of `own` (a meter's [`own_ratios`]) next to `ratio` with more
/// clicks (`up`) or fewer in the same span, if any.
pub fn step_clicks(own: &[Ratio], ratio: Ratio, up: bool) -> Option<Ratio> {
    let same_span = own.iter().copied().filter(|r| r.span == ratio.span);
    if up {
        same_span.filter(|r| r.n > ratio.n).min_by_key(|r| r.n)
    } else {
        same_span.filter(|r| r.n < ratio.n).max_by_key(|r| r.n)
    }
}

/// The ratio of `own` (a meter's [`own_ratios`]) with the next longer
/// (`up`) or shorter span than `ratio`'s, its clicks as near `ratio`'s as
/// that span allows (the fewer on a tie), if any.
pub fn step_span(own: &[Ratio], ratio: Ratio, up: bool) -> Option<Ratio> {
    let span = if up {
        own.iter().map(|r| r.span).filter(|&s| s > ratio.span).min()
    } else {
        own.iter().map(|r| r.span).filter(|&s| s < ratio.span).max()
    }?;
    own.iter().copied().filter(|r| r.span == span).min_by_key(|r| (r.n.abs_diff(ratio.n), r.n))
}

/// The ratio whose counter bar `bar` is, whatever the levels of its clicks:
/// the same note values, with notes only where that ratio clicks (a click
/// tapped off is a rest). Ratios with the same note values (9:4 and 9:2 in
/// 4/4 are both nonuplet sixteenths) tell apart by where their rests are; the
/// one with the fewest clicks wins, if any. Any ratio of one's own counts
/// (see [`own_ratios`]).
pub fn ratio_of(bar: &Measure) -> Option<Ratio> {
    let ts = bar.time_signature();
    // Only ratios that click wherever a note of `bar` starts can fit: that
    // spares building every ratio's counter bar.
    let ticks_per_beat = DEFAULT_GRID.ticks_per_beat(&ts);
    let onsets = DEFAULT_GRID.compute_onset_ticks(bar.beats());
    let note_onsets: Vec<u32> = onsets
        .iter()
        .zip(bar.beats())
        .filter(|(_, b)| b.kind == BeatKind::Note)
        .map(|(&t, _)| t)
        .collect();
    let clicks_on_notes = |r: &Ratio| {
        let span_ticks = u32::from(r.span) * ticks_per_beat;
        span_ticks.is_multiple_of(u32::from(r.n)) && {
            let click_ticks = span_ticks / u32::from(r.n);
            note_onsets.iter().all(|t| t.is_multiple_of(click_ticks))
        }
    };
    let fits = |m: &Measure| {
        m.beats().len() == bar.beats().len()
            && m.beats().iter().zip(bar.beats()).all(|(c, b)| {
                c.duration == b.duration && (c.kind == BeatKind::Note || b.kind == BeatKind::Rest)
            })
    };
    (1..=ts.beats)
        .filter(|&span| ts.beats.is_multiple_of(span))
        .flat_map(|span| (2..=MAX_OWN_CLICKS).map(move |n| Ratio { n, span }))
        .filter(|r| gcd(u32::from(r.n), u32::from(r.span)) == 1 && clicks_on_notes(r))
        .filter(|&r| counter_bar(ts, r).is_some_and(|m| fits(&m)))
        .min_by_key(|r| u32::from(r.n) * u32::from(ts.beats / r.span))
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
        assert_eq!(bar("3/4", 2, 3), "3/4 q. q.");
        assert_eq!(bar("3/4", 4, 3), "3/4 e. e. e. e.");
        assert_eq!(bar("6/8", 4, 6), "6/8 e. e. e. e.");
        assert_eq!(bar("2/4", 3, 2), "2/4 t4 t4 t4");
        assert_eq!(bar("4/4", 5, 4), "4/4 qt4 qt4 qt4 qt4 qt4");
        assert_eq!(bar("4/4", 7, 4), "4/4 spt4 spt4 spt4 spt4 spt4 spt4 spt4");
        assert_eq!(bar("4/4", 3, 4), "4/4 t2 t2 t2");
        assert_eq!(bar("4/4", 2, 4), "4/4 h h");
        assert_eq!(bar("2/4", 5, 2), "2/4 qt8 qt8 qt8 qt8 qt8");
        assert_eq!(bar("2/4", 7, 2), "2/4 spt8 spt8 spt8 spt8 spt8 spt8 spt8");
    }

    #[test]
    fn clicks_over_an_odd_number_of_beats_rest_up_to_the_next() {
        assert_eq!(bar("5/4", 2, 5), "5/4 e r:e r:e r:e r:e e r:e r:e r:e r:e");
        assert_eq!(
            bar("5/4", 3, 5),
            "5/4 t8 r:t8 r:t8 r:t8 r:t8 t8 r:t8 r:t8 r:t8 r:t8 t8 r:t8 r:t8 r:t8 r:t8"
        );
        assert_eq!(bar("7/4", 2, 7), "7/4 e r:e r:e r:e r:e r:e r:e e r:e r:e r:e r:e r:e r:e");
        let offered = |ts: &str| {
            counter_ratios(ts.parse().unwrap()).iter().map(|r| r.to_string()).collect::<Vec<_>>()
        };
        assert_eq!(offered("5/4"), ["3:5", "4:5", "6:5", "7:5", "8:5", "9:5"]);
        assert_eq!(offered("7/4"), ["4:7", "5:7", "6:7", "8:7", "9:7"]);
    }

    #[test]
    fn equally_long_tuplets_take_the_one_that_fills_a_beat() {
        // Sextuplet sixteenths fill a quarter, triplet ones an eighth.
        assert_eq!(bar("1/4", 6, 1), "1/4 st16 st16 st16 st16 st16 st16");
        assert!(bar("5/4", 6, 5).starts_with("5/4 st16 r:st16"));
        assert!(bar("7/8", 3, 7).starts_with("7/8 t16 r:t16"));
    }

    #[test]
    fn nine_or_ten_clicks_a_beat_are_thirty_seconds() {
        let own = own_ratios("1/4".parse().unwrap());
        assert!(own.contains(&Ratio { n: 9, span: 1 }) && own.contains(&Ratio { n: 10, span: 1 }));
        assert_eq!(bar("1/4", 9, 1), format!("1/4 {}", ["nt32"; 9].join(" ")));
        assert_eq!(bar("1/4", 10, 1), format!("1/4 {}", ["dt32"; 10].join(" ")));
    }

    #[test]
    fn nine_clicks_over_five_beats() {
        let nine = |ts: &str, base: &str| format!("{ts} {}", [base; 9].join(" "));
        assert_eq!(bar("5/4", 9, 5), nine("5/4", "nt8_10"));
        assert_eq!(bar("5/8", 9, 5), nine("5/8", "nt16_10"));
        assert!(counter_ratios("5/4".parse().unwrap()).contains(&Ratio { n: 9, span: 5 }));
        assert_eq!(
            ratio_of(&counter_bar("10/8".parse().unwrap(), Ratio { n: 9, span: 10 }).unwrap()),
            Some(Ratio { n: 9, span: 10 })
        );
    }

    #[test]
    fn compound_meters_click_five_or_seven_over_a_dotted_quarter() {
        assert_eq!(
            bar("6/8", 5, 3),
            "6/8 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3"
        );
        assert_eq!(bar("3/8", 7, 3), "3/8 spt16_6 spt16_6 spt16_6 spt16_6 spt16_6 spt16_6 spt16_6");
        assert_eq!(
            ratio_of(&counter_bar("12/8".parse().unwrap(), Ratio { n: 5, span: 3 }).unwrap()),
            Some(Ratio { n: 5, span: 3 })
        );
        let offered = |ts: &str| {
            counter_ratios(ts.parse().unwrap()).iter().map(|r| r.to_string()).collect::<Vec<_>>()
        };
        assert!(offered("9/8").contains(&"5:3".to_string()));
        let own = own_ratios("6/8".parse().unwrap());
        assert!(own.contains(&Ratio { n: 7, span: 3 }));
    }

    #[test]
    fn spans_repeat_until_the_bar_is_full() {
        assert_eq!(bar("4/4", 3, 2), "4/4 t4 t4 t4 t4 t4 t4");
    }

    #[test]
    fn clicks_without_a_value_get_rests_up_to_the_next() {
        // No quintuplet over a dotted half: sixteenth quintuplets, two
        // rests after every click.
        assert_eq!(
            bar("3/4", 5, 3),
            "3/4 qt16 r:qt16 r:qt16 qt16 r:qt16 r:qt16 qt16 r:qt16 r:qt16 qt16 r:qt16 \
             r:qt16 qt16 r:qt16 r:qt16"
        );
    }

    #[test]
    fn ratios_of_common_meters() {
        let ratios = |ts: &str| {
            counter_ratios(ts.parse().unwrap()).iter().map(|r| r.to_string()).collect::<Vec<_>>()
        };
        assert_eq!(ratios("4/4"), ["3:4", "5:4", "7:4", "3:2"]);
        assert_eq!(ratios("3/4"), ["2:3", "4:3", "5:3"]);
        assert_eq!(ratios("6/8"), ["5:6", "7:6", "2:3", "4:3", "5:3", "3:2"]);
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
    fn every_offered_ratio_reads_back_as_itself() {
        for unit in [2, 4, 8, 16] {
            for beats in 1..=16 {
                let Ok(ts) = format!("{beats}/{unit}").parse::<TimeSignature>() else { continue };
                for ratio in own_ratios(ts) {
                    let bar = counter_bar(ts, ratio).unwrap();
                    assert_eq!(ratio_of(&bar), Some(ratio), "{ratio} in {ts}");
                }
            }
        }
    }

    #[test]
    fn own_ratios_include_the_offered_ones() {
        for ts in [TimeSignature::FOUR_FOUR, TimeSignature::SIX_EIGHT, TimeSignature::SEVEN_EIGHT] {
            let own = own_ratios(ts);
            assert!(counter_ratios(ts).iter().all(|r| own.contains(r)), "{ts}");
        }
        let own: Vec<_> =
            own_ratios(TimeSignature::FOUR_FOUR).iter().map(|r| r.to_string()).collect();
        assert_eq!(
            own,
            [
                "2:1", "3:1", "4:1", "5:1", "6:1", "7:1", "8:1", "9:1", "10:1", "12:1", "3:2",
                "5:2", "7:2", "9:2", "3:4", "5:4", "7:4", "9:4"
            ]
        );
    }

    #[test]
    fn steps_skip_ratios_without_a_bar() {
        let own = own_ratios(TimeSignature::FOUR_FOUR);
        let r = |n, span| Ratio { n, span };
        // 4:4 has a common factor, so from 3:4 up is 5:4.
        assert_eq!(step_clicks(&own, r(3, 4), true), Some(r(5, 4)));
        assert_eq!(step_clicks(&own, r(3, 4), false), None);
        assert_eq!(step_span(&own, r(3, 4), false), Some(r(3, 2)));
        // In two beats 4 has a common factor: 3 and 5 are as near, the
        // fewer clicks win.
        assert_eq!(step_span(&own, r(4, 1), true), Some(r(3, 2)));
        assert_eq!(step_span(&own, r(3, 4), true), None);
        // Every step lands on a ratio of one's own.
        for &ratio in &own {
            for up in [false, true] {
                for step in [step_clicks(&own, ratio, up), step_span(&own, ratio, up)] {
                    assert!(step.is_none_or(|s| own.contains(&s)));
                }
            }
        }
    }

    #[test]
    fn ratios_with_the_same_note_values_tell_apart() {
        // 9:4 and 9:2 in 4/4 are both eighteen nonuplet sixteenths.
        let ts = TimeSignature::FOUR_FOUR;
        for ratio in [Ratio { n: 9, span: 4 }, Ratio { n: 9, span: 2 }] {
            assert_eq!(ratio_of(&counter_bar(ts, ratio).unwrap()), Some(ratio));
        }
        let mut m = counter_bar(ts, Ratio { n: 9, span: 2 }).unwrap();
        m.set_click_level(1, ClickLevel::Off);
        assert_eq!(ratio_of(&m), Some(Ratio { n: 9, span: 2 }));
    }

    #[test]
    fn clicks_fold_in_their_rests() {
        let ts = TimeSignature::FOUR_FOUR;
        let mut m = counter_bar(ts, Ratio { n: 9, span: 4 }).unwrap();
        let pairs: Vec<_> = (0..9).map(|i| 2 * i..2 * i + 2).collect();
        assert_eq!(counter_clicks(&m), pairs);
        // A click tapped off stays a click.
        m.set_click_level(2, ClickLevel::Off);
        assert_eq!(counter_clicks(&m), pairs);
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
