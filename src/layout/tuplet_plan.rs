use crate::layout::beam_plan::BeamGroup;
use crate::layout::render_plan::BeatIdx;
use crate::measure::duration::{Duration, NoteValue};
use crate::measure::{BeatKind, Measure};

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
    /// Verbindung der Tuplet-Gruppe nach außen über Balken
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
    pub fn is_externally_connected(&self) -> bool { self.edge_connection != EdgeConnection::None }

    pub fn number_only(&self) -> bool {
        self.fully_beamed && !self.contains_rest && !self.is_externally_connected()
    }
}

pub fn compute_tuplet_plan(measure: &Measure, beams: &[BeamGroup]) -> Vec<TupletPlan> {
    let beats = measure.beats();
    let set = crate::measure::duration::default_duration_set();

    #[derive(Debug)]
    struct TupGroupTmp {
        start: BeatIdx,
        end: BeatIdx,
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
        // Maximalen Lauf gleicher (n,m) finden (Basis darf variieren)
        let mut k = i;
        while k < beats.len() {
            match beats[k].duration {
                Duration::Tuplet { n: nn, m: mm, .. } if nn == n && mm == m => k += 1,
                _ => break,
            }
        }

        // Bestimme feinste und gröbste Basisnote innerhalb des Laufs
        let mut run_min_base = beats[i].duration.base_note();
        let mut run_min_ticks =
            set.grid.ticks_of(&Duration::Simple(run_min_base)).unwrap_or(u32::MAX);
        let mut run_max_base = run_min_base;
        let mut run_max_ticks = set.grid.ticks_of(&Duration::Simple(run_max_base)).unwrap_or(0);
        for beat in beats.iter().take(k).skip(i) {
            let b = beat.duration.base_note();
            if let Some(t) = set.grid.ticks_of(&Duration::Simple(b)) {
                if t < run_min_ticks {
                    run_min_ticks = t;
                    run_min_base = b;
                }
                if t > run_max_ticks {
                    run_max_ticks = t;
                    run_max_base = b;
                }
            }
        }

        // Segment the run: each segment is oriented to the base of the first slot
        // (not to the finest base of the entire run). This prevents phantom segments
        // when a t16 group begins immediately after a t32 group.
        let mut start = i;
        while start < k {
            // Ziel dynamisch anhand der feinsten Basis innerhalb des Segments bestimmen
            let mut seg_min_base = beats[start].duration.base_note();
            let mut seg_min_ticks = set.grid.ticks_of(&Duration::Simple(seg_min_base)).unwrap_or(0);

            let mut acc_ticks: u32 = 0;
            let mut end = start;
            let mut has_rest = false;
            let mut reached_target = false;
            // Heuristik: Wenn die Gruppe tatsächlich in eine feinere Untergruppe gesplittet hat,
            // dann ist der nächste Slot von gleicher Basis wie der Start (z. B. t16,t16,…)
            let start_base = beats[start].duration.base_note();
            let next_same_base =
                (start + 1) < k && beats[start + 1].duration.base_note() == start_base;
            while end < k {
                // Update minimaler Basiswert
                let b = beats[end].duration.base_note();
                if let Some(bt) = set.grid.ticks_of(&Duration::Simple(b))
                    && bt < seg_min_ticks
                {
                    seg_min_ticks = bt;
                    seg_min_base = b;
                }

                if beats[end].kind == BeatKind::Rest {
                    has_rest = true;
                }
                let dt = set.grid.ticks_of(&beats[end].duration).unwrap_or(0);
                acc_ticks = acc_ticks.saturating_add(dt);
                // Zielgröße: Am Beginn eines Tuplet-Laufs orientieren wir uns an der
                // feinsten Basis des gesamten Laufs. Dadurch umfasst die erste
                // Klammer die vollständige logische Gruppe, auch wenn der erste
                // Slot feiner unterteilt wurde (z. B. t16 statt t8 bei einem Triplet).
                // Bei nachfolgenden Segmenten bleiben wir segmentlokal, um keine
                // "Phantomsegmente" über eine unmittelbar vorausgehende feinere Gruppe
                // hinweg zu erzeugen (siehe Tests).
                let target_per_group_ticks = if start == i && next_same_base {
                    // Wir starten innerhalb einer aufgespaltenen Einheit → volle logische Gruppe
                    run_max_ticks.saturating_mul(m as u32)
                } else {
                    // Standard: segmentlokales Ziel (ermöglicht verkürzte Klammern)
                    seg_min_ticks.saturating_mul(m as u32)
                };
                if acc_ticks >= target_per_group_ticks {
                    reached_target = true;
                    break;
                }
                end += 1;
            }

            // Fallback: Wenn das Ziel innerhalb des Runs nicht erreicht werden kann,
            // bilde dennoch ein Segment bis zum Ende des Runs (verkürzte Klammer),
            // sofern überhaupt etwas akkumuliert wurde.
            if !reached_target && end >= start {
                // Wir sind bis ans Run-Ende gelaufen → nutze den letzten verfügbaren Index
                end = k.saturating_sub(1);
                reached_target = end >= start;
            }

            // Wie viele Noten enthält [start..=end]? (nur für fully_beamed später relevant)
            // Für die Segment-Erstellung selbst akzeptieren wir auch Segmente mit nur Rests,
            // da Tuplet-Klammern über reine Pausen hinweg ebenfalls semantisch sinnvoll sind.
            if reached_target {
                // Wichtig: Bewahre die Slot‑Grenzen (inkl. evtl. Rests) — das entspricht der logischen Spannweite
                tmp.push(TupGroupTmp {
                    start,
                    end,
                    n,
                    m,
                    // Basiswahl: wenn wir das logische Ziel verwendet haben, nutze run_max_base,
                    // sonst seg_min_base.
                    base: if start == i && next_same_base { run_max_base } else { seg_min_base },
                    contains_rest: has_rest,
                });
                start = end + 1;
            } else {
                // unvollständig/zu klein → versuche ab nächstem Slot erneut
                start += 1;
            }
        }
        i = k;
    }

    // fully_beamed bestimmen: wenn alle Noten (min. 2) der Gruppe innerhalb einer BeamGroup liegen
    // und alle benachbarten Paare continuity >= 1 haben.
    let mut out: Vec<TupletPlan> = Vec::with_capacity(tmp.len());
    for g in tmp.into_iter() {
        let note_idxs: Vec<BeatIdx> =
            (g.start..=g.end).filter(|&ix| beats[ix].kind == BeatKind::Note).collect();
        let fully = if g.contains_rest || note_idxs.len() < 2 {
            false
        } else {
            // Prüfe gegen jede BeamGroup
            let mut ok_any = false;
            'bg: for bg in beams.iter() {
                // Alle Noten enthalten?
                if note_idxs.iter().all(|ix| bg.beat_indices.contains(ix)) {
                    // Mappe BeatIndex -> Position in der BeamGroup
                    let mut pos_map = std::collections::HashMap::new();
                    for (li, gi2) in bg.beat_indices.iter().enumerate() {
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

        // Externe Balkenverbindungen links/rechts an den Rändern feststellen
        let mut ext_left = false;
        let mut ext_right = false;
        let first_note = note_idxs.first().copied();
        let last_note = note_idxs.last().copied();

        if let Some(fi) = first_note
            && fi > 0
            && beats[fi - 1].kind == BeatKind::Note
        {
            for bg in beams.iter() {
                let pos_prev = bg.beat_indices.iter().position(|&x| x == fi - 1);
                let pos_cur = bg.beat_indices.iter().position(|&x| x == fi);
                if let (Some(lp), Some(lc)) = (pos_prev, pos_cur) {
                    // Adjacent in der Gruppe und continuity >=1 zwischen ihnen?
                    let a = lp.min(lc);
                    let b = lp.max(lc);
                    if b == a + 1 && *bg.continuity.get(a).unwrap_or(&0) >= 1 {
                        ext_left = true;
                        break;
                    }
                }
            }
        }

        if let Some(li) = last_note
            && li + 1 < beats.len()
            && beats[li + 1].kind == BeatKind::Note
        {
            for bg in beams.iter() {
                let pos_cur = bg.beat_indices.iter().position(|&x| x == li);
                let pos_next = bg.beat_indices.iter().position(|&x| x == li + 1);
                if let (Some(lc), Some(ln)) = (pos_cur, pos_next) {
                    let a = lc.min(ln);
                    let b = lc.max(ln);
                    if b == a + 1 && *bg.continuity.get(a).unwrap_or(&0) >= 1 {
                        ext_right = true;
                        break;
                    }
                }
            }
        }

        let edge_connection = match (ext_left, ext_right) {
            (false, false) => EdgeConnection::None,
            (true, false) => EdgeConnection::Left,
            (false, true) => EdgeConnection::Right,
            (true, true) => EdgeConnection::Both,
        };

        out.push(TupletPlan {
            // Wichtig: Die dargestellte Tuplet-Zahl ist der Zähler n des (n,m)-Verhältnisses
            // und NICHT die Anzahl der Slots im geschnittenen Segment. Das Segment kann kürzer
            // sein (z. B. wenn der erste Slot zu einer größeren Basis „verschmilzt“), die
            // semantische Tuplet bleibt aber eine „3“ (Triplet), „5“ (Quintuplet), etc.
            count: g.n,
            start: g.start,
            end: g.end,
            base: g.base,
            fully_beamed: fully,
            contains_rest: g.contains_rest,
            edge_connection,
        });
    }

    out
}
