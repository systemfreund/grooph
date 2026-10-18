//! Click bars: what the metronome plays in each meter, written as a measure.
//!
//! Every note of a meter's click bar is a click; its level comes from the
//! note: accented is [`ClickLevel::Strong`], plain [`ClickLevel::Medium`],
//! soft (a ghost note) [`ClickLevel::Soft`], and a rest is no click
//! ([`ClickLevel::Off`]). Tuplets and dotted notes work like in any rhythm,
//! so "eighth, then a triplet without its middle note" is just a bar.
//!
//! The default bar has one note per beat group of [`TimeSignature::groups`],
//! as long as the group, the first one accented: the downbeat and primary
//! beats as they always sounded. [`ClickBars`] keeps the bars that differ
//! from it, one per meter, so every measure in that meter clicks the same.

use crate::BeatKind::{Note, Rest};
use crate::grid::DEFAULT_GRID;
use crate::notation::parse_score;
use crate::{Beat, BeatIdx, Measure, TimeSignature};
use serde::{Deserialize, Serialize};

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
        match (beat.kind, beat.accented, beat.soft) {
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

impl Measure {
    /// Give the beat at `idx` the note, accent and soft flag of `level`. The
    /// duration stays, so the measure stays valid.
    pub fn set_click_level(&mut self, idx: BeatIdx, level: ClickLevel) {
        let Some(b) = self.beats.get_mut(idx) else {
            return;
        };
        b.kind = if level == ClickLevel::Off { Rest } else { Note };
        b.accented = level == ClickLevel::Strong;
        b.soft = level == ClickLevel::Soft;
    }
}

/// Notation code of the plain or dotted note `ticks` long, if there is one.
fn code_for_ticks(ticks: u32) -> Option<&'static str> {
    const CODES: [&str; 7] = ["q.", "q", "e.", "e", "s.", "s", "th"];
    CODES.into_iter().find(|code| {
        crate::notation::duration_of_code(code).and_then(|d| DEFAULT_GRID.ticks_of(&d))
            == Some(ticks)
    })
}

/// Code of one beat unit of `ts` split into `parts` equal notes.
fn part_code(ts: &TimeSignature, parts: u32) -> Option<&'static str> {
    Some(match (ts.beat_unit, parts) {
        (4, 1) => "q",
        (4, 2) | (8, 1) => "e",
        (4, 3) => "t8",
        (4, 4) | (8, 2) | (16, 1) => "s",
        (8, 3) => "t16",
        (8, 4) | (16, 2) => "th",
        (16, 3) => "t32",
        _ => return None,
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
    let unit_code = part_code(&ts, 1);
    let mut tokens = Vec::new();
    for (i, &beats) in ts.groups().iter().enumerate() {
        let accent = if i == 0 { ">" } else { "" };
        match (code_for_ticks(beats as u32 * unit), unit_code) {
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

/// Click bar of `ts` with every beat unit split into `parts` notes (2, 3 or
/// 4; 1 is one note per beat unit). Group starts keep the level of the
/// default bar (first strong, others medium), all other clicks are soft.
/// `None` if the meter cannot be split that way.
pub fn subdivided_click_bar(ts: TimeSignature, parts: u32) -> Option<Measure> {
    let code = part_code(&ts, parts)?;
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

/// Whether two bars hold the same meter and beats.
pub(crate) fn same_bar(a: &Measure, b: &Measure) -> bool {
    a.time_signature() == b.time_signature() && a.beats() == b.beats()
}

/// The click bar of every meter: the default unless one was set.
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
        self.custom_for(ts).cloned().unwrap_or_else(|| default_click_bar(ts))
    }

    /// The bar of `ts` if it is not the default.
    pub fn custom_for(&self, ts: TimeSignature) -> Option<&Measure> {
        self.custom.iter().find(|m| m.time_signature() == ts)
    }

    /// Set the bar of its meter; the default bar is not stored.
    pub fn set(&mut self, bar: Measure) {
        let ts = bar.time_signature();
        self.custom.retain(|m| m.time_signature() != ts);
        if !same_bar(&bar, &default_click_bar(ts)) {
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
    fn bars_are_kept_per_meter() {
        let mut bars = ClickBars::default();
        let seven = TimeSignature::SEVEN_EIGHT;
        let mut bar = default_click_bar(seven);
        bar.set_click_level(1, ClickLevel::Off);
        bars.set(bar);
        assert_eq!(text(&bars.for_meter(seven)), "7/8 >q r:q. q");
        // Another grouping of 7/8 is another meter.
        assert_eq!(text(&bars.for_meter(seven.with_grouping(1))), "2+2+3/8 >q q q.");
        assert_eq!(text(&bars.for_meter(TimeSignature::FOUR_FOUR)), "4/4 >q q q q");
        // Setting the default forgets the custom bar.
        bars.set(default_click_bar(seven));
        assert_eq!(bars, ClickBars::default());
    }
}
