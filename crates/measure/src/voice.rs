//! Measures and their voices.
//!
//! A [`Measure`] holds its voices ([`VoiceBar`]), all in its meter. Each
//! voice keeps the meter for itself, as it is filled, edited and laid out
//! on its own; only this module sets it, through the measure, so the voices
//! of a measure never part.

use crate::BeatKind::Rest;
use crate::beat::StoredBeat;
use crate::duration::{Duration, NoteValue};
use crate::editing::Modification;
use crate::grid::DEFAULT_GRID;
use crate::tuplet::{TupletGroup, TupletMark};
use crate::{Beat, BeatKind, Instrument, InvalidMeasure, MeasureError, TimeSignature, Voicing};
use either::Either;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};

/// Index of a voice in its [`Measure`]: 0 is the first, the upper one.
pub type VoiceIdx = usize;

/// A measure: its meter and its voices, played together.
///
/// A voice is one rhythm through the measure, written with its own stems:
/// voice 0 with the stems up, voice 1 below it with the stems down (hands
/// and feet of a drum part, or a click bar and its counter rhythm). Every
/// measure has at least one voice; all share its time signature.
///
/// Code names the voice it reads or edits: [`Measure::first_voice`] is the
/// upper one (the only one of a click bar or a line rhythm),
/// [`Measure::voices`] and [`Measure::voice_mut`] reach the others.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "RawMeasure")]
pub struct Measure {
    /// Never empty; every voice in the same meter.
    voices: Vec<VoiceBar>,
}

/// One voice of a [`Measure`]: a sequence of beats filling its meter.
///
/// Invariant: the beats have supported durations and fill the measure
/// exactly; a beat is in a tuplet group exactly if its duration is a tuplet;
/// every group is two or more beats of its first beat's ratio that fill the
/// group's span (see [`crate::tuplet`]).
/// The constructors and editing methods keep it; deserializing checks it
/// (see [`InvalidMeasure`]).
#[derive(Clone, PartialEq, Eq)]
pub struct VoiceBar {
    pub(crate) beats: Vec<Beat>,
    /// The meter of its measure, set by the measure only (this module);
    /// saved with the measure.
    time_signature: TimeSignature,
}

/// A [`Measure`] as saved, before its invariant is checked: with its
/// voices, or as before them with its one voice's beats and tuplets inline.
#[derive(Deserialize)]
struct RawMeasure {
    time_signature: TimeSignature,
    #[serde(default, deserialize_with = "present")]
    voices: Option<Vec<RawVoice>>,
    #[serde(default, deserialize_with = "present")]
    beats: Option<Vec<StoredBeat>>,
    #[serde(flatten)]
    anchors: OldAnchors,
}

/// A [`VoiceBar`] as saved.
#[derive(Deserialize)]
struct RawVoice {
    beats: Vec<StoredBeat>,
    #[serde(flatten)]
    anchors: OldAnchors,
}

/// The tuplet groups of a voice saved before [`TupletMark`]s: its beats
/// name their group by an id, and this table the group's base by the id.
#[derive(Deserialize, Default)]
#[serde(default)]
struct OldAnchors {
    tuplet_anchors: HashMap<u32, OldAnchor>,
}

/// A group of [`OldAnchors`]; of its other fields only the base counts,
/// the rest follows from the beats.
#[derive(Deserialize)]
struct OldAnchor {
    base_hint: NoteValue,
}

/// A field that is there, as `Some` (a missing one is `None` by default).
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

impl TryFrom<RawMeasure> for Measure {
    type Error = InvalidMeasure;

    fn try_from(raw: RawMeasure) -> Result<Self, Self::Error> {
        let RawMeasure { time_signature, voices, beats, anchors } = raw;
        let voices = match (voices, beats) {
            (Some(voices), _) if !voices.is_empty() => voices,
            (_, Some(beats)) => vec![RawVoice { beats, anchors }],
            _ => return Err(InvalidMeasure::NoVoice),
        };
        let voices = voices
            .into_iter()
            .map(|voice| VoiceBar::checked(voice, time_signature))
            .collect::<Result<_, _>>()?;
        Ok(Measure { voices })
    }
}

impl Serialize for Measure {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // A map, like the measures saved before voices.
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("time_signature", &self.time_signature())?;
        map.serialize_entry("voices", &self.voices)?;
        map.end()
    }
}

impl Serialize for VoiceBar {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // A map, like the voices saved with their tuplet anchors.
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("beats", &self.beats)?;
        map.end()
    }
}

impl VoiceBar {
    /// The voice saved as `raw`, in `time_signature`, if it holds.
    fn checked(raw: RawVoice, time_signature: TimeSignature) -> Result<Self, InvalidMeasure> {
        let beats = marked_beats(raw)?;
        let mut ticks = 0u64;
        for (idx, beat) in beats.iter().enumerate() {
            // `supports` first: `ticks_of` overflows on durations like a
            // note with forty dots.
            let beat_ticks = DEFAULT_GRID
                .supports(&beat.duration)
                .then(|| DEFAULT_GRID.ticks_of(&beat.duration))
                .flatten()
                .ok_or(InvalidMeasure::UnsupportedDuration { idx })?;
            ticks += u64::from(beat_ticks);
        }
        let expected = DEFAULT_GRID.ticks_per_measure(&time_signature);
        if ticks != u64::from(expected) {
            return Err(InvalidMeasure::WrongLength { ticks, expected });
        }
        check_tuplet_groups(&beats)?;
        Ok(VoiceBar { beats, time_signature })
    }
}

/// The beats of `raw` with their tuplet marks: as saved, or made from the
/// group ids and anchors of a voice saved before marks. A run of beats with
/// one id is a group (a lone beat is none, as it was then).
fn marked_beats(raw: RawVoice) -> Result<Vec<Beat>, InvalidMeasure> {
    let RawVoice { beats: stored, anchors } = raw;
    let mut beats: Vec<Beat> = stored.iter().map(|s| s.beat).collect();
    let mut idx = 0;
    while idx < stored.len() {
        let Some(id) = stored[idx].group_id else {
            idx += 1;
            continue;
        };
        let anchor = anchors
            .tuplet_anchors
            .get(&id)
            .ok_or(InvalidMeasure::UnknownTupletGroup { idx, id })?;
        let len = stored[idx..].iter().take_while(|s| s.group_id == Some(id)).count();
        if len >= 2 {
            beats[idx].tuplet = Some(TupletMark::Start { base: anchor.base_hint });
            for beat in &mut beats[idx + 1..idx + len] {
                beat.tuplet = Some(TupletMark::Continue);
            }
        }
        idx += len;
    }
    Ok(beats)
}

/// Whether the tuplet marks of `beats` make whole groups: every tuplet in a
/// group, every group two or more beats of one ratio that fill its span,
/// and no mark outside a group.
fn check_tuplet_groups(beats: &[Beat]) -> Result<(), InvalidMeasure> {
    let mut idx = 0;
    while idx < beats.len() {
        let beat = &beats[idx];
        match (beat.tuplet, beat.duration.as_tuplet_spec()) {
            (None, None) => {
                idx += 1;
                continue;
            }
            (Some(TupletMark::Start { .. }), Some(_)) => {}
            _ => return Err(InvalidMeasure::OutsideTupletGroup { idx }),
        }
        let group = TupletGroup::starting_at(beats, idx).expect("a group starts here");
        let members = &beats[group.start_idx..=group.end_idx];
        if let Some(off) = members.iter().position(|b| {
            !b.duration.as_tuplet_spec().is_some_and(|spec| spec.same_grid(&group.spec))
        }) {
            return Err(InvalidMeasure::OutsideTupletGroup { idx: idx + off });
        }
        let ticks: u64 =
            members.iter().filter_map(|b| DEFAULT_GRID.ticks_of(&b.duration)).map(u64::from).sum();
        let expected = group.span_ticks();
        if members.len() < 2 || ticks != u64::from(expected) {
            return Err(InvalidMeasure::TupletGroupLength { idx, ticks, expected });
        }
        idx = group.end_idx + 1;
    }
    Ok(())
}

impl Measure {
    /// A measure of one voice of rests in `time_signature`.
    pub fn new(time_signature: TimeSignature) -> Self {
        Self::new_init(time_signature, Rest)
    }

    /// A measure of one voice, a note or a rest on every beat unit.
    pub fn new_init(time_signature: TimeSignature, init: BeatKind) -> Self {
        Measure { voices: vec![VoiceBar::new_init(time_signature, init)] }
    }

    /// The voices, the first one upper.
    pub fn voices(&self) -> &[VoiceBar] {
        &self.voices
    }

    /// The measure's meter, the same in every voice.
    pub fn time_signature(&self) -> TimeSignature {
        self.voices[0].time_signature()
    }

    /// The first voice: the upper one, and the only one of a click bar or
    /// a rhythm on one line.
    pub fn first_voice(&self) -> &VoiceBar {
        &self.voices[0]
    }

    /// The first voice, to edit; see [`Measure::first_voice`].
    pub fn first_voice_mut(&mut self) -> &mut VoiceBar {
        &mut self.voices[0]
    }

    pub fn voice(&self, voice: VoiceIdx) -> Option<&VoiceBar> {
        self.voices.get(voice)
    }

    pub fn voice_mut(&mut self, voice: VoiceIdx) -> Option<&mut VoiceBar> {
        self.voices.get_mut(voice)
    }

    /// Voice `voice` alone, as a measure of its own, if there is one.
    pub fn single_voice(&self, voice: VoiceIdx) -> Option<Measure> {
        self.voices.get(voice).map(|v| Measure { voices: vec![v.clone()] })
    }

    /// Voice `voice` all `init` again, a note or a rest on every beat unit;
    /// the other voices stay. No change if the measure has no such voice.
    pub fn clear_voice(&mut self, voice: VoiceIdx, init: BeatKind) {
        let ts = self.time_signature();
        if let Some(v) = self.voices.get_mut(voice) {
            *v = VoiceBar::new_init(ts, init);
        }
    }

    /// A measure of rests in `time_signature`, as many voices as this one.
    pub fn empty_like(&self, time_signature: TimeSignature) -> Self {
        Measure { voices: vec![VoiceBar::new_init(time_signature, Rest); self.voices.len()] }
    }

    /// The voice a new head of `instrument` goes to in this measure when it
    /// is not on the beat already: the voice that has the instrument
    /// somewhere in the measure; a measure of one voice that holds notes
    /// stays so (voice 0, a fill made on purpose in a two-voice score);
    /// otherwise the voice `voicing` writes the instrument in.
    pub fn voice_for(&self, instrument: Instrument, voicing: &Voicing) -> VoiceIdx {
        let has = |voice: &VoiceBar, i: Instrument| {
            voice.beats().iter().any(|b| b.heads().get(i).is_some())
        };
        if let Some(v) = self.voices.iter().position(|voice| has(voice, instrument)) {
            return v;
        }
        let holds_notes = Instrument::ALL.into_iter().any(|i| has(&self.voices[0], i));
        if self.voices.len() == 1 && holds_notes {
            return 0;
        }
        voicing.voice_of(instrument)
    }

    /// Add voices of rests below this measure's until it has `count`.
    pub fn fill_voices(&mut self, count: usize) {
        let ts = self.time_signature();
        while self.voices.len() < count {
            self.voices.push(VoiceBar::new_init(ts, Rest));
        }
    }

    /// The instruments of the kit that two voices strike at the same time,
    /// with the tick: a drum is played once at a time, by one voice. The
    /// one line's rhythm is left out, as the metronome's click and counter
    /// rhythm sound together as two rhythms.
    pub fn doubled_instruments(&self) -> Vec<(u32, Instrument)> {
        let mut struck: Vec<(u32, Instrument, usize)> = Vec::new();
        let mut doubled = Vec::new();
        for (v, voice) in self.voices.iter().enumerate() {
            let onsets = DEFAULT_GRID.compute_onset_ticks(voice.beats());
            for (beat, tick) in voice.beats().iter().zip(onsets) {
                for head in beat.heads().iter().filter(|h| h.instrument() != Instrument::Rhythm) {
                    let instrument = head.instrument();
                    if struck.iter().any(|&(t, i, w)| t == tick && i == instrument && w != v) {
                        doubled.push((tick, instrument));
                    }
                    struck.push((tick, instrument, v));
                }
            }
        }
        doubled
    }

    /// Strike each drum of the kit by one voice at a time again (see
    /// [`Measure::doubled_instruments`]), for a score made elsewhere, such
    /// as an old link: a head struck by two voices at once stays in the
    /// voice `voicing` writes its instrument in,
    /// or the uppermost, and leaves the others, its accent going along. A
    /// beat left without heads is a rest. Reports whether one was doubled.
    pub fn undouble_instruments(&mut self, voicing: &Voicing) -> bool {
        let doubled = self.doubled_instruments();
        for &(tick, instrument) in &doubled {
            let hits: Vec<(VoiceIdx, usize)> = (self.voices.iter().enumerate())
                .filter_map(|(v, voice)| {
                    let onsets = DEFAULT_GRID.compute_onset_ticks(voice.beats());
                    let mut beats = voice.beats().iter().zip(onsets);
                    let at = beats.position(|(beat, t)| {
                        t == tick && beat.heads().get(instrument).is_some()
                    })?;
                    Some((v, at))
                })
                .collect();
            let Some(&keep) =
                hits.iter().find(|&&(v, _)| v == voicing.voice_of(instrument)).or(hits.first())
            else {
                continue;
            };
            let mut accented = false;
            for &(v, at) in hits.iter().filter(|&&hit| hit != keep) {
                let beat = &mut self.voices[v].beats[at];
                let mut heads = *beat.heads();
                accented |= heads.remove(instrument).is_some_and(|h| h.accented());
                beat.set_heads(heads);
            }
            if accented && let Some(head) = self.voices[keep.0].beats[keep.1].head_mut(instrument) {
                head.set_accented(true);
            }
        }
        !doubled.is_empty()
    }

    /// Add copies of the voices of `other` below this measure's. Fails, and
    /// adds none, if `other` is in another meter.
    pub fn push_voices(&mut self, other: &Measure) -> Result<(), DifferentMeter> {
        if other.time_signature() != self.time_signature() {
            return Err(DifferentMeter);
        }
        self.voices.extend_from_slice(&other.voices);
        Ok(())
    }

    /// Change the time signature of every voice (see
    /// [`VoiceBar::refit`]); if one cannot change, none does. Reports the
    /// change of the measure as a whole.
    pub fn set_time_signature(
        &mut self,
        new_ts: TimeSignature,
    ) -> Result<Modification, MeasureError> {
        let old_ts = self.time_signature();
        let mut voices = self.voices.clone();
        for voice in &mut voices {
            voice.refit(new_ts)?;
        }
        self.voices = voices;
        Ok(Modification::ChangeTimeSignature(old_ts, new_ts))
    }
}

/// A voice in another meter than the measure's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DifferentMeter;

impl std::fmt::Display for DifferentMeter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("a voice in another meter than its measure")
    }
}

impl std::error::Error for DifferentMeter {}

impl VoiceBar {
    fn new_init(time_signature: TimeSignature, init: BeatKind) -> Self {
        let mut s = Self { beats: Vec::new(), time_signature };
        s.fill_measure(init, &[Duration::Simple(time_signature.beat_note_value().unwrap())]);
        s
    }

    pub fn time_signature(&self) -> TimeSignature {
        self.time_signature
    }

    /// Change the voice's time signature using the default policy:
    /// - If the new measure is shorter: remove whole beats from the end; if a tuplet is touched,
    ///   remove the entire tuplet group (no splitting).
    /// - If the new measure is longer (or space remains after removals): pad with rests using the
    ///   new signature's primary beat unit.
    ///
    /// Only through [`Measure::set_time_signature`], which keeps its voices
    /// in one meter.
    fn refit(&mut self, new_ts: TimeSignature) -> Result<(), MeasureError> {
        if self.time_signature == new_ts {
            return Ok(());
        }

        // Compute current used ticks and target capacity in ticks
        let mut used: u32 =
            self.beats.iter().map(|b| DEFAULT_GRID.ticks_of(&b.duration).unwrap_or(0)).sum();
        let new_max = DEFAULT_GRID.ticks_per_measure(&new_ts);

        // If we need to shrink: remove from the tail, whole beats or entire tuplet groups
        if used > new_max {
            let mut i: isize = self.beats.len() as isize - 1;
            while i >= 0 && used > new_max {
                let idx = i as usize;
                let beat = self.beats[idx];
                if let Some(TupletGroup { start_idx, end_idx, .. }) = self.tuplet_group_at(idx) {
                    // Sum ticks for the span and remove it
                    let span_ticks: u32 = (start_idx..=end_idx)
                        .map(|k| DEFAULT_GRID.ticks_of(&self.beats[k].duration).unwrap_or(0))
                        .sum();
                    self.beats.drain(start_idx..=end_idx);
                    used = used.saturating_sub(span_ticks);
                    i = start_idx as isize - 1;
                } else {
                    let removed = DEFAULT_GRID.ticks_of(&beat.duration).unwrap_or(0);
                    self.beats.remove(idx);
                    used = used.saturating_sub(removed);
                    i -= 1;
                }
            }
        }

        // Apply the new time signature
        self.time_signature = new_ts;

        // If we have remaining space, pad with rests
        if used < new_max {
            let remaining = new_max - used;
            let insert_at = self.beats.len();
            self.fill_at(insert_at, remaining, &[], Either::Right(Rest))?;
        }

        Ok(())
    }
}

impl Debug for Measure {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for (i, voice) in self.voices.iter().enumerate() {
            if i > 0 {
                write!(f, " ; ")?;
            }
            voice.fmt(f)?;
        }
        Ok(())
    }
}
