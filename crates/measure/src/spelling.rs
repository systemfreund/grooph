//! Writing notes and rests from where the notes start.
//!
//! A drum hit does not sustain: the rhythm is where each note starts, the
//! written length is spelling. This module writes a measure from its note
//! onsets. The caller decides how long each note should sound (the
//! generator at random, the editor's cleanup up to the end of its beat);
//! the rules here decide what can be written, and write the time between
//! as rests. One rhythm therefore comes in several spellings (`8 8`,
//! `16 16-rest 16 16-rest`, a half note or a quarter and a quarter rest).
//!
//! Without ties a note may only run over a beat boundary where it can be
//! written as one value: from a beat as a dotted quarter or half, from the
//! "and" as a quarter or dotted quarter. In `x/4` the beats are grouped
//! like the eighths of `n/8` (4/4 = 2+2, 5/4 = 3+2, ...) and a note leaves
//! its group only as a half centred on the group boundary (`4 2 4` over the
//! middle of a 4/4 bar), so the boundary otherwise stays visible; the same
//! groups merge two silent beats into a half rest. In `x/8` and `x/16` a
//! note stays in its beat group. A note that cannot be written as long as
//! wanted is shortened; its onset never moves.
//!
//! A note from the "and" over a beat needs eighths around it: with a
//! sixteenth, a dotted eighth or an offbeat sixteenth start in the beat it
//! starts or ends in, the figure is read in quarters and the note would hide
//! the beat (`16 16 4 8`). Such a note is shortened, and the beat it ends in
//! keeps eighth values (two-level rhythmic parsing).
//!
//! Beats marked verbatim (tuplets) are written by the caller as they are
//! and end every note before them. A beat that blocks (a beat rest of the
//! generator's "space" setting) takes no sustained note either, so the
//! space stays visible as rests.
//!
//! Positions and lengths are in thirty-seconds of the measure ("units").

use crate::TimeSignature;
use crate::duration::{Duration, NoteValue, e, h, q, s, th};

/// Units (thirty-seconds) in a sixteenth.
pub(crate) const UNITS_PER_SIXTEENTH: u32 = 2;

/// One beat of the measure, a beat group in `x/8` and `x/16`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Span {
    pub start: u32,
    pub len: u32,
    /// Written by the caller as it is (tuplets).
    pub verbatim: bool,
    /// No note may sound into this beat.
    pub blocks: bool,
    /// A quarter beat of an `x/4` meter; only these let notes cross them.
    pub simple: bool,
    /// Notes and rests may join beats of the same group only.
    pub group: usize,
    /// Position of the beat in its group.
    pub in_group: usize,
}

impl Span {
    pub fn end(&self) -> u32 {
        self.start + self.len
    }
}

/// The beats of `ts`: quarters in `x/4`, grouped like the eighths of `n/8`;
/// the beat groups in `x/8` and `x/16` (6/8 = 3+3, 7/8 as picked), each a
/// group of its own. `None` for other beat units.
pub(crate) fn meter_spans(ts: TimeSignature) -> Option<Vec<Span>> {
    let beats = usize::from(ts.beats);
    let (lens, groups): (Vec<u32>, Vec<u8>) = match ts.beat_unit {
        4 => (
            vec![4 * UNITS_PER_SIXTEENTH; beats],
            crate::grouping::compound_groups(ts.beats).into_owned(),
        ),
        8 | 16 => {
            let unit = 16 / u32::from(ts.beat_unit) * UNITS_PER_SIXTEENTH;
            // Groups longer than three in twos, with a three at the end
            // for an odd length, as the generator draws them (4/16).
            let lens: Vec<u32> = ts
                .groups()
                .iter()
                .flat_map(|&g| {
                    let threes = u32::from(g > 3 && g % 2 == 1);
                    let twos = if g > 3 { (u32::from(g) - 3 * threes) / 2 } else { 0 };
                    let whole = (g <= 3).then_some(u32::from(g));
                    std::iter::repeat_n(2, twos as usize)
                        .chain(std::iter::repeat_n(3, threes as usize))
                        .chain(whole)
                })
                .map(|g| g * unit)
                .collect();
            let count = lens.len();
            (lens, vec![1; count])
        }
        _ => return None,
    };
    let mut beats_of_group = groups
        .iter()
        .enumerate()
        .flat_map(|(group, &size)| (0..size as usize).map(move |in_group| (group, in_group)));
    let mut start = 0;
    let mut spans = Vec::with_capacity(lens.len());
    for len in lens {
        let (group, in_group) = beats_of_group.next()?;
        spans.push(Span {
            start,
            len,
            verbatim: false,
            blocks: false,
            simple: ts.beat_unit == 4,
            group,
            in_group,
        });
        start += len;
    }
    Some(spans)
}

/// The note value of `len` units, if one value has that length.
pub(crate) fn duration_of(len: u32) -> Option<Duration> {
    Some(match len {
        1 => th(),
        2 => s(),
        3 => Duration::Dotted { base: NoteValue::Sixteenth, dots: 1 },
        4 => e(),
        6 => Duration::Dotted { base: NoteValue::Eighth, dots: 1 },
        8 => q(),
        12 => Duration::Dotted { base: NoteValue::Quarter, dots: 1 },
        16 => h(),
        _ => return None,
    })
}

fn value(len: u32) -> Duration {
    duration_of(len).unwrap_or_else(|| unreachable!("no single value of {len} units"))
}

/// Whether a note of `len` units can be written at `start` as one value.
fn fits(spans: &[Span], start: u32, len: u32) -> bool {
    let beat = &spans[beat_at(spans, start)];
    let offset = start - beat.start;
    if start + len <= beat.end() {
        return match len {
            1..=3 => true,
            // Eighths start on a sixteenth.
            4 | 6 => offset.is_multiple_of(2),
            // Inside a dotted-quarter group a quarter starts on an eighth.
            8 => beat.len == 8 || offset.is_multiple_of(4),
            12 => beat.len == 12,
            _ => false,
        };
    }
    let Some(last) = spans.iter().find(|b| b.start < start + len && start + len <= b.end()) else {
        return false;
    };
    if !(beat.simple && last.simple) {
        return false;
    }
    if beat.group == last.group {
        return match offset {
            0 => matches!(len, 12 | 16),
            4 => matches!(len, 8 | 12),
            _ => false,
        };
    }
    // Into the next group only as a half centred on the boundary: `4 2 4`.
    let boundary = spans.iter().find(|b| b.group == last.group).unwrap().start;
    last.group == beat.group + 1 && start + len / 2 == boundary && (offset, len) == (0, 16)
}

/// Index of the beat that holds `unit`.
fn beat_at(spans: &[Span], unit: u32) -> usize {
    spans.iter().position(|b| b.start <= unit && unit < b.end()).expect("unit in measure")
}

/// Mark the beat of a value at `start` as read in sixteenths when the value
/// is shorter than an eighth or not a multiple of it (a sixteenth, a dotted
/// eighth), or starts off the eighth grid.
fn mark_fine(spans: &[Span], start: u32, len: u32, fine: &mut [bool]) {
    if !start.is_multiple_of(4) || !len.is_multiple_of(4) {
        fine[beat_at(spans, start)] = true;
    }
}

/// Rest values for `start..end` inside free (not verbatim) beats.
fn push_rests(spans: &[Span], mut start: u32, end: u32, fine: &mut [bool], out: &mut Vec<Piece>) {
    while start < end {
        let i = beat_at(spans, start);
        let beat = spans[i];
        let offset = start - beat.start;
        // Two silent quarter beats at the start of a pair in their group.
        let half = offset == 0
            && beat.len == 8
            && beat.in_group.is_multiple_of(2)
            && end >= beat.start + 16
            && spans.get(i + 1).is_some_and(|n| n.len == 8 && n.group == beat.group);
        let len = if half {
            16
        } else {
            let rest = end.min(beat.end()) - start;
            match beat.len {
                _ if offset == 0 && rest == beat.len && duration_of(rest).is_some() => beat.len,
                // `8.-rest 16` for the figure `---x`.
                8 if offset == 0 && rest == 6 => 6,
                // A quarter rest on the first or second eighth of a
                // dotted-quarter beat.
                12 if offset.is_multiple_of(4) && offset <= 4 && rest >= 8 => 8,
                _ if offset.is_multiple_of(4) && rest >= 4 => 4,
                _ if offset.is_multiple_of(2) && rest >= 2 => 2,
                _ => 1,
            }
        };
        mark_fine(spans, start, len, fine);
        out.push(Piece::Rest(value(len)));
        start += len;
    }
}

/// How long notes may be.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lengths {
    /// Grid for note ends, in units.
    pub step: u32,
    /// Notes may stop before the next onset inside their beat, e.g.
    /// `16 16-rest`. Without it a shortened note still fills its beat.
    pub short_notes: bool,
}

/// What [`write`] asks the caller about a note: how long it should sound.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ask {
    /// Where the note starts.
    pub onset: u32,
    /// The longest it may sound: up to the next onset, a verbatim or a
    /// blocking beat, or the end of the measure.
    pub longest: u32,
    /// The end of the beat it starts in.
    pub beat_end: u32,
}

/// One written value of the measure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Piece {
    /// A note starting at `onset`.
    Note {
        onset: u32,
        duration: Duration,
    },
    Rest(Duration),
    /// The verbatim beat of this index, which the caller writes.
    Verbatim(usize),
}

/// Write the notes starting at `onsets` (ascending, none in a verbatim
/// beat) and the rests between them over `spans`. `wanted` says how long
/// each note should sound (at least `lengths.step`, at most
/// [`Ask::longest`]); the rules above may shorten it.
pub(crate) fn write(
    spans: &[Span],
    onsets: &[u32],
    lengths: Lengths,
    mut wanted: impl FnMut(Ask) -> u32,
) -> Vec<Piece> {
    let Lengths { step, short_notes } = lengths;
    // Beats read in sixteenths: a note off the eighth grid starts in them,
    // or a sixteenth or dotted value was written in them.
    let mut fine: Vec<bool> = spans
        .iter()
        .map(|b| {
            !b.verbatim
                && onsets.iter().any(|&o| b.start <= o && o < b.end() && !o.is_multiple_of(4))
        })
        .collect();
    // Beats a syncopated note ends in: their values stay eighths or longer.
    let mut coarse = vec![false; spans.len()];
    let mut out = Vec::new();
    // Start of the time not yet written in the current run of free beats.
    let mut written = 0;
    let mut next_onset = onsets.iter().copied().peekable();
    for (i, beat) in spans.iter().enumerate() {
        if beat.verbatim {
            out.push(Piece::Verbatim(i));
            written = beat.end();
            continue;
        }
        while let Some(onset) = next_onset.next_if(|&o| o < beat.end()) {
            push_rests(spans, written, onset, &mut fine, &mut out);
            // A note sounds at most up to the next onset, a verbatim beat or
            // a blocking one.
            let barrier = spans[i + 1..]
                .iter()
                .find(|b| b.verbatim || b.blocks)
                .map_or(spans.last().unwrap().end(), |b| b.start);
            let limit = next_onset.peek().map_or(barrier, |&o| o.min(barrier));
            let longest = limit - onset;
            let want = wanted(Ask { onset, longest, beat_end: beat.end() });
            // After a syncopation the beat stays in eighths.
            let want = if coarse[i] { want.max(4) } else { want };
            // Without short notes a note ends on the next onset or a beat.
            let ends_well = |len: u32| {
                short_notes || onset + len == limit || spans.iter().any(|b| b.end() == onset + len)
            };
            // Over a beat from its "and" only between beats read in eighths.
            let syncopated = |len: u32| onset != beat.start && onset + len > beat.end();
            let in_eighths = |len: u32| {
                if coarse[i] && !len.is_multiple_of(4) {
                    return false;
                }
                !syncopated(len) || !fine[i] && !fine[beat_at(spans, onset + len - 1)]
            };
            let len = (1..=want)
                .rev()
                .filter(|len| len.is_multiple_of(step) && ends_well(*len))
                .find(|&len| fits(spans, onset, len) && in_eighths(len))
                .unwrap_or(step);
            if syncopated(len) {
                coarse[beat_at(spans, onset + len - 1)] = true;
            }
            mark_fine(spans, onset, len, &mut fine);
            out.push(Piece::Note { onset, duration: value(len) });
            written = onset + len;
        }
        let run_ends = spans.get(i + 1).is_none_or(|n| n.verbatim);
        if run_ends {
            push_rests(spans, written, beat.end(), &mut fine, &mut out);
            written = beat.end();
        }
    }
    out
}

/// The editor's cleanup: every note as long as it can be written up to
/// the next onset, at most to the end of its beat. Across beats the time
/// is written as rests, so a backbeat reads `4-rest 4 4-rest 4`.
pub(crate) fn to_beat_end(ask: Ask) -> u32 {
    ask.longest.min(ask.beat_end - ask.onset)
}
