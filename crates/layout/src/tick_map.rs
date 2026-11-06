//! Where a point in time of a bar is drawn.
//!
//! Everything drawn along the notes that stands for a point in time, the
//! playback cursor, the guide's anchor, the count labels, the stickings and
//! the timing marks, takes its x from the bar's [`TickMap`], so that they
//! agree on it: on a head, between two, in a tuplet or on a rest.

use crate::pixel_layout::{MeasureLayout, onset_head};
use egui::{Rangef, Rect};
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::{BeatIdx, Measure, VoiceIdx};
use std::ops::Range;

/// A column of a bar: a tick where a beat starts in any of its voices
/// ([`Measure::onsets`]), at the head standing for it ([`onset_head`]).
/// With one voice the columns are its beats; a lower voice's notes between
/// the upper voice's are columns too.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Column {
    /// Ticks from the start of the bar.
    pub tick: u32,
    /// The x of the head's centre.
    pub x: f32,
    /// The beat whose head it is.
    pub voice: VoiceIdx,
    pub beat_idx: BeatIdx,
}

/// Where a column starts and ends: from its tick to the next column's, or
/// to the end of the bar, and from its head to the next head, or to the
/// barline.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnEdges {
    pub ticks: Range<u32>,
    pub xs: Rangef,
}

/// The x of each tick of a bar as laid out.
///
/// A tick on a column is at its head. Between two columns it is
/// interpolated by tick from one head to the next; after the last column,
/// from its head to the barline, so nothing of a bar leaves it (the `&` of
/// a closing quarter sits before the barline, not in front of the next
/// bar's `1`).
#[derive(Debug, Clone, PartialEq)]
pub struct TickMap {
    /// Empty where the layout has no head for an onset (a layout of
    /// another bar).
    columns: Vec<Column>,
    /// Ticks from the start of the bar to its end.
    bar_ticks: u32,
    /// The left edge of the notes' area, after the clef and meter.
    left: f32,
    /// The barline.
    right: f32,
}

impl TickMap {
    /// The map of `measure` laid out as `voices` in `rect`.
    pub fn new(measure: &Measure, voices: &[MeasureLayout], rect: Rect) -> Self {
        let columns: Option<Vec<Column>> = measure
            .onsets()
            .iter()
            .map(|o| {
                let head = onset_head(voices, o)?;
                Some(Column {
                    tick: o.tick,
                    x: head.center.x,
                    voice: o.voice,
                    beat_idx: o.beat_idx,
                })
            })
            .collect();
        TickMap {
            columns: columns.unwrap_or_default(),
            bar_ticks: DEFAULT_GRID.ticks_per_measure(&measure.time_signature()),
            left: voices.first().map_or(rect.left(), |v| v.notes_left_edge),
            right: rect.right(),
        }
    }

    /// The columns, in order of their ticks; the first is at 0. None when
    /// the layout is of another bar.
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Ticks from the start of the bar to its end.
    pub fn bar_ticks(&self) -> u32 {
        self.bar_ticks
    }

    /// The column at or before `tick`.
    pub fn column_at(&self, tick: f64) -> Option<usize> {
        self.columns.iter().rposition(|c| c.tick as f64 <= tick)
    }

    /// Where column `i` starts and ends. Panics if there is no column `i`.
    pub fn edges(&self, i: usize) -> ColumnEdges {
        let c = &self.columns[i];
        let (end_tick, end_x) =
            self.columns.get(i + 1).map_or((self.bar_ticks, self.right), |n| (n.tick, n.x));
        ColumnEdges { ticks: c.tick..end_tick, xs: Rangef::new(c.x, end_x) }
    }

    /// Where the column at or before `tick` starts and ends.
    pub fn edges_at(&self, tick: f64) -> Option<ColumnEdges> {
        self.column_at(tick).map(|i| self.edges(i))
    }

    /// The x of `tick`, between the start and the end of the bar: on a
    /// column at its head, in it interpolated toward the next head or the
    /// barline. A bar without columns (a layout of another bar) spreads its
    /// ticks evenly over the notes' area.
    pub fn x_at_tick(&self, tick: f64) -> f32 {
        let tick = tick.clamp(0.0, self.bar_ticks as f64);
        let Some(edges) = self.edges_at(tick) else {
            let frac = tick / self.bar_ticks.max(1) as f64;
            return self.left + (self.right - self.left) * frac as f32;
        };
        let x = edges.xs.min + (edges.xs.max - edges.xs.min) * edges.frac(tick) as f32;
        x.max(self.left).min(self.right)
    }

    /// The x of the head of the column at exactly `tick`, if there is one.
    /// Stickings take it: a sticking belongs to a head and is written under
    /// or over it, never between two. On a column it is [`Self::x_at_tick`].
    pub fn head_x_at(&self, tick: u32) -> Option<f32> {
        self.columns.iter().find(|c| c.tick == tick).map(|c| c.x)
    }

    /// The playback cursor's x at `tick` (taken modulo the bar). It walks
    /// like [`Self::x_at_tick`], except in the last column: it reaches the
    /// barline after `split` of it and spends the rest entering the next bar
    /// ([`Self::entering_frac`]), where this returns `None`. `None` too for
    /// a bar without columns.
    pub fn cursor_x(&self, tick: f64, split: f64) -> Option<f32> {
        let t = self.wrap(tick)?;
        let i = self.column_at(t)?;
        if i + 1 < self.columns.len() {
            return Some(self.x_at_tick(t));
        }
        let edges = self.edges(i);
        let frac = edges.frac(t);
        (frac < split).then(|| {
            let x0 = edges.xs.min;
            x0 + (edges.xs.max - x0) * (frac / split) as f32
        })
    }

    /// How far the cursor has entered the next bar at `tick` (taken modulo
    /// the bar), from 0 to 1: past `split` of the last column (see
    /// [`Self::cursor_x`]). `None` before that, and for a bar without
    /// columns.
    pub fn entering_frac(&self, tick: f64, split: f64) -> Option<f64> {
        let t = self.wrap(tick)?;
        let last = self.columns.len().checked_sub(1)?;
        let edges = self.edges(last);
        if t < edges.ticks.start as f64 || edges.ticks.is_empty() {
            return None;
        }
        let frac = edges.frac(t);
        if frac < split {
            return None;
        }
        if split >= 1.0 {
            return Some(1.0);
        }
        Some((frac - split) / (1.0 - split))
    }

    /// Pixels per tick around column `i`: toward the next column, for the
    /// last from the one before. A timing mark is scaled by the spacing of
    /// the notes around it; the stretch from the last head to the barline
    /// is not set by the notes' spacing (see [`Self::cursor_x`]), so the
    /// last column takes the one before it.
    pub fn px_per_tick(&self, i: usize) -> Option<f64> {
        let slope = |a: usize, b: usize| {
            let (ca, cb) = (self.columns.get(a)?, self.columns.get(b)?);
            let dt = (cb.tick as i64 - ca.tick as i64) as f64;
            (dt != 0.0).then(|| (cb.x - ca.x) as f64 / dt)
        };
        slope(i, i + 1).or_else(|| i.checked_sub(1).and_then(|prev| slope(prev, i)))
    }

    /// The column whose head is nearest `x`.
    pub fn column_near(&self, x: f32) -> Option<usize> {
        (0..self.columns.len()).min_by(|&a, &b| {
            (self.columns[a].x - x).abs().total_cmp(&(self.columns[b].x - x).abs())
        })
    }

    /// Moves the bar by `dx`.
    pub(crate) fn translate(&mut self, dx: f32) {
        let TickMap { columns, bar_ticks: _, left, right } = self;
        columns.iter_mut().for_each(|c| c.x += dx);
        *left += dx;
        *right += dx;
    }

    /// `tick` in the bar, taken modulo it; 0 for one before it. `None` for
    /// a bar of no ticks.
    fn wrap(&self, tick: f64) -> Option<f64> {
        let total = self.bar_ticks as f64;
        if total <= 0.0 {
            return None;
        }
        if tick.is_sign_negative() {
            return Some(0.0);
        }
        let t = tick % total;
        Some(if t.is_nan() { 0.0 } else { t })
    }
}

impl ColumnEdges {
    /// How far `tick` is into the column, from 0 at its start to 1 at its
    /// end.
    fn frac(&self, tick: f64) -> f64 {
        let span = (self.ticks.end - self.ticks.start) as f64;
        if span > 0.0 { ((tick - self.ticks.start as f64) / span).clamp(0.0, 1.0) } else { 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::staff_layout::{PlacedMeasure, StaffLayout};
    use crate::staff_layout::{ROOM_BELOW_EM, StaffOpts, build_staff_layout};
    use egui::{FontFamily, FontId, Pos2, vec2};
    use grooph_measure::Score;
    use grooph_measure::duration::{e, q, t8};
    use grooph_measure::notation::parse_score;

    fn opts() -> StaffOpts {
        let em = 20.0;
        StaffOpts {
            rect: Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            min_measure_width_em: 6.0,
            note_width_em: 0.6,
            system_spacing_em: 0.5,
            layout_clef_first: true,
            time_signature_first: true,
            room_below_em: ROOM_BELOW_EM,
            text_rows: Default::default(),
            bar_rests: true,
            veil_hidden_rests: false,
        }
    }

    fn staff(text: &str) -> (Score, StaffLayout) {
        let score = parse_score(text).unwrap();
        let staff = build_staff_layout(&score, &opts());
        (score, staff)
    }

    fn heads(placed: &PlacedMeasure, voice: VoiceIdx) -> Vec<f32> {
        placed.voices[voice].notes.iter().map(|n| n.center.x).collect()
    }

    fn ticks(d: grooph_measure::duration::Duration) -> f64 {
        DEFAULT_GRID.ticks_of(&d).unwrap() as f64
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn beats_are_at_their_heads() {
        let (_, staff) = staff("4/4 q q e e q");
        let placed = staff.placed(0).unwrap();
        let map = &placed.ticks;
        let heads = heads(placed, 0);
        for (i, beat) in [0.0, 1.0, 2.0, 2.5, 3.0].into_iter().enumerate() {
            assert_eq!(map.x_at_tick(beat * ticks(q())), heads[i], "beat {beat}");
        }
        assert_eq!(map.columns().len(), 5);
    }

    #[test]
    fn a_tick_between_columns_is_interpolated() {
        let (_, staff) = staff("4/4 h q q");
        let placed = staff.placed(0).unwrap();
        let heads = heads(placed, 0);
        // Beat 2 halfway through the half note.
        let x = placed.ticks.x_at_tick(ticks(q()));
        assert!(close(x, (heads[0] + heads[1]) / 2.0), "{x} {heads:?}");
        let edges = placed.ticks.edges_at(ticks(q())).unwrap();
        assert_eq!(edges.ticks, 0..2 * ticks(q()) as u32);
        assert_eq!((edges.xs.min, edges.xs.max), (heads[0], heads[1]));
    }

    #[test]
    fn the_end_of_the_bar_is_the_barline() {
        let (_, staff) = staff("4/4 q q q q");
        let placed = staff.placed(0).unwrap();
        let map = &placed.ticks;
        let last = *heads(placed, 0).last().unwrap();
        let right = placed.rect.right();
        assert_eq!(map.x_at_tick(map.bar_ticks() as f64), right);
        assert_eq!(map.x_at_tick(10.0 * map.bar_ticks() as f64), right);
        assert!(close(map.x_at_tick(3.5 * ticks(q())), (last + right) / 2.0));
        let edges = map.edges(3);
        assert_eq!(edges.ticks.end, map.bar_ticks());
        assert_eq!(edges.xs.max, right);
        assert_eq!(map.x_at_tick(-5.0), heads(placed, 0)[0]);
    }

    #[test]
    fn tuplets_are_columns_of_their_own() {
        let (_, staff) = staff("2/4 t8 t8 t8 q");
        let placed = staff.placed(0).unwrap();
        let map = &placed.ticks;
        let heads = heads(placed, 0);
        let t = ticks(t8());
        for (i, &head) in heads[..3].iter().enumerate() {
            assert_eq!(map.x_at_tick(i as f64 * t), head);
        }
        // The `&` of beat 1 halfway through the second triplet eighth.
        assert!(close(map.x_at_tick(ticks(e())), (heads[1] + heads[2]) / 2.0));
        assert_eq!(map.edges(1).ticks, t as u32..2 * t as u32);
    }

    #[test]
    fn rests_are_columns_like_notes() {
        let (_, staff) = staff("4/4 r:e e r:q h");
        let placed = staff.placed(0).unwrap();
        let heads = heads(placed, 0);
        assert_eq!(placed.ticks.columns().len(), 4);
        assert_eq!(placed.ticks.x_at_tick(0.0), heads[0]);
        assert_eq!(placed.ticks.x_at_tick(ticks(q())), heads[2]);
    }

    #[test]
    fn a_lower_voice_adds_columns() {
        let (_, staff) = staff("4/4 h h ; q q q q");
        let placed = staff.placed(0).unwrap();
        let map = &placed.ticks;
        let lower = heads(placed, 1);
        let cols: Vec<(u32, VoiceIdx)> = map.columns().iter().map(|c| (c.tick, c.voice)).collect();
        let q = ticks(q()) as u32;
        assert_eq!(cols, [(0, 0), (q, 1), (2 * q, 0), (3 * q, 1)]);
        assert_eq!(map.x_at_tick(q as f64), lower[1]);
        assert_eq!(map.head_x_at(q), Some(lower[1]));
        assert_eq!(map.head_x_at(q / 2), None);
    }

    #[test]
    fn a_bar_of_rests_is_mapped_like_any() {
        // The editor's ghost bar: rests, drawn as one whole rest, each beat
        // keeping a hidden layout.
        let (_, staff) = staff("4/4 q q q q | r:q r:q r:q r:q");
        let placed = staff.placed(1).unwrap();
        let map = &placed.ticks;
        assert_eq!(map.columns().len(), 4);
        let xs: Vec<f32> = (0..=4).map(|b| map.x_at_tick(b as f64 * ticks(q()))).collect();
        assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
        assert_eq!(xs[4], placed.rect.right());
    }

    #[test]
    fn a_bar_without_heads_is_spread_evenly() {
        // A layout of another bar has no head for the onsets: no columns,
        // ticks evenly over the notes' area, no cursor.
        let (score, staff) = staff("4/4 q q q q | e e e e e e e e");
        let placed = staff.placed(0).unwrap();
        let map = TickMap::new(&score.measures()[1], &placed.voices, placed.rect);
        assert!(map.columns().is_empty());
        let left = placed.layout().notes_left_edge;
        let right = placed.rect.right();
        assert!(close(map.x_at_tick(2.0 * ticks(q())), (left + right) / 2.0));
        assert_eq!(map.cursor_x(0.0, 0.5), None);
        assert_eq!(map.entering_frac(3.9 * ticks(q()), 0.5), None);
    }

    #[test]
    fn the_cursor_reaches_the_barline_at_the_split() {
        let (_, staff) = staff("4/4 q q q q");
        let placed = staff.placed(0).unwrap();
        let map = &placed.ticks;
        let last = *heads(placed, 0).last().unwrap();
        let right = placed.rect.right();
        let q = ticks(q());
        // Between heads like `x_at_tick`.
        assert_eq!(map.cursor_x(1.5 * q, 0.5), Some(map.x_at_tick(1.5 * q)));
        // A quarter of the last note is halfway to the barline at a split
        // of a half, and past it the cursor enters the next bar.
        assert!(close(map.cursor_x(3.25 * q, 0.5).unwrap(), (last + right) / 2.0));
        assert_eq!(map.cursor_x(3.5 * q, 0.5), None);
        assert_eq!(map.entering_frac(3.25 * q, 0.5), None);
        assert!((map.entering_frac(3.75 * q, 0.5).unwrap() - 0.5).abs() < 1e-9);
        // Modulo the bar.
        assert_eq!(map.cursor_x(4.0 * q + 1.5 * q, 0.5), map.cursor_x(1.5 * q, 0.5));
    }

    #[test]
    fn timing_marks_take_the_spacing_around_their_column() {
        let (_, staff) = staff("4/4 q q h");
        let placed = staff.placed(0).unwrap();
        let map = &placed.ticks;
        let heads = heads(placed, 0);
        let q = ticks(q());
        let slope = |a: usize, b: usize, dt: f64| (heads[b] - heads[a]) as f64 / dt;
        assert!((map.px_per_tick(0).unwrap() - slope(0, 1, q)).abs() < 1e-9);
        assert!((map.px_per_tick(1).unwrap() - slope(1, 2, q)).abs() < 1e-9);
        // The last, from the one before.
        assert!((map.px_per_tick(2).unwrap() - slope(1, 2, q)).abs() < 1e-9);
    }

    #[test]
    fn a_moved_staff_moves_its_maps() {
        let (_, mut staff) = staff("4/4 q q q q");
        let before = staff.placed(0).unwrap().ticks.clone();
        staff.translate(vec2(7.0, 3.0));
        let after = &staff.placed(0).unwrap().ticks;
        for tick in [0.0, 100.0, before.bar_ticks() as f64] {
            assert!(close(after.x_at_tick(tick), before.x_at_tick(tick) + 7.0));
        }
    }
}
