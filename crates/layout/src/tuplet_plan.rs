use crate::beam_plan::BeamGroup;
use grooph_measure::duration::{Duration, NoteValue};
use grooph_measure::{BeatIdx, BeatKind, Measure};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TupletPlan {
    /// E.g. 3 for a triplet
    pub count: u8,
    /// Inclusive span: start..=end
    pub start: BeatIdx,
    pub end: BeatIdx,
    /// Base note value
    pub base: NoteValue,
    /// The whole group is beamed together (no bracket needed)
    pub fully_beamed: bool,
    /// The group contains a rest not under a beam (always bracketed)
    pub contains_rest: bool,
    /// How the group's beams connect to its neighbours
    pub edge_connection: EdgeConnection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeConnection {
    None,
    Left,
    Right,
    Both,
}

impl TupletPlan {
    pub fn is_externally_connected(&self) -> bool {
        self.edge_connection != EdgeConnection::None
    }

    pub fn number_only(&self) -> bool {
        self.fully_beamed && !self.contains_rest && !self.is_externally_connected()
    }
}

/// A tuplet span found in a measure, independent of beaming
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TupletSpan {
    pub count: u8,
    pub start: BeatIdx,
    pub end: BeatIdx, // inclusive
    pub base: NoteValue,
    pub contains_rest: bool,
}

/// Finds tuplet spans from the measure alone, without beaming information.
pub(crate) fn detect_tuplet_spans(measure: &Measure) -> Vec<TupletSpan> {
    let beats = measure.beats();
    measure
        .tuplet_groups()
        .into_iter()
        .filter_map(|group| {
            let anchor = measure.tuplets().get(group.id)?;
            let contains_rest =
                (group.start_idx..=group.end_idx).any(|i| beats[i].kind == BeatKind::Rest);
            Some(TupletSpan {
                count: anchor.n,
                start: group.start_idx,
                end: group.end_idx,
                base: anchor.base_hint,
                contains_rest,
            })
        })
        .collect()
}

pub fn compute_tuplet_plan(measure: &Measure, beams: &[BeamGroup]) -> Vec<TupletPlan> {
    let beats = measure.beats();
    let spans = detect_tuplet_spans(measure);

    let mut out: Vec<TupletPlan> = Vec::with_capacity(spans.len());
    // A rest under a beam, a stemlet or a rest between two beamed notes,
    // counts as a note: it breaks nothing.
    let beamed = |ix: BeatIdx| beams.iter().any(|bg| bg.beat_indices.contains(&ix));
    let under_beam = |ix: BeatIdx| {
        beams.iter().any(|bg| match (bg.beat_indices.first(), bg.beat_indices.last()) {
            (Some(&first), Some(&last)) => first <= ix && ix <= last,
            _ => false,
        })
    };
    for mut g in spans.into_iter() {
        let note_idxs: Vec<BeatIdx> = (g.start..=g.end)
            .filter(|&ix| beats[ix].kind == BeatKind::Note || beamed(ix))
            .collect();
        g.contains_rest =
            (g.start..=g.end).any(|ix| beats[ix].kind == BeatKind::Rest && !under_beam(ix));

        let fully_beamed = if g.contains_rest || note_idxs.len() < 2 {
            false
        } else {
            let mut ok_any = false;
            'bg: for bg in beams.iter() {
                if note_idxs.iter().all(|ix| bg.beat_indices.contains(ix)) {
                    let mut pos_map = std::collections::HashMap::new();
                    for (li, gi2) in bg.beat_indices.iter().enumerate() {
                        pos_map.insert(*gi2, li);
                    }
                    let mut ok = true;
                    for pair in note_idxs.windows(2) {
                        let a = pair[0];
                        let b = pair[1];
                        let la = *pos_map.get(&a).unwrap();
                        let lb = *pos_map.get(&b).unwrap();
                        if la >= lb {
                            ok = false;
                            break;
                        }
                        for cidx in la..lb {
                            if *bg.continuity.get(cidx).unwrap_or(&0) < 1 {
                                ok = false;
                                break;
                            }
                        }
                        if !ok {
                            break;
                        }
                    }
                    if ok {
                        ok_any = true;
                        break 'bg;
                    }
                }
            }
            ok_any
        };

        // Whether the beam at `ix` runs on to a beat outside the tuplet, to
        // the left (`step` -1) or right (+1), over rests or not. Other
        // tuplets never share a beam with it.
        let beamed_out = |ix: BeatIdx, step: isize| {
            beams.iter().any(|bg| {
                let Some(p) = bg.beat_indices.iter().position(|&x| x == ix) else {
                    return false;
                };
                let (q, link) = if step < 0 {
                    let Some(q) = p.checked_sub(1) else { return false };
                    (q, q)
                } else {
                    (p + 1, p)
                };
                bg.beat_indices.get(q).is_some_and(|&n| {
                    (n < g.start || n > g.end)
                        && !matches!(beats[n].duration, Duration::Tuplet(_))
                        && *bg.continuity.get(link).unwrap_or(&0) >= 1
                })
            })
        };
        let ext_left = note_idxs.first().is_some_and(|&fi| beamed_out(fi, -1));
        let ext_right = note_idxs.last().is_some_and(|&li| beamed_out(li, 1));

        let edge_connection = if g.contains_rest {
            EdgeConnection::None
        } else {
            match (ext_left, ext_right) {
                (false, false) => EdgeConnection::None,
                (true, false) => EdgeConnection::Left,
                (false, true) => EdgeConnection::Right,
                (true, true) => EdgeConnection::Both,
            }
        };

        out.push(TupletPlan {
            count: g.count,
            start: g.start,
            end: g.end,
            base: g.base,
            fully_beamed,
            contains_rest: g.contains_rest,
            edge_connection,
        });
    }

    out
}
