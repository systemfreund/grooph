use crate::fill::best_fill_for_gap;
use crate::measure::{Beat, Measure, TimeSignature};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotContent {
    Note,
    Rest,
}

/// A tree of weighted groups and leaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RhythmNode {
    /// Weighted group: subdivide the current span proportionally to weights.
    /// Children length must equal `weights.len()`.
    Weighted { weights: Vec<u32>, children: Vec<RhythmNode> },
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
        Self { time_signature, root: RhythmNode::Leaf(SlotContent::Rest) }
    }

    /// Subdivide the node at `path` (empty path -> root) into `n` equal slots.
    pub fn subdivide(&mut self, path: &[usize], n: usize, init: SlotContent) -> bool {
        if n == 0 {
            return false;
        }
        let node = Self::get_mut(&mut self.root, path);
        match node {
            Some(RhythmNode::Weighted { .. }) | Some(RhythmNode::Leaf(_)) => {
                let children = vec![RhythmNode::Leaf(init); n];
                let weights = vec![1u32; n];
                *node.unwrap() = RhythmNode::Weighted { weights, children };
                true
            }
            None => false,
        }
    }

    /// Facade: Insert an n-in-m tuplet over `m_units` adjacent unit-children starting at `start_idx` under the node at `parent_path`.
    /// This rewrites the parent weighted group by replacing those children with a single child that spans `m_units` units,
    /// whose subtree is an equal weighted group of `n_tuplet` leaves initialized with `init`.
    pub fn insert_tuplet(
        &mut self,
        parent_path: &[usize],
        start_idx: usize,
        m_units: usize,
        n_tuplet: u8,
        init: SlotContent,
    ) -> bool {
        if m_units == 0 || n_tuplet == 0 {
            return false;
        }
        // Navigate to the parent node
        let parent_opt = Self::get_mut(&mut self.root, parent_path);
        match parent_opt {
            Some(RhythmNode::Weighted { weights, children }) => {
                if start_idx >= children.len() {
                    return false;
                }
                // We will cover exactly m_units adjacent "unit" children, counted by number of children, not by weights.
                // Require that the selected range exists and that each selected child has implied unit=1 in the parent.
                // To keep the facade simple, enforce that the selected slice has total weight == m_units.
                let end_idx = start_idx.saturating_add(m_units);
                if end_idx > children.len() {
                    return false;
                }
                let slice_weight: u32 = weights[start_idx..end_idx].iter().copied().sum();
                if slice_weight != m_units as u32 {
                    return false;
                }

                // Build inner tuplet group: equal n_tuplet leaves
                let inner_children = vec![RhythmNode::Leaf(init); n_tuplet as usize];
                let inner_weights = vec![1u32; n_tuplet as usize];
                let tuplet_node =
                    RhythmNode::Weighted { weights: inner_weights, children: inner_children };

                // Splice: replace [start_idx..end_idx) with single child of weight m_units
                children.splice(start_idx..end_idx, std::iter::once(tuplet_node));
                weights.splice(start_idx..end_idx, std::iter::once(m_units as u32));

                true
            }
            Some(RhythmNode::Leaf(_)) => {
                // Parent is a leaf; cannot insert under a leaf. If path points to leaf, we can replace leaf with a group spanning 1 unit then insert?
                // Minimal approach: turn the leaf into a weighted group of m_units units covering itself, then insert at index 0.
                // But to keep changes minimal, return false for now.
                false
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
            RhythmNode::Weighted { weights: _w, children } => {
                let idx = path[0];
                children.get_mut(idx).and_then(|child| Self::get_mut(child, &path[1..]))
            }
            RhythmNode::Leaf(_) => None,
        }
    }

    /// Compute measure total ticks (delegates to time signature helper)
    fn measure_ticks(&self) -> i32 { self.time_signature.measure_duration_ticks() }

    /// Flatten this rhythm measure into a sequence of beats inside a Measure, preserving onsets implied by leaves with Note content.
    pub fn flatten_to_measure(&self) -> Option<Measure> {
        let mut out = Measure::new(self.time_signature.clone());
        let total = self.measure_ticks();
        if Self::flatten_node(&self.root, total, &mut out) { Some(out) } else { None }
    }

    fn flatten_node(node: &RhythmNode, span_ticks: i32, out: &mut Measure) -> bool {
        match node {
            RhythmNode::Leaf(SlotContent::Note) => Self::fill_span(out, span_ticks, true),
            RhythmNode::Leaf(SlotContent::Rest) => Self::fill_span(out, span_ticks, false),
            RhythmNode::Weighted { weights, children } => {
                let sum_w: i32 = weights.iter().map(|&w| w as i32).sum();
                if sum_w <= 0 || span_ticks % sum_w != 0 {
                    return false;
                }
                let unit = span_ticks / sum_w;
                for (w, child) in weights.iter().zip(children.iter()) {
                    let child_span = unit * (*w as i32);
                    if !Self::flatten_node(child, child_span, out) {
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

impl RhythmMeasure {
    /// Derive a rhythm tree from a concrete Measure using pure inference.
    /// The resulting tree, when flattened, reproduces the original measure's beats exactly.
    pub fn derive_from_measure(m: &Measure) -> Option<RhythmMeasure> {
        use crate::duration::default_duration_set;
        let set = default_duration_set();
        let ts = m.time_signature();
        let unit_ticks = set.grid.ticks_per_whole / (ts.beat_unit as i32);

        // Pack beats as (ticks, content, duration)
        let mut beats: Vec<(i32, SlotContent, crate::duration::Duration)> = Vec::new();
        for b in m.beats().iter() {
            let t = set.grid.ticks_of(&b.duration)?;
            let sc = match b.kind {
                crate::measure::BeatKind::Note => SlotContent::Note,
                crate::measure::BeatKind::Rest => SlotContent::Rest,
            };
            beats.push((t, sc, b.duration));
        }

        // Cluster beats into top-level children whose total ticks are multiples of unit_ticks
        let mut root_weights: Vec<u32> = Vec::new();
        let mut root_children: Vec<RhythmNode> = Vec::new();
        let mut i = 0usize;
        while i < beats.len() {
            let start = i;
            let mut sum = 0i32;
            while i < beats.len() {
                sum += beats[i].0;
                i += 1;
                if sum % unit_ticks == 0 {
                    break;
                }
            }
            if sum % unit_ticks != 0 {
                return None;
            }
            let weight = (sum / unit_ticks) as u32;
            root_weights.push(weight);
            let child = Self::build_cluster_subtree(&beats[start..i])?;
            root_children.push(child);
        }

        // If there were no beats (empty measure), represent as a single Rest leaf
        if beats.is_empty() {
            return Some(RhythmMeasure {
                time_signature: ts,
                root: RhythmNode::Leaf(SlotContent::Rest),
            });
        }

        Some(RhythmMeasure {
            time_signature: ts,
            root: RhythmNode::Weighted { weights: root_weights, children: root_children },
        })
    }

    fn build_cluster_subtree(
        slice: &[(i32, SlotContent, crate::duration::Duration)],
    ) -> Option<RhythmNode> {
        if let Some(node) = Self::try_uniform_tuplet(slice) {
            return Some(node);
        }
        // General proportional subgroup: weights from gcd of ticks
        let mut g = 0i32;
        for (t, _, _) in slice.iter() {
            g = gcd_i32(g, *t);
        }
        if g <= 0 {
            return None;
        }
        let weights: Vec<u32> = slice.iter().map(|(t, _, _)| (*t / g) as u32).collect();
        let children: Vec<RhythmNode> =
            slice.iter().map(|(_, sc, _)| RhythmNode::Leaf(*sc)).collect();
        Some(RhythmNode::Weighted { weights, children })
    }

    fn try_uniform_tuplet(
        slice: &[(i32, SlotContent, crate::duration::Duration)],
    ) -> Option<RhythmNode> {
        use crate::duration::Duration;
        if slice.is_empty() {
            return None;
        }
        let first_dur = slice[0].2;
        let (n, _m, _base) = match first_dur {
            Duration::Tuplet { n, m, base } => (n as usize, m, base),
            _ => return None,
        };
        if slice.len() != n {
            return None;
        }
        if !slice.iter().all(|(_, _, d)| *d == first_dur) {
            return None;
        }
        let weights = vec![1u32; n];
        let children: Vec<RhythmNode> =
            slice.iter().map(|(_, sc, _)| RhythmNode::Leaf(*sc)).collect();
        Some(RhythmNode::Weighted { weights, children })
    }
}

fn gcd_i32(mut a: i32, mut b: i32) -> i32 {
    if a == 0 {
        return b.abs();
    }
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a.abs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::{Duration, NoteValue};
    fn q() -> Duration { Duration::Simple(NoteValue::Quarter) }
    fn e() -> Duration { Duration::Simple(NoteValue::Eighth) }
    fn s16() -> Duration { Duration::Simple(NoteValue::Sixteenth) }
    fn t8() -> Duration { Duration::Tuplet { n: 3, m: 2, base: NoteValue::Eighth } }
    fn sx16() -> Duration { Duration::Tuplet { n: 6, m: 4, base: NoteValue::Sixteenth } }

    fn assert_flattened(rm: RhythmMeasure, expected_beat_durations: Vec<Duration>) {
        let m = rm.flatten_to_measure().unwrap();
        println!("{:?}", m);
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
        let rm: RhythmMeasure = RhythmMeasure::new(TimeSignature::TWO_SIXTEENTH);
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

    #[test]
    fn derive_from_measure_triplet_then_eighth_in_3_8() {
        use crate::duration::default_duration_set;
        let mut m = Measure::new(TimeSignature { beats: 3, beat_unit: 8 });
        let t8 = Duration::Tuplet { n: 3, m: 2, base: NoteValue::Eighth };
        let e = Duration::Simple(NoteValue::Eighth);
        assert!(m.add_beat(Beat::note(t8)).is_ok());
        assert!(m.add_beat(Beat::note(t8)).is_ok());
        assert!(m.add_beat(Beat::note(t8)).is_ok());
        assert!(m.add_beat(Beat::note(e)).is_ok());

        // Derive rhythm tree and flatten back to a measure
        let rm = RhythmMeasure::derive_from_measure(&m).expect("derive");
        let m2 = rm.flatten_to_measure().expect("flatten");

        println!("{:?}", rm);
        println!("{}", m);
        println!("{}", m2);

        // Compare tick sequences for equality
        let set = default_duration_set();
        let ticks1: Vec<i32> =
            m.beats().iter().map(|b| set.grid.ticks_of(&b.duration).unwrap()).collect();
        let ticks2: Vec<i32> =
            m2.beats().iter().map(|b| set.grid.ticks_of(&b.duration).unwrap()).collect();
        assert_eq!(ticks1, ticks2);
    }
}
