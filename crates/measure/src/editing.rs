use crate::BeatKind::{Note, Rest};
use crate::duration::NoteValue::{Eighth, Sixteenth};
use crate::duration::{Duration, NoteValue, TupletSpec};
use crate::editing::Modification::{DissolveTuplet, ToggleAccent};
use crate::grid::DEFAULT_GRID;
use crate::tuplet::TupletGroupId;
use crate::{Beat, BeatIdx, BeatKind, TimeSignature, VoiceBar};
use either::Either;

#[derive(Debug)]
pub enum Modification {
    SetBeat(BeatIdx, Beat),                            // contains the new beat
    SetTuplet(GroupSpan, TupletSpec),                  // contains the new state
    DissolveTuplet(GroupSpan, TupletSpec),             // contains the dissolved tuplet spec
    ToggleKind(BeatIdx, BeatKind),                     // contains the new beat kind
    ToggleAccent(BeatIdx, bool),                       // contains the new accented state
    ToggleSoft(BeatIdx, bool),                         // contains the new soft state
    ToggleDotted(BeatIdx, u8),                         // contains the new dot count
    ChangeTimeSignature(TimeSignature, TimeSignature), // (old, new)
}

#[derive(Debug, PartialEq, Eq)]
pub struct GroupSpan {
    pub id: TupletGroupId,
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

impl VoiceBar {
    /// Toggle the user accent flag at index `idx`.
    pub fn toggle_accent(&mut self, idx: BeatIdx) -> Option<Modification> {
        if let Some(b) = self.beats.get(idx) {
            let accented = !b.accented();
            if let Some(b) = self.beats.get_mut(idx)
                && b.is_note()
            {
                b.set_accented(accented);
            }
            Some(ToggleAccent(idx, accented))
        } else {
            None
        }
    }

    /// Toggle the soft (ghost note) flag at index `idx`; a soft note loses
    /// its accent. No change on a rest.
    pub fn toggle_soft(&mut self, idx: BeatIdx) -> Option<Modification> {
        let b = self.beats.get_mut(idx)?;
        let soft = !b.soft();
        if b.is_note() {
            b.set_soft(soft);
        }
        Some(Modification::ToggleSoft(idx, soft))
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
            let mut new_beat = current;
            new_beat.duration = dur;
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
            let kind = if let Some(override_kind) = beat_kind { override_kind } else { cur.kind() };
            // A note of the kit keeps its heads (`set_beat` keeps each
            // one's accent).
            let new_beat = match kind {
                Note if cur.is_kit() => Beat::with_heads(new_dur, *cur.heads()),
                Note => Beat::note(new_dur),
                Rest => Beat::rest(new_dur),
            };
            self.set_beat(idx, new_beat).ok()?;
            Some(Modification::SetBeat(idx, new_beat))
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
        let start_idx = self.tuplet_group_at(idx).map_or(idx, |g| g.start_idx);
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
                // Remember where the notes sit, but only when there is a next target
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
                        // Carry the remembered notes over to the new group
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
                    // A new tuplet group starts with every slot as a note
                    for i in group_span.start_idx..=group_span.end_idx {
                        self.beats[i].set_kind(Note);
                    }
                    result = Some(Modification::SetTuplet(group_span, *next_target));
                }
            }
        };

        result
    }

    /// Dissolves the tuplet group containing `idx`.
    ///
    /// - Replaces the group's whole span with plain (non-tuplet) rests.
    /// - If the group held a note, the first new beat becomes a note; an
    ///   accent anywhere in the group moves to the first new beat.
    /// - Clears `tuplet_group_id` in the span and unregisters the anchor.
    /// - Returns the dissolved span and spec, or `None` if `idx` is not in a
    ///   tuplet.
    pub fn dissolve_tuplet_group(&mut self, idx: BeatIdx) -> Option<Modification> {
        if idx >= self.beats.len() {
            return None;
        }

        if let Some(GroupSpan { start_idx, end_idx, id: group_id }) = self.tuplet_group_at(idx) {
            // Whether the group held at least one note or accent
            let mut had_any_note = false;
            let mut had_any_accent = false;
            for b in &self.beats[start_idx..=end_idx] {
                if b.is_note() {
                    had_any_note = true;
                }
                if b.accented() {
                    had_any_accent = true;
                }
            }

            self.beats.drain(start_idx..=end_idx);

            let anchor = self.tuplets.get(group_id).unwrap();
            let span_ticks = anchor.target_ticks;
            self.fill_at(start_idx, span_ticks, &[], Either::Right(Rest)).unwrap();

            // Make the first new beat a note and carry the accent over
            if start_idx < self.beats.len() {
                self.beats[start_idx].tuplet_group_id = None;
                if had_any_note {
                    self.beats[start_idx].set_kind(Note);
                }
                self.beats[start_idx].set_accented(had_any_accent);
            }

            self.tuplets.unregister(group_id).map(|anchor| {
                DissolveTuplet(
                    GroupSpan { start_idx, end_idx, id: group_id },
                    TupletSpec { n: anchor.n, m: anchor.m, base: anchor.base_hint },
                )
            })
        } else {
            None
        }
    }

    /// Onsets of the notes in the tuplet group starting at `start_idx`, as
    /// `(offset_ticks, accented)`: grid ticks from the group start, sorted
    /// ascending.
    ///
    /// `None` if no tuplet group starts at `start_idx`.
    fn tuplet_group_note_offsets(&self, start_idx: BeatIdx) -> Option<Vec<(u32, bool)>> {
        let group = self.tuplet_group_at(start_idx)?;
        if group.start_idx != start_idx {
            return None;
        }
        let anchor = self.tuplets.get(group.id)?;
        let span_ticks = anchor.target_ticks;
        let onsets = DEFAULT_GRID.compute_onset_ticks(&self.beats);
        let start_onset = onsets[start_idx];

        let mut offsets: Vec<(u32, bool)> = (group.start_idx..=group.end_idx)
            .filter(|&i| self.beats[i].is_note())
            .map(|i| ((onsets[i] - start_onset).min(span_ticks), self.beats[i].accented()))
            .collect();
        offsets.sort_by_key(|e| e.0);
        Some(offsets)
    }

    /// Maps notes captured from an old tuplet group (offsets from
    /// [`Self::tuplet_group_note_offsets`]) onto the new group starting at
    /// `start_idx`.
    ///
    /// - Computes the new group's slot onsets relative to its start.
    /// - Greedily assigns each source note to the nearest free slot (the lower
    ///   index wins a tie), so no slot gets two notes.
    /// - All other slots become rests; accents follow their source note.
    ///
    /// Returns `false` if no tuplet group starts at `start_idx`.
    fn apply_tuplet_projection_at(
        &mut self,
        start_idx: BeatIdx,
        source_offsets: &[(u32, bool)],
    ) -> bool {
        let Some(group) = self.tuplet_group_at(start_idx) else {
            return false;
        };
        if !matches!(self.beats[group.start_idx].duration, Duration::Tuplet { .. }) {
            return false;
        }

        // Onsets of the new group, relative to its start
        let onsets = DEFAULT_GRID.compute_onset_ticks(&self.beats);
        let start_onset = onsets[group.start_idx];
        let mut target_rel: Vec<(u32, usize)> =
            (group.start_idx..=group.end_idx).map(|i| (onsets[i] - start_onset, i)).collect();
        // Work with whatever slots the group has, even if the count is unexpected
        target_rel.sort_by_key(|e| e.0);
        let tlen = target_rel.len();
        if tlen == 0 {
            return false;
        }

        // Start with every slot a rest without an accent
        for j in group.start_idx..=group.end_idx {
            self.beats[j].set_kind(Rest);
        }

        // Greedy: give each source note the nearest free slot
        let mut used = vec![false; tlen];
        for &(src_rel, src_accent) in source_offsets.iter() {
            // Find the nearest free slot
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
                self.beats[ti].set_kind(Note);
                if src_accent {
                    self.beats[ti].set_accented(true);
                }
                used[k] = true;
            }
        }

        true
    }

    /// Turns the beat at `idx` into a tuplet group (`n` in the time of `m`,
    /// on `base`).
    ///
    /// - Returns `None` without changes if `idx` is out of range or already
    ///   in a tuplet.
    /// - Otherwise replaces the beat with a tuplet beat of the same kind
    ///   (note or rest) via `set_beat`, which registers the anchor and fills
    ///   the rest of the group. If the measure has no room, it stays
    ///   unchanged and this returns `None`.
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
            let base_ticks = DEFAULT_GRID.ticks_of(&Duration::Simple(tuplet_spec.base)).unwrap();
            let group_span = (tuplet_spec.m as u32) * base_ticks;

            let mut consumed = 0u32;
            let mut k = idx;
            while consumed < group_span {
                if k >= self.beats.len() {
                    // If we run out of beats, set_beat will handle the error.
                    break;
                }
                // Check if we are about to absorb the following beat that is a note
                if k > idx && self.beats[k].is_note() {
                    return None;
                }
                consumed += DEFAULT_GRID.ticks_of(&self.beats[k].duration).unwrap();
                k += 1;
            }
        }

        let new_duration = Duration::Tuplet(tuplet_spec);
        let new_beat = Beat::with_heads(new_duration, *cur.heads());
        if self.set_beat(idx, new_beat).is_ok() { self.tuplet_group_at(idx) } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::NoteValue::Eighth;
    use crate::duration::{e, t8};
    use crate::{Beat, Measure, TimeSignature};
    use std::assert_matches;

    #[test]
    fn a_new_duration_keeps_a_kit_notes_heads() {
        use crate::notation::{format_score, parse_score};
        let mut score = parse_score("4/4 >S-H:q H:q r:h ; K:h K:h").unwrap();
        let hands = score.measures_mut()[0].voice_mut(0).unwrap();
        hands.modify_beat(0, NoteValue::Eighth, None).unwrap();
        hands.modify_beat(2, NoteValue::Eighth, Some(Note)).unwrap();
        let text = format_score(&score).unwrap();
        assert!(text.starts_with("4/4 >S-H:e"), "{text}");
        // A dot, a tuplet and a longer note keep each head's accent too.
        let hands = score.measures_mut()[0].voice_mut(0).unwrap();
        hands.toggle_dotted(0).unwrap();
        hands.modify_beat(0, NoteValue::Quarter, None).unwrap();
        let text = format_score(&score).unwrap();
        assert!(text.starts_with("4/4 >S-H:q"), "{text}");
        let hands = score.measures_mut()[0].voice_mut(0).unwrap();
        hands.toggle_dotted(0).unwrap();
        assert!(format_score(&score).unwrap().starts_with("4/4 >S-H:q."));
        let mut score = parse_score("4/4 >S-(H):q H:q r:h ; K:h K:h").unwrap();
        let hands = score.measures_mut()[0].voice_mut(0).unwrap();
        hands.set_tuplet(0, Some(TupletSpec { n: 3, m: 2, base: Eighth }), true).unwrap();
        let text = format_score(&score).unwrap();
        assert!(text.starts_with("4/4 >S-(H):t8"), "{text}");
        // A rest made a note is the one line's until the kit has its palette.
        let line = parse_score("4/4 >q (q) r:h").unwrap();
        let mut bar = line.measures()[0].clone();
        bar.first_voice_mut().modify_beat(1, NoteValue::Eighth, None).unwrap();
        assert!(
            !bar.first_voice().beats()[1].soft(),
            "a duration change on the one line drops the ghost, as before"
        );
    }

    #[test]
    fn a_counter_sextuplet_of_thirty_seconds_can_be_edited() {
        use crate::counter::{Ratio, counter_bar};
        use crate::grid::DEFAULT_GRID;
        let mut m = counter_bar("1/4".parse().unwrap(), Ratio { n: 12, span: 1 }).unwrap();
        assert!(m.first_voice_mut().modify_beat(1, NoteValue::Sixteenth, None).is_some());
        assert!(m.first_voice_mut().modify_beat(0, NoteValue::Sixteenth, None).is_some());
        let total: u32 =
            m.first_voice().beats().iter().filter_map(|b| DEFAULT_GRID.ticks_of(&b.duration)).sum();
        assert_eq!(total, DEFAULT_GRID.ticks_per_measure(&m.time_signature()));
    }

    #[test]
    fn dissolve_tuplet_initializes_note_when_group_contains_any_note_and_preserves_accent() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // A triplet eighth at the start: the first slot a note, the others prefilled as rests
        m.first_voice_mut().set_beat(0, Beat::note(t8())).unwrap();
        m.first_voice_mut().set_beat(2, Beat::note(t8())).unwrap();
        m.first_voice_mut().toggle_accent(2);

        assert_matches!(
            m.first_voice_mut().dissolve_tuplet_group(1),
            Some(DissolveTuplet(
                GroupSpan { start_idx: 0, end_idx: 2, id: TupletGroupId(1) },
                TupletSpec { n: 3, m: 2, base: Eighth }
            ))
        );

        // The first new beat is an accented note
        assert!(m.first_voice().beats()[0].tuplet_group_id.is_none());
        assert_eq!(m.first_voice().beats()[0].kind(), Note);
        assert!(
            m.first_voice().beats()[0].accented(),
            "accent should be preserved on first replacement beat"
        );
    }

    #[test]
    fn dissolve_tuplet_all_rests_results_in_rest_and_no_accent() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Make a triplet eighth group of rests: position 0 is already a rest,
        // so convert straight from it
        assert_matches!(
            m.first_voice_mut().convert_to_tuplet(0, TupletSpec { n: 3, m: 2, base: Eighth }, true),
            Some(GroupSpan { start_idx: 0, end_idx: 2, id: _ })
        );
        // All three slots are rests
        for i in 0..3 {
            assert_eq!(m.first_voice().beats()[i].kind(), Rest);
        }

        assert_matches!(
            m.first_voice_mut().dissolve_tuplet_group(1),
            Some(DissolveTuplet(
                GroupSpan { start_idx: 0, end_idx: 2, id: TupletGroupId(1) },
                TupletSpec { n: 3, m: 2, base: Eighth }
            ))
        );

        assert!(m.first_voice().beats()[0].tuplet_group_id.is_none());
        assert_eq!(m.first_voice().beats()[0].kind(), Rest);
        assert!(!m.first_voice().beats()[0].accented());
    }

    #[test]
    fn convert_quarter_to_triplet_eighth_group_in_four_four() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Default fill: four quarter rests
        assert_matches!(
            m.first_voice_mut().convert_to_tuplet(0, TupletSpec { n: 3, m: 2, base: Eighth }, true),
            Some(GroupSpan { start_idx: 0, end_idx: 2, id: _ }),
            "Conversion to triplet should succeed"
        );

        // Three triplet eighths at the first three positions, sharing a group id
        let id0 = m.first_voice().beats()[0].tuplet_group_id;
        assert!(id0.is_some());
        for i in 0..3 {
            assert_eq!(m.first_voice().beats()[i].duration, t8());
            assert_eq!(m.first_voice().beats()[i].tuplet_group_id, id0);
        }

        // The anchor spans one quarter
        let gid = id0.unwrap();
        let anchor = m.first_voice().tuplets().get(gid).expect("anchor must exist");
        let base_quarter_ticks = DEFAULT_GRID.ticks_of(&Duration::Simple(Eighth)).unwrap() * 2;
        assert_eq!(anchor.target_ticks, base_quarter_ticks);
    }

    #[test]
    fn convert_noop_when_already_tuplet() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // First make a triplet at position 0
        m.first_voice_mut().set_beat(0, Beat::note(t8())).unwrap();

        let before = m.clone();
        assert_matches!(
            m.first_voice_mut().convert_to_tuplet(0, TupletSpec { n: 3, m: 2, base: Eighth }, true),
            None,
            "Should not convert when already a tuplet"
        );
        assert_eq!(format!("{:?}", before), format!("{:?}", m));
    }

    #[test]
    fn convert_to_tuplet_initializes_notes_when_source_is_note() {
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        m.first_voice_mut().set_beat(0, Beat::note(Duration::Simple(Eighth))).unwrap();
        m.first_voice_mut()
            .set_tuplet(0, Some(TupletSpec { n: 3, m: 2, base: Eighth }), true)
            .unwrap();

        assert_eq!(
            m.first_voice().beats().to_vec(),
            vec![Beat::note(t8()), Beat::note(t8()), Beat::note(t8())]
        );
    }

    #[test]
    fn convert_to_tuplet_initializes_rests_when_source_is_rest() {
        let mut m = Measure::new(TimeSignature::TWO_EIGHT);
        m.first_voice_mut()
            .set_tuplet(0, Some(TupletSpec { n: 3, m: 2, base: Eighth }), true)
            .unwrap();
        assert_eq!(
            m.first_voice().beats().to_vec(),
            vec![Beat::note(t8()), Beat::note(t8()), Beat::note(t8())]
        );
    }

    #[test]
    fn convert_absorbs_rests_ok() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Setup: [Rest(e), Rest(e), Rest(q)...] by replacing the first Q with E (fills the remainder with E)
        m.first_voice_mut().set_beat(0, Beat::rest(e())).unwrap();
        // Now convert idx 0 (Rest(e)) to Triplet Eighths (span Q). Needs to absorb idx 1 (Rest(e)).
        assert_matches!(
            m.first_voice_mut().convert_to_tuplet(
                0,
                TupletSpec { n: 3, m: 2, base: Eighth },
                false
            ),
            Some(GroupSpan { start_idx: 0, end_idx: 2, id: _ }),
        );
        // Should have created a tuplet group at 0
        assert!(m.first_voice().beats()[0].tuplet_group_id.is_some());
    }

    #[test]
    fn convert_aborts_on_note_if_no_overwrite() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Setup: [Rest(e), Note(e), Rest(q)...]
        m.first_voice_mut().set_beat(0, Beat::rest(e())).unwrap();
        m.first_voice_mut().set_beat(1, Beat::note(e())).unwrap();

        // Try to convert idx 0 to Triplet Eighths (span Q). Needs to absorb idx 1, which is Note.
        assert_matches!(
            m.first_voice_mut().convert_to_tuplet(
                0,
                TupletSpec { n: 3, m: 2, base: Eighth },
                false
            ),
            None
        );
        // Verify state unchanged (idx 1 is still Note)
        assert_eq!(m.first_voice().beats()[1].kind(), Note);
        assert!(m.first_voice().beats()[0].tuplet_group_id.is_none());
    }

    #[test]
    fn convert_overwrites_note_if_overwrite_true() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        // Setup: [Rest(e), Note(e), Rest(q)...]
        m.first_voice_mut().set_beat(0, Beat::rest(e())).unwrap();
        m.first_voice_mut().set_beat(1, Beat::note(e())).unwrap();

        // Try to convert idx 0 to Triplet Eighths (span Q). Overwrite=true.
        assert_matches!(
            m.first_voice_mut().convert_to_tuplet(0, TupletSpec { n: 3, m: 2, base: Eighth }, true),
            Some(GroupSpan { start_idx: 0, end_idx: 2, id: _ }),
        );
        // Verify tuplet created
        assert!(m.first_voice().beats()[0].tuplet_group_id.is_some());
    }
}
