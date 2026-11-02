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
//! tuplet of rests only becomes plain rests. A tuplet group over several
//! beats is written as one group per beat where its notes allow (twelve
//! triplet eighths as four triplets, not one triplet of halves), and a
//! tuplet on the coarsest places its notes allow (a sextuplet sounding on
//! every other note as a triplet of eighths). Inside a tuplet, notes last
//! up to the next one and rests merge, as far as one value of its ratio
//! lasts (`t8 r:t8 t8` is `t4 t8`). [`VoiceBar::from_onsets`]
//! writes a voice from onsets alone, e.g. for heads moved between voices.

use crate::duration::{Duration, TupletSpec, q};
use crate::grid::DEFAULT_GRID;
use crate::spelling::{self, Lengths, Piece};
use crate::{Beat, Heads, Measure, TimeSignature, TupletMark, VoiceBar};
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
fn rewrite(ts: TimeSignature, notes: &[(u32, Beat)], kept: &[Kept]) -> Option<VoiceBar> {
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
                beat.tuplet = None;
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
    // Kept tuplets are copied whole, with their marks; respelled notes
    // lose theirs.
    let voice = VoiceBar::from_parts(beats, ts);
    debug_assert_eq!(
        voice.beats.iter().filter_map(|b| DEFAULT_GRID.ticks_of(&b.duration)).sum::<u32>(),
        DEFAULT_GRID.ticks_per_measure(&ts)
    );
    Some(voice)
}

/// Write each tuplet group of `beats` that spans whole beats of the meter
/// (`spans`) as one group per beat, where it can be: a beat starts on each
/// beat, every beat holds two or more of them, and a beat is `m` of a note
/// value. Twelve triplet eighths of a triplet of halves over a 4/4 bar are
/// read as four triplets of eighths, one per beat. Only the marks change.
fn split_at_beats(beats: &mut [Beat], spans: &[spelling::Span]) {
    let ticks = DEFAULT_GRID.compute_onset_ticks(beats);
    let groups: Vec<_> = (0..beats.len())
        .filter_map(|i| crate::tuplet::TupletGroup::starting_at(beats, i))
        .collect();
    for group in groups {
        let unit = unit_ticks();
        let start = ticks[group.start_idx];
        let end = start + group.span_ticks();
        let inside: Vec<_> =
            spans.iter().filter(|s| start <= s.start * unit && s.end() * unit <= end).collect();
        let covers = inside.first().is_some_and(|s| s.start * unit == start)
            && inside.last().is_some_and(|s| s.end() * unit == end)
            && inside.len() >= 2;
        if !covers {
            continue;
        }
        // The first beat of the group in each beat of the meter, with its base.
        let mut firsts = Vec::new();
        for span in &inside {
            let Some(first) =
                (group.start_idx..=group.end_idx).find(|&i| ticks[i] == span.start * unit)
            else {
                break;
            };
            let after = (first..=group.end_idx).take_while(|&i| ticks[i] < span.end() * unit);
            let Some(base) = crate::tuplet::base_spanning(span.len * unit, group.spec.m) else {
                break;
            };
            if after.count() < 2 {
                break;
            }
            firsts.push((first, base));
        }
        if firsts.len() != inside.len() {
            continue;
        }
        let ends = firsts.iter().skip(1).map(|&(i, _)| i).chain([group.end_idx + 1]);
        for (&(first, base), end) in firsts.iter().zip(ends) {
            crate::tuplet::mark_group(&mut beats[first..end], base);
        }
    }
}

/// Write each tuplet group of `beats` on the coarsest places its notes
/// allow: a sextuplet of sixteenths that sounds on every other note is a
/// triplet of eighths. The places are the longest value that divides the
/// group's span and every onset of a note in it, if that is longer than
/// the group's shortest beat and a value of the group's ratio in lowest
/// terms (6:4 is written 3:2). Each note takes one place, rests fill the
/// others; the span, and so the onsets, stay.
fn reduce_tuplets(beats: &mut Vec<Beat>) {
    rewrite_groups(beats, |members, _, span| reduced_group(members, span));
}

/// Replace the beats of each tuplet group of `beats` by what `rewrite`
/// gives for them (its beats, onset tick and span), where it gives some.
fn rewrite_groups(
    beats: &mut Vec<Beat>,
    mut rewrite: impl FnMut(&[Beat], u32, u32) -> Option<Vec<Beat>>,
) {
    let mut idx = 0;
    while idx < beats.len() {
        let Some(group) = crate::tuplet::TupletGroup::starting_at(beats, idx) else {
            idx += 1;
            continue;
        };
        let start = DEFAULT_GRID.compute_onset_ticks(beats)[group.start_idx];
        let range = group.start_idx..=group.end_idx;
        let len = match rewrite(&beats[range.clone()], start, group.span_ticks()) {
            Some(new) => {
                let len = new.len();
                beats.splice(range, new);
                len
            }
            None => group.end_idx + 1 - group.start_idx,
        };
        idx = group.start_idx + len;
    }
}

/// The beats of a tuplet group (`members`, over `span` ticks) on the
/// coarsest places its notes allow (see [`reduce_tuplets`]), if coarser.
fn reduced_group(members: &[Beat], span: u32) -> Option<Vec<Beat>> {
    let TupletSpec { n, m, .. } = members[0].duration.as_tuplet_spec()?;
    let ticks = DEFAULT_GRID.compute_onset_ticks(members);
    let notes: Vec<(u32, Beat)> =
        members.iter().zip(&ticks).filter(|(b, _)| b.is_note()).map(|(b, &t)| (t, *b)).collect();
    let step = notes.iter().fold(span, |g, &(t, _)| crate::math::gcd(g, t));
    let shortest = members.iter().filter_map(|b| DEFAULT_GRID.ticks_of(&b.duration)).min()?;
    let places = span / step;
    if step <= shortest || places < 2 {
        return None;
    }
    // The value of the ratio in lowest terms that lasts `step`.
    let common = crate::math::gcd(u32::from(n), u32::from(m));
    let (n, m) =
        (u8::try_from(u32::from(n) / common).ok()?, u8::try_from(u32::from(m) / common).ok()?);
    let value = crate::grid::NOTE_VALUES.into_iter().find(|&v| {
        let d = Duration::Tuplet(TupletSpec { n, m, base: v });
        DEFAULT_GRID.supports(&d) && DEFAULT_GRID.ticks_of(&d) == Some(step)
    })?;
    let duration = Duration::Tuplet(TupletSpec { n, m, base: value });
    let base = crate::tuplet::base_spanning(span, m)?;
    let mut out: Vec<Beat> = (0..places).map(|_| Beat::rest(duration)).collect();
    for (t, note) in notes {
        let mut beat = note;
        beat.duration = duration;
        out[(t / step) as usize] = beat;
    }
    crate::tuplet::mark_group(&mut out, base);
    Some(out)
}

/// Spell the notes inside each tuplet group like plain ones: a note lasts
/// up to the next one (`t8 r:t8 t8` is `t4 t8`) and rests next to each
/// other merge (`r:t8 r:t8 t8` is `r:t4 t8`), as far as one value of the
/// group's ratio lasts and never over a beat of the meter (`spans`). The
/// onsets stay; a group left with fewer than two beats stays as it is.
fn merge_in_tuplets(beats: &mut Vec<Beat>, spans: &[spelling::Span]) {
    rewrite_groups(beats, |members, start, _| merged_group(members, start, spans));
}

/// The beats of a tuplet group (`members`, from tick `start`) with notes
/// and rests merged (see [`merge_in_tuplets`]), if any merge.
fn merged_group(members: &[Beat], start: u32, spans: &[spelling::Span]) -> Option<Vec<Beat>> {
    let spec = members[0].duration.as_tuplet_spec()?;
    // Every supported value of the ratio, not only the common ones: a 5:3
    // quarter holds two 5:3 eighths.
    let values: Vec<Duration> = crate::grid::NOTE_VALUES
        .into_iter()
        .map(|base| Duration::Tuplet(TupletSpec { base, ..spec }))
        .filter(|d| DEFAULT_GRID.supports(d))
        .collect();
    let value_of =
        |ticks: u32| values.iter().copied().find(|d| DEFAULT_GRID.ticks_of(d) == Some(ticks));
    let beat_end = |tick: u32| {
        spans.iter().map(|s| s.end() * unit_ticks()).find(|&end| end > tick).unwrap_or(u32::MAX)
    };
    let len = |b: &Beat| DEFAULT_GRID.ticks_of(&b.duration).unwrap_or(0);
    let mut out: Vec<Beat> = Vec::new();
    let (mut i, mut at) = (0, start);
    while i < members.len() {
        let first = members[i];
        let limit = beat_end(at);
        // Take as many following rests as one value lasts.
        let mut best = (1, len(&first));
        let mut total = len(&first);
        for (j, next) in members.iter().enumerate().skip(i + 1) {
            if !next.is_rest() || at + total + len(next) > limit {
                break;
            }
            total += len(next);
            if value_of(total).is_some() {
                best = (j + 1 - i, total);
            }
        }
        let mut beat = first;
        if best.0 > 1 {
            beat.duration = value_of(best.1)?;
        }
        out.push(beat);
        at += best.1;
        i += best.0;
    }
    if out.len() == members.len() || out.len() < 2 {
        return None;
    }
    let Some(TupletMark::Start { base }) = members[0].tuplet else {
        return None;
    };
    crate::tuplet::mark_group(&mut out, base);
    Some(out)
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
        // Groups over beats first: each of their beats is then respelled
        // or kept like any tuplet inside one beat.
        let mut split = self.clone();
        reduce_tuplets(&mut split.beats);
        let spans = spelling::meter_spans(self.time_signature())?;
        split_at_beats(&mut split.beats, &spans);
        merge_in_tuplets(&mut split.beats, &spans);
        split.respelled_by_beat()
    }

    fn respelled_by_beat(&self) -> Option<VoiceBar> {
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
        rewrite(ts, &notes, &kept)
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
        rewrite(ts, &written, &[])
    }
}

impl Measure {
    /// Whether [`Measure::clean_up`] would change a voice.
    pub fn needs_clean_up(&self) -> bool {
        self.voices().iter().any(|v| v.respelled().is_some_and(|r| *v != r))
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
            if *old != voice {
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
        assert_eq!(twice, once, "cleaning {input} twice");
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
        assert_eq!(cleaned("4/4 t8 r:t8 t8 q r:q e r:e"), "4/4 [t4 t8] q r:q q");
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
    }

    #[test]
    fn a_tuplet_over_beats_is_written_per_beat() {
        // The 3:4 polyrhythm, each triplet half in four: four triplets of
        // eighths. The notes stay, the groups follow the beats.
        let m = measure("4/4 [t8 t8 t8 t8 t8 t8 t8 t8 t8 t8 t8 t8]");
        assert_eq!(m.first_voice().tuplet_groups().len(), 1);
        assert!(m.needs_clean_up());
        let once = m.cleaned_up();
        let groups = once.first_voice().tuplet_groups();
        assert_eq!(groups.len(), 4);
        assert!(groups.iter().all(|g| g.spec.base == crate::duration::NoteValue::Eighth));
        assert_eq!(
            cleaned("4/4 [t8 t8 t8 t8 t8 t8 t8 t8 t8 t8 t8 t8]"),
            "4/4 t8 t8 t8 t8 t8 t8 t8 t8 t8 t8 t8 t8"
        );
        // Six eighths of a triplet of quarters: two triplets of eighths.
        assert_eq!(cleaned("4/4 [t8 t8 t8 t8 t8 t8] r:h"), "4/4 t8 t8 t8 t8 t8 t8 r:h");
        // A note over a beat keeps the group whole.
        assert_eq!(cleaned("4/4 t2 t8 t8 t8 t8 t2"), "4/4 t2 t8 t8 t8 t8 t2");
        assert_eq!(cleaned("4/4 t4 t4 t4 t4 t4 t4"), "4/4 t4 t4 t4 t4 t4 t4");
        assert_eq!(cleaned("4/4 t4 t4 t4 r:h"), "4/4 t4 t4 t4 r:h");
        // Compound and odd meters split at their own beats: ten quintuplet
        // eighths over a 6/8 bar are five per dotted quarter, a triplet of
        // quarters over the twos of 2+2+3/8 one triplet of eighths each.
        assert_eq!(
            cleaned("6/8 [qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3]"),
            "6/8 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3"
        );
        assert_eq!(cleaned("2+2+3/8 [t8 t8 t8 t8 t8 t8] e e e"), "2+2+3/8 t8 t8 t8 t8 t8 t8 e e e");
        // Notes of other values split where each beat starts one.
        assert_eq!(cleaned("4/4 [t8 t4 t4 t8] r:h"), "4/4 t8 t4 [t4 t8] r:h");
    }

    #[test]
    fn a_tuplet_is_written_on_the_coarsest_places_its_notes_allow() {
        // A sextuplet sounding on every other note is a triplet of eighths;
        // accents and ghost notes stay on their notes.
        assert_eq!(
            cleaned("4/4 >st16 r:st16 (st16) r:st16 st16 r:st16 r:q r:h"),
            "4/4 >t8 (t8) t8 r:q r:h"
        );
        assert_eq!(
            cleaned("4/4 st16 r:st16 r:st16 r:st16 st16 r:st16 r:q r:h"),
            "4/4 [t4 t8] r:q r:h"
        );
        // Ten quintuplet sixteenths over a dotted quarter, every other one:
        // five quintuplet eighths.
        let ten = "10:6s r:10:6s ".repeat(5);
        assert_eq!(cleaned(&format!("6/8 {}r:q.", ten)), "6/8 qt8_3 qt8_3 qt8_3 qt8_3 qt8_3 r:q.");
        assert_eq!(
            cleaned("2+2+3/8 st16 r:st16 st16 r:st16 st16 r:st16 r:q r:q."),
            "2+2+3/8 t8 t8 t8 r:q r:q."
        );
        // A note off the coarser places keeps the sextuplet; its notes
        // still last up to the next one.
        assert_eq!(
            cleaned("4/4 st16 st16 r:st16 r:st16 st16 r:st16 r:q r:h"),
            "4/4 st16 st8 r:st16 st8 r:q r:h"
        );
        // A ratio with no coarser places (5:4) only merges.
        assert_eq!(
            cleaned("4/4 qt16 r:qt16 qt16 r:qt16 qt16 r:q r:h"),
            "4/4 [qt8 qt8 qt16] r:q r:h"
        );
        // Kept: a sextuplet with every note sounding.
        let full = "4/4 st16 st16 st16 st16 st16 st16 r:q r:h";
        assert_eq!(cleaned(full), full);
    }

    #[test]
    fn notes_in_a_tuplet_last_to_the_next_one() {
        // The shuffle: a note held over the rest after it.
        assert_eq!(cleaned("4/4 t8 r:t8 t8 t8 r:t8 t8 r:h"), "4/4 [t4 t8] [t4 t8] r:h");
        // Rests next to each other merge.
        assert_eq!(cleaned("4/4 r:t8 r:t8 t8 q r:h"), "4/4 [r:t4 t8] q r:h");
        // Into any value of the ratio: a 5:3 quarter in 6/8.
        assert_eq!(
            cleaned("6/8 qt8_3 r:qt8_3 qt8_3 qt8_3 qt8_3 r:q."),
            "6/8 [5:3q qt8_3 qt8_3 qt8_3] r:q."
        );
        // And the longest that fits: of four rests after it, a note in a
        // sextuplet takes three as a 6:4 quarter; five places are no value.
        assert_eq!(
            cleaned("4/4 st16 st16 r:st16 r:st16 r:st16 r:st16 r:q r:h"),
            "4/4 st16 6:4q r:st16 r:q r:h"
        );
        assert_eq!(cleaned("2+2+3/8 t8 r:t8 t8 q r:q."), "2+2+3/8 [t4 t8] q r:q.");
        assert_eq!(cleaned("4/4 t8 t8 r:t8 q r:h"), "4/4 t8 t4 q r:h");
    }

    #[test]
    fn a_dropped_tuplet_leaves_no_marks() {
        let m = measure("4/4 q r:t8 r:t8 r:t8 r:h");
        assert_eq!(m.first_voice().tuplet_groups().len(), 1);
        let once = m.cleaned_up();
        assert!(once.first_voice().beats().iter().all(|b| b.tuplet.is_none()));
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
    fn a_stale_mark_on_a_plain_note_is_cleaned_up() {
        let mut m = measure("4/4 q q q q");
        let base = crate::duration::NoteValue::Eighth;
        m.first_voice_mut().beats[0].tuplet = Some(crate::TupletMark::Start { base });
        assert!(m.needs_clean_up());
        assert!(m.clean_up());
        assert_eq!(m.first_voice().beats()[0].tuplet, None);
        assert!(!m.needs_clean_up());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "tuplet groups")]
    fn a_voice_put_together_checks_its_tuplet_groups() {
        // Half a group: its last beats without the first.
        let m = measure("4/4 t8 t8 t8 q r:h");
        let beats = m.first_voice().beats()[1..].to_vec();
        VoiceBar::from_parts(beats, TimeSignature::FOUR_FOUR);
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
