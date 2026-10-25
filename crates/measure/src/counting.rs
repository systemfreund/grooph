//! The counting overlay: where the counts of a measure fall and what is said
//! on them.
//!
//! Counting takes two steps. [`build_count_slots`] first finds the counted
//! positions of a measure on a fixed grid, whatever its notes: which beat a
//! position lies in, how finely that beat is divided and which part of it the
//! position is ([`CountPos`]). A [`CountingSystem`] then names each position
//! ("1 e & a", "ta ka di mi", "du ta de ta"), or leaves it unnamed. The positions know nothing of syllables
//! and the systems nothing of ticks.

use crate::grid::DEFAULT_GRID;
use crate::grouping::default_groups_for;
use crate::{Measure, TimeSignature};
use serde::{Deserialize, Serialize};

/// How finely the count divides the beats of a measure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subdivision {
    /// One count per beat: "1 2 3 4", "1 2" in 6/8.
    Beats,
    /// Every beat unit in two: "1 & 2 &", sixteenths in x/8.
    Halves,
    /// Every beat unit in four: "1 e & a".
    Quarters,
    /// Every beat in three ("1 trip let"); a group of eighths or sixteenths
    /// in its own units ("1 la li 2 la li" in 6/8, "1 & 2 la li 3 &" in 7/8).
    Triplets,
}

impl Subdivision {
    /// Whether the subdivision counts sensibly in `ts`: never finer than
    /// sixteenths (no "1 e & a" on 32nds in 6/8).
    pub fn fits(self, ts: &TimeSignature) -> bool {
        match self {
            Self::Halves => ts.beat_unit as u32 * 2 <= 16,
            Self::Quarters => ts.beat_unit as u32 * 4 <= 16,
            Self::Beats | Self::Triplets => true,
        }
    }

    /// How many parts a beat of `units` beat units is divided into.
    fn parts(self, ts: &TimeSignature, units: u8) -> u32 {
        let units = units as u32;
        match self {
            Self::Beats => 1,
            Self::Halves => units * 2,
            Self::Quarters => units * 4,
            // In x/8 and x/16 the beat units themselves are the subdivision.
            Self::Triplets if ts.beat_unit >= 8 => units,
            Self::Triplets => units * 3,
        }
    }
}

/// The words a count is spoken in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CountingSystem {
    /// "1 e & a", "1 trip let", "1 la li".
    #[default]
    Numbers,
    /// Takadimi (Hoffman, Pelto and White): every beat is "ta", its parts
    /// are named by their place in the beat ("ta di", "ta ka di mi", "ta ki
    /// da"), the same in simple and compound meters.
    Takadimi,
    /// Gordon's Music Learning Theory: "du de" and "du ta de ta" in duple,
    /// "du da di" in triple, and "du be" and "du ba bi" for the groups of the
    /// unusual meters (5/8, 7/8).
    Gordon,
}

impl CountingSystem {
    /// The label of `pos`, `None` where the system has nothing to say. The
    /// syllable systems count what they have no syllables for (quintuplets,
    /// septuplets) in numbers.
    pub fn label(self, pos: &CountPos) -> Option<String> {
        let syllable = match self {
            Self::Numbers => return numbers(pos),
            Self::Takadimi => takadimi(pos),
            Self::Gordon => gordon(pos),
        };
        syllable.map(str::to_string).or_else(|| numbers(pos))
    }
}

/// What the overlay counts: the beats in some subdivision, the tuplets, or
/// both. A tuplet's count takes the place of the beat count where both fall
/// on the same tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CountConfig {
    pub subdivision: Option<Subdivision>,
    pub tuplets: bool,
    pub system: CountingSystem,
}

/// The stretch of music a [`CountPos`] divides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountSpan {
    /// A beat (primary group) of `units` beat units: one in x/4, two or three
    /// in x/8 and x/16. `uneven`: the measure mixes beats of different sizes
    /// (2+2+3 in 7/8).
    Beat { units: u8, uneven: bool },
    /// A tuplet group.
    Tuplet,
}

/// A counted position, independent of what is said on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountPos {
    pub span: CountSpan,
    /// The beat (primary group) the position starts in, from 1.
    pub beat: u32,
    /// The beat unit the position starts in, from 1.
    pub unit: u32,
    /// The position is part `part` (from 0) of `parts` equal parts of its span.
    pub part: u8,
    pub parts: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountSlot {
    pub start_tick: u32,
    pub end_tick: u32,
    pub pos: CountPos,
    pub label: Option<String>,
}

impl CountSlot {
    pub fn in_tuplet(&self) -> bool {
        self.pos.span == CountSpan::Tuplet
    }
}

/// The counted positions of `measure` with their labels, by start tick; at
/// the same tick a tuplet's slot comes first.
pub fn build_count_slots(measure: &Measure, config: &CountConfig) -> Vec<CountSlot> {
    let ts = measure.time_signature();
    let grid = Grid::new(&ts);
    if grid.ticks_per_beat == 0 || grid.measure_ticks == 0 {
        return Vec::new();
    }

    let mut slots = Vec::new();
    if let Some(subdivision) = config.subdivision.filter(|s| s.fits(&ts)) {
        let uneven = grid.beats.windows(2).any(|w| w[0].2 != w[1].2);
        for &(start_tick, end_tick, units) in &grid.beats {
            let span = CountSpan::Beat { units, uneven };
            let parts = subdivision.parts(&ts, units);
            grid.push_slots(&mut slots, config.system, span, start_tick, end_tick, parts);
        }
    }
    if config.tuplets {
        for (start_tick, end_tick, n) in tuplet_spans(measure, grid.measure_ticks) {
            let span = CountSpan::Tuplet;
            grid.push_slots(&mut slots, config.system, span, start_tick, end_tick, n as u32);
        }
    }

    slots.sort_by_key(|s| (s.start_tick, !s.in_tuplet()));
    slots
}

struct Grid {
    ticks_per_beat: u32,
    measure_ticks: u32,
    /// The beats (primary groups) as start tick, end tick and size in beat
    /// units.
    beats: Vec<(u32, u32, u8)>,
}

impl Grid {
    fn new(ts: &TimeSignature) -> Self {
        let ticks_per_beat = DEFAULT_GRID.ticks_per_beat(ts);
        let measure_ticks = DEFAULT_GRID.ticks_per_measure(ts);
        let mut beats = Vec::new();
        let mut start_tick = 0;
        for &units in default_groups_for(ts).iter() {
            if start_tick >= measure_ticks {
                break;
            }
            let end_tick = (start_tick + units as u32 * ticks_per_beat).min(measure_ticks);
            beats.push((start_tick, end_tick, units));
            start_tick = end_tick;
        }
        Self { ticks_per_beat, measure_ticks, beats }
    }

    fn beat_at(&self, tick: u32) -> u32 {
        self.beats.iter().position(|&(start, end, _)| (start..end).contains(&tick)).unwrap_or(0)
            as u32
            + 1
    }

    /// Divide `start_tick..end_tick` into `parts` equal slots; nothing if the
    /// grid cannot.
    fn push_slots(
        &self,
        slots: &mut Vec<CountSlot>,
        system: CountingSystem,
        span: CountSpan,
        start_tick: u32,
        end_tick: u32,
        parts: u32,
    ) {
        let ticks = end_tick.saturating_sub(start_tick);
        if parts == 0 || parts > u8::MAX as u32 || ticks == 0 || !ticks.is_multiple_of(parts) {
            return;
        }
        let step = ticks / parts;
        for part in 0..parts {
            let slot_start = start_tick + step * part;
            let pos = CountPos {
                span,
                beat: self.beat_at(slot_start),
                unit: slot_start / self.ticks_per_beat + 1,
                part: part as u8,
                parts: parts as u8,
            };
            slots.push(CountSlot {
                start_tick: slot_start,
                end_tick: slot_start + step,
                pos,
                label: system.label(&pos),
            });
        }
    }
}

/// The tuplet groups of `measure` as start tick, end tick and note count.
fn tuplet_spans(measure: &Measure, measure_ticks: u32) -> Vec<(u32, u32, u8)> {
    let beats = measure.beats();
    if beats.is_empty() {
        return Vec::new();
    }
    let onsets = DEFAULT_GRID.compute_onset_ticks(beats);
    measure
        .tuplet_groups()
        .into_iter()
        .filter_map(|group| {
            let anchor = measure.tuplets().get(group.id)?;
            let start_tick = onsets[group.start_idx];
            Some((start_tick, (start_tick + anchor.target_ticks).min(measure_ticks), anchor.n))
        })
        .collect()
}

/// Counting in numbers. A beat divided into its own units counts the beats
/// ("1 la li 2 la li" in 6/8); units divided further count the units
/// ("1 & 2 & 3 &" in 6/8, "1 e & a" in 4/4). Triplets are "1 trip let",
/// other tuplets are numbered through.
fn numbers(pos: &CountPos) -> Option<String> {
    let part = pos.part as usize;
    let text = |texts: &[&str], i: usize| texts.get(i - 1).map(|t| t.to_string());
    match pos.span {
        CountSpan::Tuplet if pos.parts == 3 => match part {
            0 => Some(pos.unit.to_string()),
            p => text(&["trip", "let"], p),
        },
        CountSpan::Tuplet => Some((part + 1).to_string()),
        CountSpan::Beat { units, .. } if pos.parts == 1 || pos.parts == units => match part {
            0 => Some(pos.beat.to_string()),
            p => text(
                match units {
                    2 => &["&"],
                    3 => &["la", "li"],
                    4 => &["e", "&", "a"],
                    _ => &[],
                },
                p,
            ),
        },
        CountSpan::Beat { units, .. } if units > 0 && pos.parts.is_multiple_of(units) => {
            let per_unit = (pos.parts / units) as usize;
            match part % per_unit {
                0 => Some(pos.unit.to_string()),
                p => text(
                    match per_unit {
                        2 => &["&"],
                        3 => &["trip", "let"],
                        4 => &["e", "&", "a"],
                        _ => &[],
                    },
                    p,
                ),
            }
        }
        CountSpan::Beat { .. } => None,
    }
}

/// Takadimi names a part by its place in the beat (or tuplet), whatever the
/// meter: halves "ta di", thirds "ta ki da", quarters "ta ka di mi", sixths
/// "ta va ki di da ma".
fn takadimi(pos: &CountPos) -> Option<&'static str> {
    let syllables: &[&str] = match pos.parts {
        1 => &["ta"],
        2 => &["ta", "di"],
        3 => &["ta", "ki", "da"],
        4 => &["ta", "ka", "di", "mi"],
        6 => &["ta", "va", "ki", "di", "da", "ma"],
        _ => &[],
    };
    syllables.get(pos.part as usize).copied()
}

/// Gordon names the beat "du" and its parts by the meter: duple "du de",
/// triple "du da di", each split once more with "ta" ("du ta de ta"). The
/// groups of an unusual meter are "du be" and "du ba bi".
fn gordon(pos: &CountPos) -> Option<&'static str> {
    let unusual = matches!(pos.span, CountSpan::Beat { uneven: true, .. });
    let syllables: &[&str] = match (pos.parts, unusual) {
        (1, _) => &["du"],
        (2, false) => &["du", "de"],
        (2, true) => &["du", "be"],
        (3, false) => &["du", "da", "di"],
        (3, true) => &["du", "ba", "bi"],
        (4, false) => &["du", "ta", "de", "ta"],
        (4, true) => &["du", "ta", "be", "ta"],
        (6, false) => &["du", "ta", "da", "ta", "di", "ta"],
        (6, true) => &["du", "ta", "ba", "ta", "bi", "ta"],
        _ => &[],
    };
    syllables.get(pos.part as usize).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notation::parse_score;

    fn config(subdivision: Option<Subdivision>, tuplets: bool) -> CountConfig {
        CountConfig { subdivision, tuplets, ..CountConfig::default() }
    }

    /// The labels of the first measure of `score`, "_" for an unnamed slot.
    fn count(score: &str, config: CountConfig) -> String {
        let measure = parse_score(score).unwrap().measures.remove(0);
        let slots = build_count_slots(&measure, &config);
        let labels: Vec<String> =
            slots.into_iter().map(|s| s.label.unwrap_or_else(|| "_".to_string())).collect();
        labels.join(" ")
    }

    fn beats(score: &str, subdivision: Subdivision) -> String {
        count(score, config(Some(subdivision), false))
    }

    use Subdivision::{Beats, Halves, Quarters, Triplets};

    /// Every subdivision in numbers, with the tuplet overlay, as recorded
    /// before counting was split into positions and systems. "*" marks a
    /// tuplet's slot.
    #[test]
    fn numbers_count_as_before() {
        let cases: &[(&str, Option<Subdivision>, &str)] = &[
            ("4/4 h h", Some(Beats), "1 2 3 4"),
            ("4/4 h h", Some(Halves), "1 & 2 & 3 & 4 &"),
            ("4/4 h h", Some(Quarters), "1 e & a 2 e & a 3 e & a 4 e & a"),
            ("4/4 h h", Some(Triplets), "1 trip let 2 trip let 3 trip let 4 trip let"),
            ("3/4 h q", Some(Beats), "1 2 3"),
            ("3/4 h q", Some(Halves), "1 & 2 & 3 &"),
            ("3/4 h q", Some(Quarters), "1 e & a 2 e & a 3 e & a"),
            ("3/4 h q", Some(Triplets), "1 trip let 2 trip let 3 trip let"),
            ("6/8 q. q.", Some(Beats), "1 2"),
            ("6/8 q. q.", Some(Halves), "1 & 2 & 3 & 4 & 5 & 6 &"),
            ("6/8 q. q.", Some(Quarters), ""),
            ("6/8 q. q.", Some(Triplets), "1 la li 2 la li"),
            ("9/8 q. q. q.", Some(Beats), "1 2 3"),
            ("9/8 q. q. q.", Some(Halves), "1 & 2 & 3 & 4 & 5 & 6 & 7 & 8 & 9 &"),
            ("9/8 q. q. q.", Some(Quarters), ""),
            ("9/8 q. q. q.", Some(Triplets), "1 la li 2 la li 3 la li"),
            ("12/8 q. q. q. q.", Some(Beats), "1 2 3 4"),
            (
                "12/8 q. q. q. q.",
                Some(Halves),
                "1 & 2 & 3 & 4 & 5 & 6 & 7 & 8 & 9 & 10 & 11 & 12 &",
            ),
            ("12/8 q. q. q. q.", Some(Quarters), ""),
            ("12/8 q. q. q. q.", Some(Triplets), "1 la li 2 la li 3 la li 4 la li"),
            ("5/8 q. q", Some(Beats), "1 2"),
            ("5/8 q. q", Some(Halves), "1 & 2 & 3 & 4 & 5 &"),
            ("5/8 q. q", Some(Quarters), ""),
            ("5/8 q. q", Some(Triplets), "1 la li 2 &"),
            ("7/8 q q. q", Some(Beats), "1 2 3"),
            ("7/8 q q. q", Some(Halves), "1 & 2 & 3 & 4 & 5 & 6 & 7 &"),
            ("7/8 q q. q", Some(Quarters), ""),
            ("7/8 q q. q", Some(Triplets), "1 & 2 la li 3 &"),
            ("7/16 e e. e", Some(Beats), "1 2 3"),
            ("7/16 e e. e", Some(Halves), ""),
            ("7/16 e e. e", Some(Quarters), ""),
            ("7/16 e e. e", Some(Triplets), "1 & 2 la li 3 &"),
            ("5/8 q q.", Some(Beats), "1 2"),
            ("5/8 q q.", Some(Halves), "1 & 2 & 3 & 4 & 5 &"),
            ("5/8 q q.", Some(Quarters), ""),
            ("5/8 q q.", Some(Triplets), "1 la li 2 &"),
            ("4/4 r:e e q h", Some(Beats), "1 2 3 4"),
            ("4/4 r:e e q h", Some(Halves), "1 & 2 & 3 & 4 &"),
            ("4/4 r:e e q h", Some(Quarters), "1 e & a 2 e & a 3 e & a 4 e & a"),
            ("4/4 r:e e q h", Some(Triplets), "1 trip let 2 trip let 3 trip let 4 trip let"),
            ("4/4 t8 t8 t8 q h", Some(Beats), "1* 1 trip* let* 2 3 4"),
            ("4/4 t8 t8 t8 q h", Some(Halves), "1* 1 trip* & let* 2 & 3 & 4 &"),
            ("4/4 t8 t8 t8 q h", Some(Quarters), "1* 1 e trip* & let* a 2 e & a 3 e & a 4 e & a"),
            (
                "4/4 t8 t8 t8 q h",
                Some(Triplets),
                "1* 1 trip* trip let* let 2 trip let 3 trip let 4 trip let",
            ),
            ("4/4 t8 t8 t8 q h", None, "1* trip* let*"),
            ("4/4 t4 t4 t4 h", Some(Beats), "1* 1 trip* 2 let* 3 4"),
            ("4/4 t4 t4 t4 h", Some(Halves), "1* 1 & trip* 2 let* & 3 & 4 &"),
            ("4/4 t4 t4 t4 h", Some(Quarters), "1* 1 e & trip* a 2 e let* & a 3 e & a 4 e & a"),
            (
                "4/4 t4 t4 t4 h",
                Some(Triplets),
                "1* 1 trip trip* let 2 let* trip let 3 trip let 4 trip let",
            ),
            ("4/4 t4 t4 t4 h", None, "1* trip* let*"),
            ("4/4 qt16 qt16 qt16 qt16 qt16 q h", Some(Beats), "1* 1 2* 3* 4* 5* 2 3 4"),
            ("4/4 qt16 qt16 qt16 qt16 qt16 q h", Some(Halves), "1* 1 2* 3* & 4* 5* 2 & 3 & 4 &"),
            (
                "4/4 qt16 qt16 qt16 qt16 qt16 q h",
                Some(Quarters),
                "1* 1 2* e 3* & 4* a 5* 2 e & a 3 e & a 4 e & a",
            ),
            (
                "4/4 qt16 qt16 qt16 qt16 qt16 q h",
                Some(Triplets),
                "1* 1 2* trip 3* 4* let 5* 2 trip let 3 trip let 4 trip let",
            ),
            ("4/4 qt16 qt16 qt16 qt16 qt16 q h", None, "1* 2* 3* 4* 5*"),
            ("4/4 st16 st16 st16 st16 st16 st16 q h", Some(Beats), "1* 1 2* 3* 4* 5* 6* 2 3 4"),
            (
                "4/4 st16 st16 st16 st16 st16 st16 q h",
                Some(Halves),
                "1* 1 2* 3* 4* & 5* 6* 2 & 3 & 4 &",
            ),
            (
                "4/4 st16 st16 st16 st16 st16 st16 q h",
                Some(Quarters),
                "1* 1 2* e 3* 4* & 5* a 6* 2 e & a 3 e & a 4 e & a",
            ),
            (
                "4/4 st16 st16 st16 st16 st16 st16 q h",
                Some(Triplets),
                "1* 1 2* 3* trip 4* 5* let 6* 2 trip let 3 trip let 4 trip let",
            ),
            ("4/4 st16 st16 st16 st16 st16 st16 q h", None, "1* 2* 3* 4* 5* 6*"),
            ("6/8 2:3e 2:3e q.", Some(Beats), "1* 1 2* 2"),
            ("6/8 2:3e 2:3e q.", Some(Halves), "1* 1 & 2 2* & 3 & 4 & 5 & 6 &"),
            ("6/8 2:3e 2:3e q.", Some(Quarters), "1* 2*"),
            ("6/8 2:3e 2:3e q.", Some(Triplets), "1* 1 la 2* li 2 la li"),
            ("6/8 2:3e 2:3e q.", None, "1* 2*"),
            ("6/8 4:3e 4:3e 4:3e 4:3e q.", Some(Beats), "1* 1 2* 3* 4* 2"),
            ("6/8 4:3e 4:3e 4:3e 4:3e q.", Some(Halves), "1* 1 & 2* 2 3* & 3 4* & 4 & 5 & 6 &"),
            ("6/8 4:3e 4:3e 4:3e 4:3e q.", Some(Quarters), "1* 2* 3* 4*"),
            ("6/8 4:3e 4:3e 4:3e 4:3e q.", Some(Triplets), "1* 1 2* la 3* li 4* 2 la li"),
            ("6/8 4:3e 4:3e 4:3e 4:3e q.", None, "1* 2* 3* 4*"),
        ];
        for &(score, subdivision, expected) in cases {
            let measure = parse_score(score).unwrap().measures.remove(0);
            let slots = build_count_slots(&measure, &config(subdivision, true));
            let labels: Vec<String> = slots
                .iter()
                .map(|s| {
                    let label = s.label.clone().unwrap_or_else(|| "_".to_string());
                    if s.in_tuplet() { label + "*" } else { label }
                })
                .collect();
            assert_eq!(labels.join(" "), expected, "{score} {subdivision:?}");
        }
    }

    #[test]
    fn five_eight_in_two_and_three() {
        let two_three = TimeSignature::new(5, 8).with_grouping(1);
        assert_eq!(*two_three.groups(), [2, 3]);
        let labels = |subdivision| {
            let slots =
                build_count_slots(&Measure::new(two_three), &config(Some(subdivision), false));
            slots.iter().filter_map(|s| s.label.clone()).collect::<Vec<_>>().join(" ")
        };
        assert_eq!(labels(Beats), "1 2");
        assert_eq!(labels(Halves), "1 & 2 & 3 & 4 & 5 &");
        assert_eq!(labels(Triplets), "1 & 2 la li");
    }

    #[test]
    fn simple_meters() {
        assert_eq!(beats("3/4 h q", Beats), "1 2 3");
        assert_eq!(beats("3/4 h q", Halves), "1 & 2 & 3 &");
        assert_eq!(beats("2/4 q q", Quarters), "1 e & a 2 e & a");
        assert_eq!(beats("3/4 h q", Triplets), "1 trip let 2 trip let 3 trip let");
    }

    #[test]
    fn eighth_meters_count_their_groups() {
        assert_eq!(beats("6/8 q. q.", Beats), "1 2");
        assert_eq!(beats("6/8 q. q.", Halves), "1 & 2 & 3 & 4 & 5 & 6 &");
        assert_eq!(beats("6/8 q. q.", Triplets), "1 la li 2 la li");
        assert_eq!(beats("9/8 q. q. q.", Triplets), "1 la li 2 la li 3 la li");
        assert_eq!(beats("7/8 q q. q", Beats), "1 2 3");
        assert_eq!(beats("7/8 q q. q", Triplets), "1 & 2 la li 3 &");
        assert_eq!(beats("5/8 q. q", Triplets), "1 la li 2 &");
        assert_eq!(beats("7/16 e e. e", Triplets), "1 & 2 la li 3 &");
    }

    #[test]
    fn the_count_follows_the_picked_grouping() {
        let two_two_three = TimeSignature::SEVEN_EIGHT.with_grouping(1);
        assert_eq!(*two_two_three.groups(), [2, 2, 3]);
        let measure = Measure::new(two_two_three);
        let slots = build_count_slots(&measure, &config(Some(Triplets), false));
        let labels: Vec<_> = slots.iter().filter_map(|s| s.label.as_deref()).collect();
        assert_eq!(labels, ["1", "&", "2", "&", "3", "la", "li"]);
    }

    #[test]
    fn nothing_finer_than_sixteenths() {
        assert!(Quarters.fits(&TimeSignature::FOUR_FOUR));
        assert!(!Quarters.fits(&TimeSignature::SIX_EIGHT));
        assert_eq!(beats("6/8 q. q.", Quarters), "");
        assert!(Halves.fits(&TimeSignature::SIX_EIGHT));
        assert!(!Halves.fits(&TimeSignature::FOUR_SIXTEENTH));
        assert_eq!(beats("7/16 e e. e", Halves), "");
    }

    #[test]
    fn tuplets_count_on_their_own() {
        let tuplets = config(None, true);
        assert_eq!(count("4/4 t8 t8 t8 q h", tuplets), "1 trip let");
        assert_eq!(count("4/4 q t4 t4 t4 q", tuplets), "2 trip let");
        assert_eq!(count("4/4 qt16 qt16 qt16 qt16 qt16 q h", tuplets), "1 2 3 4 5");
        assert_eq!(count("6/8 2:3e 2:3e q.", tuplets), "1 2");
    }

    #[test]
    fn a_tuplet_count_comes_first_on_a_shared_tick() {
        let measure = parse_score("4/4 t8 t8 t8 q h").unwrap().measures.remove(0);
        let slots = build_count_slots(&measure, &config(Some(Beats), true));
        let at_zero: Vec<_> = slots.iter().filter(|s| s.start_tick == 0).collect();
        assert_eq!(at_zero.len(), 2);
        assert!(at_zero[0].in_tuplet());
        assert!(!at_zero[1].in_tuplet());
    }

    #[test]
    fn positions_describe_the_beat() {
        let measure = parse_score("7/8 q q. q").unwrap().measures.remove(0);
        let slots = build_count_slots(&measure, &config(Some(Halves), false));
        // The second sixteenth of the "la" in the 3-group: beat 2, unit 4.
        let pos = slots[7].pos;
        assert_eq!(pos.span, CountSpan::Beat { units: 3, uneven: true });
        assert_eq!((pos.beat, pos.unit, pos.part, pos.parts), (2, 4, 3, 6));
    }

    fn syllables(score: &str, subdivision: Option<Subdivision>, system: CountingSystem) -> String {
        count(score, CountConfig { subdivision, tuplets: false, system })
    }

    #[test]
    fn takadimi_names_the_place_in_the_beat() {
        use CountingSystem::Takadimi;
        assert_eq!(syllables("2/4 q q", Some(Beats), Takadimi), "ta ta");
        assert_eq!(syllables("2/4 q q", Some(Halves), Takadimi), "ta di ta di");
        assert_eq!(syllables("2/4 q q", Some(Quarters), Takadimi), "ta ka di mi ta ka di mi");
        assert_eq!(syllables("2/4 q q", Some(Triplets), Takadimi), "ta ki da ta ki da");
        // Compound meters divide the dotted quarter.
        assert_eq!(syllables("6/8 q. q.", Some(Beats), Takadimi), "ta ta");
        assert_eq!(syllables("6/8 q. q.", Some(Triplets), Takadimi), "ta ki da ta ki da");
        assert_eq!(
            syllables("6/8 q. q.", Some(Halves), Takadimi),
            "ta va ki di da ma ta va ki di da ma"
        );
        // Odd meters: each group by its own size.
        assert_eq!(syllables("7/8 q q. q", Some(Triplets), Takadimi), "ta di ta ki da ta di");
        assert_eq!(syllables("5/8 q. q", Some(Halves), Takadimi), "ta va ki di da ma ta ka di mi");
    }

    #[test]
    fn gordon_follows_the_meter() {
        use CountingSystem::Gordon;
        assert_eq!(syllables("2/4 q q", Some(Halves), Gordon), "du de du de");
        assert_eq!(syllables("2/4 q q", Some(Quarters), Gordon), "du ta de ta du ta de ta");
        assert_eq!(syllables("2/4 q q", Some(Triplets), Gordon), "du da di du da di");
        assert_eq!(syllables("6/8 q. q.", Some(Triplets), Gordon), "du da di du da di");
        assert_eq!(
            syllables("6/8 q. q.", Some(Halves), Gordon),
            "du ta da ta di ta du ta da ta di ta"
        );
        // Unusual meters.
        assert_eq!(syllables("7/8 q q. q", Some(Triplets), Gordon), "du be du ba bi du be");
        assert_eq!(syllables("5/8 q. q", Some(Triplets), Gordon), "du ba bi du be");
        assert_eq!(
            syllables("7/8 q q. q", Some(Halves), Gordon),
            "du ta be ta du ta ba ta bi ta du ta be ta"
        );
    }

    #[test]
    fn syllable_systems_count_tuplets_they_have_no_names_for() {
        for (system, triplet, duplet) in [
            (CountingSystem::Takadimi, "ta ki da", "ta di"),
            (CountingSystem::Gordon, "du da di", "du de"),
        ] {
            let tuplets = CountConfig { subdivision: None, tuplets: true, system };
            assert_eq!(count("4/4 t8 t8 t8 q h", tuplets), triplet);
            assert_eq!(count("4/4 qt16 qt16 qt16 qt16 qt16 q h", tuplets), "1 2 3 4 5");
            // A duplet in 6/8 divides its span in two.
            assert_eq!(count("6/8 2:3e 2:3e q.", tuplets), duplet);
        }
    }
}
