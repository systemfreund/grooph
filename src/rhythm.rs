use crate::fill::best_fill_for_gap;
use crate::measure::{Beat, Measure, TimeSignature};

/// Authoritative rhythm representation: a tree of equal-time slots (groups) and leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotContent {
    /// Click at the start of the span; remainder of the span is silent.
    Note,
    /// Entire span is silent unless subdivided further.
    Rest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RhythmNode {
    /// Subdivide the current span into `n` equal slots.
    /// Children length must equal `n`.
    Group { n: usize, children: Vec<RhythmNode> },
    /// Leaf occupying the entire span.
    Leaf(SlotContent),
}

#[derive(Clone, Debug)]
pub struct RhythmMeasure {
    pub time_signature: TimeSignature,
    pub root: RhythmNode,
}

impl RhythmMeasure {
    pub fn new(time_signature: TimeSignature) -> Self {
        Self {
            time_signature,
            root: RhythmNode::Leaf(SlotContent::Rest),
        }
    }

    pub fn toggle_leaf(&mut self, path: &[usize]) -> bool {
        match Self::get_mut(&mut self.root, path) {
            Some(RhythmNode::Leaf(content)) => {
                *content = match content {
                    SlotContent::Note => SlotContent::Rest,
                    SlotContent::Rest => SlotContent::Note,
                };
                true
            }
            _ => false,
        }
    }

    /// Subdivide the node at `path` (empty path -> root) into `n` equal slots.
    pub fn subdivide(&mut self, path: &[usize], n: usize, init: SlotContent) -> bool {
        if n == 0 {
            return false;
        }
        let node = Self::get_mut(&mut self.root, path);
        match node {
            Some(RhythmNode::Group { .. }) | Some(RhythmNode::Leaf(_)) => {
                let children = vec![RhythmNode::Leaf(init); n];
                *node.unwrap() = RhythmNode::Group { n, children };
                true
            }
            None => false,
        }
    }

    /// Helper: traverse to mutable node at path.
    fn get_mut<'a>(node: &'a mut RhythmNode, path: &[usize]) -> Option<&'a mut RhythmNode> {
        if path.is_empty() {
            return Some(node);
        }
        match node {
            RhythmNode::Group { n: _n, children } => {
                let idx = path[0];
                children
                    .get_mut(idx)
                    .and_then(|child| Self::get_mut(child, &path[1..]))
            }
            RhythmNode::Leaf(_) => None,
        }
    }

    /// Compute measure total ticks (delegates to time signature helper)
    fn measure_ticks(&self) -> i32 {
        self.time_signature.measure_duration_ticks()
    }

    /// Flatten this rhythm measure into a sequence of beats inside a Measure, preserving onsets implied by leaves with Note content.
    pub fn flatten_to_measure(&self) -> Option<Measure> {
        let mut out = Measure::new(self.time_signature.clone());
        let total = self.measure_ticks();
        if Self::flatten_node(&self.root, total, &mut out) {
            Some(out)
        } else {
            None
        }
    }

    fn flatten_node(node: &RhythmNode, span_ticks: i32, out: &mut Measure) -> bool {
        match node {
            RhythmNode::Leaf(SlotContent::Note) => Self::fill_span(out, span_ticks, true),
            RhythmNode::Leaf(SlotContent::Rest) => Self::fill_span(out, span_ticks, false),
            RhythmNode::Group { n, children } => {
                let n = *n as i32;
                if span_ticks % n != 0 {
                    return false;
                }
                let slot = span_ticks / n;
                for child in children {
                    if !Self::flatten_node(child, slot, out) {
                        return false;
                    }
                }
                true
            }
        }
    }

    /// Fill a span with minimal-token exact durations. If `first_is_note` then emit a Note for the first token and Rests after.
    fn fill_span(out: &mut Measure, ticks: i32, first_is_note: bool) -> bool {
        if ticks <= 0 {
            return true;
        }
        if let Some(seq) = best_fill_for_gap(ticks) {
            if first_is_note {
                if let Some((first, rest)) = seq.split_first() {
                    if out.add_beat(Beat::note(*first)).is_err() {
                        return false;
                    }
                    for d in rest {
                        if out.add_beat(Beat::rest(*d)).is_err() {
                            return false;
                        }
                    }
                }
            } else {
                for d in seq {
                    if out.add_beat(Beat::rest(d)).is_err() {
                        return false;
                    }
                }
            }
            true
        } else {
            // No exact fill possible → fail
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::{Duration, NoteValue};
    fn q() -> Duration {
        Duration::Simple(NoteValue::Quarter)
    }
    fn e() -> Duration {
        Duration::Simple(NoteValue::Eighth)
    }
    fn s16() -> Duration {
        Duration::Simple(NoteValue::Sixteenth)
    }
    fn t8() -> Duration {
        Duration::Tuplet {
            n: 3,
            m: 2,
            base: NoteValue::Eighth,
        }
    }
    fn sx16() -> Duration {
        Duration::Tuplet {
            n: 6,
            m: 4,
            base: NoteValue::Sixteenth,
        }
    }

    fn assert_flattened(rm: RhythmMeasure, expected_beat_durations: Vec<Duration>) {
        let m = rm.flatten_to_measure().unwrap();
        println!("{}", m);
        let beats = m.beats();
        assert_eq!(beats.len(), expected_beat_durations.len());
        for (idx, b) in beats.iter().enumerate() {
            assert_eq!(b.duration, expected_beat_durations[idx]);
        }
    }

    #[test]
    fn flatten_empty_measure_over_four_four() {
        let rm = RhythmMeasure::new(TimeSignature::FOUR_FOUR);
        assert_flattened(rm, vec![q(), q(), q(), q()]);
    }

    #[test]
    fn flatten_triplet_over_one_four() {
        let mut rm = RhythmMeasure::new(TimeSignature::ONE_FOUR);
        assert!(rm.subdivide(&[], 3, SlotContent::Rest));
        assert_flattened(rm, vec![t8(), t8(), t8()]);
    }

    #[test]
    fn flatten_empty_measure_over_seven_eight() {
        let rm = RhythmMeasure::new(TimeSignature::SEVEN_EIGHT);
        assert_flattened(rm, vec![q(), q(), q(), e()]);
    }

    #[test]
    fn flatten_triplet_over_two_eight() {
        let mut rm = RhythmMeasure::new(TimeSignature::TWO_EIGHT);
        assert!(rm.subdivide(&[], 3, SlotContent::Rest));
        assert_flattened(rm, vec![t8(), t8(), t8()]);
    }

    #[test]
    fn flatten_triplet_with_last_slot_subdivided_into_two() {
        let mut rm = RhythmMeasure::new(TimeSignature::ONE_FOUR);
        assert!(rm.subdivide(&[], 3, SlotContent::Rest));
        assert!(rm.subdivide(&[2], 2, SlotContent::Rest));

        // falsch?
        assert_flattened(rm, vec![t8(), t8(), sx16(), sx16()]);
    }

    #[test]
    fn flatten_two_sixteenth_measure_with_several_subdivision() {
        let mut rm = RhythmMeasure::new(TimeSignature::TWO_SIXTEENTH);
        assert_flattened(rm, vec![e()]);
        // rm.subdivide(&[], 1, SlotContent::Note);
    }

    #[test]
    fn cannot_flatten_triplets_over_one_sixteenth() {
        let mut rm = RhythmMeasure::new(TimeSignature::ONE_SIXTEENTH);
        rm.subdivide(&[], 3, SlotContent::Note);
        // Cannot flatten this structure because we don't support 32nd-triplets over 1/16 notes.
        assert!(rm.flatten_to_measure().is_none());
    }

    #[test]
    fn flatten_16th_triplets_over_two_sixteenth() {
        let mut rm = RhythmMeasure::new(TimeSignature::TWO_SIXTEENTH);
        rm.subdivide(&[], 3, SlotContent::Note);
        assert_flattened(rm, vec![sx16(), sx16(), sx16()]);
    }
}
