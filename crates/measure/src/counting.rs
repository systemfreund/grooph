//! The counting overlay: where the counts of a measure fall and what is said
//! on them.
//!
//! Counting takes two steps. [`build_count_slots`] first finds the counted
//! positions of a measure on a fixed grid, whatever its notes: which beat a
//! position lies in, how finely that beat is divided and which part of it the
//! position is ([`CountPos`]). A [`CountingSystem`] then names each position
//! ("1 e & a"), or leaves it unnamed. The positions know nothing of syllables
//! and the systems nothing of ticks.

use crate::grid::DEFAULT_GRID;
use crate::grouping::default_groups_for;
use crate::{Measure, TimeSignature};

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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CountingSystem {
    /// "1 e & a", "1 trip let", "1 la li".
    #[default]
    Numbers,
}

impl CountingSystem {
    /// The label of `pos`, `None` where the system has nothing to say.
    pub fn label(self, pos: &CountPos) -> Option<String> {
        match self {
            Self::Numbers => numbers(pos),
        }
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
    /// in x/8 and x/16.
    Beat { units: u8 },
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
        for &(start_tick, end_tick, units) in &grid.beats {
            let span = CountSpan::Beat { units };
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
        CountSpan::Beat { units } if pos.parts == 1 || pos.parts == units => match part {
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
        CountSpan::Beat { units } if units > 0 && pos.parts.is_multiple_of(units) => {
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
        assert_eq!(pos.span, CountSpan::Beat { units: 3 });
        assert_eq!((pos.beat, pos.unit, pos.part, pos.parts), (2, 4, 3, 6));
    }
}
