use crate::duration::Duration;
use crate::fill::best_fill_for_gap;
use crate::measure::{Beat, Measure, TimeSignature};

/// Authoritative rhythm representation: a tree of equal-time slots (groups) and leaves.
#[derive(Clone, Debug, PartialEq, Eq)]
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
        let mut result = Self { time_signature, root: RhythmNode::Leaf(SlotContent::Rest) };
        result.init_beat_grid();
        result
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

    pub fn init_beat_grid(&mut self) -> bool {
        // Split root into one box per beat (e.g. 7 for 7/8, 4 for 4/4)
        let n = self.time_signature.beats as usize;
        // Initialize all children as Rest
        if self.subdivide(&[], n, SlotContent::Rest) {
            true
        } else {
            false
        }
    }

    /// Subdivide the node at `path` (empty path -> root) into `n` equal slots.
    pub fn subdivide(&mut self, path: &[usize], n: usize, init: SlotContent) -> bool {
        if n == 0 { return false; }
        let node = Self::get_mut(&mut self.root, path);
        match node {
            Some(RhythmNode::Group { .. }) | Some(RhythmNode::Leaf(_)) => {
                let children = vec![RhythmNode::Leaf(init.clone()); n];
                *node.unwrap() = RhythmNode::Group { n, children };
                true
            }
            None => false,
        }
    }

    pub fn unsplit(&mut self, path: &[usize]) -> bool {
        match Self::get_mut(&mut self.root, path) {
            Some(RhythmNode::Group { n: _, children }) => {
                let mut any_note = false;
                // let mut all_rest = true;
                for ch in children.iter() {
                    match ch {
                        RhythmNode::Leaf(SlotContent::Note) => {
                            any_note = true;
                            // all_rest = false;
                        }
                        RhythmNode::Leaf(SlotContent::Rest) => {}
                        RhythmNode::Group { .. } => {
                            // nested groups -> policy: treat as note
                            any_note = true;
                            // all_rest = false;
                        }
                    }
                }
                let leaf = if any_note { SlotContent::Note } else { SlotContent::Rest };
                *children = Vec::new();
                *Self::get_mut(&mut self.root, path).unwrap() = RhythmNode::Leaf(leaf);
                true
            }
            _ => false,
        }
    }

    /// Helper: traverse to mutable node at path.
    fn get_mut<'a>(node: &'a mut RhythmNode, path: &[usize]) -> Option<&'a mut RhythmNode> {
        if path.is_empty() { return Some(node); }
        match node {
            RhythmNode::Group { n: _n, children } => {
                let idx = path[0];
                children.get_mut(idx).and_then(|child| Self::get_mut(child, &path[1..]))
            }
            RhythmNode::Leaf(_) => None,
        }
    }

    /// Compute measure total ticks (delegates to time signature helper)
    fn measure_ticks(&self) -> i32 { self.time_signature.measure_duration_ticks() }

    /// Flatten this rhythm measure into a sequence of beats inside a Measure, preserving onsets implied by leaves with Note content.
    pub fn flatten_to_measure(&self) -> Measure {
        let mut out = Measure::new(self.time_signature.clone());
        let total = self.measure_ticks();
        Self::flatten_node(&self.root, total, &mut out);
        out
    }

    fn flatten_node(node: &RhythmNode, span_ticks: i32, out: &mut Measure) {
        match node {
            RhythmNode::Leaf(SlotContent::Note) => {
                Self::fill_span(out, span_ticks, true);
            }
            RhythmNode::Leaf(SlotContent::Rest) => {
                Self::fill_span(out, span_ticks, false);
            }
            RhythmNode::Group { n, children } => {
                let n = *n as i32;
                let slot = span_ticks / n;
                // If not divisible, we still try to distribute; but the UI should avoid that.
                for child in children {
                    Self::flatten_node(child, slot, out);
                }
            }
        }
    }

    /// Fill a span with minimal-token exact durations. If `first_is_note` then emit a Note for the first token and Rests after.
    fn fill_span(out: &mut Measure, ticks: i32, first_is_note: bool) {
        if ticks <= 0 { return; }
        if let Some(seq) = best_fill_for_gap(ticks) {
            if first_is_note {
                if let Some((first, rest)) = seq.split_first() {
                    let _ = out.add_beat(Beat::note(*first));
                    for d in rest { let _ = out.add_beat(Beat::rest(*d)); }
                }
            } else {
                for d in seq { let _ = out.add_beat(Beat::rest(d)); }
            }
        } else {
            // Fallback: tile with the smallest duration
            if let Some(&smallest) = Duration::DURATIONS.iter().min_by_key(|d| d.ticks()) {
                let mut rem = ticks;
                if first_is_note { let _ = out.add_beat(Beat::note(smallest)); rem -= smallest.ticks(); }
                while rem > 0 { let _ = out.add_beat(Beat::rest(smallest)); rem -= smallest.ticks(); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::Duration::{TripletEighth, SextupletSixteenth, Quarter, Eighth};

    #[test]
    fn flatten_empty_measure_over_four_four() {
        let rm = RhythmMeasure::new(TimeSignature::FOUR_FOUR);
        let m = rm.flatten_to_measure();
        let beats = m.beats();
        assert_eq!(beats.len(), 4);
        for b in beats.iter() {
            assert_eq!(b.duration.ticks(), Quarter.ticks());
        }
    }

    #[test]
    fn flatten_empty_measure_over_seven_eight() {
        let rm = RhythmMeasure::new(TimeSignature::SEVEN_EIGHT);
        let m = rm.flatten_to_measure();
        let beats = m.beats();
        assert_eq!(beats.len(), 7);
        for b in beats.iter() {
            assert_eq!(b.duration.ticks(), Eighth.ticks());
        }
    }

    #[test]
    fn flatten_triplet_over_one_four() {
        let mut rm = RhythmMeasure::new(TimeSignature::ONE_FOUR);
        // Subdivide root into 3 slots (triplet) and keep all as notes
        assert!(rm.subdivide(&[], 3, SlotContent::Rest));
        let m = rm.flatten_to_measure();
        let beats = m.beats();
        assert_eq!(beats.len(), 3);
        for b in beats {
            assert_eq!(b.duration.ticks(), TripletEighth.ticks());
        }
    }

    #[test]
    fn flatten_triplet_with_last_slot_subdivided_into_two() {
        let mut rm = RhythmMeasure::new(TimeSignature::ONE_FOUR);
        assert!(rm.subdivide(&[], 3, SlotContent::Rest));
        // Subdivide third slot (index 2) into two and keep both as notes
        assert!(rm.subdivide(&[2], 2, SlotContent::Rest));
        let m = rm.flatten_to_measure();
        let beats = m.beats();
        assert_eq!(beats.len(), 4);
        assert_eq!(beats[0].duration.ticks(), TripletEighth.ticks());
        assert_eq!(beats[1].duration.ticks(), TripletEighth.ticks());
        assert_eq!(beats[2].duration.ticks(), SextupletSixteenth.ticks());
        assert_eq!(beats[3].duration.ticks(), SextupletSixteenth.ticks());
    }

    #[test]
    fn cannot_flatten_triplets_over_one_sixteenth() {
        let mut rm = RhythmMeasure::new(TimeSignature::ONE_SIXTEENTH);
        rm.subdivide(&[], 3, SlotContent::Note);
        // Cannot flatten this structure because we don't support triplets over 16th
        let m = rm.flatten_to_measure();
        println!("{:?}", m)
    }
}
