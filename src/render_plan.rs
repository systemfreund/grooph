use crate::beaming::{BeamPlan, compute_beam_plan};
use crate::duration::{Duration, NoteValue};
use crate::measure::{Beat, BeatKind, Measure};

/// Logische Beat-Index innerhalb eines Taktes (0-basiert)
pub type BeatIdx = usize;

#[derive(Debug, Clone, PartialEq)]
pub struct NoteLayout {
    pub beat: BeatIdx,
    /// einfache Raster-Position, aktuell = beat als f32 (UI skaliert das später)
    pub x_logical: f32,
    pub duration: Duration,
    pub is_rest: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeamGroupPlan {
    /// Indizes der Noten (keine Rests) in dieser Beam-Gruppe
    pub note_indices: Vec<BeatIdx>,
    /// Per-Note Anzahl der Beam-Ebenen (gleich lang wie note_indices)
    pub beam_counts: Vec<u8>,
    /// Für jedes benachbarte Paar (i->i+1) wie viele Beams durchgezogen werden
    /// Länge = note_indices.len() - 1
    pub continuity: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TupletPlan {
    /// z. B. 3 für Triplet
    pub count: u8,
    /// inklusive Endindex (geschlossenes Intervall): start..=end
    pub start: BeatIdx,
    pub end: BeatIdx,
    /// Basis-Notenwert (für ggf. spätere Darstellungen nützlich)
    pub base: NoteValue,
    /// true, wenn die Gruppe vollständig mit Balken verbunden ist (keine Klammer nötig)
    pub fully_beamed: bool,
    /// true, wenn irgendeine Pause innerhalb der Gruppe liegt (dann immer Klammer)
    pub contains_rest: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderPlan {
    pub notes: Vec<NoteLayout>,
    pub beams: Vec<BeamGroupPlan>,
    pub tuplets: Vec<TupletPlan>,
}

pub fn plan_measure(measure: &Measure) -> RenderPlan {
    let beats = measure.beats();

    // Notes/Rest-Layouts mit trivialer logischer X-Position (Beat-Index)
    let mut notes: Vec<NoteLayout> = Vec::with_capacity(beats.len());
    for (i, b) in beats.iter().enumerate() {
        notes.push(NoteLayout {
            beat: i,
            x_logical: i as f32,
            duration: b.duration,
            is_rest: b.kind == BeatKind::Rest,
        });
    }

    let BeamPlan { groups } = compute_beam_plan(measure);
    let beams: Vec<BeamGroupPlan> = groups
        .into_iter()
        .map(|g| BeamGroupPlan {
            note_indices: g.note_indices,
            beam_counts: g.beam_counts,
            continuity: g.continuity,
        })
        .collect();

    // Tuplets bestimmen
    let tuplets = discover_tuplets(measure, &beams);

    RenderPlan { notes, beams, tuplets }
}

fn discover_tuplets(measure: &Measure, beams: &Vec<BeamGroupPlan>) -> Vec<TupletPlan> {
    let beats = measure.beats();
    let set = crate::duration::default_duration_set();

    #[derive(Debug)]
    struct TupGroupTmp {
        start: usize,
        end: usize,
        n: u8,
        m: u8,
        base: NoteValue,
        contains_rest: bool,
    }

    let mut tmp: Vec<TupGroupTmp> = Vec::new();
    let mut i = 0usize;
    while i < beats.len() {
        let Duration::Tuplet { n, m, .. } = beats[i].duration else {
            i += 1;
            continue;
        };
        // Maximalen Lauf gleicher (n,m) finden
        let mut k = i;
        while k < beats.len() {
            match beats[k].duration {
                Duration::Tuplet { n: nn, m: mm, .. } if nn == n && mm == m => k += 1,
                _ => break,
            }
        }
        // Den Lauf in logische Gruppen nach Tick-Summe aufteilen
        let mut start = i;
        while start < k {
            let first_dur = beats[start].duration;
            let target_ticks = set.grid.ticks_of(&first_dur).unwrap_or(0).saturating_mul(n as u32);
            let mut acc_ticks: u32 = 0;
            let mut end = start;
            let mut has_rest = false;
            while end < k {
                if beats[end].kind == BeatKind::Rest {
                    has_rest = true;
                }
                let dt = set.grid.ticks_of(&beats[end].duration).unwrap_or(0);
                acc_ticks = acc_ticks.saturating_add(dt);
                if acc_ticks >= target_ticks {
                    break;
                }
                end += 1;
            }
            tmp.push(TupGroupTmp {
                start,
                end,
                n,
                m,
                base: beats[start].duration.base_note(),
                contains_rest: has_rest,
            });
            start = end + 1;
        }
        i = k;
    }

    // fully_beamed bestimmen: wenn alle Noten (min. 2) der Gruppe innerhalb einer BeamGroup liegen
    // und alle benachbarten Paare continuity >= 1 haben.
    let mut out: Vec<TupletPlan> = Vec::with_capacity(tmp.len());
    for g in tmp.into_iter() {
        let note_idxs: Vec<usize> =
            (g.start..=g.end).filter(|&ix| beats[ix].kind == BeatKind::Note).collect();
        let fully = if g.contains_rest || note_idxs.len() < 2 {
            false
        } else {
            // Prüfe gegen jede BeamGroup
            let mut ok_any = false;
            'bg: for bg in beams.iter() {
                // Alle Noten enthalten?
                if note_idxs.iter().all(|ix| bg.note_indices.contains(ix)) {
                    // Mappe BeatIndex -> Position in der BeamGroup
                    let mut pos_map = std::collections::HashMap::new();
                    for (li, gi2) in bg.note_indices.iter().enumerate() {
                        pos_map.insert(*gi2, li);
                    }
                    // Prüfe alle benachbarten Paare auf continuity >= 1
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

        out.push(TupletPlan {
            count: g.n,
            start: g.start,
            end: g.end,
            base: g.base,
            fully_beamed: fully,
            contains_rest: g.contains_rest,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::{e, t8};
    use crate::measure::BeatKind::Note;
    use crate::measure::{Beat, Measure, TimeSignature};

    #[test]
    fn beaming_group_within_primary_boundaries_in_seven_eight() {
        // 7/8 mit Achteln, Standardgruppierung 2+3+2.
        // Die mittlere Gruppe (Beats 2..=4, 0-basiert) sollte beamed sein.
        let mut m = Measure::new(TimeSignature::SEVEN_EIGHT);
        for i in 0..7 {
            m.set_beat_at(i, Beat::note(e())).unwrap();
        }

        let plan = plan_measure(&m);
        let has_group = plan.beams.iter().any(|g| {
            let idxs = &g.note_indices;
            if !(idxs.contains(&2) && idxs.contains(&3) && idxs.contains(&4)) {
                return false;
            }
            let mut pos_map = std::collections::HashMap::new();
            for (i, gi) in idxs.iter().enumerate() {
                pos_map.insert(*gi, i);
            }
            let l2 = *pos_map.get(&2).unwrap();
            let l3 = *pos_map.get(&3).unwrap();
            let l4 = *pos_map.get(&4).unwrap();
            if !(l2 < l3 && l3 < l4) {
                return false;
            }
            g.continuity.get(l2).copied().unwrap_or(0) >= 1
                && g.continuity.get(l3).copied().unwrap_or(0) >= 1
        });
        assert!(has_group, "Beats 3-5 sollten in 7/8 beamed sein (2+3+2, mittlere Gruppe)");
    }

    #[test]
    fn triplet_bracket_over_beats_4_to_6() {
        // Konstruiere einen 4/4, in dem Beats 3..=5 (0-basiert) eine Triole bilden
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Setze zunächst 6 Achtel, dann eine Triole über die nächsten 3 Achtel-Schlitze
        for i in 0..3 {
            m.set_beat_at(i, Beat::note(e())).unwrap();
        }
        // Drei Achtel-Triolett-Noten
        m.set_beat_at(3, Beat::note(t8())).unwrap();
        m.set_beat_at(4, Beat::note(t8())).unwrap();
        m.set_beat_at(5, Beat::note(t8())).unwrap();

        let plan = plan_measure(&m);
        let ok = plan.tuplets.iter().any(|t| t.count == 3 && t.start == 3 && t.end == 5);
        assert!(ok, "Triplet-Klammer sollte über Beats 4–6 (3..=5) liegen");
    }

    #[test]
    fn triplet_bracket_over_beats_when_preceding_beat_is_connected_to_triplet_with_beams() {
        let mut m = Measure::new_init(TimeSignature::SEVEN_EIGHT, Note);
        m.set_beat_at(3, Beat::note(t8())).unwrap();
        m.set_beat_at(4, Beat::note(t8())).unwrap();
        m.set_beat_at(5, Beat::note(t8())).unwrap();

        let plan = plan_measure(&m);

        // Beam connects the triplets (3,4,5) with the preceding beat (2).
        // Expected because of the default 2+3+2 grouping in 7/8.
        assert_eq!(plan.beams[1].note_indices, vec![2, 3, 4, 5]);

        // We expect a bracket to be drawn over the triplets (3,4,5) to visually distinguish them
        // from the preceding beat.
        assert!(!plan.tuplets[0].fully_beamed);
    }
}
