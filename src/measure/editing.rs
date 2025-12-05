use crate::measure::BeatKind::{Note, Rest};
use crate::measure::duration::NoteValue::{Eighth, Sixteenth};
use crate::measure::duration::{Duration, NoteValue, TupletSpec};
use crate::measure::editing::Modification::{DissolveTuplet, ToggleAccent};
use crate::measure::{Beat, BeatIdx, BeatKind, Measure, TimeSignature};
use either::Either;

#[derive(Debug)]
pub enum Modification {
    SetBeat(BeatIdx, Beat),                // contains the new beat
    SetTuplet(GroupSpan, TupletSpec),      // contains the new state
    DissolveTuplet(GroupSpan, TupletSpec), // contains the dissolved tuplet spec
    ToggleKind(BeatIdx, BeatKind),         // contains the new beat kind
    ToggleAccent(BeatIdx, bool),           // contains the new accented state
    ToggleDotted(BeatIdx, u8),             // contains the new dot count
    ChangeTimeSignature(TimeSignature, TimeSignature), // (old, new)
}

#[derive(Debug, PartialEq, Eq)]
pub struct GroupSpan {
    pub id: u32,
    pub start_idx: BeatIdx,
    pub end_idx: BeatIdx, // inclusive
}

pub const CYCLE_TUPLET_SPECS: [TupletSpec; 5] = [
    TupletSpec { n: 3, m: 2, base: Eighth }, // t8 - 1/8 triplets
    TupletSpec { n: 5, m: 4, base: Sixteenth }, // qt16 - 1/16 quintuplets
    TupletSpec { n: 6, m: 4, base: Sixteenth }, // s16 - 1/16 sextuplets
    TupletSpec { n: 7, m: 4, base: Sixteenth }, // spt16 - 1/16 septuplets
    TupletSpec { n: 9, m: 8, base: Sixteenth }, // nt16 - 1/16 nonuplets
];

impl Measure<'_> {
    /// Toggle the user accent flag at index `idx`.
    pub fn toggle_accent(&mut self, idx: BeatIdx) -> Option<Modification> {
        if let Some(b) = self.beats.get(idx) {
            let accented = !b.accented;
            if let Some(b) = self.beats.get_mut(idx)
                && b.kind == Note
            {
                b.accented = accented;
            }
            Some(ToggleAccent(idx, accented))
        } else {
            None
        }
    }

    /// Toggle dotted (one dot) for the beat at `idx`.
    /// - Simple(base) -> Dotted { base, dots: 1 }
    /// - Dotted { base, dots: 1 } -> Simple(base)
    ///
    /// No-op for other cases (tuplets, multi-dot) or if replacement doesn't fit.
    pub fn toggle_dotted(&mut self, idx: BeatIdx) -> Option<Modification> {
        if idx >= self.beats.len() {
            return None;
        }
        let current = self.beats[idx];
        let mut new_dots = 0u8;
        let new_dur = match current.duration {
            Duration::Simple(base) => {
                new_dots = 1;
                Some(Duration::Dotted { base, dots: 1 })
            }
            Duration::Dotted { base, dots: 1 } => Some(Duration::Simple(base)),
            _ => None,
        };
        if let Some(dur) = new_dur {
            let new_beat = Beat {
                duration: dur,
                kind: current.kind,
                accented: current.accented,
                tuplet_group_id: current.tuplet_group_id,
            };
            if self.set_beat(idx, new_beat).is_ok() {
                return Some(Modification::ToggleDotted(idx, new_dots));
            }
        }
        None
    }

    pub fn modify_beat(
        &mut self,
        idx: BeatIdx,
        base: NoteValue,
        beat_kind: Option<BeatKind>,
    ) -> Option<Modification> {
        let cur = self.beats()[idx];
        let new_dur_opt = match cur.duration {
            Duration::Tuplet(spec) => {
                Some(Duration::Tuplet(TupletSpec { n: spec.n, m: spec.m, base }))
            }
            _ => Some(Duration::Simple(base)),
        };

        if let Some(new_dur) = new_dur_opt {
            let kind = if let Some(override_kind) = beat_kind { override_kind } else { cur.kind };
            let new_beat = match kind {
                Note => Beat::note(new_dur),
                Rest => Beat::rest(new_dur),
            };
            self.set_beat(idx, new_beat)
                .map(|_| Some(Modification::SetBeat(idx, new_beat)))
                .unwrap_or(None)
        } else {
            None
        }
    }

    pub fn set_tuplet(
        &mut self,
        idx: BeatIdx,
        tuplet_spec: Option<TupletSpec>,
        overwrite: bool,
    ) -> Option<Modification> {
        let start_idx = self.find_group_span(idx).map_or(idx, |g| g.start_idx);
        let cur_beat = self.beats()[start_idx];
        let mut result: Option<Modification> = None;
        let mut captured_offsets: Option<Vec<(u32, bool)>> = None;
        match cur_beat.duration {
            Duration::Tuplet(current_spec) => {
                let next_target = if let Some(explicit_tuplet) = &tuplet_spec {
                    if current_spec == *explicit_tuplet {
                        // Dissolve if the same tuplet is requested
                        None
                    } else {
                        Some(explicit_tuplet)
                    }
                } else {
                    // Cycle tuplets:
                    CYCLE_TUPLET_SPECS
                        .iter()
                        .position(|spec| *spec == current_spec)
                        .and_then(|next_idx| CYCLE_TUPLET_SPECS.get(next_idx + 1))
                };

                // Dissolve current group from its start
                // Vor dem Auflösen ggf. Noten‑Offsets erfassen, nur wenn wir ein nächstes Ziel haben
                if next_target.is_some() {
                    captured_offsets = self.tuplet_group_note_offsets(start_idx);
                }
                result = self.dissolve_tuplet_group(start_idx);
                if let Some(DissolveTuplet(..)) = result {
                    // Try to convert to the next target if defined, also at group start
                    if let Some(tuplet_spec) = next_target
                        && let Some(group_span) =
                            self.convert_to_tuplet(start_idx, *tuplet_spec, overwrite)
                    {
                        result = Some(Modification::SetTuplet(group_span, *tuplet_spec));
                        // Nach erfolgreicher Rekreation ggf. Projektion anwenden
                        if let Some(ref src) = captured_offsets {
                            let _ = self.apply_tuplet_projection_at(start_idx, src);
                        }
                    }
                }
            }
            _ => {
                let next_target = if let Some(explicit_tuplet) = &tuplet_spec {
                    explicit_tuplet
                } else {
                    &CYCLE_TUPLET_SPECS[0]
                };

                if let Some(group_span) = self.convert_to_tuplet(start_idx, *next_target, overwrite)
                {
                    result = Some(Modification::SetTuplet(group_span, *next_target));
                }
            }
        };

        result
    }

    /// Löst die Tuplet‑Gruppe auf, in der sich `idx` befindet.
    ///
    /// Verhalten:
    /// - Ersetzt die gesamte Spanne der Gruppe durch eine einfache (nicht‑Tuplet) Auffüllung.
    /// - Initialisierung der neuen Beats: Standard sind Rests; wenn die aufgelöste Gruppe mindestens
    ///   eine Note enthielt, wird der erste neu eingefügte Beat als Note angelegt.
    /// - Ein vorhandener Akzent innerhalb der Gruppe wird auf den ersten neu eingefügten Beat übernommen.
    /// - Entfernt die `tuplet_group_id` in diesem Bereich und löscht den verknüpften Anchor.
    /// - Rückgabe: `true` bei erfolgreicher Auflösung, sonst `false` (z. B. wenn kein Tuplet an `idx`).
    pub fn dissolve_tuplet_group(&mut self, idx: BeatIdx) -> Option<Modification> {
        if idx >= self.beats.len() {
            return None;
        }

        if let Some(GroupSpan { start_idx, end_idx, id: group_id }) = self.find_group_span(idx) {
            // Merke, ob die Gruppe mindestens eine Note bzw. einen Akzent enthielt
            let mut had_any_note = false;
            let mut had_any_accent = false;
            for b in &self.beats[start_idx..=end_idx] {
                if b.kind == Note {
                    had_any_note = true;
                }
                if b.accented {
                    had_any_accent = true;
                }
            }

            self.beats.drain(start_idx..=end_idx);

            let anchor = self.tuplet_anchors.get(&group_id).unwrap();
            let span_ticks = anchor.target_ticks;
            self.fill_at(start_idx, span_ticks, &[], Either::Right(Rest)).unwrap();

            // Post‑Processing: ersten neu eingefügten Beat ggf. als Note setzen und Akzent übernehmen
            if start_idx < self.beats.len() {
                self.beats[start_idx].tuplet_group_id = None;
                if had_any_note {
                    self.beats[start_idx].kind = Note;
                }
                self.beats[start_idx].accented = had_any_accent;
            }

            self.tuplet_anchors.remove(&group_id).map(|anchor| {
                DissolveTuplet(
                    GroupSpan { start_idx, end_idx, id: group_id },
                    TupletSpec { n: anchor.n, m: anchor.m, base: anchor.base_hint },
                )
            })
        } else {
            None
        }
    }

    /// Find the first and last index with the same tuplet group id as the beat at the given index.
    /// `None`, if the beat is not in a tuplet group.
    pub(crate) fn find_group_span(&self, idx: BeatIdx) -> Option<GroupSpan> {
        let id = self.beats[idx].tuplet_group_id?;

        let mut start_idx = idx;
        while start_idx > 0 && self.beats[start_idx - 1].tuplet_group_id == Some(id) {
            start_idx -= 1;
        }
        let mut end_idx = idx + 1;
        while end_idx < self.beats.len() && self.beats[end_idx].tuplet_group_id == Some(id) {
            end_idx += 1;
        }
        if start_idx >= end_idx - 1 {
            return None;
        }

        Some(GroupSpan { start_idx, end_idx: end_idx - 1, id })
    }

    /// Liefert für die Tuplet‑Gruppe, die bei `start_idx` beginnt, die relativen Onset‑Ticks
    /// aller gesetzten Noten innerhalb der Gruppe. Jeder Eintrag enthält `(offset_ticks, accented)`.
    /// Die Offsets sind relativ zum Gruppenstart, 0‑basiert, in Grid‑Ticks, und stets aufsteigend sortiert.
    ///
    /// Rückgabe `None`, wenn an `start_idx` keine Tuplet‑Gruppe beginnt.
    fn tuplet_group_note_offsets(&self, start_idx: BeatIdx) -> Option<Vec<(u32, bool)>> {
        if start_idx >= self.beats.len() {
            return None;
        }
        let b0 = self.beats[start_idx];
        let gid = b0.tuplet_group_id?;
        // sicherstellen, dass start_idx wirklich der Gruppenanfang ist
        if start_idx > 0 && self.beats[start_idx - 1].tuplet_group_id == Some(gid) {
            return None;
        }

        let anchor = self.tuplet_anchors.get(&gid)?;
        let span_ticks = anchor.target_ticks;
        // Onsets der gesamten Measure berechnen und dann relative Offsets der Gruppe extrahieren
        let onsets = self.grid.compute_onset_ticks(&self.beats);
        let start_onset = onsets[start_idx];

        let mut offsets: Vec<(u32, bool)> = Vec::new();
        let mut idx = start_idx;
        while idx < self.beats.len() && self.beats[idx].tuplet_group_id == Some(gid) {
            if self.beats[idx].kind == Note {
                let rel = onsets[idx] - start_onset;
                // Kappen vorsichtshalber auf die Spanne (sollte nicht notwendig sein)
                let rel = rel.min(span_ticks);
                offsets.push((rel, self.beats[idx].accented));
            }
            idx += 1;
        }
        offsets.sort_by_key(|e| e.0);
        Some(offsets)
    }

    /// Projiziert eine zuvor erfasste Notenbelegung (relative Onset‑Ticks innerhalb einer alten Tuplet‑Gruppe)
    /// auf die neu erzeugte Tuplet‑Gruppe, die an `start_idx` beginnt.
    ///
    /// Algorithmus:
    /// - Ermittelt die Onset‑Ticks der Slots der neuen Gruppe relativ zum Gruppenstart.
    /// - Ordnet jede Quell‑Note dem nächstgelegenen freien Ziel‑Slot zu (bei Gleichstand kleinere Index‑Präferenz),
    ///   sodass keine Duplikate entstehen (greedy Zuordnung).
    /// - Setzt alle Slots standardmäßig auf Rest, die gemappten auf Note; Akzente werden pro Quell‑Note übertragen.
    ///
    /// Rückgabe: `true` bei Erfolg, `false` falls an `start_idx` keine Tuplet‑Gruppe beginnt.
    fn apply_tuplet_projection_at(
        &mut self,
        start_idx: BeatIdx,
        source_offsets: &[(u32, bool)],
    ) -> bool {
        if start_idx >= self.beats.len() {
            return false;
        }
        let b0 = self.beats[start_idx];
        let gid = match b0.tuplet_group_id {
            Some(id) => id,
            None => return false,
        };

        // Grenzen der Gruppe bestimmen
        let mut end = start_idx + 1;
        while end < self.beats.len() && self.beats[end].tuplet_group_id == Some(gid) {
            end += 1;
        }

        // n bestimmen (Anzahl Slots)
        let n = match b0.duration {
            Duration::Tuplet(TupletSpec { n, .. }) => n as usize,
            _ => return false,
        };

        // Onsets der neuen Gruppe (relativ) berechnen
        let onsets = self.grid.compute_onset_ticks(&self.beats);
        let start_onset = onsets[start_idx];
        let mut target_rel: Vec<(u32, usize)> = Vec::with_capacity(n);
        let mut i = start_idx;
        while i < end {
            let rel = onsets[i] - start_onset;
            target_rel.push((rel, i));
            i += 1;
        }
        // Sicherheitsnetz: falls unerwartet mehr/ weniger Slots, weiter mit vorhandenen
        target_rel.sort_by_key(|e| e.0);
        let tlen = target_rel.len();
        if tlen == 0 {
            return false;
        }

        // Alle Slots zunächst auf Rest setzen und Akzent löschen; Tremolo löschen
        for j in start_idx..end {
            self.beats[j].kind = Rest;
            self.beats[j].accented = false;
        }

        // Greedy Zuordnung: für jede Quell‑Note den nächstgelegenen freien Ziel‑Slot suchen
        let mut used = vec![false; tlen];
        for &(src_rel, src_accent) in source_offsets.iter() {
            // finde nächstgelegenen Index
            let mut best_k: Option<BeatIdx> = None;
            let mut best_dist: u32 = u32::MAX;
            for (k, (trel, _tidx)) in target_rel.iter().enumerate() {
                if used[k] {
                    continue;
                }
                let dist = (*trel).abs_diff(src_rel);
                if dist < best_dist || (dist == best_dist && best_k.map(|x| k < x).unwrap_or(true))
                {
                    best_dist = dist;
                    best_k = Some(k);
                }
            }
            if let Some(k) = best_k {
                let ti = target_rel[k].1;
                self.beats[ti].kind = Note;
                if src_accent {
                    self.beats[ti].accented = true;
                }
                used[k] = true;
            }
        }

        true
    }

    /// Wandelt den Beat an `idx` in eine Tuplet‑Gruppe des Typs (n in der Zeit von m, Basis `base`) um.
    ///
    /// Bedingungen/Verhalten:
    /// - Wenn `idx` außerhalb liegt oder der Beat dort bereits ein Tuplet ist, passiert nichts und es wird `None` zurückgegeben.
    /// - Ansonsten wird versucht, den Beat durch einen Tuplet‑Beat gleicher Art (Note/Rest) zu ersetzen.
    ///   Die Methode delegiert an `set_beat_at`, welches die gesamte Gruppe inkl. Anchor anlegt und
    ///   verbleibende Slots der Gruppe auffüllt. Reicht der Platz im Takt nicht aus, bleibt
    ///   der Takt unverändert und die Funktion liefert `None`.
    fn convert_to_tuplet(
        &mut self,
        idx: usize,
        tuplet_spec: TupletSpec,
        overwrite: bool,
    ) -> Option<GroupSpan> {
        if idx >= self.beats.len() {
            return None;
        }
        let cur = self.beats[idx];
        if matches!(cur.duration, Duration::Tuplet { .. }) {
            return None;
        }

        // Protection: Stop if we would absorb a note and overwrite is false
        if !overwrite {
            let base_ticks = self.grid.ticks_of(&Duration::Simple(tuplet_spec.base)).unwrap();
            let group_span = (tuplet_spec.m as u32) * base_ticks;

            let mut consumed = 0u32;
            let mut k = idx;
            while consumed < group_span {
                if k >= self.beats.len() {
                    // If we run out of beats, set_beat_at will handle the error.
                    break;
                }
                // Check if we are about to absorb the following beat that is a note
                if k > idx && self.beats[k].kind == Note {
                    return None;
                }
                consumed += self.grid.ticks_of(&self.beats[k].duration).unwrap();
                k += 1;
            }
        }

        let new_duration = Duration::Tuplet(tuplet_spec);
        let mut new_beat = Beat::new(new_duration, cur.kind);
        new_beat.accented = cur.accented;
        if self.set_beat(idx, new_beat).is_ok() {
            let group_span = self.find_group_span(idx);

            // Wenn der ursprüngliche Beat eine Note war, initialisieren wir die ganze neue Tuplet‑Gruppe als Noten
            if cur.kind == Note
                && let Some(group_span) = &group_span
            {
                for i in group_span.start_idx..=group_span.end_idx {
                    self.beats[i].kind = Note;
                }
            }

            group_span
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::duration::NoteValue::Eighth;
    use crate::measure::duration::{e, t8};
    use crate::measure::{Beat, TimeSignature};
    use std::assert_matches::assert_matches;

    #[test]
    fn dissolve_tuplet_initializes_note_when_group_contains_any_note_and_preserves_accent() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Erzeuge Triplet‑1/8 am Anfang, erster Slot Note, restliche werden als Rests vorinitialisiert
        m.set_beat(0, Beat::note(t8())).unwrap();
        m.set_beat(2, Beat::note(t8())).unwrap();
        m.toggle_accent(2);

        assert_matches!(
            m.dissolve_tuplet_group(1),
            Some(DissolveTuplet(
                GroupSpan { start_idx: 0, end_idx: 2, id: 1 },
                TupletSpec { n: 3, m: 2, base: Eighth }
            ))
        );

        // Erwartung: erster neu eingefügter Beat ist eine Note und akzentuiert
        assert!(m.beats()[0].tuplet_group_id.is_none());
        assert_eq!(m.beats()[0].kind, Note);
        assert!(m.beats()[0].accented, "accent should be preserved on first replacement beat");
    }

    #[test]
    fn dissolve_tuplet_all_rests_results_in_rest_and_no_accent() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Erzeuge Triplet‑1/8 Gruppe als Rests: Standardzustand ist bereits Rest an 0
        // also direkt Triplet konvertieren aus einem Rest heraus
        assert_matches!(
            m.convert_to_tuplet(0, TupletSpec { n: 3, m: 2, base: Eighth }, true),
            Some(GroupSpan { start_idx: 0, end_idx: 2, id: _ })
        );
        // Sicherheitscheck: alle drei Slots sind Rests
        for i in 0..3 {
            assert_eq!(m.beats()[i].kind, Rest);
        }

        assert_matches!(
            m.dissolve_tuplet_group(1),
            Some(DissolveTuplet(
                GroupSpan { start_idx: 0, end_idx: 2, id: 1 },
                TupletSpec { n: 3, m: 2, base: Eighth }
            ))
        );

        assert!(m.beats()[0].tuplet_group_id.is_none());
        assert_eq!(m.beats()[0].kind, Rest);
        assert!(!m.beats()[0].accented);
    }

    #[test]
    fn convert_quarter_to_triplet_eighth_group_in_four_four() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Standardfüllung: 4x Viertel Rests
        assert_matches!(
            m.convert_to_tuplet(0, TupletSpec { n: 3, m: 2, base: Eighth }, true),
            Some(GroupSpan { start_idx: 0, end_idx: 2, id: _ }),
            "Conversion to triplet should succeed"
        );

        // Erwartung: drei Triplet‑Achtel an den ersten drei Positionen mit gleicher group_id
        let id0 = m.beats()[0].tuplet_group_id;
        assert!(id0.is_some());
        for i in 0..3 {
            assert_eq!(m.beats()[i].duration, t8());
            assert_eq!(m.beats()[i].tuplet_group_id, id0);
        }

        // Anchor‑Span entspricht einer Viertel‑Spanne
        let gid = id0.unwrap();
        let anchor = m.tuplet_anchors.get(&gid).expect("anchor must exist");
        let base_quarter_ticks = m.grid.ticks_of(&Duration::Simple(Eighth)).unwrap() * 2;
        assert_eq!(anchor.target_ticks, base_quarter_ticks);
    }

    #[test]
    fn convert_noop_when_already_tuplet() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Erzeuge zuerst ein Triplet an Position 0
        m.set_beat(0, Beat::note(t8())).unwrap();

        let before = m.clone();
        assert_matches!(
            m.convert_to_tuplet(0, TupletSpec { n: 3, m: 2, base: Eighth }, true),
            None,
            "Should not convert when already a tuplet"
        );
        // Unverändert
        assert_eq!(format!("{:?}", before), format!("{:?}", m));
    }

    #[test]
    fn convert_to_tuplet_initializes_notes_when_source_is_note() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Stelle sicher, dass Quelle eine Note ist (nicht Rest)
        m.set_beat(0, Beat::note(Duration::Simple(Eighth))).unwrap();
        // Wandle an derselben Position in Triplet‑Achtel um
        assert_matches!(
            m.convert_to_tuplet(0, TupletSpec { n: 3, m: 2, base: Eighth }, true),
            Some(GroupSpan { start_idx: 0, end_idx: 2, id: _ })
        );

        for i in 0..3 {
            assert_eq!(m.beats()[i].duration, t8());
            assert_eq!(m.beats()[i].kind, Note, "tuplet slot {} should be a note", i);
        }
    }

    #[test]
    fn convert_to_tuplet_initializes_rests_when_source_is_rest() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Standard ist Rest an Index 0
        assert_matches!(
            m.convert_to_tuplet(0, TupletSpec { n: 3, m: 2, base: Eighth }, true),
            Some(GroupSpan { start_idx: 0, end_idx: 2, id: _ }),
        );
        for i in 0..3 {
            assert_eq!(m.beats()[i].duration, t8());
            assert_eq!(m.beats()[i].kind, Rest, "tuplet slot {} should be a rest", i);
        }
    }

    #[test]
    fn convert_absorbs_rests_ok() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Setup: [Rest(e), Rest(e), Rest(q)...] by replacing the first Q with E (fills the remainder with E)
        m.set_beat(0, Beat::rest(e())).unwrap();
        // Now convert idx 0 (Rest(e)) to Triplet Eighths (span Q). Needs to absorb idx 1 (Rest(e)).
        assert_matches!(
            m.convert_to_tuplet(0, TupletSpec { n: 3, m: 2, base: Eighth }, false),
            Some(GroupSpan { start_idx: 0, end_idx: 2, id: _ }),
        );
        // Should have created a tuplet group at 0
        assert!(m.beats()[0].tuplet_group_id.is_some());
    }

    #[test]
    fn convert_aborts_on_note_if_no_overwrite() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Setup: [Rest(e), Note(e), Rest(q)...]
        m.set_beat(0, Beat::rest(e())).unwrap();
        m.set_beat(1, Beat::note(e())).unwrap();

        // Try to convert idx 0 to Triplet Eighths (span Q). Needs to absorb idx 1, which is Note.
        assert_matches!(
            m.convert_to_tuplet(0, TupletSpec { n: 3, m: 2, base: Eighth }, false),
            None
        );
        // Verify state unchanged (idx 1 is still Note)
        assert_eq!(m.beats()[1].kind, Note);
        assert!(m.beats()[0].tuplet_group_id.is_none());
    }

    #[test]
    fn convert_overwrites_note_if_overwrite_true() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Setup: [Rest(e), Note(e), Rest(q)...]
        m.set_beat(0, Beat::rest(e())).unwrap();
        m.set_beat(1, Beat::note(e())).unwrap();

        // Try to convert idx 0 to Triplet Eighths (span Q). Overwrite=true.
        assert_matches!(
            m.convert_to_tuplet(0, TupletSpec { n: 3, m: 2, base: Eighth }, true),
            Some(GroupSpan { start_idx: 0, end_idx: 2, id: _ }),
        );
        // Verify tuplet created
        assert!(m.beats()[0].tuplet_group_id.is_some());
    }
}
