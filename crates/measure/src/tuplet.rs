//! Tuplet groups.
//!
//! The beats of a tuplet group stand next to each other in their voice. The
//! first carries [`TupletMark::Start`] with the group's base note value, the
//! others [`TupletMark::Continue`]. Everything else follows from the beats:
//! the ratio `n:m` is the one of their [`TupletSpec`]s, and the group spans
//! `m` of its base (see [`TupletGroup::span_ticks`]), so it ends where its
//! beats fill that span.
//!
//! The marks are positions, not names: two voices with the same beats and
//! groups are equal, and saving them needs no table on the side.

use crate::duration::{Duration, NoteValue, TupletSpec};
use crate::grid::DEFAULT_GRID;
use crate::{Beat, BeatIdx};
use serde::{Deserialize, Serialize};

/// Where a beat stands in its tuplet group.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TupletMark {
    /// The first beat of a group of `m` `base` notes' time (the `m` of the
    /// beats' ratio). The base can differ from the first beat's own value:
    /// a triplet of eighths can start with a triplet quarter.
    Start { base: NoteValue },
    /// A later beat of the group started before it.
    Continue,
}

impl TupletMark {
    /// The mark of a beat behind this one in the same group.
    pub(crate) fn follower(mark: Option<TupletMark>) -> Option<TupletMark> {
        mark.map(|_| TupletMark::Continue)
    }
}

/// A tuplet group of a voice: its beats and its ratio.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TupletGroup {
    pub start_idx: BeatIdx,
    /// Inclusive.
    pub end_idx: BeatIdx,
    /// The group's ratio on its base note value: `n` notes of `base` in the
    /// time of `m`. The beats are of this ratio, with their own values.
    pub spec: TupletSpec,
}

impl TupletGroup {
    /// The group's length in grid ticks: `m` of its base.
    pub fn span_ticks(&self) -> u32 {
        span_ticks(self.spec)
    }

    /// The group starting at `start_idx` of `beats`, by its marks. `None`
    /// if no group starts there.
    pub(crate) fn starting_at(beats: &[Beat], start_idx: BeatIdx) -> Option<TupletGroup> {
        let first = beats.get(start_idx)?;
        let Some(TupletMark::Start { base }) = first.tuplet else {
            return None;
        };
        let TupletSpec { n, m, .. } = first.duration.as_tuplet_spec()?;
        let len = beats[start_idx + 1..]
            .iter()
            .take_while(|b| b.tuplet == Some(TupletMark::Continue))
            .count();
        Some(TupletGroup { start_idx, end_idx: start_idx + len, spec: TupletSpec { n, m, base } })
    }
}

/// `m` of `spec`'s base in grid ticks: the span of a group of `spec`.
pub(crate) fn span_ticks(spec: TupletSpec) -> u32 {
    let base = DEFAULT_GRID.ticks_of(&Duration::Simple(spec.base)).unwrap_or(0);
    u32::from(spec.m) * base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::{q, t8};
    use crate::{Measure, TimeSignature};

    #[test]
    fn a_group_spans_m_of_its_base() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.first_voice_mut().set_beat(0, Beat::note(t8())).unwrap();
        let group = m.first_voice().tuplet_group_at(1).unwrap();
        assert_eq!((group.start_idx, group.end_idx), (0, 2));
        assert_eq!(group.span_ticks(), DEFAULT_GRID.ticks_of(&q()).unwrap());
        assert_eq!(
            m.first_voice().beats()[0].tuplet,
            Some(TupletMark::Start { base: NoteValue::Eighth })
        );
        assert_eq!(m.first_voice().beats()[2].tuplet, Some(TupletMark::Continue));
        assert_eq!(m.first_voice().beats()[3].tuplet, None);
    }
}
