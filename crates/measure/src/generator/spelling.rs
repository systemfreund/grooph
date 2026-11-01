//! Writing drawn figures as notes and rests.
//!
//! The figures only decide where notes *start*. This step decides how long
//! each note sounds, at random: up to the next onset or shorter. The
//! measure is then written from the onsets by [`crate::spelling`], whose
//! rules shorten a note that cannot be written as long as drawn.
//!
//! Triplet beats are written as drawn and end every note before them. A
//! beat rest from the "space" setting takes no sustained note either, so
//! the space stays visible as rests.

use super::{Figure, Rng};
use crate::TimeSignature;
use crate::duration::Duration;
use crate::spelling::{self, Ask, Lengths, Piece, Span, UNITS_PER_SIXTEENTH};

/// Probability that a note sounds as long as it can be written.
const LEGATO_PROBABILITY: f32 = 0.5;

/// How notes may be shortened.
#[derive(Clone, Copy, Debug)]
pub(super) struct Style {
    /// Grid for note ends, in sixteenths.
    pub step: u32,
    /// Notes may stop before the next onset inside their beat, e.g.
    /// `16 16-rest`. Without it a shortened note still fills its beat.
    pub short_notes: bool,
}

/// A drawn figure and whether it is a beat rest from the "space" setting.
#[derive(Clone, Copy, Debug)]
pub(super) struct Drawn {
    pub figure: Figure,
    pub space: bool,
}

fn as_beats(
    pieces: Vec<Piece>,
    verbatim: impl Fn(usize) -> Vec<(Duration, bool)>,
) -> Vec<(Duration, bool)> {
    pieces
        .into_iter()
        .flat_map(|piece| match piece {
            Piece::Note { duration, .. } => vec![(duration, true)],
            Piece::Rest(duration) => vec![(duration, false)],
            Piece::Verbatim(i) => verbatim(i),
        })
        .collect()
}

/// Write `drawn` (one figure per generator beat) as beats of a measure.
pub(super) fn spell(
    ts: TimeSignature,
    drawn: &[Drawn],
    style: Style,
    rng: &mut Rng,
) -> Vec<(Duration, bool)> {
    let mut spans = spelling::meter_spans(ts).expect("the generator writes x/4, x/8 and x/16");
    debug_assert_eq!(spans.len(), drawn.len());
    for (span, d) in spans.iter_mut().zip(drawn) {
        debug_assert_eq!(span.len, d.figure.span.sixteenths() * UNITS_PER_SIXTEENTH);
        span.verbatim = d.figure.pattern.triplet;
        span.blocks = d.space;
    }
    let onsets: Vec<u32> = drawn
        .iter()
        .zip(&spans)
        .filter(|(_, b)| !b.verbatim)
        .flat_map(|(d, b)| {
            d.figure.pattern.onset_slots().map(move |slot| b.start + slot * UNITS_PER_SIXTEENTH)
        })
        .collect();
    let step = style.step * UNITS_PER_SIXTEENTH;
    let lengths = Lengths { step, short_notes: style.short_notes };
    let pieces = spelling::write(&spans, &onsets, lengths, |Ask { onset, longest, beat_end }| {
        if rng.chance(LEGATO_PROBABILITY) {
            longest
        } else if style.short_notes {
            step * (1 + rng.below((longest / step) as usize) as u32)
        } else {
            longest.min(beat_end - onset)
        }
    });
    as_beats(pieces, |i| drawn[i].figure.pattern.beats())
}

/// One beat of `len` sixteenths with notes on the `onsets` bits, every note
/// as long as it can be written up to the next one.
pub(super) fn legato(len: u32, onsets: u8) -> Vec<(Duration, bool)> {
    let beat = Span {
        start: 0,
        len: len * UNITS_PER_SIXTEENTH,
        verbatim: false,
        blocks: false,
        simple: false,
        group: 0,
        in_group: 0,
    };
    let starts: Vec<u32> =
        (0..len).filter(|slot| onsets & 1 << slot != 0).map(|s| s * UNITS_PER_SIXTEENTH).collect();
    let lengths = Lengths { step: UNITS_PER_SIXTEENTH, short_notes: true };
    as_beats(spelling::write(&[beat], &starts, lengths, |ask| ask.longest), |_| unreachable!())
}
