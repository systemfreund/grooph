//! The counting overlay: where the counts of a measure fall and what is said
//! on them.
//!
//! Counting takes two steps. [`build_count_slots`] first finds the counted
//! positions of a measure on a fixed grid, whatever its notes: which beat a
//! position lies in, how finely that beat is divided and which part of it the
//! position is ([`CountPos`]). A [`CountingSystem`] then names each position
//! ("1 e & a", "ta ka di mi", "du ta de ta"), or leaves it unnamed. The
//! positions know nothing of syllables and the systems nothing of ticks.

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
    /// The label of `pos`, `None` where the system has nothing to say.
    pub fn label(self, pos: &CountPos) -> Option<String> {
        match self {
            Self::Numbers => numbers(pos),
            Self::Takadimi => takadimi(pos).map(str::to_string),
            Self::Gordon => gordon(pos).map(str::to_string),
        }
    }
}

/// What the overlay counts: the beats in a subdivision, in a system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountConfig {
    pub subdivision: Subdivision,
    pub system: CountingSystem,
}

/// A counted position, independent of what is said on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountPos {
    /// The size of the beat (primary group) the position lies in, in beat
    /// units: one in x/4, two or three in x/8 and x/16.
    pub units: u8,
    /// The measure mixes beats of different sizes (2+2+3 in 7/8).
    pub uneven: bool,
    /// The beat the position lies in, from 1.
    pub beat: u32,
    /// The beat unit the position starts in, from 1.
    pub unit: u32,
    /// The position is part `part` (from 0) of `parts` equal parts of its beat.
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

/// The counted positions of `measure` with their labels, by start tick.
/// Empty when the subdivision is too fine for the time signature.
pub fn build_count_slots(measure: &Measure, config: &CountConfig) -> Vec<CountSlot> {
    let ts = measure.time_signature();
    let ticks_per_beat = DEFAULT_GRID.ticks_per_beat(&ts);
    let measure_ticks = DEFAULT_GRID.ticks_per_measure(&ts);
    if ticks_per_beat == 0 || measure_ticks == 0 || !config.subdivision.fits(&ts) {
        return Vec::new();
    }

    let groups = default_groups_for(&ts);
    let uneven = groups.windows(2).any(|w| w[0] != w[1]);
    let mut slots = Vec::new();
    let mut start_tick = 0;
    for (beat, &units) in (1..).zip(groups.iter()) {
        if start_tick >= measure_ticks {
            break;
        }
        let end_tick = (start_tick + units as u32 * ticks_per_beat).min(measure_ticks);
        let parts = config.subdivision.parts(&ts, units);
        let ticks = end_tick - start_tick;
        // A part must fall on the grid.
        if parts > 0 && parts <= u8::MAX as u32 && ticks.is_multiple_of(parts) {
            let step = ticks / parts;
            for part in 0..parts {
                let slot_start = start_tick + step * part;
                let pos = CountPos {
                    units,
                    uneven,
                    beat,
                    unit: slot_start / ticks_per_beat + 1,
                    part: part as u8,
                    parts: parts as u8,
                };
                slots.push(CountSlot {
                    start_tick: slot_start,
                    end_tick: slot_start + step,
                    pos,
                    label: config.system.label(&pos),
                });
            }
        }
        start_tick = end_tick;
    }
    slots
}

/// Counting in numbers. A beat divided into its own units counts the beats
/// ("1 la li 2 la li" in 6/8); units divided further count the units
/// ("1 & 2 & 3 &" in 6/8, "1 e & a" in 4/4, "1 trip let").
fn numbers(pos: &CountPos) -> Option<String> {
    let part = pos.part as usize;
    let units = pos.units;
    let text = |texts: &[&str], i: usize| texts.get(i - 1).map(|t| t.to_string());
    match pos.parts {
        parts if parts == 1 || parts == units => match part {
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
        parts if units > 0 && parts.is_multiple_of(units) => {
            let per_unit = (parts / units) as usize;
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
        _ => None,
    }
}

/// Takadimi names a part by its place in the beat, whatever the
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
    let syllables: &[&str] = match (pos.parts, pos.uneven) {
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

    use Subdivision::{Beats, Halves, Quarters, Triplets};

    /// The labels of the first measure of `score`, "_" for an unnamed slot.
    fn count(score: &str, subdivision: Subdivision, system: CountingSystem) -> String {
        let measure = parse_score(score).unwrap().measures.remove(0);
        labels(&measure, subdivision, system)
    }

    fn labels(measure: &Measure, subdivision: Subdivision, system: CountingSystem) -> String {
        let slots = build_count_slots(measure, &CountConfig { subdivision, system });
        let labels: Vec<String> =
            slots.into_iter().map(|s| s.label.unwrap_or_else(|| "_".to_string())).collect();
        labels.join(" ")
    }

    fn beats(score: &str, subdivision: Subdivision) -> String {
        count(score, subdivision, CountingSystem::Numbers)
    }

    /// Every subdivision in numbers, as recorded before counting was split
    /// into positions and systems.
    #[test]
    fn numbers_count_as_before() {
        let cases: &[(&str, Subdivision, &str)] = &[
            ("4/4 h h", Beats, "1 2 3 4"),
            ("4/4 h h", Halves, "1 & 2 & 3 & 4 &"),
            ("4/4 h h", Quarters, "1 e & a 2 e & a 3 e & a 4 e & a"),
            ("4/4 h h", Triplets, "1 trip let 2 trip let 3 trip let 4 trip let"),
            ("3/4 h q", Beats, "1 2 3"),
            ("3/4 h q", Halves, "1 & 2 & 3 &"),
            ("3/4 h q", Quarters, "1 e & a 2 e & a 3 e & a"),
            ("3/4 h q", Triplets, "1 trip let 2 trip let 3 trip let"),
            ("6/8 q. q.", Beats, "1 2"),
            ("6/8 q. q.", Halves, "1 & 2 & 3 & 4 & 5 & 6 &"),
            ("6/8 q. q.", Quarters, ""),
            ("6/8 q. q.", Triplets, "1 la li 2 la li"),
            ("9/8 q. q. q.", Beats, "1 2 3"),
            ("9/8 q. q. q.", Halves, "1 & 2 & 3 & 4 & 5 & 6 & 7 & 8 & 9 &"),
            ("9/8 q. q. q.", Quarters, ""),
            ("9/8 q. q. q.", Triplets, "1 la li 2 la li 3 la li"),
            ("12/8 q. q. q. q.", Beats, "1 2 3 4"),
            ("12/8 q. q. q. q.", Halves, "1 & 2 & 3 & 4 & 5 & 6 & 7 & 8 & 9 & 10 & 11 & 12 &"),
            ("12/8 q. q. q. q.", Quarters, ""),
            ("12/8 q. q. q. q.", Triplets, "1 la li 2 la li 3 la li 4 la li"),
            ("5/8 q. q", Beats, "1 2"),
            ("5/8 q. q", Halves, "1 & 2 & 3 & 4 & 5 &"),
            ("5/8 q. q", Quarters, ""),
            ("5/8 q. q", Triplets, "1 la li 2 &"),
            ("7/8 q q. q", Beats, "1 2 3"),
            ("7/8 q q. q", Halves, "1 & 2 & 3 & 4 & 5 & 6 & 7 &"),
            ("7/8 q q. q", Quarters, ""),
            ("7/8 q q. q", Triplets, "1 & 2 la li 3 &"),
            ("7/16 e e. e", Beats, "1 2 3"),
            ("7/16 e e. e", Halves, ""),
            ("7/16 e e. e", Quarters, ""),
            ("7/16 e e. e", Triplets, "1 & 2 la li 3 &"),
            ("5/8 q q.", Beats, "1 2"),
            ("5/8 q q.", Halves, "1 & 2 & 3 & 4 & 5 &"),
            ("5/8 q q.", Quarters, ""),
            ("5/8 q q.", Triplets, "1 la li 2 &"),
            ("4/4 r:e e q h", Beats, "1 2 3 4"),
            ("4/4 r:e e q h", Halves, "1 & 2 & 3 & 4 &"),
            ("4/4 r:e e q h", Quarters, "1 e & a 2 e & a 3 e & a 4 e & a"),
            ("4/4 r:e e q h", Triplets, "1 trip let 2 trip let 3 trip let 4 trip let"),
        ];
        for &(score, subdivision, expected) in cases {
            assert_eq!(beats(score, subdivision), expected, "{score} {subdivision:?}");
        }
    }

    #[test]
    fn five_eight_in_two_and_three() {
        let two_three = TimeSignature::new(5, 8).with_grouping(1);
        assert_eq!(*two_three.groups(), [2, 3]);
        let count = |subdivision| labels(&Measure::new(two_three), subdivision, Default::default());
        assert_eq!(count(Beats), "1 2");
        assert_eq!(count(Halves), "1 & 2 & 3 & 4 & 5 &");
        assert_eq!(count(Triplets), "1 & 2 la li");
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
        assert_eq!(labels(&measure, Triplets, Default::default()), "1 & 2 & 3 la li");
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
    fn tuplets_are_counted_by_the_beats_they_fall_in() {
        assert_eq!(beats("4/4 t8 t8 t8 q h", Halves), "1 & 2 & 3 & 4 &");
        assert_eq!(beats("4/4 qt16 qt16 qt16 qt16 qt16 q h", Triplets), beats("4/4 h h", Triplets));
    }

    #[test]
    fn positions_describe_the_beat() {
        let measure = parse_score("7/8 q q. q").unwrap().measures.remove(0);
        let config = CountConfig { subdivision: Halves, system: CountingSystem::Numbers };
        let slots = build_count_slots(&measure, &config);
        // The second sixteenth of the "la" in the 3-group: beat 2, unit 4.
        let pos = slots[7].pos;
        assert_eq!((pos.units, pos.uneven), (3, true));
        assert_eq!((pos.beat, pos.unit, pos.part, pos.parts), (2, 4, 3, 6));
    }

    #[test]
    fn takadimi_names_the_place_in_the_beat() {
        let takadimi = |score, subdivision| count(score, subdivision, CountingSystem::Takadimi);
        assert_eq!(takadimi("2/4 q q", Beats), "ta ta");
        assert_eq!(takadimi("2/4 q q", Halves), "ta di ta di");
        assert_eq!(takadimi("2/4 q q", Quarters), "ta ka di mi ta ka di mi");
        assert_eq!(takadimi("2/4 q q", Triplets), "ta ki da ta ki da");
        // Compound meters divide the dotted quarter.
        assert_eq!(takadimi("6/8 q. q.", Beats), "ta ta");
        assert_eq!(takadimi("6/8 q. q.", Triplets), "ta ki da ta ki da");
        assert_eq!(takadimi("6/8 q. q.", Halves), "ta va ki di da ma ta va ki di da ma");
        // Odd meters: each group by its own size.
        assert_eq!(takadimi("7/8 q q. q", Triplets), "ta di ta ki da ta di");
        assert_eq!(takadimi("5/8 q. q", Halves), "ta va ki di da ma ta ka di mi");
    }

    #[test]
    fn gordon_follows_the_meter() {
        let gordon = |score, subdivision| count(score, subdivision, CountingSystem::Gordon);
        assert_eq!(gordon("2/4 q q", Halves), "du de du de");
        assert_eq!(gordon("2/4 q q", Quarters), "du ta de ta du ta de ta");
        assert_eq!(gordon("2/4 q q", Triplets), "du da di du da di");
        assert_eq!(gordon("6/8 q. q.", Triplets), "du da di du da di");
        assert_eq!(gordon("6/8 q. q.", Halves), "du ta da ta di ta du ta da ta di ta");
        // Unusual meters.
        assert_eq!(gordon("7/8 q q. q", Triplets), "du be du ba bi du be");
        assert_eq!(gordon("5/8 q. q", Triplets), "du ba bi du be");
        assert_eq!(gordon("7/8 q q. q", Halves), "du ta be ta du ta ba ta bi ta du ta be ta");
    }
}
