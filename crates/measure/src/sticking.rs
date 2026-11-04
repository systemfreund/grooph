//! Sticking: which hand plays a beat.
//!
//! A beat with a hand instrument ([`Instrument::is_hand`]) can carry one
//! sticking for all its heads ([`Beat::sticking`]): a chord of hi-hat and
//! snare in one voice gets one. It is stored on the beat, so it moves with
//! its note when notes before it are inserted, deleted or split.
//!
//! The staff writes one sign per column, a point in time where beats of
//! one or more voices start ([`Measure::stickings`]).
//!
//! [`Instrument::is_hand`]: crate::Instrument::is_hand

use crate::grid::DEFAULT_GRID;
use crate::{Beat, Measure};
use serde::{Deserialize, Serialize};

/// Which hand plays a beat.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Sticking {
    Right,
    Left,
    /// Both hands at once, written R above L.
    Both,
}

impl Sticking {
    pub const ALL: [Sticking; 3] = [Sticking::Right, Sticking::Left, Sticking::Both];

    /// The sign of one column where beats with `a` and `b` start: the
    /// same sticking stays, two different ones are both hands.
    pub fn merge(a: Sticking, b: Sticking) -> Sticking {
        if a == b { a } else { Sticking::Both }
    }

    /// The letters written for it, top first: `R`, `L`, or `R` above `L`.
    pub fn letters(self) -> &'static [&'static str] {
        match self {
            Sticking::Right => &["R"],
            Sticking::Left => &["L"],
            Sticking::Both => &["R", "L"],
        }
    }
}

impl Measure {
    /// One sticking per column (tick) where a beat of any voice with a
    /// sticking starts, in time order: that beat's, or, where beats there
    /// carry different ones or one carries both hands, both
    /// ([`Sticking::merge`]).
    pub fn stickings(&self) -> Vec<(u32, Sticking)> {
        let mut columns: Vec<(u32, Sticking)> = Vec::new();
        for voice in self.voices() {
            let ticks = DEFAULT_GRID.compute_onset_ticks(voice.beats());
            for (tick, sticking) in ticks.into_iter().zip(voice.beats().iter().map(Beat::sticking))
            {
                let Some(sticking) = sticking else { continue };
                match columns.iter_mut().find(|(t, _)| *t == tick) {
                    Some((_, s)) => *s = Sticking::merge(*s, sticking),
                    None => columns.push((tick, sticking)),
                }
            }
        }
        columns.sort_by_key(|&(tick, _)| tick);
        columns
    }
}

#[cfg(test)]
mod tests {
    use super::Sticking::{Both, Left, Right};
    use super::*;
    use crate::duration::q;
    use crate::notation::parse_score;

    #[test]
    fn a_column_merges_the_stickings_of_its_voices() {
        let score = parse_score("4/4 H:q@R H:q@L H:q@R H:q ; K:q r:q K:q r:q").unwrap();
        let ticks = DEFAULT_GRID.ticks_of(&q()).unwrap();
        assert_eq!(
            score.measures()[0].stickings(),
            [(0, Right), (ticks, Left), (2 * ticks, Right)]
        );
        // The same sticking in both voices stays; different ones, or both
        // hands in one voice, are both.
        let score = parse_score("4/4 S:q@R S:q@R S:q@RL r:q ; T1:q@R FT:q@L T1:q r:q").unwrap();
        assert_eq!(score.measures()[0].stickings(), [(0, Right), (ticks, Both), (2 * ticks, Both)]);
        // Beats of one voice between the other's are their own columns.
        let score = parse_score("4/4 S:h@L S:h ; FT:q FT:q@R r:h").unwrap();
        assert_eq!(score.measures()[0].stickings(), [(0, Left), (ticks, Right)]);
    }
}
