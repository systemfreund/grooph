//! Rewriting a voice from its onsets: the editor's "Clean up".
//!
//! Editing by hand can leave a measure spelled against the conventions:
//! notes held over a beat, more rests than needed. A drum hit does not
//! sustain, so the rhythm of a voice is only where each note starts, with
//! its heads (instrument, accent, ghost, technique); the written lengths
//! are spelling. [`VoiceBar::respelled`] writes the voice again from those
//! onsets by the rules of [`crate::spelling`]: each note as long as it can
//! be written up to the next one, but only to the end of its beat
//! ([`spelling::to_beat_end`]); the rest of the time as rests, merged per
//! beat and into half rests at the start of a pair of beats.
//!
//! Tuplets stay as written, with the beats they share a beat with; a
//! tuplet of rests only becomes plain rests. [`VoiceBar::from_onsets`]
//! writes a voice from onsets alone, e.g. for heads moved between voices.

use crate::duration::q;
use crate::grid::DEFAULT_GRID;
use crate::spelling::{self, Lengths, Piece};
use crate::{Beat, Heads, Measure, TimeSignature, TupletRegistry, VoiceBar};
use std::collections::HashSet;
use std::ops::Range;

/// Grid ticks in a unit of [`crate::spelling`], a thirty-second.
const fn unit_ticks() -> u32 {
    DEFAULT_GRID.ticks_per_whole / 32
}

fn to_unit(tick: u32) -> Option<u32> {
    tick.is_multiple_of(unit_ticks()).then(|| tick / unit_ticks())
}

/// Beats of the voice kept as they are, over `units`.
struct Kept<'a> {
    units: Range<u32>,
    beats: &'a [Beat],
}

/// Write a voice of `ts` with notes at `notes` (unit, the beat whose heads
/// and duration-free content it plays, ascending) and the `kept` runs.
/// `None` if a kept run does not cover whole beats of the meter or a note
/// falls inside one.
fn rewrite(
    ts: TimeSignature,
    notes: &[(u32, Beat)],
    kept: &[Kept],
    mut tuplets: TupletRegistry,
) -> Option<VoiceBar> {
    let mut spans = spelling::meter_spans(ts)?;
    for run in kept {
        let covered =
            spans.iter_mut().filter(|s| s.start < run.units.end && run.units.start < s.end());
        for span in covered {
            span.verbatim = true;
        }
    }
    // Every kept run fills its beats exactly, so they can be copied.
    let mut runs = Vec::new();
    for (i, span) in spans.iter().enumerate() {
        if span.verbatim && (i == 0 || !spans[i - 1].verbatim) {
            let end = spans[i..].iter().take_while(|s| s.verbatim).last()?.end();
            runs.push(span.start..end);
        }
    }
    let mut kept_beats: Vec<(u32, Beat)> = Vec::new();
    for run in &runs {
        let inside: Vec<&Kept> =
            kept.iter().filter(|k| run.start <= k.units.start && k.units.end <= run.end).collect();
        let mut at = run.start;
        for k in inside {
            // The gap before a kept run inside the beats: plain beats of
            // the voice must fill it, which the caller passed as notes or
            // left out as rests. Such gaps are not copied; refuse them.
            if k.units.start != at {
                return None;
            }
            // Tuplet notes start between units; rounded down, each still
            // falls in the beat it starts in, which is all this is for.
            let ticks = DEFAULT_GRID.compute_onset_ticks(k.beats);
            kept_beats.extend(ticks.iter().zip(k.beats).map(|(&t, &b)| (at + t / unit_ticks(), b)));
            at = k.units.end;
        }
        if at != run.end {
            return None;
        }
    }
    if notes.iter().any(|(unit, _)| runs.iter().any(|r| r.contains(unit))) {
        return None;
    }
    let onsets: Vec<u32> = notes.iter().map(|&(unit, _)| unit).collect();
    let lengths = Lengths { step: 1, short_notes: true };
    let pieces = spelling::write(&spans, &onsets, lengths, spelling::to_beat_end);
    let mut beats = Vec::new();
    let mut next_note = notes.iter();
    for piece in pieces {
        match piece {
            Piece::Note { duration, .. } => {
                let (_, note) = next_note.next()?;
                let mut beat = *note;
                beat.duration = duration;
                beat.tuplet_group_id = None;
                beats.push(beat);
            }
            Piece::Rest(duration) => beats.push(Beat::rest(duration)),
            Piece::Verbatim(i) => {
                let span = spans[i];
                beats.extend(
                    kept_beats
                        .iter()
                        .filter(|(unit, _)| span.start <= *unit && *unit < span.end())
                        .map(|&(_, b)| b),
                );
            }
        }
    }
    let groups: HashSet<_> = beats.iter().filter_map(|b| b.tuplet_group_id).collect();
    tuplets.retain(|id| groups.contains(&id));
    let voice = VoiceBar::from_parts(beats, ts, tuplets);
    debug_assert_eq!(
        voice.beats.iter().filter_map(|b| DEFAULT_GRID.ticks_of(&b.duration)).sum::<u32>(),
        DEFAULT_GRID.ticks_per_measure(&ts)
    );
    Some(voice)
}

impl VoiceBar {
    /// The voice written again from its onsets (see the module docs): the
    /// same notes at the same times, with the same heads, spelled for
    /// reading. Applying it again changes nothing.
    ///
    /// `None` where the voice cannot be written so: a meter not over
    /// quarters, eighths or sixteenths, or a tuplet that starts or ends
    /// inside a beat where no beat of the voice starts.
    pub fn respelled(&self) -> Option<VoiceBar> {
        let ts = self.time_signature();
        let ticks = DEFAULT_GRID.compute_onset_ticks(&self.beats);
        let end_of =
            |idx: usize| ticks[idx] + DEFAULT_GRID.ticks_of(&self.beats[idx].duration).unwrap_or(0);
        let groups = self.tuplet_groups();
        let in_group = |idx: usize| groups.iter().any(|g| (g.start_idx..=g.end_idx).contains(&idx));
        // A tuplet value outside a group cannot be written from its onset.
        if (0..self.beats.len()).any(|i| self.beats[i].duration.is_tuplet() && !in_group(i)) {
            return None;
        }
        // A tuplet whose notes all start on the plain grid (a triplet with
        // its first note only) is a plain rhythm written as a tuplet: its
        // notes are respelled like the others. A tuplet of rests only is a
        // rest. Other tuplets are kept as written.
        let mut kept = Vec::new();
        // Every other beat, one by one, and those tuplets as a whole: where
        // they are, the beats to keep if they stay as written, and the
        // notes to respell if not.
        type Other<'a> = (Range<u32>, &'a [Beat], Vec<(u32, Beat)>);
        let mut others: Vec<Other> = Vec::new();
        let mut silent = Vec::new();
        for g in &groups {
            let beats = &self.beats[g.start_idx..=g.end_idx];
            let units = to_unit(ticks[g.start_idx])?..to_unit(end_of(g.end_idx))?;
            let plain_notes: Option<Vec<(u32, Beat)>> = (g.start_idx..=g.end_idx)
                .filter(|&i| self.beats[i].is_note())
                .map(|i| to_unit(ticks[i]).map(|unit| (unit, self.beats[i])))
                .collect();
            match plain_notes {
                Some(notes) if notes.is_empty() => silent.push((units, beats)),
                Some(notes) => others.push((units, beats, notes)),
                None => kept.push(Kept { units, beats }),
            }
        }
        for (idx, beat) in self.beats.iter().enumerate() {
            if !in_group(idx) {
                let units = to_unit(ticks[idx])?..to_unit(end_of(idx))?;
                let notes = if beat.is_note() { vec![(units.start, *beat)] } else { vec![] };
                others.push((units, std::slice::from_ref(beat), notes));
            }
        }
        // A silent tuplet is a plain rest where one value fills it, also
        // where it stays as written next to a kept tuplet.
        let rests: Vec<Option<Beat>> = silent
            .iter()
            .map(|(u, _)| spelling::duration_of(u.end - u.start).map(Beat::rest))
            .collect();
        for ((units, beats), rest) in silent.into_iter().zip(&rests) {
            let beats = rest.as_ref().map_or(beats, std::slice::from_ref);
            others.push((units, beats, vec![]));
        }
        // The beats of the meter a kept tuplet is in are kept as written,
        // and so is every other beat sounding in them, with the beats of
        // the meter it reaches into.
        let mut spans = spelling::meter_spans(ts)?;
        let overlap =
            |s: &spelling::Span, units: &Range<u32>| s.start < units.end && units.start < s.end();
        for k in &kept {
            for span in spans.iter_mut().filter(|s| overlap(s, &k.units)) {
                span.verbatim = true;
            }
        }
        loop {
            let mut grew = false;
            for (units, ..) in &others {
                if spans.iter().any(|s| s.verbatim && overlap(s, units)) {
                    for span in spans.iter_mut().filter(|s| !s.verbatim && overlap(s, units)) {
                        span.verbatim = true;
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        let mut notes = Vec::new();
        let mut plain: Vec<Kept> = Vec::new();
        for (units, beats, free) in others {
            if spans.iter().any(|s| s.verbatim && overlap(s, &units)) {
                // Kept as written next to the tuplet, rests too.
                plain.push(Kept { units, beats });
            } else {
                notes.extend(free);
            }
        }
        notes.sort_by_key(|&(unit, _)| unit);
        kept.extend(plain);
        kept.sort_by_key(|k| k.units.start);
        rewrite(ts, &notes, &kept, self.tuplets.clone())
    }

    /// A voice of `ts` playing `heads` at each tick of `notes` (ticks from
    /// the start of the measure, ascending), written as
    /// [`VoiceBar::respelled`] writes notes. `None` if a tick is off the
    /// thirty-second grid, outside the measure or given twice, if a note
    /// has no heads, or for a meter not over quarters, eighths or
    /// sixteenths.
    pub fn from_onsets(
        ts: TimeSignature,
        notes: impl IntoIterator<Item = (u32, Heads)>,
    ) -> Option<VoiceBar> {
        let end = to_unit(DEFAULT_GRID.ticks_per_measure(&ts))?;
        let mut written: Vec<(u32, Beat)> = Vec::new();
        for (tick, heads) in notes {
            let unit = to_unit(tick)?;
            let after_last = written.last().is_none_or(|&(last, _)| last < unit);
            if heads.is_empty() || unit >= end || !after_last {
                return None;
            }
            written.push((unit, Beat::with_heads(q(), heads)));
        }
        rewrite(ts, &written, &[], TupletRegistry::new())
    }
}

/// Whether `a` and `b` have the same beats, tuplet groups included
/// ([`Beat`]'s `PartialEq` leaves them out), and as many tuplet anchors
/// (a respelled voice only drops anchors no beat names).
pub(crate) fn same_beats(a: &VoiceBar, b: &VoiceBar) -> bool {
    a.beats == b.beats
        && a.beats.iter().zip(&b.beats).all(|(x, y)| x.tuplet_group_id == y.tuplet_group_id)
        && a.tuplets.len() == b.tuplets.len()
}

impl Measure {
    /// Whether [`Measure::clean_up`] would change a voice.
    pub fn needs_clean_up(&self) -> bool {
        self.voices().iter().any(|v| v.respelled().is_some_and(|r| !same_beats(v, &r)))
    }

    /// The measure with every voice [respelled](VoiceBar::respelled): the
    /// editor's "Clean up". A voice that cannot be respelled stays as it
    /// is.
    pub fn cleaned_up(&self) -> Measure {
        let mut out = self.clone();
        out.clean_up();
        out
    }

    /// Respell every voice in place (see [`Measure::cleaned_up`]). Returns
    /// whether a voice changed.
    pub fn clean_up(&mut self) -> bool {
        let mut changed = false;
        for idx in 0..self.voices().len() {
            let Some(voice) = self.voice(idx).and_then(VoiceBar::respelled) else {
                continue;
            };
            let old = self.voice_mut(idx).expect("voice in range");
            if !same_beats(old, &voice) {
                *old = voice;
                changed = true;
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notation::{format_score, parse_score};
    use crate::onsets::HeadOnset;
    use crate::{Head, Instrument, Score};

    fn measure(s: &str) -> Measure {
        parse_score(s).unwrap().into_measures().remove(0)
    }

    fn text(m: &Measure) -> String {
        format_score(&Score::new(vec![m.clone()])).unwrap()
    }

    /// Clean `input` up, check that the onsets stay and a second cleanup
    /// changes nothing, and give the result as text.
    fn cleaned(input: &str) -> String {
        let m = measure(input);
        let once = m.cleaned_up();
        let ticks = |m: &Measure| -> Vec<(u32, usize, Head)> {
            m.head_onsets()
                .iter()
                .map(|&HeadOnset { tick, voice, head, .. }| (tick, voice, head))
                .collect()
        };
        assert_eq!(ticks(&once), ticks(&m), "onsets of {input}");
        let twice = once.cleaned_up();
        assert_eq!(text(&twice), text(&once), "cleaning {input} twice");
        for (a, b) in once.voices().iter().zip(twice.voices()) {
            let ids =
                |v: &VoiceBar| v.beats().iter().map(|b| b.tuplet_group_id).collect::<Vec<_>>();
            assert_eq!(ids(a), ids(b));
        }
        text(&once)
    }

    #[test]
    fn notes_last_to_the_end_of_their_beat_only() {
        // A backbeat held over the beats reads as quarters and rests.
        assert_eq!(cleaned("4/4 r:q h q"), "4/4 r:q q r:q q");
        assert_eq!(cleaned("4/4 r:q q r:q q"), "4/4 r:q q r:q q");
        // `8 16-rest 16` becomes `8. 16`.
        assert_eq!(cleaned("4/4 e r:s s r:q r:h"), "4/4 e. s r:q r:h");
        // A note from the "and" stops at the beat.
        assert_eq!(cleaned("4/4 e q e r:h"), "4/4 e e r:e e r:h");
        // Rests merge per beat and into a half rest on beat 1 or 3.
        assert_eq!(cleaned("4/4 q r:e r:e r:s r:s r:e r:e r:e"), "4/4 q r:q r:h");
        // Never over the middle of the bar.
        assert_eq!(cleaned("4/4 q r:q r:q q"), "4/4 q r:q r:q q");
        assert_eq!(cleaned("4/4 r:s s r:e r:q r:h"), "4/4 r:s e. r:q r:h");
        assert_eq!(cleaned("4/4 th th r:s r:e r:q r:h"), "4/4 th s. r:e r:q r:h");
    }

    #[test]
    fn accents_ghosts_and_kit_heads_stay_on_their_notes() {
        assert_eq!(cleaned("4/4 >e r:e (e) r:e q. e"), "4/4 >q (q) q r:e e");
        assert_eq!(
            cleaned("4/4 S-Ho:e r:e K:h r:q ; K:q. r:e r:h"),
            "4/4 S-Ho:q K:q r:h ; K:q r:q r:h"
        );
    }

    #[test]
    fn an_empty_bar_is_rests_per_beat_pair() {
        assert_eq!(cleaned("4/4 r:e r:e r:q r:s r:s r:e r:q"), "4/4 r:h r:h");
        // 3/4: a half rest from beat 1, then a quarter rest. Drawn as one
        // whole rest by the layout (`bar_rest`).
        assert_eq!(cleaned("3/4 r:q r:e r:e r:q"), "3/4 r:h r:q");
        assert_eq!(cleaned("6/8 r:e r:e r:e r:e r:e r:e"), "6/8 r:q. r:q.");
    }

    #[test]
    fn compound_meters_write_dotted_quarter_beats() {
        assert_eq!(cleaned("6/8 e r:e e r:q. "), "6/8 q e r:q.");
        assert_eq!(cleaned("6/8 q. e r:q"), "6/8 q. q.");
        assert_eq!(cleaned("6/8 r:e e r:e r:e r:e e"), "6/8 r:e q r:q e");
        assert_eq!(cleaned("12/8 q. r:q. r:e r:e e r:q."), "12/8 q. r:q. r:q e r:q.");
    }

    #[test]
    fn odd_meters_follow_the_picked_grouping() {
        // 2+3+2: notes stay in their group.
        assert_eq!(cleaned("7/8 e e q. r:e e"), "7/8 e e q. r:e e");
        assert_eq!(cleaned("7/8 e e e r:q e r:e"), "7/8 e e q. q");
        assert_eq!(cleaned("7/8 q q q e"), "7/8 q q e r:e e");
        // 3+2+2 changes where the beats are.
        assert_eq!(cleaned("3+2+2/8 q q q e"), "3+2+2/8 q e r:e e r:e e");
    }

    #[test]
    fn tuplets_stay_as_written() {
        assert_eq!(cleaned("4/4 t8 r:t8 t8 q r:q e r:e"), "4/4 t8 r:t8 t8 q r:q q");
        // A tuplet of rests only becomes a plain rest.
        assert_eq!(cleaned("4/4 q r:t8 r:t8 r:t8 r:h"), "4/4 q r:q r:h");
        // So does a tuplet whose notes all start on the plain grid.
        assert_eq!(
            cleaned("4/4 H:q H:q H:q H:q ; K:h K:t8 r:t8 r:t8 r:q"),
            "4/4 H:q H:q H:q H:q ; K:q r:q K:q r:q"
        );
        assert_eq!(cleaned("4/4 st16 r:st16 r:st16 st16 r:st16 r:st16 r:q r:h"), "4/4 e e r:q r:h");
        // A tuplet from the "and" keeps the beats it shares as written,
        // and a note from there into the next beat keeps that one too.
        assert_eq!(cleaned("4/4 e t8 t8 t8 e r:h"), "4/4 e t8 t8 t8 e r:h");
        assert_eq!(cleaned("4/4 e t8 t8 t8 q r:e q"), "4/4 e t8 t8 t8 q r:e q");
        assert_eq!(cleaned("4/4 e t8 t8 t8 q e r:s r:s s s"), "4/4 e t8 t8 t8 q e r:e s s");
        assert_eq!(cleaned("4/4 e t8 t8 t8 q e q"), "4/4 e t8 t8 t8 q e q");
        let m = measure("4/4 t8 t8 t8 h e r:e");
        let once = m.cleaned_up();
        let groups = once.first_voice().tuplet_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(once.first_voice().tuplets().len(), 1);
    }

    #[test]
    fn a_dropped_tuplet_loses_its_anchor() {
        let m = measure("4/4 q r:t8 r:t8 r:t8 r:h");
        assert_eq!(m.first_voice().tuplets().len(), 1);
        assert!(m.cleaned_up().first_voice().tuplets().is_empty());
    }

    #[test]
    fn every_voice_is_cleaned_on_its_own() {
        // Hands in eighths, the feet with long notes and many rests.
        assert_eq!(
            cleaned("4/4 H:e H:e S-H:e H:e H:e H:e S-H:e H:e ; K:h r:e K:e r:q"),
            "4/4 H:e H:e S-H:e H:e H:e H:e S-H:e H:e ; K:q r:q r:e K:e r:q"
        );
    }

    #[test]
    fn a_silent_tuplet_next_to_a_kept_one_becomes_a_rest() {
        assert_eq!(
            cleaned("4/4 t16 t16 t16 r:t16 r:t16 r:t16 r:q r:h"),
            "4/4 t16 t16 t16 r:e r:q r:h"
        );
    }

    #[test]
    fn an_orphan_anchor_is_cleaned_up() {
        let mut m = measure("4/4 q q q q");
        m.first_voice_mut().tuplets.register(3, 2, 1, crate::duration::NoteValue::Eighth);
        assert!(m.needs_clean_up());
        assert!(m.clean_up());
        assert!(m.first_voice().tuplets().is_empty());
    }

    #[test]
    fn a_stale_group_id_on_a_plain_note_is_cleaned_up() {
        let mut m = measure("4/4 q q q q");
        let voice = m.first_voice_mut();
        let id = voice.tuplets.register(3, 2, 1, crate::duration::NoteValue::Eighth);
        voice.beats[0].tuplet_group_id = Some(id);
        assert!(m.needs_clean_up());
        assert!(m.clean_up());
        assert_eq!(m.first_voice().beats()[0].tuplet_group_id, None);
        assert!(m.first_voice().tuplets().is_empty());
        assert!(!m.needs_clean_up());
    }

    #[test]
    fn clean_up_says_whether_it_changed_the_measure() {
        let mut m = measure("4/4 r:q h q");
        assert!(m.clean_up());
        assert!(!m.clean_up());
    }

    #[test]
    fn from_onsets_writes_a_voice() {
        let ts = TimeSignature::FOUR_FOUR;
        let q = DEFAULT_GRID.ticks_per_beat(&ts);
        let kick = || Heads::new([Head::new(Instrument::Kick)]).unwrap();
        let voice = VoiceBar::from_onsets(ts, [(0, kick()), (q * 5 / 2, kick())]).unwrap();
        let mut m = Measure::new(ts);
        *m.first_voice_mut() = voice;
        assert_eq!(text(&m), "4/4 K:q r:q r:e K:e r:q");
        assert!(VoiceBar::from_onsets(ts, [(q, kick()), (0, kick())]).is_none());
        assert!(VoiceBar::from_onsets(ts, [(4 * q, kick())]).is_none());
        assert!(VoiceBar::from_onsets(ts, [(1, kick())]).is_none());
    }
}
