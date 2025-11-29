use crate::layout::beam_plan::BeamGroup;
use crate::layout::render_plan::BeatIdx;
use crate::measure::duration::{Duration, NoteValue, TupletSpec};
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

/// Beam-unabhängiger, konsolidierter Typ für erkannte Tuplet-Spans
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TupletSpan {
    pub count: u8,
    pub start: BeatIdx,
    pub end: BeatIdx, // inklusiv
    pub base: NoteValue,
    pub contains_rest: bool,
}

/// Erkenne Tuplet-Spans ausschließlich aus dem Measure, ohne Beaming-Infos.
pub(crate) fn detect_tuplet_spans(measure: &Measure) -> Vec<TupletSpan> {
    let beats = measure.beats();

    // ID-gestützter Pfad zuerst
    let has_ids = beats.iter().any(|b| b.tuplet_group_id.is_some());
    if has_ids {
        let mut ranges: std::collections::BTreeMap<u32, (usize, usize)> = Default::default();
        for (ix, b) in beats.iter().enumerate() {
            if let Some(id) = b.tuplet_group_id {
                ranges
                    .entry(id)
                    .and_modify(|rng| {
                        rng.0 = rng.0.min(ix);
                        rng.1 = rng.1.max(ix);
                    })
                    .or_insert((ix, ix));
            }
        }

        let mut out: Vec<TupletSpan> = Vec::with_capacity(ranges.len());
        for (id, (start, end)) in ranges.into_iter() {
            if let Some(anchor) = measure.tuplet_anchors.get(&id) {
                let contains_rest = (start..=end).any(|i| beats[i].kind == BeatKind::Rest);
                out.push(TupletSpan {
                    count: anchor.n,
                    start,
                    end,
                    base: anchor.base_hint,
                    contains_rest,
                });
            }
        }
        return out;
    }

    // Legacy-Erkennung: Runs gleicher (n,m) in Gruppen segmentieren
    let mut spans: Vec<TupletSpan> = Vec::new();
    let mut i = 0usize;
    while i < beats.len() {
        let Duration::Tuplet(TupletSpec { n, m, .. }) = beats[i].duration else {
            i += 1;
            continue;
        };
        // Maximalen Run gleicher (n,m) finden
        let mut k = i;
        while k < beats.len() {
            match beats[k].duration {
                Duration::Tuplet(TupletSpec { n: nn, m: mm, .. }) if nn == n && mm == m => k += 1,
                _ => break,
            }
        }

        // Gröbste Basisnote im Run bestimmen (als Gruppenbasis)
        let mut run_max_base = beats[i].duration.base_note();
        let mut run_max_ticks = measure.grid.ticks_of(&Duration::Simple(run_max_base)).unwrap_or(0);
        for beat in beats.iter().take(k).skip(i) {
            let b = beat.duration.base_note();
            if let Some(t) = measure.grid.ticks_of(&Duration::Simple(b)) {
                if t > run_max_ticks {
                    run_max_ticks = t;
                    run_max_base = b;
                }
            }
        }

        // Run in logische Spans zerlegen: Ziel = m * (Ticks der gröbsten Basis)
        let mut start = i;
        while start < k {
            let mut acc_ticks: u32 = 0;
            let mut end = start;
            let mut has_rest = false;
            let mut reached = false;
            while end < k {
                if beats[end].kind == BeatKind::Rest { has_rest = true; }
                let dt = measure.grid.ticks_of(&beats[end].duration).unwrap_or(0);
                acc_ticks = acc_ticks.saturating_add(dt);
                let target = run_max_ticks.saturating_mul(m as u32);
                if acc_ticks >= target { reached = true; break; }
                end += 1;
            }
            if !reached && end >= start {
                end = k.saturating_sub(1);
                reached = end >= start;
            }
            if reached {
                spans.push(TupletSpan {
                    count: n,
                    start,
                    end,
                    base: run_max_base,
                    contains_rest: has_rest,
                });
                start = end + 1;
            } else {
                start += 1;
            }
        }
        i = k;
    }

    spans
}

pub fn compute_tuplet_plan(measure: &Measure, beams: &[BeamGroup]) -> Vec<TupletPlan> {
    let beats = measure.beats();
    let spans = detect_tuplet_spans(measure);

    let mut out: Vec<TupletPlan> = Vec::with_capacity(spans.len());
    for g in spans.into_iter() {
        let note_idxs: Vec<BeatIdx> = (g.start..=g.end)
            .filter(|&ix| beats[ix].kind == BeatKind::Note)
            .collect();

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
                        if la >= lb { ok = false; break; }
                        for cidx in la..lb {
                            if *bg.continuity.get(cidx).unwrap_or(&0) < 1 { ok = false; break; }
                        }
                        if !ok { break; }
                    }
                    if ok { ok_any = true; break 'bg; }
                }
            }
            ok_any
        };

        // Externe Balkenverbindungen links/rechts feststellen (Tuplet-Nachbarn ignorieren)
        let mut ext_left = false;
        let mut ext_right = false;
        let first_note = note_idxs.first().copied();
        let last_note = note_idxs.last().copied();

        if let Some(fi) = first_note
            && fi > 0
            && beats[fi - 1].kind == BeatKind::Note
            && !matches!(beats[fi - 1].duration, Duration::Tuplet(_))
        {
            for bg in beams.iter() {
                let pos_prev = bg.beat_indices.iter().position(|&x| x == fi - 1);
                let pos_cur = bg.beat_indices.iter().position(|&x| x == fi);
                if let (Some(lp), Some(lc)) = (pos_prev, pos_cur) {
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
            && !matches!(beats[li + 1].duration, Duration::Tuplet(_))
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
