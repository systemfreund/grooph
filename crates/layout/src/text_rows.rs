//! Rows of text along the staff: the count labels and the stickings.
//!
//! Each row sits above or below the staff, one or two lines high. The rows
//! of one side stack from the notes out, in a fixed order: the sticking
//! next to the notes, the count outside it. A row keeps clear of the
//! notes' ink (heads, stems, beams, accents, technique marks, tuplets) and
//! of the staff's middle; [`crate::staff_layout::build_staff_layout`]
//! places them in each row of the staff and gives the rows the room.

use egui::Rect;
use grooph_measure::{Measure, Sticking};

/// Which side of the staff a row of text goes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Above,
    Below,
}

/// The rows of text a staff shows, each on its side or not at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextRows {
    /// Which hand plays each note (see [`Measure::stickings`]).
    pub sticking: Option<Side>,
    /// The count labels.
    pub count: Option<Side>,
}

/// What a row of text writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextRowKind {
    Sticking,
    Count,
}

/// A row of text placed in a measure.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedTextRow {
    pub kind: TextRowKind,
    pub side: Side,
    /// The y of the middle of each line, top first.
    pub lines: Vec<f32>,
    /// The height of a line, in px.
    pub line_height: f32,
}

impl PlacedTextRow {
    /// The top of its first line.
    pub fn top(&self) -> f32 {
        self.lines[0] - self.line_height / 2.0
    }

    /// The bottom of its last line.
    pub fn bottom(&self) -> f32 {
        self.lines[self.lines.len() - 1] + self.line_height / 2.0
    }

    /// The text's size, in px.
    pub fn text_size(&self) -> f32 {
        self.line_height * TEXT_SIZE_EM / LINE_EM
    }

    pub(crate) fn translate(&mut self, dy: f32) {
        self.lines.iter_mut().for_each(|y| *y += dy);
    }
}

/// The height of a line of text, in em: its letters and a little room.
pub const LINE_EM: f32 = 0.45;
/// The size of the letters, in em.
pub const TEXT_SIZE_EM: f32 = 0.4;
/// The nearest a row comes to the staff's middle, in em.
const NEAR_LINE_EM: f32 = 0.525;
/// Room between the notes' ink and a row, in em.
const INK_GAP_EM: f32 = 0.075;
/// Room kept past the outermost row, in em.
pub(crate) const TEXT_MARGIN_EM: f32 = 0.025;

/// The lines a measure's sticking row takes: none without a sticking, two
/// where a column shows both hands (R above L), else one. Single letters
/// stay on the line next to the notes.
pub fn sticking_lines(measure: &Measure) -> usize {
    let stickings = measure.stickings();
    if stickings.iter().any(|&(_, s)| s == Sticking::Both) {
        2
    } else {
        usize::from(!stickings.is_empty())
    }
}

/// The `rows` of a measure whose staff's middle line is at `line_y`, its
/// notes' ink `ink`, with a sticking row of `sticking_lines` lines.
pub(crate) fn place(
    rows: TextRows,
    sticking_lines: usize,
    ink: Option<Rect>,
    line_y: f32,
    em: f32,
) -> Vec<PlacedTextRow> {
    let line_height = LINE_EM * em;
    let mut below =
        (line_y + NEAR_LINE_EM * em).max(ink.map_or(f32::MIN, |r| r.bottom() + INK_GAP_EM * em));
    let mut above =
        (line_y - NEAR_LINE_EM * em).min(ink.map_or(f32::MAX, |r| r.top() - INK_GAP_EM * em));
    let wanted = [
        (TextRowKind::Sticking, rows.sticking, sticking_lines),
        (TextRowKind::Count, rows.count, 1),
    ];
    let mut placed = Vec::new();
    for (kind, side, lines) in wanted {
        let Some(side) = side.filter(|_| lines > 0) else { continue };
        let top = match side {
            Side::Below => below,
            Side::Above => above - lines as f32 * line_height,
        };
        let ys = (0..lines).map(|i| top + (i as f32 + 0.5) * line_height).collect();
        match side {
            Side::Below => below += lines as f32 * line_height,
            Side::Above => above = top,
        }
        placed.push(PlacedTextRow { kind, side, lines: ys, line_height });
    }
    placed
}
