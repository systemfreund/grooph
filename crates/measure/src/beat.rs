use crate::beat::BeatKind::{Note, Rest};
use crate::duration::Duration;
use crate::head::{Head, Heads, Instrument};
use crate::sticking::Sticking;
use crate::tuplet::TupletMark;
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Formatter};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BeatKind {
    Note,
    Rest,
}

/// A slot of the rhythm, and what is played in it.
///
/// The slot is the duration, maybe in a tuplet group (see [`TupletMark`]):
/// what the measure's rules keep filled. What is played are the [`Heads`]: none for a rest,
/// one for a note, several for a chord (see [`crate::head`]). A beat with a
/// hand instrument may carry a [`Sticking`].
#[derive(Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawBeat")]
pub struct Beat {
    pub duration: Duration,
    heads: Heads,
    /// Its place in a tuplet group. [`crate::VoiceBar::set_beat`] sets it
    /// from the slot; a beat made elsewhere has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tuplet: Option<TupletMark>,
    /// Which hand plays it: only on a beat with a hand instrument
    /// ([`Beat::takes_sticking`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    sticking: Option<Sticking>,
}

impl Beat {
    /// A note of the rhythm's one instrument ([`Instrument::Rhythm`]).
    pub fn note(duration: Duration) -> Self {
        Self::note_of(duration, Instrument::Rhythm)
    }

    /// A note of one plain head of `instrument`.
    pub fn note_of(duration: Duration, instrument: Instrument) -> Self {
        Self::with_heads(duration, Heads::one(Head::new(instrument)))
    }

    /// A rest.
    pub fn rest(duration: Duration) -> Self {
        Self::with_heads(duration, Heads::NONE)
    }

    /// A plain [`Beat::note`] or a [`Beat::rest`].
    pub fn new(duration: Duration, kind: BeatKind) -> Self {
        match kind {
            Note => Self::note(duration),
            Rest => Self::rest(duration),
        }
    }

    /// A beat playing `heads`, a rest if there are none.
    pub fn with_heads(duration: Duration, heads: Heads) -> Self {
        Self { duration, heads, tuplet: None, sticking: None }
    }

    pub fn heads(&self) -> &Heads {
        &self.heads
    }

    /// Play `heads` in this slot instead, a rest if there are none. The
    /// sticking stays while a hand instrument is left.
    pub fn set_heads(&mut self, heads: Heads) {
        self.heads = heads;
        self.sticking = self.sticking.filter(|_| self.takes_sticking());
    }

    /// Which hand plays the beat, if that is written.
    pub fn sticking(&self) -> Option<Sticking> {
        self.sticking
    }

    /// Whether the beat can carry a sticking: a head of it is played by
    /// hand ([`Instrument::is_hand`]).
    pub fn takes_sticking(&self) -> bool {
        self.heads.iter().any(|h| h.instrument().is_hand())
    }

    /// Write `sticking` on the beat, or none. Fails, and changes nothing,
    /// for a sticking on a beat that takes none.
    pub fn set_sticking(&mut self, sticking: Option<Sticking>) -> Result<(), NoHand> {
        if sticking.is_some() && !self.takes_sticking() {
            return Err(NoHand);
        }
        self.sticking = sticking;
        Ok(())
    }

    /// The head of `instrument`, to change its accent, ghost or technique.
    pub fn head_mut(&mut self, instrument: Instrument) -> Option<&mut Head> {
        self.heads.get_mut(instrument)
    }

    pub fn kind(&self) -> BeatKind {
        if self.heads.is_empty() { Rest } else { Note }
    }

    pub fn is_rest(&self) -> bool {
        self.heads.is_empty()
    }

    pub fn is_note(&self) -> bool {
        !self.is_rest()
    }

    /// Whether a head plays an instrument of the kit, not the one line's
    /// sound.
    pub fn is_kit(&self) -> bool {
        self.heads.iter().any(|h| h.instrument() != Instrument::Rhythm)
    }

    /// Whether a head is accented.
    pub fn accented(&self) -> bool {
        self.heads.iter().any(Head::accented)
    }

    /// Whether a head is soft (a ghost note).
    pub fn soft(&self) -> bool {
        self.heads.iter().any(Head::soft)
    }

    /// Accent every head, or none; an accented head is not soft. No change
    /// on a rest.
    pub fn set_accented(&mut self, accented: bool) {
        self.heads.iter_mut().for_each(|h| h.set_accented(accented));
    }

    /// Take the accents of `old`, the beat this one replaces: each head the
    /// accent of `old`'s head of its instrument, a head `old` has not the
    /// accent of `old` as a whole. No change on a rest. A beat without a
    /// sticking of its own takes `old`'s too, if it takes one.
    pub fn carry_accents_from(&mut self, old: &Beat) {
        for head in self.heads.iter_mut() {
            let accented = old.heads.get(head.instrument()).map_or(old.accented(), Head::accented);
            head.set_accented(accented);
        }
        if self.sticking.is_none() && self.takes_sticking() {
            self.sticking = old.sticking;
        }
    }

    /// Make every head soft, or none; a soft head is not accented. No
    /// change on a rest.
    pub fn set_soft(&mut self, soft: bool) {
        self.heads.iter_mut().for_each(|h| h.set_soft(soft));
    }

    /// Make the beat a rest, or a note of the one line: a note stays as it
    /// is, a rest becomes a plain [`Beat::note`].
    pub fn set_kind(&mut self, kind: BeatKind) {
        self.set_kind_of(kind, Instrument::Rhythm);
    }

    /// Make the beat a rest, or a note: a note stays as it is, a rest
    /// becomes a plain head of `instrument`.
    pub fn set_kind_of(&mut self, kind: BeatKind, instrument: Instrument) {
        match kind {
            Rest => self.set_heads(Heads::NONE),
            Note if self.is_rest() => self.set_heads(Heads::one(Head::new(instrument))),
            Note => {}
        }
    }

    /// The beat made a rest or a note of the one line (see
    /// [`Beat::set_kind`]).
    pub fn with_kind(&self, kind: BeatKind) -> Self {
        self.with_kind_of(kind, Instrument::Rhythm)
    }

    /// The beat made a rest or a note of `instrument` (see
    /// [`Beat::set_kind_of`]).
    pub fn with_kind_of(&self, kind: BeatKind, instrument: Instrument) -> Self {
        let mut beat = *self;
        beat.set_kind_of(kind, instrument);
        beat
    }
}

impl Debug for Beat {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.is_rest() {
            write!(f, "(")?
        }
        self.duration.fmt(f)?;
        if self.is_rest() {
            write!(f, ")")?
        }
        Ok(())
    }
}

/// A sticking on a beat without a hand instrument: a rest, or only the
/// kick or the hi-hat's foot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoHand;

impl std::fmt::Display for NoHand {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("a sticking needs a beat played by hand")
    }
}

impl std::error::Error for NoHand {}

/// `beats` without their tuplet marks, to compare durations and heads with
/// beats made by hand.
#[cfg(test)]
pub(crate) fn unmarked(beats: &[Beat]) -> Vec<Beat> {
    beats.iter().map(|b| Beat { tuplet: None, ..*b }).collect()
}

/// A [`Beat`] as saved: with its heads, or as before them with a kind and
/// the flags of its one head; in a tuplet group by its mark, or as before
/// marks by the id of its group (see [`StoredBeat`]).
#[derive(Deserialize)]
struct RawBeat {
    duration: Duration,
    #[serde(default)]
    tuplet: Option<TupletMark>,
    #[serde(default)]
    tuplet_group_id: Option<u32>,
    #[serde(default, deserialize_with = "present")]
    heads: Option<Heads>,
    #[serde(default, deserialize_with = "present")]
    kind: Option<BeatKind>,
    #[serde(default)]
    accented: bool,
    #[serde(default)]
    soft: bool,
    #[serde(default)]
    sticking: Option<Sticking>,
}

/// A field that is there, as `Some` (a missing one is `None` by default).
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

impl TryFrom<RawBeat> for Beat {
    type Error = &'static str;

    fn try_from(raw: RawBeat) -> Result<Self, Self::Error> {
        // A beat read on its own drops an old group id: only a voice can
        // turn ids into marks (see `StoredBeat`), and no saved data holds a
        // tuplet beat outside its voice.
        StoredBeat::try_from(raw).map(|stored| stored.beat)
    }
}

/// A [`Beat`] as saved in a voice, with the id of its tuplet group if it was
/// saved before [`TupletMark`]s: the voice turns the ids into marks.
#[derive(Deserialize)]
#[serde(try_from = "RawBeat")]
pub(crate) struct StoredBeat {
    pub(crate) beat: Beat,
    pub(crate) group_id: Option<u32>,
}

impl TryFrom<RawBeat> for StoredBeat {
    type Error = &'static str;

    fn try_from(raw: RawBeat) -> Result<Self, Self::Error> {
        let mut beat = match (raw.heads, raw.kind) {
            (Some(heads), _) => Beat::with_heads(raw.duration, heads),
            (None, Some(kind)) => {
                let mut beat = Beat::new(raw.duration, kind);
                // Saved both soft and accented, the accent wins, as it is
                // written and played.
                beat.set_soft(raw.soft);
                beat.set_accented(raw.accented);
                beat
            }
            (None, None) => return Err("a beat needs its heads"),
        };
        beat.tuplet = raw.tuplet;
        // A sticking saved on a beat without a hand is left out, as the
        // editor never writes one there.
        let _ = beat.set_sticking(raw.sticking);
        // An id counts only where there are no marks yet.
        let group_id = raw.tuplet_group_id.filter(|_| raw.tuplet.is_none());
        Ok(StoredBeat { beat, group_id })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::NoteValue;
    use crate::duration::{e, t8};
    use crate::head::Technique;

    #[test]
    fn a_beat_saved_before_heads_opens_as_one_rhythm_head() {
        let old = "(duration: Simple(Eighth), kind: Note, accented: true, tuplet_group_id: None)";
        let beat: Beat = ron::from_str(old).unwrap();
        let mut accented = Beat::note(e());
        accented.set_accented(true);
        assert_eq!(beat, accented);
        let old = "(duration: Simple(Eighth), kind: Rest, accented: false, soft: false, \
                   tuplet_group_id: Some(3))";
        let stored: StoredBeat = ron::from_str(old).unwrap();
        assert!(stored.beat.is_rest());
        assert_eq!(stored.group_id, Some(3));
        let old = "(duration: Simple(Eighth), kind: Note, accented: false, soft: true, \
                   tuplet_group_id: None)";
        let beat: Beat = ron::from_str(old).unwrap();
        let mut ghost = Beat::note(e());
        ghost.set_soft(true);
        assert_eq!(beat, ghost);
    }

    #[test]
    fn a_beat_round_trips_with_its_heads() {
        let open = Head::new(Instrument::HiHat).with_technique(Technique::Open).unwrap();
        let mut ghost = Head::new(Instrument::Snare);
        ghost.set_soft(true);
        let mut beat = Beat::with_heads(t8(), Heads::new([open, ghost]).unwrap());
        beat.tuplet = Some(TupletMark::Start { base: NoteValue::Eighth });
        let back: Beat = ron::from_str(&ron::to_string(&beat).unwrap()).unwrap();
        assert_eq!(back, beat);
        assert!(back.soft() && !back.accented());
        assert!(
            ron::from_str::<Beat>("(duration: Simple(Eighth), tuplet_group_id: None)").is_err()
        );
    }

    #[test]
    fn a_sticking_needs_a_hand_and_round_trips() {
        let mut beat = Beat::with_heads(
            e(),
            Heads::new([Head::new(Instrument::Kick), Head::new(Instrument::HiHat)]).unwrap(),
        );
        beat.set_sticking(Some(Sticking::Left)).unwrap();
        let text = ron::to_string(&beat).unwrap();
        let back: Beat = ron::from_str(&text).unwrap();
        assert_eq!(back.sticking(), Some(Sticking::Left));
        assert_eq!(back, beat);
        // Without a sticking nothing more is saved.
        assert!(!ron::to_string(&Beat::note(e())).unwrap().contains("sticking"));
        // The kick alone takes none; the hi-hat taken away takes it along.
        let mut heads = *beat.heads();
        heads.remove(Instrument::HiHat);
        beat.set_heads(heads);
        assert_eq!(beat.sticking(), None);
        assert_eq!(beat.set_sticking(Some(Sticking::Right)), Err(NoHand));
        assert_eq!(Beat::rest(e()).set_sticking(Some(Sticking::Right)), Err(NoHand));
        let mut line = Beat::note(e());
        line.set_sticking(Some(Sticking::Both)).unwrap();
        line.set_kind(Rest);
        assert_eq!(line.sticking(), None, "a rest has no hand");
        // A saved sticking without a hand is dropped.
        let kick = "(duration: Simple(Eighth), heads: [(instrument: Kick)], sticking: Some(Right))";
        assert_eq!(ron::from_str::<Beat>(kick).unwrap().sticking(), None);
    }

    #[test]
    fn kind_and_flags_follow_the_heads() {
        let mut b = Beat::note(e());
        b.set_accented(true);
        assert!(b.is_note() && b.accented());
        b.set_kind(Note);
        assert!(b.accented(), "a note stays as it is");
        b.set_kind(Rest);
        assert!(b.is_rest() && !b.accented());
        b.set_accented(true);
        assert!(!b.accented(), "a rest has no head to accent");
        assert_eq!(b.with_kind(Note), Beat::note(e()));
    }
}
