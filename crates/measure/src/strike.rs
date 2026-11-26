//! Striking an instrument between the beats of a voice, and tidying a beat
//! after a note is taken away.
//!
//! On the kit an instrument goes to the voice it belongs to, at the time of
//! the cursor in the other voice. Where nothing of that voice starts then,
//! [`Measure::start_note`] splits the rest or note sounding then, so the
//! new note starts at that time. The time from the start of what was split
//! to the end of the beat is written by the rules of Clean up
//! ([`VoiceBar::respelled`]): notes last up to the next one, at most to the
//! end of their beat, and rests merge per beat. Where the time lies inside
//! a tuplet of the other voice, the voice gets that tuplet over the same
//! span. Everything else keeps its spelling.
//!
//! [`VoiceBar::tidy_beat_of`] writes the beat of a note made a rest again
//! the way Clean up writes it: the note before it lasts up to the next one
//! again, and a tuplet left with rests only becomes a rest.

use crate::cleanup::{merged_group, to_unit, unit_ticks};
use crate::duration::Duration;
use crate::grid::DEFAULT_GRID;
use crate::spelling::{self, Lengths, Piece};
use crate::{Beat, BeatIdx, Head, Heads, Instrument, Measure, TupletGroup, VoiceBar, VoiceIdx};
use std::ops::Range;

/// What starts at a unit of the span [`Measure::start_note`] writes.
#[derive(Clone, Copy)]
enum Start {
    /// A note, as it is (its duration and mark are written anew).
    Note(Beat),
    /// A place holder that makes the spelling start a value here; it is
    /// written as a rest.
    Gap,
}

/// The ticks each beat of `beats` starts and ends at.
fn bounds(beats: &[Beat]) -> Vec<Range<u32>> {
    let ticks = DEFAULT_GRID.compute_onset_ticks(beats);
    beats
        .iter()
        .zip(ticks)
        .map(|(b, t)| t..t + DEFAULT_GRID.ticks_of(&b.duration).unwrap_or(0))
        .collect()
}

/// What of `voice` cannot be cut apart, in ticks: each tuplet group as a
/// whole, every other beat on its own.
fn pieces_of(voice: &VoiceBar) -> Vec<Range<u32>> {
    let bounds = bounds(voice.beats());
    let groups = voice.tuplet_groups();
    let mut out = Vec::new();
    let mut idx = 0;
    while idx < bounds.len() {
        match groups.iter().find(|g| g.start_idx == idx) {
            Some(g) => {
                out.push(bounds[g.start_idx].start..bounds[g.end_idx].end);
                idx = g.end_idx + 1;
            }
            None => {
                out.push(bounds[idx].clone());
                idx += 1;
            }
        }
    }
    out
}

/// The beats of the meter of `ts` in ticks: quarters in `x/4`, the beat
/// groups in `x/8` and `x/16`.
fn meter_beats(spans: &[spelling::Span]) -> Vec<Range<u32>> {
    spans.iter().map(|s| s.start * unit_ticks()..s.end() * unit_ticks()).collect()
}

/// `range` grown to beats of the meter (`beats`) until none of `pieces`
/// (of each voice) straddles one of its ends.
fn grown(mut range: Range<u32>, beats: &[Range<u32>], pieces: &[&[Range<u32>]]) -> Range<u32> {
    let beat_start = |t: u32| beats.iter().find(|b| b.contains(&t)).map_or(0, |b| b.start);
    let beat_end = |t: u32| beats.iter().find(|b| b.contains(&t)).map_or(t + 1, |b| b.end);
    loop {
        let mut next = range.clone();
        for p in pieces.iter().flat_map(|p| p.iter()) {
            if p.start < next.start && next.start < p.end {
                next.start = beat_start(p.start);
            }
            if p.start < next.end && next.end < p.end {
                next.end = beat_end(p.end - 1);
            }
        }
        if next == range {
            return range;
        }
        range = next;
    }
}

/// The cursor voice's tuplet at `tick`, where the time is off the plain
/// grid: its group, its start and its end in ticks.
fn group_at(voice: &VoiceBar, tick: u32) -> Option<(TupletGroup, u32, u32)> {
    let bounds = bounds(voice.beats());
    let idx = bounds.iter().position(|b| b.start == tick)?;
    let group = voice.tuplet_group_at(idx)?;
    let start = bounds[group.start_idx].start;
    Some((group, start, start + group.span_ticks()))
}

/// The beats of a group of `group`'s ratio from `start` to `end`, a rest in
/// every slot but a note at each of `notes` (ticks, ascending), merged as
/// Clean up merges them. `None` where a note is off the group's slots.
fn tuplet_beats(
    group: TupletGroup,
    start: u32,
    end: u32,
    notes: &[(u32, Beat)],
    spans: &[spelling::Span],
) -> Option<Vec<Beat>> {
    let slot = Duration::Tuplet(group.spec);
    let len = DEFAULT_GRID.ticks_of(&slot)?;
    let count = (end - start) / len;
    let mut beats: Vec<Beat> = (0..count).map(|_| Beat::rest(slot)).collect();
    for &(tick, note) in notes {
        let offset = tick - start;
        if !offset.is_multiple_of(len) {
            return None;
        }
        let mut beat = note;
        beat.duration = slot;
        beats[(offset / len) as usize] = beat;
    }
    crate::tuplet::mark_group(&mut beats, group.spec.base);
    Some(merged_group(&beats, start, spans).unwrap_or(beats))
}

/// `voice` with each plain rest over an end of `range` cut there, where
/// both parts are one value each.
fn rests_cut_at(voice: &VoiceBar, range: &Range<u32>) -> VoiceBar {
    let mut beats = Vec::new();
    for (beat, b) in voice.beats().iter().zip(bounds(voice.beats())) {
        let cut = [range.start, range.end].into_iter().find(|&t| b.start < t && t < b.end);
        let parts = cut.filter(|_| beat.is_rest() && beat.tuplet.is_none()).and_then(|t| {
            let first = spelling::duration_of((t - b.start) / unit_ticks())?;
            let second = spelling::duration_of((b.end - t) / unit_ticks())?;
            Some([Beat::rest(first), Beat::rest(second)])
        });
        match parts {
            Some(parts) => beats.extend(parts),
            None => beats.push(*beat),
        }
    }
    VoiceBar::from_parts(beats, voice.time_signature())
}

impl Measure {
    /// The measure with a plain head of `instrument` starting at `tick` in
    /// `voice`, where no beat of that voice starts then (see the module
    /// docs). The rest or note sounding then is split, the note keeping
    /// everything it had; a tuplet of `cursor_voice` at `tick` comes into
    /// `voice` over the same span. `None` where it cannot be written so: a
    /// beat of `voice` starts at `tick` already, the time is off the plain
    /// grid and not on a slot of such a tuplet, `voice` has a tuplet or
    /// notes off that tuplet's slots there, or a meter Clean up does not
    /// write.
    pub fn start_note(
        &self,
        voice: VoiceIdx,
        tick: u32,
        instrument: Instrument,
        cursor_voice: VoiceIdx,
    ) -> Option<Measure> {
        let target = self.voice(voice)?;
        let ts = target.time_signature();
        let spans = spelling::meter_spans(ts)?;
        let beats = target.beats();
        let bounds = bounds(beats);
        if tick >= DEFAULT_GRID.ticks_per_measure(&ts) || bounds.iter().any(|b| b.start == tick) {
            return None;
        }
        let new = Beat::with_heads(crate::duration::q(), Heads::one(Head::new(instrument)));
        // Off the plain grid: the tuplet of the cursor's voice there.
        let tuplet = match to_unit(tick) {
            Some(_) => None,
            None => {
                let (group, start, end) = group_at(self.voice(cursor_voice)?, tick)?;
                to_unit(start)?;
                to_unit(end)?;
                Some((group, start, end))
            }
        };
        let from = tuplet.map_or(tick, |(_, start, _)| start);
        let split = bounds.iter().rposition(|b| b.start <= from)?;
        let a = bounds[split].start;
        let meter = meter_beats(&spans);
        let last = [tick, tuplet.map_or(0, |(.., end)| end - 1), bounds[split].end - 1];
        let last = last.into_iter().max().unwrap_or(tick);
        let end_of_beat = meter.iter().find(|b| b.contains(&last))?.end;
        let range = grown(a..end_of_beat, &meter, &[&pieces_of(target)]);
        if range.start != a {
            return None;
        }
        let b = range.end;
        let inside: Range<usize> =
            split..bounds.iter().position(|t| t.start >= b).unwrap_or(beats.len());
        // A tuplet of this voice in the span stays: nothing is written there.
        if beats[inside.clone()].iter().any(|beat| beat.duration.is_tuplet()) {
            return None;
        }
        let in_tuplet = |t: u32| tuplet.is_some_and(|(_, start, end)| start <= t && t < end);
        // What starts where, in units: the notes before the split in its
        // beat (they shape the spelling, and stay as written), the split
        // note or a place holder for a rest starting inside a beat, the
        // notes after it, the new one, and the tuplet's ends.
        let beat_start = meter.iter().find(|m| m.contains(&a))?.start;
        let mut starts: Vec<(u32, Start)> = Vec::new();
        let ticks = DEFAULT_GRID.compute_onset_ticks(beats);
        for (i, beat) in beats.iter().enumerate() {
            let t = ticks[i];
            if t < beat_start || t >= b || in_tuplet(t) || beat.is_rest() {
                continue;
            }
            if let Some(unit) = to_unit(t) {
                starts.push((unit, Start::Note(*beat)));
            }
        }
        match tuplet {
            Some((_, start, end)) => {
                starts.push((to_unit(start)?, Start::Gap));
                if end < b && !starts.iter().any(|&(u, _)| u * unit_ticks() == end) {
                    starts.push((to_unit(end)?, Start::Gap));
                }
            }
            None => starts.push((to_unit(tick)?, Start::Note(new))),
        }
        if beats[split].is_rest() && a != beat_start && !in_tuplet(a) {
            starts.push((to_unit(a)?, Start::Gap));
        }
        starts.sort_by_key(|&(u, _)| u);
        starts.dedup_by_key(|&mut (u, _)| u);
        // Every beat of the meter outside the span is left as it is.
        let mut spans = spans;
        for span in &mut spans {
            let (s, e) = (span.start * unit_ticks(), span.end() * unit_ticks());
            span.verbatim = e <= beat_start || s >= b;
        }
        let onsets: Vec<u32> = starts.iter().map(|&(u, _)| u).collect();
        let lengths = Lengths { step: 1, short_notes: true };
        let pieces = spelling::write(&spans, &onsets, lengths, spelling::to_beat_end);
        let mut written: Vec<Beat> = Vec::new();
        let mut at = 0;
        let mut group_done = false;
        for piece in pieces {
            let (unit, beat) = match piece {
                Piece::Verbatim(i) => {
                    at = spans[i].end();
                    continue;
                }
                Piece::Note { onset, duration } => {
                    let (_, start) = starts.iter().find(|&&(u, _)| u == onset)?;
                    let beat = match *start {
                        Start::Note(note) => {
                            let mut beat = note;
                            beat.duration = duration;
                            beat.tuplet = None;
                            beat
                        }
                        Start::Gap => Beat::rest(duration),
                    };
                    (onset, beat)
                }
                Piece::Rest(duration) => (at, Beat::rest(duration)),
            };
            let len = DEFAULT_GRID.ticks_of(&beat.duration)? / unit_ticks();
            at = unit + len;
            let t = unit * unit_ticks();
            if t < a || t >= b {
                continue;
            }
            match tuplet {
                Some((group, start, end)) if in_tuplet(t) => {
                    if !group_done {
                        let mut notes: Vec<(u32, Beat)> = ticks
                            .iter()
                            .zip(beats)
                            .filter(|&(&t, beat)| in_tuplet(t) && beat.is_note())
                            .map(|(&t, beat)| (t, *beat))
                            .collect();
                        notes.push((tick, new));
                        notes.sort_by_key(|&(t, _)| t);
                        written.extend(tuplet_beats(group, start, end, &notes, &spans)?);
                        group_done = true;
                    }
                }
                _ => written.push(beat),
            }
        }
        let total: u32 = written.iter().filter_map(|b| DEFAULT_GRID.ticks_of(&b.duration)).sum();
        if total != b - a {
            return None;
        }
        let mut out_beats = beats[..inside.start].to_vec();
        out_beats.extend(written);
        out_beats.extend_from_slice(&beats[inside.end..]);
        let mut out = self.clone();
        *out.voice_mut(voice)? = VoiceBar::from_parts(out_beats, ts);
        Some(out)
    }
}

impl VoiceBar {
    /// Write the beat of the meter that the beat at `idx` is in again by
    /// the rules of Clean up ([`VoiceBar::respelled`]), with what it
    /// reaches into: after a note there was made a rest, the note before
    /// it lasts up to the next one again and rests merge; a tuplet of rests
    /// only becomes a rest, one whose notes lie on the plain grid plain
    /// notes. The rest of the voice keeps its spelling. Whether it changed;
    /// a voice Clean up cannot write stays as it is.
    pub fn tidy_beat_of(&mut self, idx: BeatIdx) -> bool {
        let Some(respelled) = self.respelled() else {
            return false;
        };
        let Some(spans) = spelling::meter_spans(self.time_signature()) else {
            return false;
        };
        let bounds = bounds(&self.beats);
        let Some(at) = bounds.get(idx).map(|b| b.start) else {
            return false;
        };
        let meter = meter_beats(&spans);
        let Some(beat) = meter.iter().find(|m| m.contains(&at)).cloned() else {
            return false;
        };
        // Clean up merges silent beats; only this beat is written, so a rest
        // of it over the next or the last one is cut at the beat.
        let respelled = rests_cut_at(&respelled, &beat);
        let range = grown(beat, &meter, &[&pieces_of(self), &pieces_of(&respelled)]);
        let cut = |bounds: &[Range<u32>]| {
            let first = bounds.iter().position(|b| b.start >= range.start).unwrap_or(bounds.len());
            let last = bounds.iter().position(|b| b.start >= range.end).unwrap_or(bounds.len());
            first..last
        };
        let mine = cut(&bounds);
        let theirs = cut(&self::bounds(&respelled.beats));
        let mut beats = self.beats[..mine.start].to_vec();
        beats.extend_from_slice(&respelled.beats[theirs]);
        beats.extend_from_slice(&self.beats[mine.end..]);
        if beats == self.beats {
            return false;
        }
        *self = VoiceBar::from_parts(beats, self.time_signature());
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::Instrument::{self, Kick, Snare};
    use crate::notation::{format_score, parse_score};
    use crate::{Measure, Score};

    fn measure(s: &str) -> Measure {
        parse_score(s).unwrap().into_measures().remove(0)
    }

    fn text(m: &Measure) -> String {
        // Always with `kit`, which is only written for a bar without heads.
        let score = Score::new(vec![m.clone()]).with_staff(crate::StaffKind::Kit);
        let text = format_score(&score).unwrap();
        match text.starts_with("kit ") {
            true => text,
            false => format!("kit {text}"),
        }
    }

    /// `input` with `instrument` started in voice `voice` at the start of
    /// beat `beat` of voice `cursor_voice`.
    fn started(
        input: &str,
        cursor_voice: usize,
        beat: usize,
        voice: usize,
        instrument: Instrument,
    ) -> Option<String> {
        let m = measure(input);
        let ticks = crate::grid::DEFAULT_GRID.compute_onset_ticks(m.voice(cursor_voice)?.beats());
        m.start_note(voice, ticks[beat], instrument, cursor_voice).map(|m| text(&m))
    }

    const EIGHTHS: &str = "H:e H:e H:e H:e H:e H:e H:e H:e";

    #[test]
    fn a_rest_sounding_then_is_split() {
        let input = format!("kit 4/4 {EIGHTHS} ; r:q r:q r:q r:q");
        assert_eq!(
            started(&input, 0, 1, 1, Kick).unwrap(),
            format!("kit 4/4 {EIGHTHS} ; r:e K:e r:q r:q r:q")
        );
        // From the lower voice into the upper one.
        let input = "kit 4/4 r:h r:h ; K:e K:e K:q K:h";
        assert_eq!(
            started(input, 1, 1, 0, Snare).unwrap(),
            "kit 4/4 r:e S:e r:q r:h ; K:e K:e K:q K:h"
        );
    }

    #[test]
    fn a_note_sounding_then_is_split_and_keeps_what_it_has() {
        let input = "kit 4/4 >Sr:q@R r:q r:h ; K:e K:e K:q K:h";
        assert_eq!(
            started(input, 1, 1, 0, Snare).unwrap(),
            "kit 4/4 >Sr:e@R S:e r:q r:h ; K:e K:e K:q K:h"
        );
        let input = format!("kit 4/4 {EIGHTHS} ; (K):q r:q r:q r:q");
        assert_eq!(
            started(&input, 0, 1, 1, Kick).unwrap(),
            format!("kit 4/4 {EIGHTHS} ; (K):e K:e r:q r:q r:q")
        );
    }

    #[test]
    fn the_span_is_written_as_clean_up_writes_it() {
        let sixteenths = "H:s H:s H:s H:s H:s H:s H:s H:s H:q H:q";
        let input = format!("kit 4/4 {sixteenths} ; r:q r:e r:e r:q r:q");
        // Outside the span the lower voice keeps its spelling (r:e r:e).
        assert_eq!(
            started(&input, 0, 1, 1, Kick).unwrap(),
            format!("kit 4/4 {sixteenths} ; r:s K:e. r:e r:e r:q r:q")
        );
        let input = format!("kit 4/4 {sixteenths} ; K:h r:e r:e r:q");
        assert_eq!(
            started(&input, 0, 5, 1, Kick).unwrap(),
            format!("kit 4/4 {sixteenths} ; K:q r:s K:e. r:e r:e r:q")
        );
        // A rest starting inside a beat keeps its start.
        let input = format!("kit 4/4 {sixteenths} ; K:s r:s r:e r:q r:h");
        assert_eq!(
            started(&input, 0, 3, 1, Kick).unwrap(),
            format!("kit 4/4 {sixteenths} ; K:s r:s r:s K:s r:q r:h")
        );
    }

    #[test]
    fn x8_meters_split_within_their_beat_group() {
        let input = "kit 6/8 H:e H:e H:e H:e H:e H:e ; r:q. r:q.";
        assert_eq!(
            started(input, 0, 1, 1, Kick).unwrap(),
            "kit 6/8 H:e H:e H:e H:e H:e H:e ; r:e K:q r:q."
        );
    }

    #[test]
    fn a_tuplet_comes_into_the_other_voice() {
        let triplets = "H:t8 H:t8 H:t8 H:q H:h";
        let input = format!("kit 4/4 {triplets} ; r:q r:q r:h");
        assert_eq!(
            started(&input, 0, 1, 1, Kick).unwrap(),
            format!("kit 4/4 {triplets} ; r:t8 K:t4 r:q r:h")
        );
        // Over a longer rest, which is split around it.
        let input = format!("kit 4/4 {triplets} ; r:h r:h");
        assert_eq!(
            started(&input, 0, 2, 1, Kick).unwrap(),
            format!("kit 4/4 {triplets} ; [r:t4 K:t8] r:q r:h")
        );
        // A note on the group's first slot is kept in it.
        let input = format!("kit 4/4 {triplets} ; K:q r:q r:h");
        assert_eq!(
            started(&input, 0, 1, 1, Kick).unwrap(),
            format!("kit 4/4 {triplets} ; K:t8 K:t4 r:q r:h")
        );
        // Against sixteenths, quintuplets or another tuplet: no.
        let input = format!("kit 4/4 {triplets} ; K:s K:s K:s K:s r:q r:h");
        assert_eq!(started(&input, 0, 1, 1, Kick), None);
        let input = format!("kit 4/4 {triplets} ; r:qt16 r:qt16 r:qt16 r:qt16 r:qt16 r:q r:h");
        assert_eq!(started(&input, 0, 1, 1, Kick), None);
    }

    #[test]
    fn nothing_is_started_where_a_beat_starts_already() {
        let input = format!("kit 4/4 {EIGHTHS} ; r:q r:q r:q r:q");
        assert_eq!(started(&input, 0, 2, 1, Kick), None);
    }

    #[test]
    fn taking_a_note_away_tidies_its_beat() {
        let tidy = |input: &str, voice: usize, idx: usize| {
            let mut m = measure(input);
            let v = m.voice_mut(voice).unwrap();
            let instrument = v.beats()[idx].heads().iter().next().unwrap().instrument();
            v.toggle_head(idx, instrument).unwrap();
            v.tidy_beat_of(idx);
            text(&m)
        };
        assert_eq!(
            tidy(&format!("kit 4/4 {EIGHTHS} ; K:e K:e r:q r:q r:q"), 1, 1),
            format!("kit 4/4 {EIGHTHS} ; K:q r:q r:q r:q")
        );
        // The other beats keep their spelling.
        assert_eq!(
            tidy("kit 4/4 r:e r:e K:e K:e r:e r:e r:q ; r:h r:h", 0, 3),
            "kit 4/4 r:e r:e K:q r:e r:e r:q ; r:h r:h"
        );
        // A tuplet of rests is a rest; the beat after it keeps its own.
        assert_eq!(
            tidy("kit 4/4 r:t8 K:t4 r:q r:h ; r:h r:h", 0, 1),
            "kit 4/4 r:q r:q r:h ; r:h r:h"
        );
        assert_eq!(
            tidy("kit 4/4 r:t8 K:t4 S:q r:h ; r:h r:h", 0, 1),
            "kit 4/4 r:q S:q r:h ; r:h r:h"
        );
        // One whose notes lie on the plain grid is plain notes.
        assert_eq!(
            tidy("kit 4/4 K:t8 K:t4 r:q r:h ; r:h r:h", 0, 1),
            "kit 4/4 K:q r:q r:h ; r:h r:h"
        );
    }
}
