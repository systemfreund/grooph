//! Measures and their voices.
//!
//! A [`Measure`] holds its voices ([`VoiceBar`]), all in its meter. Each
//! voice keeps the meter for itself, as it is filled, edited and laid out
//! on its own; only this module sets it, through the measure, so the voices
//! of a measure never part.

use crate::BeatKind::Rest;
use crate::duration::Duration;
use crate::editing::GroupSpan;
use crate::editing::Modification;
use crate::grid::DEFAULT_GRID;
use crate::{Beat, BeatKind, InvalidMeasure, MeasureError, TimeSignature, TupletRegistry};
use either::Either;
use serde::{Deserialize, Serialize};
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
/// A measure reads and edits as its first voice (it dereferences to it), so
/// code for one voice needs no change; [`Measure::voices`] and
/// [`Measure::voice_mut`] reach the others.
#[derive(Clone, Deserialize)]
#[serde(try_from = "RawMeasure")]
pub struct Measure {
    /// Never empty; every voice in the same meter.
    voices: Vec<VoiceBar>,
}

/// One voice of a [`Measure`]: a sequence of beats filling its meter.
///
/// Invariant: the beats have supported durations and fill the measure
/// exactly, every beat in a tuplet group has an anchor, and the beats of
/// every group of two or more fill its anchor's span.
/// The constructors and editing methods keep it; deserializing checks it
/// (see [`InvalidMeasure`]).
#[derive(Clone, Serialize)]
pub struct VoiceBar {
    pub(crate) beats: Vec<Beat>,
    /// The meter of its measure, set by the measure only (this module).
    #[serde(skip)]
    time_signature: TimeSignature,
    /// Tuplet group anchors + id allocator.
    #[serde(flatten)]
    pub(crate) tuplets: TupletRegistry,
}

/// A [`Measure`] as saved, before its invariant is checked: with its
/// voices, or as before them with its one voice's beats and tuplets inline.
#[derive(Deserialize)]
struct RawMeasure {
    time_signature: TimeSignature,
    #[serde(default, deserialize_with = "present")]
    voices: Option<Vec<RawVoice>>,
    #[serde(default, deserialize_with = "present")]
    beats: Option<Vec<Beat>>,
    #[serde(flatten)]
    tuplets: TupletRegistry,
}

/// A [`VoiceBar`] as saved.
#[derive(Deserialize)]
struct RawVoice {
    beats: Vec<Beat>,
    #[serde(flatten)]
    tuplets: TupletRegistry,
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
        let RawMeasure { time_signature, voices, beats, tuplets } = raw;
        let voices = match (voices, beats) {
            (Some(voices), _) if !voices.is_empty() => voices,
            (_, Some(beats)) => vec![RawVoice { beats, tuplets }],
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

impl VoiceBar {
    /// The voice saved as `raw`, in `time_signature`, if it holds.
    fn checked(raw: RawVoice, time_signature: TimeSignature) -> Result<Self, InvalidMeasure> {
        let RawVoice { beats, mut tuplets } = raw;
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
            if let Some(id) = beat.tuplet_group_id
                && tuplets.get(id).is_none()
            {
                return Err(InvalidMeasure::UnknownTupletGroup { idx, id });
            }
        }
        let expected = DEFAULT_GRID.ticks_per_measure(&time_signature);
        if ticks != u64::from(expected) {
            return Err(InvalidMeasure::WrongLength { ticks, expected });
        }
        // Anchors saved at or above the next id would be overwritten.
        tuplets.reserve_anchor_ids();
        let measure = VoiceBar { beats, time_signature, tuplets };
        // Like everywhere else, a lone beat with a group id counts as no group
        // (see `tuplet_group_at`): nothing reads its anchor's span.
        for GroupSpan { start_idx, end_idx, id } in measure.tuplet_groups() {
            let ticks: u64 = measure.beats[start_idx..=end_idx]
                .iter()
                .filter_map(|b| DEFAULT_GRID.ticks_of(&b.duration))
                .map(u64::from)
                .sum();
            let expected = measure.tuplets.get(id).map_or(0, |a| a.target_ticks);
            if ticks != u64::from(expected) {
                return Err(InvalidMeasure::TupletGroupLength { id, ticks, expected });
            }
        }
        Ok(measure)
    }
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
        let mut s = Self { beats: Vec::new(), time_signature, tuplets: TupletRegistry::new() };
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
                if let Some(GroupSpan { start_idx, end_idx, id: group_id }) =
                    self.tuplet_group_at(idx)
                {
                    // Sum ticks for the span and remove it
                    let span_ticks: u32 = (start_idx..=end_idx)
                        .map(|k| DEFAULT_GRID.ticks_of(&self.beats[k].duration).unwrap_or(0))
                        .sum();
                    // Remove anchor entry if present
                    self.tuplets.unregister(group_id);
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
