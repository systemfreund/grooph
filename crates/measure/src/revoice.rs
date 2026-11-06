//! Moving heads between the voices of a measure: "Revoice measure" and
//! "Switch voice".
//!
//! A drum hit does not sustain, so what a measure plays is where each head
//! starts ([`Measure::head_onsets`]); which voice writes it is notation.
//! [`revoice`] gives each head a voice and writes every voice again from
//! the heads it got: heads at the same tick in one voice are a chord, and
//! accent, ghost and technique stay on their head. Where a voice of the
//! measure already sounds on exactly those ticks, its beats and tuplets
//! are kept and only its heads change, so the author's spelling survives;
//! otherwise the voice is written from its onsets as
//! [`VoiceBar::respelled`] writes them. A head of a tuplet brings its whole
//! group along, with rests where the voice does not play; groups of two
//! voices over the same time meet in one of their ratio where they can. A
//! chord keeps the sticking of the beats its hand heads came from. The
//! model has no ties.

use crate::grid::DEFAULT_GRID;
use crate::onsets::HeadOnset;
use crate::{Beat, Head, Heads, Instrument, Measure, Sticking, VoiceBar, VoiceIdx, Voicing};
use std::collections::BTreeMap;
use std::ops::Range;

/// What one voice plays at a tick: its heads and the hand that plays them.
#[derive(Clone, Copy)]
struct Chord {
    heads: Heads,
    sticking: Option<Sticking>,
}

/// A tuplet group of a voice of the measure, where it sounds.
struct Group<'a> {
    ticks: Range<u32>,
    beats: &'a [Beat],
    /// Where each of its beats starts.
    onsets: Vec<u32>,
}

/// `measure` with each head in the voice `target` gives it (see the module
/// docs). The result has at least `voices` voices, more if `target` names
/// a higher one; a voice left without notes is rests. What sounds stays:
/// the same heads at the same ticks.
///
/// `None` where the heads cannot be written so: a voice would hold more
/// heads at a tick than a chord can ([`crate::head::MAX_HEADS`]), or
/// tuplets of two voices that cannot meet (triplets and quintuplets over
/// the same beat, groups that overlap in part) would come into one.
pub fn revoice(
    measure: &Measure,
    voices: usize,
    target: impl Fn(&HeadOnset) -> VoiceIdx,
) -> Option<Measure> {
    let ts = measure.time_signature();
    let onsets = measure.head_onsets();
    let count = onsets.iter().map(|o| target(o) + 1).max().unwrap_or(0).max(voices).max(1);
    let mut chords: Vec<BTreeMap<u32, Chord>> = vec![BTreeMap::new(); count];
    // The ticks of each voice where it plays a head of a tuplet.
    let mut in_tuplets: Vec<Vec<u32>> = vec![Vec::new(); count];
    for onset in &onsets {
        let to = target(onset);
        let source = &measure.voices()[onset.voice].beats()[onset.beat_idx];
        let chord =
            chords[to].entry(onset.tick).or_insert(Chord { heads: Heads::NONE, sticking: None });
        chord.heads.insert(onset.head).ok()?;
        if onset.head.instrument().is_hand() {
            chord.sticking = chord.sticking.or(source.sticking());
        }
        if source.tuplet.is_some() {
            in_tuplets[to].push(onset.tick);
        }
    }
    let groups: Vec<Group> = measure.voices().iter().flat_map(source_groups).collect();
    let groups: Vec<&Group> = groups.iter().collect();
    let mut out = Measure::new(ts);
    out.fill_voices(count);
    for (v, chords) in chords.iter().enumerate() {
        let voice = kept_spelling(measure, chords).or_else(|| {
            // Heads of a tuplet that start on the plain grid can do without
            // their group, where it does not fit the voice they go to.
            let thirty_second = DEFAULT_GRID.ticks_per_whole / 32;
            let off_grid: Vec<u32> =
                in_tuplets[v].iter().copied().filter(|t| t % thirty_second != 0).collect();
            respelled(ts, chords, &in_tuplets[v], &groups)
                .or_else(|| respelled(ts, chords, &off_grid, &groups))
        })?;
        *out.voice_mut(v).expect("voice in range") = voice;
    }
    debug_assert!(
        out.doubled_instruments().is_empty() || !measure.doubled_instruments().is_empty()
    );
    debug_assert_eq!(sound(&out), sound(measure));
    Some(out)
}

/// The heads of `measure` by tick, without their voices: what is heard.
fn sound(measure: &Measure) -> Vec<(u32, Head)> {
    let mut out: Vec<_> = measure.head_onsets().iter().map(|o| (o.tick, o.head)).collect();
    out.sort_by_key(|&(tick, head)| (tick, head.instrument()));
    out
}

/// The tuplet groups of `voice`, with where they sound.
fn source_groups(voice: &VoiceBar) -> Vec<Group<'_>> {
    let ticks = DEFAULT_GRID.compute_onset_ticks(voice.beats());
    voice
        .tuplet_groups()
        .into_iter()
        .map(|g| {
            let start = ticks[g.start_idx];
            Group {
                ticks: start..start + g.span_ticks(),
                beats: &voice.beats()[g.start_idx..=g.end_idx],
                onsets: ticks[g.start_idx..=g.end_idx].to_vec(),
            }
        })
        .collect()
}

/// `beat` playing `chord` (or a rest, without one), its duration and tuplet
/// mark kept.
fn playing(mut beat: Beat, chord: Option<&Chord>) -> Beat {
    beat.set_heads(chord.map_or(Heads::NONE, |c| c.heads));
    let sticking = chord.and_then(|c| c.sticking).filter(|_| beat.takes_sticking());
    beat.set_sticking(sticking).expect("a hand plays the beat");
    beat
}

/// A voice of `measure` whose notes start on exactly the ticks of `chords`,
/// playing them instead of its own heads.
fn kept_spelling(measure: &Measure, chords: &BTreeMap<u32, Chord>) -> Option<VoiceBar> {
    measure.voices().iter().find_map(|voice| {
        let ticks = DEFAULT_GRID.compute_onset_ticks(voice.beats());
        let notes = voice.beats().iter().zip(&ticks).filter(|(b, _)| b.is_note());
        if !notes.map(|(_, t)| t).eq(chords.keys()) {
            return None;
        }
        let mut out = voice.clone();
        for (beat, tick) in out.beats.iter_mut().zip(&ticks) {
            *beat = playing(*beat, chords.get(tick));
        }
        Some(out)
    })
}

/// Plain values a voice is first written in before it is respelled, long to
/// short. A whole rest only fits some meters; halves do in all.
const FILL_VALUES: [crate::duration::NoteValue; 5] = {
    use crate::duration::NoteValue::*;
    [Half, Quarter, Eighth, Sixteenth, ThirtySecond]
};

/// A voice of `ts` playing `chords`, written from its onsets and
/// [respelled](VoiceBar::respelled). Each tick of `in_tuplets` (a head
/// that came from a tuplet) is written in a tuplet group over the ticks of
/// one of `groups`, the longest that works: the group as written where its
/// beats start on every tick of `chords` it spans, else a group of its
/// ratio on the places those ticks need ([`merged_group`]). The group
/// plays the chords at its beats and rests at the others. `None` where no
/// group works or two of them overlap.
fn respelled(
    ts: crate::TimeSignature,
    chords: &BTreeMap<u32, Chord>,
    in_tuplets: &[u32],
    groups: &[&Group],
) -> Option<VoiceBar> {
    let written = |g: &Group| -> Option<Vec<Beat>> {
        let inside = chords.range(g.ticks.clone());
        inside.clone().all(|(t, _)| g.onsets.contains(t)).then(|| {
            g.beats.iter().zip(&g.onsets).map(|(b, t)| playing(*b, chords.get(t))).collect()
        })
    };
    // The groups as written where they can be, else in a group over a
    // longer span that meets its neighbours: a triplet of sixteenths of one
    // voice inside a triplet of quarters of the other.
    let choose = |as_written: bool| -> Option<Vec<(Range<u32>, Vec<Beat>)>> {
        let mut kept: Vec<(Range<u32>, Vec<Beat>)> = Vec::new();
        for tick in in_tuplets {
            if kept.iter().any(|(k, _)| k.contains(tick)) {
                continue;
            }
            let mut around: Vec<&&Group> =
                groups.iter().filter(|g| g.ticks.contains(tick)).collect();
            around.sort_by_key(|g| std::cmp::Reverse(g.ticks.len()));
            let either = |g: &&&Group| written(g).or_else(|| merged_group(g, chords));
            let (group, beats) = match as_written {
                true => (around.iter().find_map(|g| Some((g, written(g)?))))
                    .or_else(|| around.iter().find_map(|g| Some((g, merged_group(g, chords)?)))),
                false => around.iter().find_map(|g| Some((g, either(g)?))),
            }?;
            let ticks = group.ticks.clone();
            if kept.iter().any(|(k, _)| k.start < ticks.end && ticks.start < k.end) {
                return None;
            }
            kept.push((ticks, beats));
        }
        Some(kept)
    };
    let mut kept = choose(true).or_else(|| choose(false))?;
    kept.sort_by_key(|(k, _)| k.start);
    // Plain notes outside the groups, and the groups, in order.
    enum Event<'a> {
        Note(Chord),
        Group(&'a Range<u32>, &'a [Beat]),
    }
    let mut events: Vec<(u32, Event)> = chords
        .iter()
        .filter(|(t, _)| !kept.iter().any(|(k, _)| k.contains(t)))
        .map(|(&t, &c)| (t, Event::Note(c)))
        .collect();
    events.extend(kept.iter().map(|(k, beats)| (k.start, Event::Group(k, beats))));
    events.sort_by_key(|&(t, _)| t);

    let mut beats = Vec::new();
    let mut at = 0;
    let mut pending: Option<Chord> = None;
    for (tick, event) in events {
        fill(&mut beats, at..tick, pending.take())?;
        match event {
            Event::Note(chord) => {
                pending = Some(chord);
                at = tick;
            }
            Event::Group(ticks, group) => {
                beats.extend_from_slice(group);
                at = ticks.end;
            }
        }
    }
    fill(&mut beats, at..DEFAULT_GRID.ticks_per_measure(&ts), pending)?;
    let draft = VoiceBar::from_parts(beats, ts);
    Some(draft.respelled().unwrap_or(draft))
}

/// A tuplet group over the ticks of `group`, in its ratio, playing the
/// `chords` in them: its places are the longest value of the ratio that
/// every chord's tick and every beat of `group` starts on, one a value
/// button can write ([`VALUE_BUTTONS`](crate::editing::VALUE_BUTTONS)).
/// Two voices' groups over the same beat, spelled differently, meet so.
fn merged_group(group: &Group, chords: &BTreeMap<u32, Chord>) -> Option<Vec<Beat>> {
    use crate::duration::{Duration, TupletSpec};
    let TupletSpec { n, m, .. } = group.beats[0].duration.as_tuplet_spec()?;
    let start = group.ticks.start;
    let len = group.ticks.len() as u32;
    let ticks = chords.range(group.ticks.clone()).map(|(&t, _)| t).chain(group.onsets.clone());
    let step = ticks.fold(len, |g, t| crate::math::gcd(g, t - start));
    let duration = crate::editing::VALUE_BUTTONS.into_iter().find_map(|base| {
        let d = Duration::Tuplet(TupletSpec { n, m, base });
        (DEFAULT_GRID.supports(&d) && DEFAULT_GRID.ticks_of(&d) == Some(step)).then_some(d)
    })?;
    let mut beats: Vec<Beat> = (0..len / step)
        .map(|i| playing(Beat::rest(duration), chords.get(&(start + i * step))))
        .collect();
    crate::tuplet::mark_group(&mut beats, crate::tuplet::base_spanning(len, m)?);
    Some(beats)
}

/// Fill `ticks` with plain beats, `chord` on the first, rests after it:
/// each the longest value that fits and starts on a multiple of its
/// length. `None` where the ticks are off the thirty-second grid.
fn fill(beats: &mut Vec<Beat>, ticks: Range<u32>, mut chord: Option<Chord>) -> Option<()> {
    let mut at = ticks.start;
    while at < ticks.end {
        let (value, len) = FILL_VALUES.into_iter().find_map(|v| {
            let len = DEFAULT_GRID.ticks_of(&crate::duration::Duration::Simple(v))?;
            (len <= ticks.end - at && at.is_multiple_of(len)).then_some((v, len))
        })?;
        let beat = Beat::rest(crate::duration::Duration::Simple(value));
        beats.push(playing(beat, chord.take().as_ref()));
        at += len;
    }
    // A note needs room to start in.
    chord.is_none().then_some(())
}

impl Measure {
    /// The measure with every head in the voice `voicing` writes its
    /// instrument in, with as many voices as it has (see [`revoice`]).
    pub fn revoiced(&self, voicing: &Voicing) -> Option<Measure> {
        revoice(self, voicing.voices(), |o| voicing.voice_of(o.head.instrument()))
    }

    /// The measure with the heads of `instrument` in the other voice: the
    /// upper one if it is in the lower at `tick` (or, without a head there,
    /// first in the measure), the lower one otherwise. Only the head at
    /// `tick` with `only_at_tick`, else every one in the measure. A lower
    /// voice left without notes stays as rests, unless `voices` is 1.
    /// `None` if the measure has no such head, or as [`revoice`].
    pub fn with_voice_switched(
        &self,
        instrument: Instrument,
        tick: u32,
        only_at_tick: bool,
        voices: usize,
    ) -> Option<Measure> {
        let onsets = self.head_onsets();
        let of = |o: &&HeadOnset| o.head.instrument() == instrument;
        let from = onsets
            .iter()
            .filter(of)
            .find(|o| o.tick == tick)
            .or_else(|| onsets.iter().filter(of).find(|_| !only_at_tick))?
            .voice;
        let to = usize::from(from == 0);
        let keep = self.voices().len().min(voices).max(1);
        revoice(self, keep, |o| {
            let moves = o.head.instrument() == instrument && (!only_at_tick || o.tick == tick);
            if moves { to } else { o.voice }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notation::{format_score, parse_score};
    use crate::{Instrument, Score};

    fn measure(s: &str) -> Measure {
        parse_score(s).unwrap().into_measures().remove(0)
    }

    fn text(m: &Measure) -> String {
        format_score(&Score::new(vec![m.clone()])).unwrap()
    }

    // The styles of #301.
    const HANDS_FEET: &str = "4/4 H:e H:e S-H:e H:e H:e H:e S-H:e H:e ; K:q r:q K:e K:e r:q";
    const ONE_VOICE: &str = "4/4 K-H:e H:e S-H:e H:e K-H:e K-H:e S-H:e H:e";
    const CYMBALS_ABOVE: &str = "4/4 H:e H:e H:e H:e H:e H:e H:e H:e ; K:q S:q K:e K:e S:q";

    fn revoiced(s: &str, voicing: Voicing) -> String {
        text(&measure(s).revoiced(&voicing).expect("revoiced"))
    }

    #[test]
    fn each_preset_writes_its_style() {
        for from in [HANDS_FEET, ONE_VOICE, CYMBALS_ABOVE] {
            assert_eq!(revoiced(from, Voicing::HANDS_FEET), HANDS_FEET, "from {from}");
            assert_eq!(revoiced(from, Voicing::ONE_VOICE), ONE_VOICE, "from {from}");
            assert_eq!(revoiced(from, Voicing::CYMBALS_ABOVE), CYMBALS_ABOVE, "from {from}");
        }
    }

    #[test]
    fn accents_ghosts_techniques_and_stickings_stay_on_their_heads() {
        let m = measure("4/4 H:e@R H:e >S-Ho:e H:e H:e H:e (S)-H:e H:e ; K:q r:q K:e K:e r:q");
        let one = m.revoiced(&Voicing::ONE_VOICE).unwrap();
        assert_eq!(text(&one), "4/4 K-H:e@R H:e >S-Ho:e H:e K-H:e K-H:e (S)-H:e H:e");
        assert_eq!(text(&one.revoiced(&Voicing::HANDS_FEET).unwrap()), text(&m));
    }

    #[test]
    fn a_voice_on_the_same_ticks_keeps_its_spelling() {
        // The kick plays where the hi-hat does, written as the author did.
        let m = measure("4/4 H:q. H:e r:h ; K:e r:q K:e r:h");
        let one = m.revoiced(&Voicing::ONE_VOICE).unwrap();
        assert_eq!(text(&one), "4/4 K-H:q. K-H:e r:h");
    }

    #[test]
    fn a_tuplet_comes_along_whole() {
        let m = measure("4/4 H:t8 H:t8 H:t8 H:q H:h ; K:t8 r:t8 K:t8 r:q K:h");
        let one = m.revoiced(&Voicing::ONE_VOICE).unwrap();
        assert_eq!(text(&one), "4/4 K-H:t8 H:t8 K-H:t8 H:q K-H:h");
        // The kick's triplet moved up takes its group along, with the
        // hi-hat on its first note; the rest is written from its onsets.
        let m = measure("4/4 H:q H:q H:h ; K:t8 r:t8 K:t8 r:q r:h");
        let one = m.revoiced(&Voicing::ONE_VOICE).unwrap();
        assert_eq!(text(&one), "4/4 [K-H:t4 K:t8] H:q H:q r:q");
    }

    #[test]
    fn tuplets_that_cannot_meet_are_refused() {
        // Triplets above, a quintuplet below: no voice can hold both.
        let m = measure("4/4 H:t8 H:t8 H:t8 r:q r:h ; K:qt16 K:qt16 K:qt16 K:qt16 K:qt16 r:q r:h");
        assert_eq!(m.revoiced(&Voicing::ONE_VOICE), None);
    }

    #[test]
    fn an_empty_lower_voice_goes_with_one_voice_and_stays_otherwise() {
        let m = measure("4/4 H:q H:q H:q H:q ; r:h r:h");
        assert_eq!(m.revoiced(&Voicing::ONE_VOICE).unwrap().voices().len(), 1);
        let m = measure("4/4 H:q H:q H:q H:q");
        assert_eq!(
            text(&m.revoiced(&Voicing::HANDS_FEET).unwrap()),
            "4/4 H:q H:q H:q H:q ; r:h r:h"
        );
    }

    #[test]
    fn a_switched_instrument_goes_to_the_other_voice() {
        let m = measure(HANDS_FEET);
        // Every snare down, then back up.
        let down = m.with_voice_switched(Instrument::Snare, 0, false, 2).unwrap();
        assert_eq!(text(&down), "4/4 H:e H:e H:e H:e H:e H:e H:e H:e ; K:q S:q K:e K:e S:q");
        let up = down.with_voice_switched(Instrument::Snare, 0, false, 2).unwrap();
        assert_eq!(text(&up), HANDS_FEET);
        // Only the snare on beat 2.
        let tick = DEFAULT_GRID.ticks_per_beat(&m.time_signature());
        let one = m.with_voice_switched(Instrument::Snare, tick, true, 2).unwrap();
        assert_eq!(text(&one), "4/4 H:e H:e H:e H:e H:e H:e S-H:e H:e ; K:q S:q K:e K:e r:q");
        // No snare there: nothing to switch.
        assert_eq!(m.with_voice_switched(Instrument::Snare, 0, true, 2), None);
        // The kick up, in a score of one voice: the lower voice goes.
        let up = m.with_voice_switched(Instrument::Kick, 0, false, 1).unwrap();
        assert_eq!(text(&up), ONE_VOICE);
        let up = m.with_voice_switched(Instrument::Kick, 0, false, 2).unwrap();
        assert_eq!(text(&up), format!("{ONE_VOICE} ; r:h r:h"));
    }
}
