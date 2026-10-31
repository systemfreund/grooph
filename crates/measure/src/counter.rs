//! Counter rhythms: a second click bar the metronome plays against its click
//! bar, for polyrhythms like three against four.
//!
//! A counter rhythm is [`Ratio`] `n` even clicks in the time of `span` beat
//! units, repeated until the bar is full, written as a measure of the meter
//! like a click bar (see [`crate::click`]): its notes click at their level,
//! rests are silent. The clicks take the simplest note value that is exactly
//! as long; where there is none (three against four has no half-note
//! triplet), a shorter value with rests up to the next click, and where
//! that fails too, a tuplet of the ratio over the span (7:5 in 5/8 is seven
//! eighths in the time of five). [`CounterBars`]
//! keeps the counter rhythm of each meter that has one.

use crate::click::{ClickLevel, same_bar};
use crate::duration::NoteValue::{Eighth, Half, Quarter, Sixteenth, ThirtySecond};
use crate::duration::{Duration, TupletSpec};
use crate::grid::{DEFAULT_GRID, is_general_tuplet};
use crate::math::gcd;
use crate::notation::{code_of, parse_score};
use crate::{Measure, TimeSignature};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;

/// Most clicks a counter rhythm offers per span.
const MAX_CLICKS: u8 = 9;
/// Most clicks per span of a ratio of one's own (see [`own_ratios`]).
const MAX_OWN_CLICKS: u8 = 16;
/// Most notes and rests written for one click, so a click is never spelled
/// as a long row of tiny rests. Seven lets a click span an odd number of
/// beats in 5/4 and 7/4: 2:5 is an eighth and four eighth rests.
const MAX_SLOTS_PER_CLICK: u32 = 7;

/// `n` even clicks in the time of `span` beat units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ratio {
    pub n: u8,
    pub span: u8,
}

impl std::fmt::Display for Ratio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.n, self.span)
    }
}

impl std::str::FromStr for Ratio {
    type Err = String;

    /// `n:span`, e.g. `3:2`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let number = |t: &str| t.trim().parse::<u8>().ok().filter(|&v| v > 0);
        let (n, span) = s.split_once(':').unwrap_or((s, ""));
        match (number(n), number(span)) {
            (Some(n), Some(span)) => Ok(Ratio { n, span }),
            _ => Err(format!("'{s}' is no ratio such as 3:2")),
        }
    }
}

/// Whether `bar` is `ratio`'s own counter bar, no click edited: the one
/// rule for showing it as the ratio, in the app and in links (`poly`).
pub fn is_ratio_bar(bar: &Measure, ratio: Ratio) -> bool {
    counter_bar(bar.time_signature(), ratio).is_some_and(|m| same_bar(&m, bar))
}

/// The counter bar of `ratio` in `ts`, every click medium (the click bar
/// already clicks the downbeat loud). `None` if the
/// span does not divide the bar or the clicks fall off the tick grid.
pub fn counter_bar(ts: TimeSignature, ratio: Ratio) -> Option<Measure> {
    type Key = (u8, u8, u8, u8, u8);
    thread_local! {
        // The picker asks for the bars of every ratio on each frame.
        static BARS: RefCell<HashMap<Key, Option<Measure>>> = RefCell::new(HashMap::new());
    }
    let key = (ts.beats, ts.beat_unit, ts.grouping, ratio.n, ratio.span);
    if let Some(bar) = BARS.with_borrow(|bars| bars.get(&key).cloned()) {
        return bar;
    }
    let bar = build_counter_bar(ts, ratio);
    BARS.with_borrow_mut(|bars| bars.insert(key, bar.clone()));
    bar
}

fn build_counter_bar(ts: TimeSignature, ratio: Ratio) -> Option<Measure> {
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
    // Longest first; of equally long tuplets the first listed, its group
    // grown to fill the beat (see `grouped_up`).
    durations.sort_by_key(|&(t, _)| std::cmp::Reverse(t));
    let repeats = ts.beats / span;
    let bar_of = |d: Duration, slots: u32| {
        let code = code_of(&d)?;
        let mut tokens = Vec::new();
        for _ in 0..(repeats as u32 * n as u32) {
            tokens.push(code.clone());
            tokens.extend((1..slots).map(|_| format!("r:{code}")));
        }
        parse_score(&format!("{ts} {}", tokens.join(" "))).ok()?.into_measures().into_iter().next()
    };
    durations
        .into_iter()
        .find_map(|(ticks, d)| bar_of(grouped_up(d, ticks, beat_ticks), click_ticks / ticks))
        .or_else(|| bar_of(spanning_tuplet(ratio, span_ticks, beat_ticks)?, 1))
}

/// Tuplet `d` (each `ticks` long) with its group grown to the largest that
/// lies within a beat, up to nine notes: thirty-second triplets in 4/4 are
/// written as sextuplets, two to a beat.
fn grouped_up(d: Duration, ticks: u32, beat_ticks: u32) -> Duration {
    let Duration::Tuplet(TupletSpec { n, m, base }) = d else {
        return d;
    };
    (2..=MAX_CLICKS / n)
        .rev()
        .map(|k| Duration::Tuplet(TupletSpec { n: k * n, m: k * m, base }))
        .find(|grown| {
            let group = ticks * u32::from(grown.as_tuplet_spec().map_or(0, |s| s.n));
            beat_ticks.is_multiple_of(group) && DEFAULT_GRID.supports(grown)
        })
        .unwrap_or(d)
}

/// The tuplet of `ratio` whose group fills the span, `n` notes in the time
/// of `m` of a note value: one that is beamed (an eighth or shorter), with
/// fewer than `n` in the time if the note values allow, and `m` as near `n`
/// as they allow. 7:5 in 5/8 is seven eighths in the time of five, 8:5
/// eight eighths in the time of five, 5:7 in 7/8 five in the time of seven,
/// 9:5 in 5/4 nine eighths in the time of ten. Where `n` and `m` share a factor, the smaller groups if they lie
/// on the beats: 14:1 is two groups of seven thirty-seconds in the time of four.
fn spanning_tuplet(ratio: Ratio, span_ticks: u32, beat_ticks: u32) -> Option<Duration> {
    let (n, base, m) = [Half, Quarter, Eighth, Sixteenth, ThirtySecond]
        .into_iter()
        .filter_map(|base| {
            let base_ticks = DEFAULT_GRID.ticks_of(&Duration::Simple(base))?;
            let m = u8::try_from(span_ticks / base_ticks).ok()?;
            let d = Duration::Tuplet(TupletSpec { n: ratio.n, m, base });
            (span_ticks.is_multiple_of(base_ticks) && is_general_tuplet(&d))
                .then_some((ratio.n, base, m))
        })
        .min_by_key(|&(n, base, m)| (base.denominator() < 8, n < m, m.abs_diff(n)))?;
    let g = gcd(u32::from(n), u32::from(m)) as u8;
    let reduced = Duration::Tuplet(TupletSpec { n: n / g, m: m / g, base });
    let ticks = DEFAULT_GRID.ticks_of(&reduced)?;
    if g > 1 && is_general_tuplet(&reduced) && fits_the_beats(reduced, ticks, beat_ticks) {
        Some(reduced)
    } else {
        Some(Duration::Tuplet(TupletSpec { n, m, base }))
    }
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
/// the bar with 2 to 16 clicks and a counter bar, common factor or not (4:2
/// plays as 2:1, but is there to pick) as long as it has two clicks or
/// more without it (not 4:4 or 2:4). The offered ones
/// ([`counter_ratios`]) are among them. Ordered by span, then clicks.
pub fn own_ratios(ts: TimeSignature) -> Vec<Ratio> {
    let mut ratios = Vec::new();
    for span in (1..=ts.beats).filter(|&s| ts.beats.is_multiple_of(s)) {
        for n in 2..=MAX_OWN_CLICKS {
            let ratio = Ratio { n, span };
            // 2:4 would be 1:2, a single click.
            let reduced_n = n as u32 / gcd(n as u32, span as u32);
            if reduced_n > 1 && counter_bar(ts, ratio).is_some() {
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
/// one with the fewest clicks wins, if any. Any ratio of one's own without
/// a common factor counts (see [`own_ratios`]).
pub fn ratio_of(bar: &Measure) -> Option<Ratio> {
    named_ratio_of(bar, None)
}

/// [`ratio_of`], but `named` if `bar` is that ratio's too: 4:2 picked in
/// 2/4 stays 4:2, though its bar is 2:1's.
pub fn named_ratio_of(bar: &Measure, named: Option<Ratio>) -> Option<Ratio> {
    let ts = bar.time_signature();
    // Only ratios that click wherever a note of `bar` starts can fit: that
    // spares building every ratio's counter bar.
    let ticks_per_beat = DEFAULT_GRID.ticks_per_beat(&ts);
    let onsets = DEFAULT_GRID.compute_onset_ticks(bar.first_voice().beats());
    let note_onsets: Vec<u32> = onsets
        .iter()
        .zip(bar.first_voice().beats())
        .filter(|(_, b)| b.is_note())
        .map(|(&t, _)| t)
        .collect();
    let clicks_on_notes = |r: &Ratio| {
        let span_ticks = u32::from(r.span) * ticks_per_beat;
        span_ticks.is_multiple_of(u32::from(r.n)) && {
            let click_ticks = span_ticks / u32::from(r.n);
            note_onsets.iter().all(|t| t.is_multiple_of(click_ticks))
        }
    };
    // By length, not spelling: a bar saved before 6:5 in 5/4 was written in
    // sextuplets (triplets then) is still that ratio's.
    let ticks = |b: &crate::Beat| DEFAULT_GRID.ticks_of(&b.duration);
    let fits = |m: &Measure| {
        m.first_voice().beats().len() == bar.first_voice().beats().len()
            && m.first_voice()
                .beats()
                .iter()
                .zip(bar.first_voice().beats())
                .all(|(c, b)| ticks(c) == ticks(b) && (c.is_note() || b.is_rest()))
    };
    let is_bars = |r: &Ratio| {
        ts.beats.is_multiple_of(r.span)
            && clicks_on_notes(r)
            && counter_bar(ts, *r).is_some_and(|m| fits(&m))
    };
    named.filter(|r| r.span > 0 && is_bars(r)).or_else(|| {
        (1..=ts.beats)
            .filter(|&span| ts.beats.is_multiple_of(span))
            .flat_map(|span| (2..=MAX_OWN_CLICKS).map(move |n| Ratio { n, span }))
            .filter(|r| gcd(u32::from(r.n), u32::from(r.span)) == 1)
            .filter(is_bars)
            .min_by_key(|r| u32::from(r.n) * u32::from(ts.beats / r.span))
    })
}

/// Index of each click of `bar` (its first beat) and the beats it spans:
/// a click and the rests up to the next one. A rest a tap has made of a
/// click stays its own click, so only the rests the counter bar was
/// written with are folded in.
pub fn counter_clicks(bar: &Measure) -> Vec<std::ops::Range<usize>> {
    let ts = bar.time_signature();
    let slots = ratio_of(bar)
        .and_then(|r| counter_bar(ts, r))
        .map(|m| m.first_voice().beats().iter().map(|b| b.is_note()).collect::<Vec<_>>());
    let starts: Vec<usize> = match slots {
        Some(is_click) => (0..bar.first_voice().beats().len()).filter(|&i| is_click[i]).collect(),
        None => (0..bar.first_voice().beats().len()).collect(),
    };
    let mut ends: Vec<usize> = starts.iter().skip(1).copied().collect();
    ends.push(bar.first_voice().beats().len());
    starts.into_iter().zip(ends).map(|(s, e)| s..e).collect()
}

/// The level of the click starting at `idx` of a counter bar.
pub fn click_level(bar: &Measure, idx: usize) -> ClickLevel {
    bar.first_voice().beats().get(idx).map_or(ClickLevel::Off, ClickLevel::of)
}

/// `bar` in the spelling [`counter_bar`] writes its ratio with today, its
/// clicks at their levels; unchanged if it is no ratio's. Bars saved or
/// linked before a spelling changed (6:5 in 5/4 was triplets) keep working.
pub fn respelled(bar: Measure) -> Measure {
    let Some(mut current) = ratio_of(&bar).and_then(|r| counter_bar(bar.time_signature(), r))
    else {
        return bar;
    };
    for (idx, b) in bar.first_voice().beats().iter().enumerate() {
        current.first_voice_mut().set_click_level(idx, ClickLevel::of(b));
    }
    current
}

/// The counter bar of each meter that has one, and the ratio it was picked
/// as where that has a common factor (4:2 plays 2:1's bar). Saved as the
/// bars alone; the names are kept apart (see [`CounterBars::names`]).
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(from = "Vec<Measure>", into = "Vec<Measure>")]
pub struct CounterBars {
    bars: Vec<Measure>,
    names: Vec<(TimeSignature, Ratio)>,
}

impl From<Vec<Measure>> for CounterBars {
    fn from(bars: Vec<Measure>) -> Self {
        Self { bars: bars.into_iter().map(respelled).collect(), names: Vec::new() }
    }
}

impl From<CounterBars> for Vec<Measure> {
    fn from(bars: CounterBars) -> Self {
        bars.bars
    }
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
            && self.names == other.names
    }
}

impl Eq for CounterBars {}

impl CounterBars {
    /// The counter bar of `ts`, `None` if it has none.
    pub fn for_meter(&self, ts: TimeSignature) -> Option<&Measure> {
        self.bars.iter().find(|m| m.time_signature() == ts)
    }

    /// Give `ts` the counter bar `bar` (of that meter), or none. The ratio
    /// it was picked as stays while `bar` is that ratio's.
    pub fn set(&mut self, ts: TimeSignature, bar: Option<Measure>) {
        self.bars.retain(|m| m.time_signature() != ts);
        self.bars.extend(bar.filter(|m| m.time_signature() == ts).map(respelled));
        let named = self.name(ts);
        let kept = self.for_meter(ts).and_then(|bar| named_ratio_of(bar, named));
        self.names.retain(|&(t, _)| t != ts);
        self.names.extend(kept.filter(|&r| Some(r) == named).map(|r| (ts, r)));
    }

    /// Give `ts` the counter bar `bar` from elsewhere (a link), named as
    /// its simplest ratio, whatever `ts`'s bar was picked as.
    pub fn replace(&mut self, ts: TimeSignature, bar: Option<Measure>) {
        self.names.retain(|&(t, _)| t != ts);
        self.set(ts, bar);
    }

    /// Give `ts` the counter bar of `ratio`, picked as that ratio, or none.
    pub fn set_ratio(&mut self, ts: TimeSignature, ratio: Option<Ratio>) {
        self.bars.retain(|m| m.time_signature() != ts);
        self.bars.extend(ratio.and_then(|r| counter_bar(ts, r)));
        self.names.retain(|&(t, _)| t != ts);
        self.names.extend(ratio.filter(|_| self.for_meter(ts).is_some()).map(|r| (ts, r)));
    }

    /// The ratio `ts`'s counter bar was picked as, if any.
    pub fn name(&self, ts: TimeSignature) -> Option<Ratio> {
        self.names.iter().find(|&&(t, _)| t == ts).map(|&(_, r)| r)
    }

    /// The ratio `ts`'s counter bar plays: the one it was picked as, else
    /// the simplest (see [`named_ratio_of`]).
    pub fn ratio(&self, ts: TimeSignature) -> Option<Ratio> {
        named_ratio_of(self.for_meter(ts)?, self.name(ts))
    }

    /// The ratio each bar was picked as, to save next to the bars.
    pub fn names(&self) -> &[(TimeSignature, Ratio)] {
        &self.names
    }

    /// These bars, picked as `names` where they are those ratios' bars.
    pub fn with_names(mut self, names: &[(TimeSignature, Ratio)]) -> Self {
        for &(ts, r) in names {
            if self.for_meter(ts).is_some_and(|bar| named_ratio_of(bar, Some(r)) == Some(r)) {
                self.names.retain(|&(t, _)| t != ts);
                self.names.push((ts, r));
            }
        }
        self
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
        // Thirty-second triplets grow to sextuplets, two to a beat.
        assert_eq!(bar("1/4", 12, 1), format!("1/4 {}", ["6:4th"; 12].join(" ")));
        assert_eq!(bar("3/8", 6, 1), format!("3/8 {}", ["6:4th"; 18].join(" ")));
        assert!(bar("5/8", 6, 5).starts_with("5/8 6:4th r:6:4th"));
        let m = counter_bar("1/4".parse().unwrap(), Ratio { n: 12, span: 1 }).unwrap();
        assert_eq!(m.first_voice().tuplet_groups().len(), 2);
        assert_eq!(ratio_of(&m), Some(Ratio { n: 12, span: 1 }));
    }

    #[test]
    fn bars_saved_in_an_older_spelling_keep_their_ratio_and_levels() {
        // 6:5 in 5/4 as main wrote it: triplet sixteenths.
        let click = "t16 r:t16 r:t16 r:t16 r:t16";
        let old = format!("5/4 {}", [click; 6].join(" "));
        let mut old = parse_score(&old).unwrap().into_measures().remove(0);
        old.first_voice_mut().set_click_level(0, ClickLevel::Strong);
        old.first_voice_mut().set_click_level(5, ClickLevel::Off);
        assert_eq!(ratio_of(&old), Some(Ratio { n: 6, span: 5 }));
        assert_eq!(counter_clicks(&old).len(), 6);
        let new = respelled(old);
        assert!(text(&new).starts_with("5/4 >st16 r:st16 r:st16 r:st16 r:st16 r:st16 r:st16"));
        assert_eq!(click_level(&new, 5), ClickLevel::Off);
        assert_eq!(click_level(&new, 10), ClickLevel::Medium);
        let mut bars = CounterBars::default();
        bars.set("5/4".parse().unwrap(), Some(new.clone()));
        assert!(same_bar(bars.for_meter("5/4".parse().unwrap()).unwrap(), &new));
    }

    #[test]
    fn nine_or_ten_clicks_a_beat_are_thirty_seconds() {
        let own = own_ratios("1/4".parse().unwrap());
        assert!(own.contains(&Ratio { n: 9, span: 1 }) && own.contains(&Ratio { n: 10, span: 1 }));
        assert_eq!(bar("1/4", 9, 1), format!("1/4 {}", ["9:8th"; 9].join(" ")));
        // Ten are two quintuplets, as twelve are two sextuplets.
        assert_eq!(bar("1/4", 10, 1), format!("1/4 {}", ["5:4th"; 10].join(" ")));
    }

    #[test]
    fn nine_clicks_over_five_beats() {
        let nine = |ts: &str, base: &str| format!("{ts} {}", [base; 9].join(" "));
        // Beamed eighths in the time of ten, not quarters in the time of five.
        assert_eq!(bar("5/4", 9, 5), nine("5/4", "9:10e"));
        assert_eq!(bar("5/8", 9, 5), nine("5/8", "9:5e"));
        // One note a click, not a thirty-second and rests.
        assert_eq!(bar("7/4", 9, 7), nine("7/4", "9:14e"));
        assert!(counter_ratios("5/4".parse().unwrap()).contains(&Ratio { n: 9, span: 5 }));
        assert_eq!(
            ratio_of(&counter_bar("10/8".parse().unwrap(), Ratio { n: 9, span: 10 }).unwrap()),
            Some(Ratio { n: 9, span: 10 })
        );
    }

    #[test]
    fn odd_spans_without_a_value_take_a_tuplet_over_the_span() {
        assert_eq!(bar("5/8", 7, 5), "5/8 7:5e 7:5e 7:5e 7:5e 7:5e 7:5e 7:5e");
        assert_eq!(bar("7/8", 5, 7), "7/8 5:7e 5:7e 5:7e 5:7e 5:7e");
        assert_eq!(bar("5/16", 6, 5), "5/16 6:5s 6:5s 6:5s 6:5s 6:5s 6:5s");
        // Eight over five eighths falls on sixty-fourths.
        assert_eq!(bar("5/8", 8, 5), format!("5/8 {}", ["8:5e"; 8].join(" ")));
        assert_eq!(bar("6/16", 4, 3), "6/16 4:3s 4:3s 4:3s 4:3s 4:3s 4:3s 4:3s 4:3s");
        // Fourteen in a beat: two groups of seven, each on half the beat.
        assert_eq!(bar("1/4", 14, 1), format!("1/4 {}", ["7:4th"; 14].join(" ")));
        // Five in an eighth: no quintuplet thirty-seconds in the list.
        assert_eq!(bar("3/8", 5, 1), format!("3/8 {}", ["5:4th"; 15].join(" ")));
        for ts in ["5/8", "7/8", "11/8", "13/8", "5/16", "7/16", "9/16", "10/16", "14/16"] {
            let ts: TimeSignature = ts.parse().unwrap();
            let offered = counter_ratios(ts);
            for span in (2..=ts.beats).filter(|&s| ts.beats.is_multiple_of(s)) {
                for n in (span / 2 + 1..2 * span).filter(|&n| n <= MAX_CLICKS) {
                    let ratio = Ratio { n, span };
                    let span_ticks = u32::from(span) * DEFAULT_GRID.ticks_per_beat(&ts);
                    if gcd(u32::from(n), u32::from(span)) == 1
                        && span_ticks.is_multiple_of(u32::from(n))
                    {
                        assert!(offered.contains(&ratio), "{ratio} in {ts}");
                    }
                }
            }
        }
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
        assert_eq!(bar("7/8", 8, 7), format!("7/8 {}", ["8:7e"; 8].join(" ")));
        assert!(counter_bar(TimeSignature::FOUR_FOUR, Ratio { n: 11, span: 4 }).is_none());
    }

    #[test]
    fn ratio_of_ignores_levels() {
        let ts = TimeSignature::FOUR_FOUR;
        let mut m = counter_bar(ts, Ratio { n: 3, span: 4 }).unwrap();
        m.first_voice_mut().set_click_level(2, ClickLevel::Off);
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
                    let g = gcd(u32::from(ratio.n), u32::from(ratio.span)) as u8;
                    let reduced = Ratio { n: ratio.n / g, span: ratio.span / g };
                    assert_eq!(ratio_of(&bar), Some(reduced), "{ratio} in {ts}");
                    assert_eq!(named_ratio_of(&bar, Some(ratio)), Some(ratio), "{ratio} in {ts}");
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
                "2:1", "3:1", "4:1", "5:1", "6:1", "7:1", "8:1", "9:1", "10:1", "12:1", "14:1",
                "15:1", "3:2", "4:2", "5:2", "6:2", "7:2", "8:2", "9:2", "10:2", "12:2", "14:2",
                "15:2", "16:2", "3:4", "5:4", "6:4", "7:4", "8:4", "9:4", "10:4", "12:4", "14:4",
                "15:4", "16:4"
            ]
        );
    }

    #[test]
    fn steps_skip_ratios_without_a_bar() {
        let own = own_ratios(TimeSignature::FOUR_FOUR);
        let r = |n, span| Ratio { n, span };
        // 4:4 clicks on the beats, so from 3:4 up is 5:4.
        assert_eq!(step_clicks(&own, r(3, 4), true), Some(r(5, 4)));
        assert_eq!(step_clicks(&own, r(3, 4), false), None);
        assert_eq!(step_span(&own, r(3, 4), false), Some(r(3, 2)));
        // The clicks stay as they are, common factor or not. 11 has no bar
        // in two beats: 10 and 12 are as near, the fewer clicks win.
        assert_eq!(step_span(&own, r(4, 1), true), Some(r(4, 2)));
        assert_eq!(step_span(&own, r(11, 1), true), Some(r(10, 2)));
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
        m.first_voice_mut().set_click_level(1, ClickLevel::Off);
        assert_eq!(ratio_of(&m), Some(Ratio { n: 9, span: 2 }));
    }

    #[test]
    fn clicks_fold_in_their_rests() {
        let ts = TimeSignature::FOUR_FOUR;
        let mut m = counter_bar(ts, Ratio { n: 9, span: 4 }).unwrap();
        let pairs: Vec<_> = (0..9).map(|i| 2 * i..2 * i + 2).collect();
        assert_eq!(counter_clicks(&m), pairs);
        // A click tapped off stays a click.
        m.first_voice_mut().set_click_level(2, ClickLevel::Off);
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

    #[test]
    fn a_ratio_with_a_common_factor_keeps_its_name() {
        let ts: TimeSignature = "2/4".parse().unwrap();
        let r = |n, span| Ratio { n, span };
        assert!(own_ratios(ts).contains(&r(4, 2)));
        // Its bar is 2:1's; it is named 4:2 while picked as that.
        let bar = counter_bar(ts, r(4, 2)).unwrap();
        assert_eq!(ratio_of(&bar), Some(r(2, 1)));
        assert_eq!(named_ratio_of(&bar, Some(r(4, 2))), Some(r(4, 2)));
        assert_eq!(named_ratio_of(&bar, Some(r(3, 2))), Some(r(2, 1)));
        let mut bars = CounterBars::default();
        bars.set_ratio(ts, Some(r(4, 2)));
        assert_eq!(bars.ratio(ts), Some(r(4, 2)));
        // Clicks edited, still 4:2; another bar drops the name.
        let mut edited = bar.clone();
        edited.first_voice_mut().set_click_level(1, ClickLevel::Strong);
        bars.set(ts, Some(edited));
        assert_eq!(bars.ratio(ts), Some(r(4, 2)));
        bars.set(ts, counter_bar(ts, r(3, 2)));
        assert_eq!(bars.name(ts), None);
        // Saved as the bars and the names apart.
        bars.set_ratio(ts, Some(r(4, 2)));
        let saved: Vec<Measure> = bars.clone().into();
        let loaded = CounterBars::from(saved).with_names(bars.names());
        assert_eq!(loaded, bars);
        assert_eq!(CounterBars::from(Vec::new()).with_names(bars.names()), CounterBars::default());
        // The same bar from a link is 2:1.
        let mut linked = bars.clone();
        linked.replace(ts, Some(bar.clone()));
        assert_eq!(linked.ratio(ts), Some(r(2, 1)));
        // Picked again as 2:1, it is 2:1.
        bars.set_ratio(ts, Some(r(2, 1)));
        assert_eq!(bars.ratio(ts), Some(r(2, 1)));
    }
}
