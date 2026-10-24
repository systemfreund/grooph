//! Writing drawn cells as notes and rests.
//!
//! The cells only decide where notes *start*. This step decides how long
//! each note sounds and writes the measure from that: a note sounds up to
//! the next onset or stops earlier, and the time after it is written as
//! rests. One rhythm therefore comes in several spellings (`8 8`,
//! `16 16-rest 16 16-rest`, a half note or a quarter and a quarter rest).
//!
//! Without ties a note may only run over a beat boundary where it can be
//! written as one value: from a beat as a dotted quarter or half, from the
//! "and" as a quarter or dotted quarter. In `x/4` the beats are grouped
//! like the eighths of `n/8` (4/4 = 2+2, 5/4 = 3+2, ...) and a note leaves
//! its group only in the two syncopations centred on the group boundary
//! (`4 2 4` and `8 4 8` over the middle of a 4/4 bar), so the boundary
//! otherwise stays visible; the same groups merge two silent beats into a
//! half rest. In `x/8` and `x/16` a
//! note stays in its beat group. A note that cannot be written as long as
//! drawn is shortened; its onset never moves.
//!
//! Triplet beats are written as drawn and end every note before them. A
//! beat rest from the "space" setting takes no sustained note either, so
//! the space stays visible as rests.

use super::{BeatSpan, Cell, CellFamily, Rng};
use crate::duration::{Duration, NoteValue, e, h, q, s};

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

/// A drawn cell and whether it is a beat rest from the "space" setting.
#[derive(Clone, Copy, Debug)]
pub(super) struct Drawn {
    pub cell: &'static Cell,
    pub space: bool,
}

/// One generator beat on the measure's sixteenth grid.
#[derive(Clone, Copy, Debug)]
struct Span {
    start: u32,
    len: u32,
    /// Written as drawn (triplets).
    verbatim: bool,
    /// No note may sound into this beat.
    blocks: bool,
    /// A quarter beat of an `x/4` meter; only these let notes cross them.
    simple: bool,
    /// Notes and rests may join beats of the same group only.
    group: usize,
    /// Position of the beat in its group.
    in_group: usize,
}

impl Span {
    fn end(&self) -> u32 {
        self.start + self.len
    }
}

fn sixteenths(span: BeatSpan) -> u32 {
    match span {
        BeatSpan::Quarter => 4,
        BeatSpan::DottedQuarter => 6,
        BeatSpan::Eighth => 2,
        BeatSpan::DottedEighth => 3,
    }
}

fn duration_of(len: u32) -> Duration {
    match len {
        1 => s(),
        2 => e(),
        3 => Duration::Dotted { base: NoteValue::Eighth, dots: 1 },
        4 => q(),
        6 => Duration::Dotted { base: NoteValue::Quarter, dots: 1 },
        8 => h(),
        _ => unreachable!("no single value of {len} sixteenths"),
    }
}

fn spans(beat_unit: u8, drawn: &[Drawn]) -> Vec<Span> {
    // x/4: quarter beats grouped like the eighths of n/8. Other meters keep
    // every generator beat to itself.
    let groups: Vec<u8> = if beat_unit == 4 {
        crate::grouping::compound_groups(drawn.len() as u8).into_owned()
    } else {
        vec![1; drawn.len()]
    };
    let mut beats_of_group = groups
        .iter()
        .enumerate()
        .flat_map(|(group, &size)| (0..size as usize).map(move |in_group| (group, in_group)));
    let mut start = 0;
    drawn
        .iter()
        .map(|d| {
            let len = sixteenths(d.cell.span());
            let (group, in_group) = beats_of_group.next().expect("groups cover every beat");
            let span = Span {
                start,
                len,
                verbatim: d.cell.family == CellFamily::Triplet,
                blocks: d.space,
                simple: beat_unit == 4,
                group,
                in_group,
            };
            start += len;
            span
        })
        .collect()
}

/// Whether a note of `len` sixteenths can be written at `start` as one value.
fn fits(spans: &[Span], start: u32, len: u32) -> bool {
    let beat =
        spans.iter().find(|b| b.start <= start && start < b.end()).expect("start in measure");
    let offset = start - beat.start;
    if start + len <= beat.end() {
        return match len {
            1..=3 => true,
            // Inside a dotted-quarter group a quarter starts on an eighth.
            4 => beat.len == 4 || offset.is_multiple_of(2),
            6 => beat.len == 6,
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
            0 => matches!(len, 6 | 8),
            2 => matches!(len, 4 | 6),
            _ => false,
        };
    }
    // Into the next group only centred on the boundary: `4 2 4`, `8 4 8`.
    let boundary = spans.iter().find(|b| b.group == last.group).unwrap().start;
    last.group == beat.group + 1
        && start + len / 2 == boundary
        && matches!((offset, len), (0, 8) | (2, 4))
}

/// Rest values for `start..end` inside free (not verbatim) beats.
fn push_rests(spans: &[Span], mut start: u32, end: u32, out: &mut Vec<(Duration, bool)>) {
    while start < end {
        let i = spans.iter().position(|b| b.start <= start && start < b.end()).unwrap();
        let beat = spans[i];
        let offset = start - beat.start;
        // Two silent quarter beats at the start of a pair in their group.
        let half = offset == 0
            && beat.len == 4
            && beat.in_group.is_multiple_of(2)
            && end >= beat.start + 8
            && spans.get(i + 1).is_some_and(|n| n.len == 4 && n.group == beat.group);
        let len = if half {
            8
        } else {
            let rest = end.min(beat.end()) - start;
            match beat.len {
                _ if offset == 0 && rest == beat.len => beat.len,
                // `8.-rest 16` as in the cell library.
                4 if offset == 0 && rest == 3 => 3,
                6 if offset.is_multiple_of(2) && offset <= 2 && rest >= 4 => 4,
                _ if offset.is_multiple_of(2) && rest >= 2 => 2,
                _ => 1,
            }
        };
        out.push((duration_of(len), false));
        start += len;
    }
}

/// Write `drawn` (one cell per generator beat) as beats of a measure.
pub(super) fn spell(
    beat_unit: u8,
    drawn: &[Drawn],
    style: Style,
    rng: &mut Rng,
) -> Vec<(Duration, bool)> {
    let spans = spans(beat_unit, drawn);
    let onsets: Vec<u32> = drawn
        .iter()
        .zip(&spans)
        .filter(|(_, b)| !b.verbatim)
        .flat_map(|(d, b)| {
            let mut at = b.start;
            d.cell.beats.iter().filter_map(move |&(duration, is_note)| {
                let onset = at;
                at += super::sixteenths_of(duration);
                is_note.then_some(onset)
            })
        })
        .collect();

    let mut out = Vec::new();
    // Start of the time not yet written in the current run of free beats.
    let mut written = 0;
    let mut next_onset = onsets.iter().copied().peekable();
    for (i, (d, beat)) in drawn.iter().zip(&spans).enumerate() {
        if beat.verbatim {
            out.extend(d.cell.beats.iter().copied());
            written = beat.end();
            continue;
        }
        while let Some(onset) = next_onset.next_if(|&o| o < beat.end()) {
            push_rests(&spans, written, onset, &mut out);
            // A note sounds at most up to the next onset, a triplet beat or
            // a space rest.
            let barrier = spans[i + 1..]
                .iter()
                .find(|b| b.verbatim || b.blocks)
                .map_or(spans.last().unwrap().end(), |b| b.start);
            let limit = next_onset.peek().map_or(barrier, |&o| o.min(barrier));
            let longest = limit - onset;
            let wanted = if rng.chance(LEGATO_PROBABILITY) {
                longest
            } else if style.short_notes {
                style.step * (1 + rng.below((longest / style.step) as usize) as u32)
            } else {
                longest.min(beat.end() - onset)
            };
            // Without short notes a note ends on the next onset or a beat.
            let ends_well = |len: u32| {
                style.short_notes
                    || onset + len == limit
                    || spans.iter().any(|b| b.end() == onset + len)
            };
            let len = (1..=wanted)
                .rev()
                .filter(|len| len % style.step == 0 && ends_well(*len))
                .find(|&len| fits(&spans, onset, len))
                .unwrap_or(style.step);
            out.push((duration_of(len), true));
            written = onset + len;
        }
        let run_ends = spans.get(i + 1).is_none_or(|n| n.verbatim);
        if run_ends {
            push_rests(&spans, written, beat.end(), &mut out);
            written = beat.end();
        }
    }
    out
}
