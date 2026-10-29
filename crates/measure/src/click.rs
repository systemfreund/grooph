//! Click bars: what the metronome plays in each meter, written as a measure.
//!
//! Every note of a meter's click bar is a click; its level comes from the
//! note: accented is [`ClickLevel::Strong`], plain [`ClickLevel::Medium`],
//! soft (a ghost note) [`ClickLevel::Soft`], and a rest is no click
//! ([`ClickLevel::Off`]). Tuplets and dotted notes work like in any rhythm,
//! so "eighth, then a triplet without its middle note" is just a bar.
//!
//! A meter starts with its [`meter_click_bar`]: x/4 clicks one note per beat
//! group of [`TimeSignature::groups`] ([`default_click_bar`]), x/8 and x/16
//! every eighth or sixteenth. [`ClickBars`] keeps the bars that differ from
//! it, one per meter, so every measure in that meter clicks the same.

use crate::BeatKind::{Note, Rest};
use crate::duration::{Duration, TupletKind, TupletSpec};
use crate::grid::{DEFAULT_GRID, NOTE_VALUES};
use crate::notation::{code_of, parse_score};
use crate::{Beat, BeatIdx, Measure, TimeSignature, VoiceBar};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;

/// How loud one click sounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClickLevel {
    /// The downbeat click: an accented note.
    Strong,
    /// The click on the other primary beats: a plain note.
    Medium,
    /// A quieter click: a soft note.
    Soft,
    /// No click: a rest.
    Off,
}

impl ClickLevel {
    pub const ALL: [ClickLevel; 4] =
        [ClickLevel::Strong, ClickLevel::Medium, ClickLevel::Soft, ClickLevel::Off];

    /// The level a beat of a click bar plays at.
    pub fn of(beat: &Beat) -> Self {
        match (beat.kind(), beat.accented(), beat.soft()) {
            (Rest, ..) => ClickLevel::Off,
            (Note, true, _) => ClickLevel::Strong,
            (Note, false, true) => ClickLevel::Soft,
            (Note, false, false) => ClickLevel::Medium,
        }
    }

    /// The next level when tapping through them: strong, medium, soft, off.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&l| l == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

impl VoiceBar {
    /// Give the beat at `idx` the note, accent and soft flag of `level`. The
    /// duration stays, so the measure stays valid.
    pub fn set_click_level(&mut self, idx: BeatIdx, level: ClickLevel) {
        let Some(b) = self.beats.get_mut(idx) else {
            return;
        };
        b.set_kind(if level == ClickLevel::Off { Rest } else { Note });
        b.set_accented(level == ClickLevel::Strong);
        b.set_soft(level == ClickLevel::Soft);
    }
}

/// The plain or dotted note `ticks` long, if there is one.
fn note_of_ticks(ticks: u32) -> Option<Duration> {
    NOTE_VALUES
        .into_iter()
        .flat_map(|base| [Duration::Simple(base), Duration::Dotted { base, dots: 1 }])
        .find(|d| DEFAULT_GRID.ticks_of(d) == Some(ticks))
}

/// One beat unit of `ts` split into `parts` equal notes: the plain or
/// dotted note of that length, else the tuplet whose group fills the beat,
/// `parts` notes in the time of the largest power of two below it (three
/// eighths in the time of two, five or seven sixteenths in the time of
/// four, nine thirty-seconds in the time of eight).
fn part_duration(ts: &TimeSignature, parts: u32) -> Option<Duration> {
    let beat = DEFAULT_GRID.ticks_per_beat(ts);
    if parts == 0 || !beat.is_multiple_of(parts) {
        return None;
    }
    note_of_ticks(beat / parts).or_else(|| {
        let n = u8::try_from(parts).ok()?;
        let m = 1u8 << n.ilog2();
        let Some(Duration::Simple(base)) = note_of_ticks(beat / u32::from(m)) else {
            return None;
        };
        let d = Duration::Tuplet(TupletSpec { n, m, base });
        (m < n && DEFAULT_GRID.supports(&d)).then_some(d)
    })
}

fn parse_bar(ts: &TimeSignature, tokens: &[String]) -> Option<Measure> {
    let text = format!("{ts} {}", tokens.join(" "));
    parse_score(&text).ok()?.measures.into_iter().next()
}

/// The default click bar of `ts`: one note per beat group, as long as the
/// group, the first accented. A group with no single note value (e.g. four
/// eighths) gets a note on its first beat unit and rests after it.
pub fn default_click_bar(ts: TimeSignature) -> Measure {
    let unit = DEFAULT_GRID.ticks_per_beat(&ts);
    let unit_code = part_duration(&ts, 1).and_then(|d| code_of(&d));
    let mut tokens = Vec::new();
    for (i, &beats) in ts.groups().iter().enumerate() {
        let accent = if i == 0 { ">" } else { "" };
        match (note_of_ticks(beats as u32 * unit).and_then(|d| code_of(&d)), &unit_code) {
            (Some(code), _) => tokens.push(format!("{accent}{code}")),
            (None, Some(code)) => {
                tokens.push(format!("{accent}{code}"));
                tokens.extend((1..beats).map(|_| format!("r:{code}")));
            }
            (None, None) => {}
        }
    }
    parse_bar(&ts, &tokens).unwrap_or_else(|| Measure::new(ts))
}

/// Click bar of `ts` with every beat unit split into `parts` notes (1 is
/// one note per beat unit; see [`part_duration`]). Group starts keep the level of the
/// default bar (first strong, others medium), all other clicks are soft.
/// `None` if the meter cannot be split that way.
pub fn subdivided_click_bar(ts: TimeSignature, parts: u32) -> Option<Measure> {
    let code = code_of(&part_duration(&ts, parts)?)?;
    let mut group_starts = vec![0usize];
    for &g in ts.groups().iter() {
        group_starts.push(group_starts.last().unwrap() + g as usize);
    }
    let mut tokens = Vec::new();
    for unit in 0..ts.beats as usize {
        for part in 0..parts {
            tokens.push(match (unit, part) {
                (0, 0) => format!(">{code}"),
                (u, 0) if group_starts.contains(&u) => code.to_string(),
                _ => format!("({code})"),
            });
        }
    }
    parse_bar(&ts, &tokens)
}

/// Click bar a meter starts with: x/4 clicks its beats, x/8 and x/16 every
/// eighth or sixteenth.
pub fn meter_click_bar(ts: TimeSignature) -> Measure {
    if ts.beat_unit > 4 {
        subdivided_click_bar(ts, 1).unwrap_or_else(|| default_click_bar(ts))
    } else {
        default_click_bar(ts)
    }
}

/// One option of the subdivision picker: a click bar the app builds itself.
#[derive(Clone, Debug)]
pub struct Subdivision {
    /// Its name in links: `none` for one click per beat group, else the
    /// notation code of its notes (`e`, `s`, `t16`, …).
    pub code: String,
    /// "Sixteenths", "Eighth triplets", …; "No subdivision" for `none`.
    pub name: String,
    pub bar: Measure,
}

/// The subdivision picker's options for `ts`: one click per beat group
/// ([`default_click_bar`]), then each beat unit split into 1 to 9 clicks
/// where the meter allows ([`subdivided_click_bar`]). Bars alike are
/// offered once, under the first name. Kept per meter once built: the
/// embedded app asks every frame, and building the bars takes milliseconds.
pub fn subdivisions(ts: TimeSignature) -> Rc<[Subdivision]> {
    thread_local! {
        static OPTIONS: RefCell<Vec<(TimeSignature, Rc<[Subdivision]>)>> = const { RefCell::new(Vec::new()) };
    }
    OPTIONS.with_borrow_mut(|cache| {
        if let Some((_, options)) = cache.iter().find(|(t, _)| *t == ts) {
            return Rc::clone(options);
        }
        let options: Rc<[Subdivision]> = build_subdivisions(ts).into();
        cache.push((ts, Rc::clone(&options)));
        options
    })
}

fn build_subdivisions(ts: TimeSignature) -> Vec<Subdivision> {
    let mut options = vec![Subdivision {
        code: "none".to_owned(),
        name: "No subdivision".to_owned(),
        bar: default_click_bar(ts),
    }];
    for parts in 1..=9 {
        let Some(bar) = subdivided_click_bar(ts, parts) else {
            continue;
        };
        if options.iter().any(|o| same_bar(&o.bar, &bar)) {
            continue;
        }
        let Some(&first) = bar.beats().first() else { continue };
        let Some(code) = code_of(&first.duration) else { continue };
        options.push(Subdivision { code, name: subdivision_name(first.duration), bar });
    }
    options
}

/// The subdivision `bar` is, if the picker offers it in its meter.
pub fn subdivision_of(bar: &Measure) -> Option<Subdivision> {
    subdivisions(bar.time_signature()).iter().find(|o| same_bar(&o.bar, bar)).cloned()
}

/// "Sixteenths", "Eighth triplets", "Quintuplets": clicks of duration `d`.
fn subdivision_name(d: Duration) -> String {
    let base = d.base_note().name();
    match d.as_tuplet_spec().map(|t| t.kind()) {
        Some(TupletKind::Triplet) => format!("{base} triplets"),
        Some(kind) => kind.name().map_or_else(|| format!("{base}s"), |k| format!("{k}s")),
        None => format!("{base}s"),
    }
}

/// Whether two bars hold the same meter and beats, in every voice.
pub(crate) fn same_bar(a: &Measure, b: &Measure) -> bool {
    a.time_signature() == b.time_signature()
        && a.voices().len() == b.voices().len()
        && a.voices().iter().zip(b.voices()).all(|(u, v)| u.beats() == v.beats())
}

/// The click bar of every meter: its [`meter_click_bar`] unless one was set.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClickBars {
    /// Bars that differ from the default, at most one per meter.
    custom: Vec<Measure>,
}

impl std::fmt::Debug for ClickBars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.custom.iter().map(|m| m.time_signature())).finish()
    }
}

impl PartialEq for ClickBars {
    fn eq(&self, other: &Self) -> bool {
        self.custom.len() == other.custom.len()
            && self.custom.iter().zip(&other.custom).all(|(a, b)| same_bar(a, b))
    }
}

impl Eq for ClickBars {}

impl ClickBars {
    /// The bar `ts` clicks.
    pub fn for_meter(&self, ts: TimeSignature) -> Measure {
        self.custom_for(ts).cloned().unwrap_or_else(|| meter_click_bar(ts))
    }

    /// The bar of `ts` if it is not its [`meter_click_bar`].
    pub fn custom_for(&self, ts: TimeSignature) -> Option<&Measure> {
        self.custom.iter().find(|m| m.time_signature() == ts)
    }

    /// Set the bar of its meter; the meter's own bar is not stored.
    pub fn set(&mut self, bar: Measure) {
        let ts = bar.time_signature();
        self.custom.retain(|m| m.time_signature() != ts);
        if !same_bar(&bar, &meter_click_bar(ts)) {
            self.custom.push(bar);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notation::format_score;
    use crate::{Score, TimeSignature};

    fn text(m: &Measure) -> String {
        format_score(&Score::single(m.clone())).unwrap()
    }

    #[test]
    fn default_bar_has_one_note_per_group() {
        assert_eq!(text(&default_click_bar(TimeSignature::FOUR_FOUR)), "4/4 >q q q q");
        assert_eq!(text(&default_click_bar(TimeSignature::SIX_EIGHT)), "6/8 >q. q.");
        // 7/8 as 2+3+2.
        assert_eq!(text(&default_click_bar(TimeSignature::SEVEN_EIGHT)), "7/8 >q q. q");
    }

    #[test]
    fn levels_follow_the_notes() {
        let mut bar = default_click_bar(TimeSignature::FOUR_FOUR);
        let levels = |m: &Measure| m.beats().iter().map(ClickLevel::of).collect::<Vec<_>>();
        use ClickLevel::*;
        assert_eq!(levels(&bar), [Strong, Medium, Medium, Medium]);
        bar.set_click_level(1, Soft);
        bar.set_click_level(2, Off);
        bar.set_click_level(3, Strong);
        assert_eq!(levels(&bar), [Strong, Soft, Off, Strong]);
        assert_eq!(text(&bar), "4/4 >q (q) r:q >q");
    }

    #[test]
    fn next_cycles_through_all_levels() {
        use ClickLevel::*;
        assert_eq!(Strong.next(), Medium);
        assert_eq!(Medium.next(), Soft);
        assert_eq!(Soft.next(), Off);
        assert_eq!(Off.next(), Strong);
    }

    #[test]
    fn subdivided_bars_split_every_beat_unit() {
        let four = TimeSignature::FOUR_FOUR;
        assert_eq!(text(&subdivided_click_bar(four, 2).unwrap()), "4/4 >e (e) e (e) e (e) e (e)");
        assert_eq!(subdivided_click_bar(four, 3).unwrap().beats().len(), 12);
        let seven = subdivided_click_bar(TimeSignature::SEVEN_EIGHT, 2).unwrap();
        assert_eq!(text(&seven), "7/8 >s (s) (s) (s) s (s) (s) (s) (s) (s) s (s) (s) (s)");
        assert!(subdivided_click_bar(TimeSignature::new(7, 16), 4).is_none());
    }

    #[test]
    fn subdivisions_beyond_four_take_tuplets_that_fill_the_beat() {
        let first = |ts, parts| text(&subdivided_click_bar(ts, parts).unwrap())[4..].to_owned();
        let three = TimeSignature::new(3, 4);
        assert!(first(three, 5).starts_with(">qt16 (qt16) (qt16) (qt16) (qt16) qt16 "));
        assert!(first(three, 6).starts_with(">st16 (st16) (st16) (st16) (st16) (st16) st16 "));
        assert!(first(three, 7).starts_with(">spt16 "));
        assert!(first(three, 8).starts_with(">th (th) "));
        assert!(first(three, 9).starts_with(">9:8th "));
        assert_eq!(subdivided_click_bar(three, 7).unwrap().beats().len(), 21);
        // Over an eighth, five or seven thirty-seconds in the time of four.
        let six = TimeSignature::SIX_EIGHT;
        assert!(first(six, 5).starts_with(">5:4th (5:4th) "));
        assert_eq!(subdivided_click_bar(six, 7).unwrap().beats().len(), 42);
        // Sixty-fourths and finer are not written.
        assert!(subdivided_click_bar(six, 8).is_none());
        assert!(subdivided_click_bar(TimeSignature::new(5, 16), 5).is_none());
    }

    #[test]
    fn bars_are_kept_per_meter() {
        let mut bars = ClickBars::default();
        let seven = TimeSignature::SEVEN_EIGHT;
        let mut bar = default_click_bar(seven);
        bar.set_click_level(1, ClickLevel::Off);
        bars.set(bar);
        assert_eq!(text(&bars.for_meter(seven)), "7/8 >q r:q. q");
        // Another grouping of 7/8 is another meter.
        let other = seven.with_grouping(1);
        assert_eq!(text(&bars.for_meter(other)), "2+2+3/8 >e (e) e (e) e (e) (e)");
        assert_eq!(text(&bars.for_meter(TimeSignature::FOUR_FOUR)), "4/4 >q q q q");
        // Setting the meter's own bar forgets the custom bar.
        bars.set(meter_click_bar(seven));
        assert_eq!(bars, ClickBars::default());
    }

    #[test]
    fn meters_click_their_beat_unit_until_one_is_set() {
        // Never picked in the metronome: x/8 clicks every eighth, x/4 its beats.
        let bars = ClickBars::default();
        let five = TimeSignature::FIVE_EIGHT;
        assert_eq!(text(&bars.for_meter(five)), "5/8 >e (e) (e) e (e)");
        assert_eq!(text(&bars.for_meter(TimeSignature::SIX_EIGHT)), "6/8 >e (e) (e) e (e) (e)");
        assert_eq!(bars.for_meter("5/16".parse().unwrap()).beats().len(), 5);
        assert_eq!(text(&bars.for_meter(TimeSignature::THREE_FOUR)), "3/4 >q q q");
        // The main beats alone are a bar of their own in x/8.
        let mut bars = ClickBars::default();
        bars.set(default_click_bar(five));
        assert_eq!(text(&bars.for_meter(five)), "5/8 >q. q");
    }
}
