//! What sounds in a beat: its note heads, one per instrument.
//!
//! A [`Beat`](crate::Beat) is a slot of the rhythm (a duration, maybe in a
//! tuplet group); its [`Heads`] say what is played there. No head is a rest,
//! one head a note, several a chord on one stem (a hi-hat and a snare at
//! the same time). Accent, ghost and playing technique belong to each head:
//! in a chord the snare can be a ghost note under a plain hi-hat, and only
//! the hi-hat is open.
//!
//! Today every note is one [`Instrument::Rhythm`] head, the single line of
//! the staff; the drumkit instruments are for kit scores.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// What a note head plays. [`Instrument::Rhythm`] is the instrument of a
/// rhythm on one line, whose sound comes from the settings; the others are
/// the drumkit's.
///
/// [`Instrument::Rhythm`] comes first; the kit follows in the staff's
/// order, from low to high, so a chord's heads sort from the foot up.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum Instrument {
    /// A rhythm on one line.
    #[default]
    Rhythm,
    HiHatFoot,
    Kick,
    FloorTom,
    Snare,
    MidTom,
    HighTom,
    Ride,
    HiHat,
    /// The last variant: [`Instrument::COUNT`] counts up to it.
    Crash,
}

/// The shape of a note head.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Notehead {
    /// The usual filled (or hollow, for halves and wholes) oval.
    Normal,
    /// An x, for cymbals.
    Cross,
}

/// The mixer's volume an instrument plays at (`KitVolumes` in the audio
/// crate): one per group of the kit.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MixerGroup {
    Kick,
    Snare,
    /// The hi-hat, played by stick or by foot.
    HiHat,
    Toms,
    /// Ride and crash.
    Cymbals,
}

/// A technique an instrument is played with, and the letter written after
/// the instrument's code for it in the text notation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TechniqueSpec {
    pub technique: Technique,
    /// None for the usual stroke. A letter is unique per instrument only:
    /// `c` is a closed hi-hat and a choked crash.
    pub letter: Option<char>,
}

/// Everything about an instrument, in one place: its entry in
/// [`REGISTER`].
#[derive(Debug)]
pub struct InstrumentSpec {
    pub instrument: Instrument,
    /// Its code in the text notation (`K`, `HF`); none for the one line's
    /// rhythm, whose notes are written without one.
    pub code: Option<&'static str>,
    /// Its name on its button.
    pub label: &'static str,
    /// Its short name on its cell of the tool grid.
    pub short_label: &'static str,
    /// Where its heads sit, in half staff spaces up from the middle line of
    /// a five-line staff (the one line of a rhythm staff): the usual
    /// drumkit legend (Weinberg, Gould).
    pub staff_pos: i8,
    /// The shape of its heads, played the usual way (see [`Technique`] for
    /// the ones that change it).
    pub notehead: Notehead,
    /// The techniques it is played with, [`Technique::Normal`] first.
    pub techniques: &'static [TechniqueSpec],
    /// The mixer volume it plays at; the rhythm's is the snare's.
    pub mixer: MixerGroup,
}

const fn played(technique: Technique, letter: char) -> TechniqueSpec {
    TechniqueSpec { technique, letter: Some(letter) }
}

const NORMAL: TechniqueSpec = TechniqueSpec { technique: Technique::Normal, letter: None };

/// Every instrument, in the order of the enum (a [`Voicing`] keys its bits
/// by that order).
///
/// [`Voicing`]: crate::Voicing
pub static REGISTER: [InstrumentSpec; Instrument::COUNT] = {
    use Instrument as I;
    use Technique as T;
    [
        InstrumentSpec {
            instrument: I::Rhythm,
            code: None,
            label: "Rhythm",
            short_label: "R",
            staff_pos: 0,
            notehead: Notehead::Normal,
            techniques: &[NORMAL],
            mixer: MixerGroup::Snare,
        },
        InstrumentSpec {
            instrument: I::HiHatFoot,
            code: Some("HF"),
            label: "Hi-hat foot",
            short_label: "HF",
            staff_pos: -5,
            notehead: Notehead::Cross,
            techniques: &[NORMAL],
            mixer: MixerGroup::HiHat,
        },
        InstrumentSpec {
            instrument: I::Kick,
            code: Some("K"),
            label: "Kick",
            short_label: "BD",
            staff_pos: -3,
            notehead: Notehead::Normal,
            techniques: &[NORMAL],
            mixer: MixerGroup::Kick,
        },
        InstrumentSpec {
            instrument: I::FloorTom,
            code: Some("FT"),
            label: "Floor tom",
            short_label: "FT",
            staff_pos: -1,
            notehead: Notehead::Normal,
            techniques: &[NORMAL, played(T::Rimshot, 'r')],
            mixer: MixerGroup::Toms,
        },
        InstrumentSpec {
            instrument: I::Snare,
            code: Some("S"),
            label: "Snare",
            short_label: "SN",
            staff_pos: 1,
            notehead: Notehead::Normal,
            techniques: &[NORMAL, played(T::CrossStick, 'x'), played(T::Rimshot, 'r')],
            mixer: MixerGroup::Snare,
        },
        InstrumentSpec {
            instrument: I::MidTom,
            code: Some("T2"),
            label: "Tom 2",
            short_label: "T2",
            staff_pos: 2,
            notehead: Notehead::Normal,
            techniques: &[NORMAL, played(T::Rimshot, 'r')],
            mixer: MixerGroup::Toms,
        },
        InstrumentSpec {
            instrument: I::HighTom,
            code: Some("T1"),
            label: "Tom 1",
            short_label: "T1",
            staff_pos: 3,
            notehead: Notehead::Normal,
            techniques: &[NORMAL, played(T::Rimshot, 'r')],
            mixer: MixerGroup::Toms,
        },
        InstrumentSpec {
            instrument: I::Ride,
            code: Some("RD"),
            label: "Ride",
            short_label: "RD",
            staff_pos: 4,
            notehead: Notehead::Cross,
            techniques: &[NORMAL, played(T::Bell, 'b')],
            mixer: MixerGroup::Cymbals,
        },
        InstrumentSpec {
            instrument: I::HiHat,
            code: Some("H"),
            label: "Hi-hat",
            short_label: "HH",
            staff_pos: 5,
            notehead: Notehead::Cross,
            techniques: &[
                NORMAL,
                played(T::Open, 'o'),
                played(T::HalfOpen, 'h'),
                played(T::Closed, 'c'),
            ],
            mixer: MixerGroup::HiHat,
        },
        InstrumentSpec {
            instrument: I::Crash,
            code: Some("CR"),
            label: "Crash",
            short_label: "CR",
            staff_pos: 6,
            notehead: Notehead::Cross,
            techniques: &[NORMAL, played(T::Choke, 'c')],
            mixer: MixerGroup::Cymbals,
        },
    ]
};

impl Instrument {
    /// How many instruments there are. It sizes [`REGISTER`], so a variant
    /// added before [`Instrument::Crash`] without a register entry fails to
    /// compile. Keep `Crash` last, or name the new last variant here.
    pub const COUNT: usize = Instrument::Crash as usize + 1;

    /// Every instrument, in the order of the register.
    pub const ALL: [Instrument; Self::COUNT] = {
        let mut all = [Instrument::Rhythm; Self::COUNT];
        let mut i = 0;
        while i < all.len() {
            all[i] = REGISTER[i].instrument;
            i += 1;
        }
        all
    };

    /// The instrument's entry in the register.
    pub const fn spec(self) -> &'static InstrumentSpec {
        &REGISTER[self as usize]
    }

    /// Where the instrument's heads sit (see [`InstrumentSpec::staff_pos`]).
    pub const fn staff_pos(self) -> i8 {
        self.spec().staff_pos
    }

    /// The shape of the instrument's heads, played the usual way (see
    /// [`Technique`] for the ones that change it).
    pub const fn notehead(self) -> Notehead {
        self.spec().notehead
    }

    /// The techniques the instrument is played with, [`Technique::Normal`]
    /// first.
    pub fn techniques(self) -> impl ExactSizeIterator<Item = Technique> + Clone {
        self.spec().techniques.iter().map(|t| t.technique)
    }

    /// Whether the instrument is played with `technique`.
    pub fn allows(self, technique: Technique) -> bool {
        self.techniques().any(|t| t == technique)
    }

    /// The instrument's name on its button.
    pub const fn label(self) -> &'static str {
        self.spec().label
    }

    /// The instrument's short name on its cell of the tool grid.
    pub const fn short_label(self) -> &'static str {
        self.spec().short_label
    }
}

/// How a head is played, beyond accent and ghost. Each instrument allows
/// some ([`Instrument::techniques`]).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Technique {
    /// The instrument's usual stroke, written without a mark.
    #[default]
    Normal,
    /// An open hi-hat, written with a circle above.
    Open,
    /// A half-open hi-hat.
    HalfOpen,
    /// A closed hi-hat, written with a + above: where it is to be read,
    /// e.g. after an open one. A plain hi-hat has no mark.
    Closed,
    /// The stick laid across the snare, hitting the rim.
    CrossStick,
    /// Head and rim hit together.
    Rimshot,
    /// The bell of the ride.
    Bell,
    /// A cymbal stopped by hand right after the hit.
    Choke,
}

/// The technique is not one the instrument is played with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTechnique {
    pub instrument: Instrument,
    pub technique: Technique,
}

impl std::fmt::Display for InvalidTechnique {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} is not played {:?}", self.instrument, self.technique)
    }
}

impl std::error::Error for InvalidTechnique {}

/// One note head of a beat: an instrument, played accented, soft (a ghost
/// note) or plain, never both, with a technique the instrument allows.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawHead")]
pub struct Head {
    instrument: Instrument,
    #[serde(skip_serializing_if = "is_false")]
    accented: bool,
    /// A ghost note, written with its head in parentheses.
    #[serde(skip_serializing_if = "is_false")]
    soft: bool,
    #[serde(skip_serializing_if = "is_normal")]
    technique: Technique,
}

fn is_false(b: &bool) -> bool {
    !b
}

fn is_normal(t: &Technique) -> bool {
    *t == Technique::Normal
}

impl Head {
    /// A plain head of `instrument`, played the usual way.
    pub const fn new(instrument: Instrument) -> Self {
        Self { instrument, accented: false, soft: false, technique: Technique::Normal }
    }

    pub const fn instrument(&self) -> Instrument {
        self.instrument
    }

    pub const fn technique(&self) -> Technique {
        self.technique
    }

    pub const fn accented(&self) -> bool {
        self.accented
    }

    /// A ghost note, written with its head in parentheses.
    pub const fn soft(&self) -> bool {
        self.soft
    }

    /// Accent the head, or not. An accented head is not soft.
    pub fn set_accented(&mut self, accented: bool) {
        self.accented = accented;
        self.soft &= !accented;
    }

    /// Make the head soft (a ghost note), or not. A soft head is not
    /// accented.
    pub fn set_soft(&mut self, soft: bool) {
        self.soft = soft;
        self.accented &= !soft;
    }

    /// The head played with its instrument's next technique, after the
    /// last one the usual way again (see [`Instrument::techniques`]).
    pub fn with_next_technique(self) -> Self {
        let mut techniques = self.instrument.techniques();
        let at = techniques.clone().position(|t| t == self.technique).unwrap_or(0);
        let next = techniques.nth((at + 1) % techniques.len()).expect("in range");
        Self { technique: next, ..self }
    }

    /// The head played with `technique`, if its instrument allows it.
    pub fn with_technique(self, technique: Technique) -> Result<Self, InvalidTechnique> {
        if self.instrument.allows(technique) {
            Ok(Self { technique, ..self })
        } else {
            Err(InvalidTechnique { instrument: self.instrument, technique })
        }
    }
}

/// A [`Head`] as saved, before its technique is checked.
#[derive(Deserialize)]
struct RawHead {
    instrument: Instrument,
    #[serde(default)]
    accented: bool,
    #[serde(default)]
    soft: bool,
    #[serde(default)]
    technique: Technique,
}

impl TryFrom<RawHead> for Head {
    type Error = InvalidTechnique;

    fn try_from(raw: RawHead) -> Result<Self, Self::Error> {
        let mut head = Head::new(raw.instrument);
        // A head saved both soft and accented keeps the accent, as it is
        // written and played.
        head.set_soft(raw.soft);
        head.set_accented(raw.accented);
        head.with_technique(raw.technique)
    }
}

/// Most heads a beat holds: one per limb.
pub const MAX_HEADS: usize = 4;

/// The heads of a beat, at most one per instrument and [`MAX_HEADS`] in
/// all, sorted by instrument (from the foot up). None is a rest.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Heads {
    /// The heads, sorted, then `None`s.
    slots: [Option<Head>; MAX_HEADS],
}

/// Why heads cannot be one beat's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidHeads {
    /// More than [`MAX_HEADS`].
    TooMany,
    /// Two heads of the same instrument.
    SameInstrument(Instrument),
}

impl std::fmt::Display for InvalidHeads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InvalidHeads::TooMany => write!(f, "a beat has at most {MAX_HEADS} heads"),
            InvalidHeads::SameInstrument(i) => write!(f, "two heads of {i:?} in one beat"),
        }
    }
}

impl std::error::Error for InvalidHeads {}

impl Heads {
    /// No heads: a rest.
    pub const NONE: Heads = Heads { slots: [None; MAX_HEADS] };

    /// `head` alone.
    pub const fn one(head: Head) -> Self {
        let mut slots = [None; MAX_HEADS];
        slots[0] = Some(head);
        Heads { slots }
    }

    /// `heads` as one beat's, sorted.
    pub fn new(heads: impl IntoIterator<Item = Head>) -> Result<Self, InvalidHeads> {
        let mut all = Heads::NONE;
        for head in heads {
            all.insert(head)?;
        }
        Ok(all)
    }

    pub fn is_empty(&self) -> bool {
        self.slots[0].is_none()
    }

    pub fn len(&self) -> usize {
        self.slots.iter().take_while(|s| s.is_some()).count()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Head> {
        self.slots.iter().map_while(Option::as_ref)
    }

    /// The heads, to change their accent and ghost flags; instrument and
    /// technique stay, so the order holds.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Head> {
        self.slots.iter_mut().map_while(Option::as_mut)
    }

    /// The head of `instrument`, if any.
    pub fn get(&self, instrument: Instrument) -> Option<&Head> {
        self.iter().find(|h| h.instrument == instrument)
    }

    /// The head of `instrument`, to change its accent, ghost or technique.
    pub fn get_mut(&mut self, instrument: Instrument) -> Option<&mut Head> {
        self.iter_mut().find(|h| h.instrument == instrument)
    }

    /// Plain heads of the same instruments: no accent, ghost or technique.
    pub fn plain(&self) -> Heads {
        let mut plain = *self;
        plain.slots.iter_mut().flatten().for_each(|h| *h = Head::new(h.instrument));
        plain
    }

    /// Add `head` in its place. Fails on a second head of its instrument
    /// or a head too many.
    pub fn insert(&mut self, head: Head) -> Result<(), InvalidHeads> {
        if self.get(head.instrument).is_some() {
            return Err(InvalidHeads::SameInstrument(head.instrument));
        }
        let len = self.len();
        if len == MAX_HEADS {
            return Err(InvalidHeads::TooMany);
        }
        let at = self.iter().take_while(|h| h.instrument < head.instrument).count();
        self.slots[at..=len].rotate_right(1);
        self.slots[at] = Some(head);
        Ok(())
    }

    /// Remove the head of `instrument`, returning it.
    pub fn remove(&mut self, instrument: Instrument) -> Option<Head> {
        let at = self.iter().position(|h| h.instrument == instrument)?;
        let head = self.slots[at].take();
        self.slots[at..].rotate_left(1);
        head
    }
}

impl Serialize for Heads {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}

impl<'de> Deserialize<'de> for Heads {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let heads = Vec::<Head>::deserialize(deserializer)?;
        Heads::new(heads).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heads_sort_by_instrument_and_hold_one_of_each() {
        let mut heads =
            Heads::new([Head::new(Instrument::HiHat), Head::new(Instrument::Kick)]).unwrap();
        heads.insert(Head::new(Instrument::Snare)).unwrap();
        let order: Vec<_> = heads.iter().map(Head::instrument).collect();
        assert_eq!(order, [Instrument::Kick, Instrument::Snare, Instrument::HiHat]);
        assert_eq!(
            heads.insert(Head::new(Instrument::Snare)),
            Err(InvalidHeads::SameInstrument(Instrument::Snare))
        );
        heads.insert(Head::new(Instrument::HiHatFoot)).unwrap();
        assert_eq!(heads.insert(Head::new(Instrument::Crash)), Err(InvalidHeads::TooMany));

        assert_eq!(
            heads.remove(Instrument::Snare).map(|h| h.instrument()),
            Some(Instrument::Snare)
        );
        let order: Vec<_> = heads.iter().map(Head::instrument).collect();
        assert_eq!(order, [Instrument::HiHatFoot, Instrument::Kick, Instrument::HiHat]);
        assert_eq!(heads.remove(Instrument::Snare), None);
        // Equal heads are equal whatever order they came in.
        let again = Heads::new([
            Head::new(Instrument::HiHat),
            Head::new(Instrument::HiHatFoot),
            Head::new(Instrument::Kick),
        ])
        .unwrap();
        assert_eq!(heads, again);
    }

    #[test]
    fn a_head_is_accented_or_soft_never_both() {
        let mut head = Head::new(Instrument::Snare);
        head.set_accented(true);
        head.set_soft(true);
        assert!(head.soft() && !head.accented());
        head.set_accented(true);
        assert!(head.accented() && !head.soft());
        head.set_accented(false);
        assert!(!head.accented() && !head.soft());
    }

    #[test]
    fn every_instrument_has_its_entry_with_letters_unique_per_instrument() {
        for (at, spec) in REGISTER.iter().enumerate() {
            let i = spec.instrument;
            assert_eq!(i as usize, at, "{i:?} out of the enum's order");
            assert_eq!(i.spec().instrument, i);
            assert_eq!(Instrument::ALL[at], i);
            let (first, others) =
                spec.techniques.split_first().unwrap_or_else(|| panic!("{i:?} has techniques"));
            assert_eq!(*first, TechniqueSpec { technique: Technique::Normal, letter: None });
            let mut letters: Vec<char> =
                others.iter().map(|t| t.letter.expect("a letter")).collect();
            letters.sort();
            letters.dedup();
            assert_eq!(letters.len(), others.len(), "{i:?}: a letter twice");
        }
    }

    #[test]
    fn the_register_keeps_the_values_of_the_tables_it_replaced() {
        use Instrument::*;
        use MixerGroup as M;
        use Notehead::{Cross, Normal};
        #[rustfmt::skip]
        let expected = [
            (Rhythm, None, "Rhythm", "R", 0, Normal, M::Snare),
            (HiHatFoot, Some("HF"), "Hi-hat foot", "HF", -5, Cross, M::HiHat),
            (Kick, Some("K"), "Kick", "BD", -3, Normal, M::Kick),
            (FloorTom, Some("FT"), "Floor tom", "FT", -1, Normal, M::Toms),
            (Snare, Some("S"), "Snare", "SN", 1, Normal, M::Snare),
            (MidTom, Some("T2"), "Tom 2", "T2", 2, Normal, M::Toms),
            (HighTom, Some("T1"), "Tom 1", "T1", 3, Normal, M::Toms),
            (Ride, Some("RD"), "Ride", "RD", 4, Cross, M::Cymbals),
            (HiHat, Some("H"), "Hi-hat", "HH", 5, Cross, M::HiHat),
            (Crash, Some("CR"), "Crash", "CR", 6, Cross, M::Cymbals),
        ];
        for (i, code, label, short, pos, notehead, mixer) in expected {
            let spec = i.spec();
            assert_eq!(
                (
                    spec.code,
                    spec.label,
                    spec.short_label,
                    spec.staff_pos,
                    spec.notehead,
                    spec.mixer
                ),
                (code, label, short, pos, notehead, mixer),
                "{i:?}"
            );
        }
    }

    #[test]
    fn a_head_takes_only_its_instruments_techniques() {
        let hh = Head::new(Instrument::HiHat);
        assert_eq!(hh.with_technique(Technique::Open).unwrap().technique(), Technique::Open);
        assert_eq!(
            Head::new(Instrument::Snare).with_technique(Technique::Bell),
            Err(InvalidTechnique { instrument: Instrument::Snare, technique: Technique::Bell })
        );
        for i in Instrument::ALL {
            assert_eq!(i.techniques().next(), Some(Technique::Normal), "{i:?}");
        }
    }

    #[test]
    fn heads_round_trip_and_saved_ones_are_checked() {
        let open = Head::new(Instrument::HiHat).with_technique(Technique::Open).unwrap();
        let mut ghost = Head::new(Instrument::Snare);
        ghost.set_soft(true);
        let heads = Heads::new([open, ghost]).unwrap();
        let text = ron::to_string(&heads).unwrap();
        assert_eq!(ron::from_str::<Heads>(&text).unwrap(), heads);
        assert!(ron::from_str::<Head>("(instrument: Snare, technique: Bell)").is_err());
        let both: Head = ron::from_str("(instrument: Snare, accented: true, soft: true)").unwrap();
        assert!(both.accented() && !both.soft(), "the accent wins");
        let twice = "[(instrument: Kick), (instrument: Kick)]";
        assert!(ron::from_str::<Heads>(twice).unwrap_err().to_string().contains("two heads"));
    }

    #[test]
    fn the_kit_sorts_from_the_foot_up_on_the_staff() {
        // Heads sort by instrument; on the staff that is from low to high.
        let kit = &Instrument::ALL[1..];
        assert!(kit.windows(2).all(|w| w[0] < w[1] && w[0].staff_pos() < w[1].staff_pos()));
        assert_eq!(Instrument::ALL[0], Instrument::Rhythm);
    }

    #[test]
    fn all_lists_every_instrument() {
        // `ALL` holds the variants in declaration order without a gap, up to
        // the last one: a new variant fails to compile in `is_last` until
        // it is placed, and then this test until `ALL` holds it.
        fn is_last(i: Instrument) -> bool {
            use Instrument::*;
            match i {
                Crash => true,
                Rhythm | HiHatFoot | Kick | FloorTom | Snare | MidTom | HighTom | Ride | HiHat => {
                    false
                }
            }
        }
        for (n, &i) in Instrument::ALL.iter().enumerate() {
            assert_eq!(i as usize, n, "{i:?}");
        }
        assert!(is_last(Instrument::ALL[Instrument::ALL.len() - 1]));
    }
}
