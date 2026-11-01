//! The staves a score is written on, described once: their lines, where
//! each instrument's heads sit, where the rests go, and whether the rows
//! start with the clef. The layout and the renderer read a staff from its
//! [`Staff`] and never ask which kind it is; [`Staff::of`] is the one place
//! that does.
//!
//! Staff positions count half staff spaces up from the middle line: 0 on
//! it, 1 just above it, -2 a staff space below.

use crate::glyphs;
use grooph_measure::duration::{Duration, NoteValue};
use grooph_measure::{Beat, BeatKind, Instrument, StaffKind};

/// A staff, as the layout and the renderer need it.
#[derive(Debug)]
pub struct Staff {
    /// The lines, as staff positions, from the lowest up.
    pub lines: &'static [i8],
    /// The instruments written on it, the one a new note gets by default
    /// first.
    pub instruments: &'static [Instrument],
    /// Where the heads of an instrument sit.
    place: fn(Instrument) -> i8,
    /// Where a rest that is no stemlet sits (see [`Staff::rest_pos`]).
    rest_place: fn(&Beat, Voicing) -> i8,
    /// Whether every row starts with the clef, not only the first.
    pub clef_every_row: bool,
}

/// How a voice shares its measure, for the place of its rests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Voicing {
    /// Another voice is in the measure.
    pub shared: bool,
    /// Stems point down: the lower voice.
    pub stem_down: bool,
}

/// The one line of a rhythm, also the metronome's click bar and its counter
/// rhythm: every instrument on the line.
static ONE_LINE: Staff = Staff {
    lines: &[0],
    instruments: &[Instrument::Rhythm],
    place: |_| 0,
    rest_place: line_rest_place,
    clef_every_row: false,
};

/// The kit's five lines: each instrument at its own place. Every row starts
/// with the clef, which tells the five lines from a pitched staff's.
static FIVE_LINES: Staff = Staff {
    lines: &[-4, -2, 0, 2, 4],
    instruments: &[
        Instrument::Snare,
        Instrument::Kick,
        Instrument::HiHat,
        Instrument::HiHatFoot,
        Instrument::Ride,
        Instrument::Crash,
        Instrument::HighTom,
        Instrument::MidTom,
        Instrument::FloorTom,
    ],
    place: Instrument::staff_pos,
    rest_place: kit_rest_place,
    clef_every_row: true,
};

impl Staff {
    /// The staff a score of `kind` is written on.
    pub fn of(kind: StaffKind) -> &'static Staff {
        match kind {
            StaffKind::Line => &ONE_LINE,
            StaffKind::Kit => &FIVE_LINES,
        }
    }

    /// The instrument new notes get when `chosen` is picked: `chosen` if
    /// the staff writes it, else its first (the one line's rhythm).
    pub fn instrument_or(&self, chosen: Instrument) -> Instrument {
        match self.instruments.contains(&chosen) {
            true => chosen,
            false => self.instruments[0],
        }
    }

    /// The instrument whose heads sit nearest staff position `pos`.
    pub fn instrument_near(&self, pos: f32) -> Instrument {
        let off = |i: &Instrument| (f32::from((self.place)(*i)) - pos).abs();
        *self.instruments.iter().min_by(|a, b| off(a).total_cmp(&off(b))).expect("an instrument")
    }

    /// Where the heads of `instrument` sit.
    pub fn place_of(&self, instrument: Instrument) -> i8 {
        (self.place)(instrument)
    }

    /// The highest line.
    pub fn top(&self) -> i8 {
        *self.lines.last().expect("a staff has a line")
    }

    /// The lowest line.
    pub fn bottom(&self) -> i8 {
        self.lines[0]
    }

    /// Whether two voices share one line, each keeping to its side of it.
    /// On a staff of several lines each instrument has its own place, and
    /// the voices need no side.
    pub fn one_line(&self) -> bool {
        self.lines.len() == 1
    }

    /// Whether `voicing`'s voice shares the line with another and keeps to
    /// its side of it: its heads sit beside the line, its rests clear it,
    /// and a rest under a beam is a stemlet.
    pub fn shares_line(&self, voicing: Voicing) -> bool {
        voicing.shared && self.one_line()
    }

    /// The side of a shared line a voice keeps to: 1 above, -1 below.
    fn side(voicing: Voicing) -> i8 {
        if voicing.stem_down { -1 } else { 1 }
    }

    /// Where a head of `instrument` sits in a voice of `voicing`: at the
    /// instrument's place, or beside a shared line.
    pub fn head_pos(&self, instrument: Instrument, voicing: Voicing) -> i8 {
        match self.shares_line(voicing) {
            true => Self::side(voicing),
            false => (self.place)(instrument),
        }
    }

    /// Where the rest `beat` sits in a voice of `voicing`. On a staff of one
    /// line a stemlet (a rest under a beam) reaches from the line, or from
    /// beside it when shared, like a note's stem. On a staff of several
    /// lines every rest keeps its own place.
    pub fn rest_pos(&self, beat: &Beat, in_beam: bool, voicing: Voicing) -> i8 {
        match (in_beam && self.one_line(), self.shares_line(voicing)) {
            (true, true) => Self::side(voicing),
            (true, false) => 0,
            (false, _) => (self.rest_place)(beat, voicing),
        }
    }

    /// Where an empty bar's whole rest hangs: as a whole rest of a voice
    /// alone.
    pub fn bar_rest_pos(&self) -> i8 {
        (self.rest_place)(&Beat::rest(Duration::Simple(NoteValue::Whole)), Voicing::default())
    }

    /// The ledger lines a glyph at `pos` needs: one at every line position
    /// past the outermost line up to it, outward. Only heads and half and
    /// whole rests (`with_ledgers`) take them; other rests float free.
    pub fn ledgers(&self, pos: i8, with_ledgers: bool) -> impl Iterator<Item = i8> {
        let (from, step) = match pos {
            _ if !with_ledgers => (0, 0),
            p if p > self.top() => (self.top() + 2, 2),
            p if p < self.bottom() => (self.bottom() - 2, -2),
            _ => (0, 0),
        };
        std::iter::successors((step != 0).then_some(from), move |&l| Some(l + step))
            .take_while(move |&l| step != 0 && (l - pos) * step.signum() <= 0)
    }

    /// Whether a glyph at `pos` sits on a line, of the staff or a ledger
    /// line it has (see [`Staff::ledgers`]): its dots move up into the
    /// space above.
    pub fn on_line(&self, pos: i8, with_ledgers: bool) -> bool {
        self.lines.contains(&pos) || self.ledgers(pos, with_ledgers).any(|l| l == pos)
    }
}

/// Whether a rest of `beat`'s value takes ledger lines: half and whole rests
/// are told apart by the line they sit on or hang from.
pub fn rest_takes_ledgers(beat: &Beat) -> bool {
    matches!(beat.duration.base_note(), NoteValue::Half | NoteValue::Whole)
}

/// Least room between a rest and a staff line it keeps clear of, in staff
/// spaces.
const REST_CLEARANCE_SS: f32 = 0.25;

/// How many half staff spaces `rest` moves off the line, away from it, for
/// its ink to clear the line by [`REST_CLEARANCE_SS`]: up when `below` is
/// false, down when it is true.
fn clear_of_line(rest: glyphs::Glyph, below: bool) -> i8 {
    let reach = if below { rest.ne[1] } else { -rest.sw[1] };
    (2.0 * (REST_CLEARANCE_SS + reach)).ceil() as i8
}

/// A rest on the one line. Alone it sits on the line; in the lower voice it
/// hangs a staff space lower, clear of an upper voice's, except half and
/// whole rests: they are told apart only by sitting on the line or hanging
/// from it. On a line shared by two voices each rest clears the line on its
/// voice's side, and a lower voice's half rest, which would sit on it,
/// moves to a ledger line a staff space below. (A whole rest, hanging from
/// the line, would need one above in the upper voice, but no meter holds a
/// whole note.)
fn line_rest_place(beat: &Beat, voicing: Voicing) -> i8 {
    debug_assert_eq!(beat.kind(), BeatKind::Rest);
    let base = beat.duration.base_note();
    let keeps_line = rest_takes_ledgers(beat);
    match (voicing.shared, voicing.stem_down) {
        (true, true) if base == NoteValue::Half => -2,
        (true, _) if keeps_line => 0,
        (true, down) => {
            let rest = glyphs::rest_glyph_for_duration(beat.duration);
            Staff::side(voicing) * clear_of_line(rest, down)
        }
        (false, true) if !keeps_line => -2,
        (false, _) => 0,
    }
}

/// A rest on the kit's staff: in the middle with one voice (a whole rest
/// hangs from the fourth line), in its voice's half with two, clear of the
/// other's notes: a half rest on the fourth or first line, a whole rest
/// hanging from the fourth or second, the others in the second space up or
/// down from the middle line (both a half space further out, clear of the heads and the top and bottom line).
fn kit_rest_place(beat: &Beat, voicing: Voicing) -> i8 {
    let base = beat.duration.base_note();
    match (voicing.shared, voicing.stem_down, base) {
        (false, _, NoteValue::Whole) => 2,
        (false, ..) => 0,
        (true, false, NoteValue::Whole) => 2,
        (true, false, NoteValue::Half) => 2,
        (true, false, _) => 4,
        (true, true, NoteValue::Whole) => -2,
        (true, true, NoteValue::Half) => -4,
        (true, true, _) => -4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledgers_run_from_the_outermost_line_to_the_glyph() {
        let kit = Staff::of(StaffKind::Kit);
        assert_eq!(kit.ledgers(5, true).collect::<Vec<_>>(), Vec::<i8>::new());
        assert_eq!(kit.ledgers(6, true).collect::<Vec<_>>(), vec![6]);
        assert_eq!(kit.ledgers(9, true).collect::<Vec<_>>(), vec![6, 8]);
        assert_eq!(kit.ledgers(-8, true).collect::<Vec<_>>(), vec![-6, -8]);
        assert_eq!(kit.ledgers(3, true).count(), 0);
        let line = Staff::of(StaffKind::Line);
        assert_eq!(line.ledgers(-2, true).collect::<Vec<_>>(), vec![-2]);
        assert_eq!(line.ledgers(-1, true).count(), 0);
        // A rest that is no half or whole rest takes none.
        assert_eq!(line.ledgers(-4, false).count(), 0);
    }

    #[test]
    fn a_staff_writes_its_instruments_and_finds_them_by_place() {
        let kit = Staff::of(StaffKind::Kit);
        assert_eq!(kit.instrument_or(Instrument::HiHat), Instrument::HiHat);
        assert_eq!(kit.instrument_or(Instrument::Rhythm), Instrument::Snare);
        assert_eq!(kit.instrument_near(5.2), Instrument::HiHat);
        assert_eq!(kit.instrument_near(-2.6), Instrument::Kick);
        assert_eq!(kit.instrument_near(20.0), Instrument::Crash);
        // Every drum of the kit can be picked on its staff.
        let drums = Instrument::ALL.into_iter().filter(|&i| i != Instrument::Rhythm);
        for drum in drums {
            assert!(kit.instruments.contains(&drum), "{drum:?}");
        }
        let line = Staff::of(StaffKind::Line);
        assert_eq!(line.instrument_or(Instrument::Snare), Instrument::Rhythm);
        assert_eq!(line.instrument_near(3.0), Instrument::Rhythm);
    }

    #[test]
    fn a_head_on_a_line_or_its_ledger_is_on_a_line() {
        let kit = Staff::of(StaffKind::Kit);
        assert!(kit.on_line(4, true) && kit.on_line(6, true) && !kit.on_line(5, true));
        let line = Staff::of(StaffKind::Line);
        assert!(line.on_line(0, false) && !line.on_line(1, true));
        assert!(line.on_line(-2, true) && !line.on_line(4, false));
    }
}
