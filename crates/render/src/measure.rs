use crate::beat::draw_beat;
use crate::glyph::draw_glyph;
use eframe::egui;
use eframe::egui::{
    Align2, Color32, FontFamily, FontId, Painter, Pos2, Rangef, Rect, Stroke, pos2,
};
use grooph_layout::pixel_layout::{
    BeamLayout, LayoutOpts, MeasureLayout, NoteLayout, TimeSignatureLayout, TupletLayout,
    build_measure_layout,
};
use grooph_measure::counting::{CountConfig, CountSlot, build_count_slots};
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::{Measure, TimeSignature};

pub fn draw_measure(
    ui: &mut egui::Ui,
    measure: &Measure,
    opts: &LayoutOpts,
    cursor_idx: Option<usize>,
    playback_tick: Option<f64>,
    count_config: Option<&CountConfig>,
) -> MeasureLayout {
    let measure_layout = build_measure_layout(measure, opts);
    render_measure_at(
        ui,
        measure,
        &measure_layout,
        opts,
        cursor_idx,
        playback_tick.map(|t| (t, ui.visuals().selection.stroke.color.to_opaque())),
        count_config,
        true,
        None,
        None,
    );
    measure_layout
}

/// Render a measure given a pre-built `MeasureLayout`.
///
/// `draw_staff_line` controls whether the horizontal staff line is drawn —
/// the multi-measure renderer turns it off and draws one continuous line per
/// system instead. All cursor/playback drawing is scoped to this measure; the
/// caller decides whether to forward `cursor_idx` / `count_tick` based on
/// the active measure.
///
/// `cursor_x` is the pre-computed playback cursor X (in the same coordinate
/// space as `measure_layout`). The cursor's home measure depends on the
/// playback phase: during the first half of a measure's last note the cursor
/// lives in *this* measure, during the second half it visually moves into the
/// following measure. The caller (`draw_staff`) decides via `current_cursor_x`
/// and passes `Some(x)` only to the measure that actually hosts the cursor.
///
/// `lit` draws one note (its index) in the given colour instead of the text
/// colour.
///
/// The function orchestrates independent decoration phases — counting layer,
/// staff line, clef + time signature, notes/beams/tuplets, edit cursor,
/// playback cursor. Each phase is a small helper; reorder or extend by
/// editing this function.
#[expect(clippy::too_many_arguments)]
pub(crate) fn render_measure_at(
    ui: &mut egui::Ui,
    measure: &Measure,
    measure_layout: &MeasureLayout,
    opts: &LayoutOpts,
    cursor_idx: Option<usize>,
    count_tick: Option<(f64, Color32)>,
    count_config: Option<&CountConfig>,
    draw_staff_line: bool,
    cursor_x: Option<f32>,
    lit: Option<(usize, Color32)>,
) {
    let color = ui.visuals().text_color();
    let painter = ui.painter();
    let rect = opts.rect;

    if let Some(config) = count_config {
        draw_count_layer(
            painter,
            measure,
            measure_layout,
            opts,
            opts.y_center(),
            config,
            ui.visuals().text_color(),
            count_tick,
        );
    }

    if draw_staff_line {
        draw_staff_line_segment(painter, rect, opts.y_center(), opts.em, color);
    }

    if let Some(clef) = &measure_layout.clef {
        draw_glyph(painter, clef, &opts.font_id.family, color);
    }
    if let Some(ts_layout) = &measure_layout.time_signature {
        draw_time_signature(painter, ts_layout, measure.time_signature(), &opts.font_id, color);
    }

    draw_notes(painter, measure_layout, color, lit, opts);

    if let Some(idx) = cursor_idx {
        draw_edit_cursor(ui, painter, measure_layout, idx, opts);
    }

    if let Some(x) = cursor_x {
        let color = ui.visuals().selection.stroke.color;
        draw_playback_cursor(painter, opts.y_center(), x, opts.em, color);
    }
}

fn draw_staff_line_segment(painter: &Painter, rect: Rect, y: f32, em: f32, color: Color32) {
    painter.hline(Rangef::new(rect.left(), rect.right()), y, Stroke::new(0.02 * em, color));
}

/// Draws the meter digits at the laid-out positions, with a grouping other
/// than the usual one written above them.
pub fn draw_time_signature(
    painter: &Painter,
    ts_layout: &TimeSignatureLayout,
    ts: TimeSignature,
    font_id: &FontId,
    color: Color32,
) {
    for digit in ts_layout.beats.iter().chain(&ts_layout.beat_unit) {
        draw_glyph(painter, digit, &font_id.family, color);
    }
    // A grouping other than the usual one is written above the meter, e.g.
    // "2+2+3" over 7/8.
    if ts.grouping != 0
        && let (Some(first), Some(last)) = (ts_layout.beats.first(), ts_layout.beats.last())
    {
        let em = font_id.size;
        let x = (first.origin.x + last.origin.x + last.glyph.advance * em * 0.25) / 2.0;
        let pos = egui::pos2(x, first.origin.y - 0.4 * em);
        let font = FontId::proportional(0.28 * em);
        painter.text(pos, Align2::CENTER_BOTTOM, ts.groups_label(), font, color);
    }
}

/// The blinking edit cursor at note `idx`.
pub(crate) fn draw_edit_cursor(
    ui: &egui::Ui,
    painter: &Painter,
    measure_layout: &MeasureLayout,
    idx: usize,
    opts: &LayoutOpts,
) {
    let Some(nl) = measure_layout.notes.get(idx) else { return };

    // Blink: full on/off cycle with 50% duty.
    let blink_period = 1.0_f64;
    let duty = 0.5_f64;
    let t = ui.input(|i| i.time);
    let visible = (t % blink_period) / blink_period < duty;
    let alpha = if visible { 220u8 } else { 40u8 };

    let c = measure_layout.notes[idx].center;
    let top = c.y + 0.5 * opts.em;
    let bottom = c.y - 0.5 * opts.em;
    let base = if ui.visuals().dark_mode { Color32::YELLOW } else { Color32::BLUE };
    let cursor_color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha);
    painter.vline(nl.center.x, Rangef::new(top, bottom), Stroke::new(0.03 * opts.em, cursor_color));
    // Drive animation between input events.
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
}

fn draw_playback_cursor(painter: &Painter, y: f32, x: f32, em: f32, base: Color32) {
    draw_playback_cursor_span(painter, Rangef::new(y - 0.7 * em, y + 0.7 * em), x, em, base);
}

/// The playback cursor over `ys`.
fn draw_playback_cursor_span(painter: &Painter, ys: Rangef, x: f32, em: f32, base: Color32) {
    painter.vline(x, ys, playback_cursor_stroke(em, base));
}

/// Stroke of the playback cursor at `em`, in a see-through `base`.
pub fn playback_cursor_stroke(em: f32, base: Color32) -> Stroke {
    Stroke::new(0.1 * em, Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 100))
}

/// The count labels of `measure`, under the line at `baseline_y`. The label
/// at the tick of `count_tick` is lit in its colour.
#[expect(clippy::too_many_arguments)]
pub(crate) fn draw_count_layer(
    painter: &Painter,
    measure: &Measure,
    measure_layout: &MeasureLayout,
    opts: &LayoutOpts,
    baseline_y: f32,
    config: &CountConfig,
    text_color: Color32,
    count_tick: Option<(f64, Color32)>,
) {
    let (playback_tick, highlight_color) = match count_tick {
        Some((tick, color)) => (Some(tick), color),
        None => (None, text_color),
    };
    let slots = build_count_slots(measure, config);
    if slots.is_empty() {
        return;
    }
    let total_ticks = DEFAULT_GRID.ticks_per_measure(&measure.time_signature());
    draw_count_labels(
        painter,
        measure,
        measure_layout,
        opts.rect,
        baseline_y,
        opts.em,
        text_color,
        highlight_color,
        &slots,
        playback_tick,
        total_ticks,
    );
}

/// Phase 1 cursor X — cursor lives inside *this* measure at `tick`.
///
/// - Inside notes (not last): linear interpolation between consecutive note
///   centers (`notes[i].center.x` → `notes[i+1].center.x`).
/// - Last note, `frac < split`: linear from `notes[last].center.x` to
///   `rect.right()` (cursor extends past the last note head, fading out).
/// - Last note, `frac >= split`: returns `None`. The cursor visually moves
///   into the *following* measure — call [`playback_cursor_entering_x`] there.
///
/// `split` (see [`last_note_split`]) is the share of the last note's duration
/// spent before the barline.
/// - Trailing gap (tick past every beat's end, i.e. an incomplete measure):
///   `Some(rect.right())`. Cursor parks at the right edge.
///
/// Returns `None` if the measure has no notes or zero total ticks.
pub fn playback_cursor_x(
    measure: &Measure,
    measure_layout: &MeasureLayout,
    rect: Rect,
    tick: f64,
    split: f64,
) -> Option<f32> {
    let ts = measure.time_signature();
    let total_ticks = DEFAULT_GRID.ticks_per_measure(&ts) as f64;
    if total_ticks <= 0.0 || measure_layout.notes.is_empty() {
        return None;
    }
    let t = clamp_tick_to_measure(tick, total_ticks);

    let onsets = DEFAULT_GRID.compute_onset_ticks(measure.beats());

    for (i, &onset) in onsets.iter().enumerate() {
        let start = onset as f64;
        let dur_ticks = DEFAULT_GRID.ticks_of(&measure.beats()[i].duration).unwrap_or(0) as f64;
        let end = start + dur_ticks;
        if t >= start && t < end {
            let frac = if dur_ticks > 0.0 { (t - start) / dur_ticks } else { 0.0 };
            // A layout of another measure has other notes: no cursor.
            let x0 = measure_layout.notes.get(i)?.center.x;

            if i + 1 < measure_layout.notes.len() {
                // Normal note: interpolate to the next note in this measure.
                let x1 = measure_layout.notes[i + 1].center.x;
                return Some(x0 + ((x1 - x0) * frac as f32));
            }

            // Last note: split into two phases. Phase 1 covers frac in
            // [0, split) and walks from the note head to the measure's right
            // edge. Phase 2 (frac >= split) renders in the following measure
            // via `playback_cursor_entering_x`.
            if frac < split {
                let phase1_frac = (frac / split) as f32;
                return Some(x0 + (rect.right() - x0) * phase1_frac);
            }
            return None;
        }
    }

    // Trailing gap: tick lies past the last beat's end (incomplete measure).
    // Hold the cursor at the right edge — visually consistent with "done".
    Some(rect.right())
}

/// X of the point in time `tick` of `measure`, for a guide anchor: a note's
/// head at its onset, then on toward the next head (the barline after the
/// last one) as the note goes on, so beat 4 of a half note on 3 sits
/// halfway. `None` if the layout has no notes for it.
pub fn beat_anchor_x(
    measure: &Measure,
    measure_layout: &MeasureLayout,
    rect: Rect,
    tick: f64,
) -> Option<f32> {
    let beats = measure.beats();
    let onsets = DEFAULT_GRID.compute_onset_ticks(beats);
    let i = onsets.iter().rposition(|&onset| onset as f64 <= tick)?;
    let start = onsets[i] as f64;
    let dur_ticks = DEFAULT_GRID.ticks_of(&beats[i].duration).unwrap_or(0) as f64;
    let frac = if dur_ticks > 0.0 { ((tick - start) / dur_ticks).clamp(0.0, 1.0) } else { 0.0 };
    let x0 = measure_layout.notes.get(i)?.center.x;
    let x1 = measure_layout.notes.get(i + 1).map_or(rect.right(), |n| n.center.x);
    Some(x0 + (x1 - x0) * frac as f32)
}

/// A guide anchor: a soft disc centred on `center`, at `intensity` (1 when
/// its beat is reached, fading to 0).
pub fn draw_beat_anchor(painter: &Painter, center: Pos2, em: f32, base: Color32, intensity: f32) {
    let alpha = |a: f32| (a * intensity.clamp(0.0, 1.0) * 255.0) as u8;
    let color = |a: f32| Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha(a));
    let radius = 0.45 * em;
    painter.circle(center, radius, color(0.3), Stroke::new(0.04 * em, color(0.9)));
}

/// Phase 2 cursor X — cursor enters *this* measure from the left.
///
/// `entry_frac` is in `[0, 1]` and maps linearly to
/// `[notes_left_edge, notes[0].center.x]`. Returns `None` if the measure has
/// no notes (nothing to enter towards).
///
/// Called when the previous measure's last note is in its second half: the
/// cursor visually re-appears in the Clef/TS area of the following measure
/// and travels to its first note head. For a 1-measure score the "following"
/// measure is the same measure, which reproduces the original single-measure
/// wrap animation.
pub fn playback_cursor_entering_x(
    measure_layout: &MeasureLayout,
    _rect: Rect,
    entry_frac: f64,
) -> Option<f32> {
    let first = measure_layout.notes.first()?;
    let left = measure_layout.notes_left_edge;
    let right = first.center.x;
    let f = entry_frac.clamp(0.0, 1.0) as f32;
    Some(left + (right - left) * f)
}

/// Share of the last note's duration the cursor spends before the barline:
/// its distance from the last note head to the barline, out of that plus the
/// following measure's distance from its left edge to its first note head.
/// The cursor then moves at an even speed across the barline.
pub fn last_note_split(
    measure_layout: &MeasureLayout,
    rect: Rect,
    next_layout: &MeasureLayout,
) -> f64 {
    let (Some(last), Some(first)) = (measure_layout.notes.last(), next_layout.notes.first()) else {
        return 0.5;
    };
    let out = (rect.right() - last.center.x).max(0.0);
    let into = (first.center.x - next_layout.notes_left_edge).max(0.0);
    if out + into <= 0.0 { 0.5 } else { (out / (out + into)) as f64 }
}

/// If `local_tick` is in the last note past `split` (see [`last_note_split`]),
/// returns the entry fraction mapped into `[0, 1]` for
/// [`playback_cursor_entering_x`].
/// Returns `None` for everything else (cursor stays in the current measure or
/// hasn't reached the wrap point yet).
pub fn last_note_entering_frac(measure: &Measure, local_tick: f64, split: f64) -> Option<f64> {
    let ts = measure.time_signature();
    let total_ticks = DEFAULT_GRID.ticks_per_measure(&ts) as f64;
    if total_ticks <= 0.0 {
        return None;
    }
    let beats = measure.beats();
    let last_i = beats.len().checked_sub(1)?;
    let onsets = DEFAULT_GRID.compute_onset_ticks(beats);
    let start = *onsets.get(last_i)? as f64;
    let dur_ticks = DEFAULT_GRID.ticks_of(&beats[last_i].duration).unwrap_or(0) as f64;
    if dur_ticks <= 0.0 {
        return None;
    }
    let t = clamp_tick_to_measure(local_tick, total_ticks);
    if t < start || t >= start + dur_ticks {
        return None;
    }
    let frac = (t - start) / dur_ticks;
    if frac < split {
        return None;
    }
    if split >= 1.0 {
        return Some(1.0);
    }
    Some((frac - split) / (1.0 - split))
}

fn clamp_tick_to_measure(tick: f64, total_ticks: f64) -> f64 {
    if tick.is_sign_negative() {
        return 0.0;
    }
    let m = tick % total_ticks;
    if m.is_nan() { 0.0 } else { m }
}

/// Draw all foreground decorations (notes, beams, tuplets) in their canonical
/// stacking order. Coordinates each phase by reading `measure_layout`; phases
/// can be invoked individually if a caller needs to reorder or skip layers.
/// `lit` gives one note's glyphs (its index) a colour of their own; beams and
/// tuplet brackets are shared, so they keep `color`.
pub fn draw_notes(
    painter: &Painter,
    measure_layout: &MeasureLayout,
    color: Color32,
    lit: Option<(usize, Color32)>,
    opts: &LayoutOpts,
) {
    draw_note_glyphs(painter, &measure_layout.notes, opts, color, lit);
    draw_beams(painter, &measure_layout.beams, color);
    draw_tuplets(painter, &measure_layout.tuplets, opts, color);
}

/// Per-note glyphs: head, stem, flag, dots, accent. The note `lit` names
/// is drawn in its colour.
pub fn draw_note_glyphs(
    painter: &Painter,
    notes: &[NoteLayout],
    opts: &LayoutOpts,
    color: Color32,
    lit: Option<(usize, Color32)>,
) {
    for (idx, note) in notes.iter().enumerate() {
        let color = lit.filter(|&(i, _)| i == idx).map_or(color, |(_, c)| c);
        draw_beat(painter, note, opts, color);
    }
}

/// Beam rectangles between stems.
pub fn draw_beams(painter: &Painter, beams: &[BeamLayout], color: Color32) {
    for seg in beams {
        painter.rect_filled(seg.rect, 0.0, color);
    }
}

/// Tuplet brackets and centered count digits.
pub fn draw_tuplets(
    painter: &Painter,
    tuplets: &[TupletLayout],
    opts: &LayoutOpts,
    color: Color32,
) {
    for t in tuplets {
        let thickness = opts.bracket_thickness();
        for seg in &t.bracket {
            // On whole pixels, like the stems, so the line and its hooks are
            // equally sharp and meet at the same pixel. Square ends: the line
            // and its hook overlap at the corner, so it stays closed.
            let (p1, p2) = (opts.snap_pos(seg.p1, thickness), opts.snap_pos(seg.p2, thickness));
            let half = (p2 - p1).normalized() * thickness * 0.5;
            painter.line_segment([p1 - half, p2 + half], Stroke::new(thickness, color));
        }
        for digit in &t.number {
            draw_glyph(painter, digit, &opts.font_id.family, color);
        }
    }
}

#[expect(clippy::too_many_arguments)]
fn draw_count_labels(
    painter: &Painter,
    measure: &Measure,
    layout: &MeasureLayout,
    rect: Rect,
    baseline_y: f32,
    em: f32,
    base_color: Color32,
    highlight_color: Color32,
    slots: &[CountSlot],
    playback_tick: Option<f64>,
    total_ticks: u32,
) {
    if total_ticks == 0 {
        return;
    }
    let selected: Vec<&CountSlot> = slots.iter().filter(|s| s.label.is_some()).collect();
    if selected.is_empty() {
        return;
    }
    let active_start = active_label_start(&selected, playback_tick, measure);
    let xs = compute_label_xs(&selected, measure, layout, rect, total_ticks);

    let font = FontId::new(em * 0.4, FontFamily::Proportional);
    let label_y = (baseline_y + 0.75 * em).min(rect.bottom() - 0.2 * em);
    let label_color =
        Color32::from_rgba_unmultiplied(base_color.r(), base_color.g(), base_color.b(), 200);

    for (slot, &x) in selected.iter().zip(xs.iter()) {
        let Some(label) = &slot.label else { continue };
        let color =
            if active_start == Some(slot.start_tick) { highlight_color } else { label_color };
        painter.text(pos2(x, label_y), Align2::CENTER_CENTER, label, font.clone(), color);
    }
}

fn active_label_start(
    slots: &[&CountSlot],
    playback_tick: Option<f64>,
    measure: &Measure,
) -> Option<u32> {
    let tick = playback_tick?;
    let total_ticks = DEFAULT_GRID.ticks_per_measure(&measure.time_signature()) as f64;
    if total_ticks <= 0.0 {
        return None;
    }
    if tick.is_nan() {
        return None;
    }
    let mut t = tick;
    if t.is_sign_negative() {
        t = 0.0;
    } else {
        t %= total_ticks;
    }

    slots.iter().find(|s| t >= s.start_tick as f64 && t < s.end_tick as f64).map(|s| s.start_tick)
}

/// Compute X positions for all selected count labels.
///
/// A label marks the musical position `slot.start_tick`. It is placed where
/// the playback cursor would be at that tick:
/// - a note or rest starting exactly there: over its center, so labels sit
///   above the notes they count, whatever the spacing scheme;
/// - otherwise inside the beat that covers the tick, interpolated by tick
///   between that beat's center and the next one's, or the barline
///   (`rect.right()`) after the last beat. A label therefore never leaves its
///   measure: the `&` of a closing quarter sits between the quarter and the
///   barline, not in front of the next measure's `1`.
///
/// Measures without laid-out notes fall back to a uniform proportional
/// mapping of the tick over the content area.
fn compute_label_xs(
    selected: &[&CountSlot],
    measure: &Measure,
    layout: &MeasureLayout,
    rect: Rect,
    total_ticks: u32,
) -> Vec<f32> {
    let left = layout.notes_left_edge;
    let right = rect.right();
    let beats = measure.beats();
    if total_ticks == 0 || beats.is_empty() || layout.notes.len() < beats.len() {
        let content_w = right - left;
        return selected
            .iter()
            .map(|s| left + s.start_tick as f32 / total_ticks.max(1) as f32 * content_w)
            .collect();
    }
    let onsets = DEFAULT_GRID.compute_onset_ticks(beats);
    let x_at = |tick: u32| -> f32 {
        // Last beat starting at or before `tick`; the first beat starts at 0.
        let i = onsets.iter().rposition(|&o| o <= tick).unwrap_or(0);
        let x0 = layout.notes[i].center.x;
        let (next_tick, x1) = match onsets.get(i + 1) {
            Some(&t) => (t, layout.notes[i + 1].center.x),
            None => (total_ticks, right),
        };
        let span = next_tick.saturating_sub(onsets[i]);
        if span == 0 || tick <= onsets[i] {
            return x0;
        }
        let frac = (tick - onsets[i]).min(span) as f32 / span as f32;
        x0 + (x1 - x0) * frac
    };
    selected.iter().map(|s| x_at(s.start_tick).clamp(left, right)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{FontFamily, FontId, Pos2};
    use grooph_measure::counting::Subdivision;
    use grooph_measure::duration::{Duration, NoteValue, e, q, s, t8};
    use grooph_measure::{Beat, Measure, TimeSignature};

    fn opts_for(rect: Rect) -> LayoutOpts {
        let em = 20.0;
        LayoutOpts {
            rect,
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            layout_clef: false,
            layout_time_signature: false,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            stem_down: false,
            shared_line: false,
        }
    }

    #[test]
    fn a_lit_note_is_drawn_in_its_colour() {
        let (base, lit) = (Color32::WHITE, Color32::RED);
        let opts = opts_for(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 100.0)));
        let mut measure = Measure::new(TimeSignature::FOUR_FOUR);
        for i in 0..4 {
            measure.set_beat(i, Beat::note(q())).unwrap();
        }
        let layout = build_measure_layout(&measure, &opts);
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            draw_notes(ui.painter(), &layout, base, Some((1, lit)), &opts);
        });
        output.textures_delta.clear();
        // Each shape's colour and the note nearest it.
        let nearest = |x: f32| {
            (0..layout.notes.len())
                .min_by(|&a, &b| {
                    let d = |i: usize| (layout.notes[i].center.x - x).abs();
                    d(a).total_cmp(&d(b))
                })
                .unwrap()
        };
        let shapes: Vec<(Color32, usize)> = output
            .shapes
            .iter()
            .filter_map(|c| {
                let color = match &c.shape {
                    egui::Shape::LineSegment { stroke, .. } => stroke.color,
                    egui::Shape::Text(text) => text.fallback_color,
                    _ => return None,
                };
                Some((color, nearest(c.shape.visual_bounding_rect().center().x)))
            })
            .collect();
        // Head and stem of the second quarter, nothing else.
        assert!(shapes.iter().filter(|s| s.0 == lit).count() >= 2, "{shapes:?}");
        for (color, note) in shapes {
            assert_eq!(color == lit, note == 1, "{color:?} at note {note}");
        }
    }

    #[test]
    fn a_lit_note_takes_its_dot_flag_and_accent_but_no_beam_or_bracket() {
        use grooph_measure::notation::parse_score;
        let lit = Color32::RED;
        let opts = opts_for(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 100.0)));
        let measure = parse_score("4/4 q. e >e e t8 t8 t8").unwrap().measures.remove(0);
        let layout = build_measure_layout(&measure, &opts);
        assert!(!layout.beams.is_empty() && !layout.tuplets.is_empty());
        let nearest = |x: f32| {
            (0..layout.notes.len())
                .min_by(|&a, &b| {
                    let d = |i: usize| (layout.notes[i].center.x - x).abs();
                    d(a).total_cmp(&d(b))
                })
                .unwrap()
        };
        // The dotted quarter, the flagged eighth, the accented beamed one.
        for idx in [0, 1, 2] {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                draw_notes(ui.painter(), &layout, Color32::WHITE, Some((idx, lit)), &opts);
            });
            output.textures_delta.clear();
            let mut lit_glyphs = 0;
            for c in &output.shapes {
                let at = nearest(c.shape.visual_bounding_rect().center().x);
                match &c.shape {
                    egui::Shape::Rect(rect) => assert_ne!(rect.fill, lit, "a beam is lit"),
                    egui::Shape::LineSegment { stroke, .. } if stroke.color == lit => {
                        assert_eq!(at, idx, "the stem of note {at}");
                    }
                    egui::Shape::Text(text) if text.fallback_color == lit => {
                        assert_eq!(at, idx, "a glyph of note {at}");
                        lit_glyphs += 1;
                    }
                    _ => {}
                }
            }
            // The head and its dot, flag or accent.
            assert!(lit_glyphs >= 2, "note {idx}: {lit_glyphs} glyphs lit");
        }
    }

    fn counting(subdivision: Subdivision) -> CountConfig {
        CountConfig { subdivision, system: Default::default() }
    }

    fn ands_config() -> CountConfig {
        counting(Subdivision::Halves)
    }

    /// Collect per-label x positions in slot-start-tick order, using the same
    /// pipeline as `draw_count_labels`.
    fn label_xs(
        measure: &Measure,
        layout: &MeasureLayout,
        rect: Rect,
        config: &CountConfig,
    ) -> Vec<f32> {
        let slots = build_count_slots(measure, config);
        let selected: Vec<&CountSlot> = slots.iter().filter(|s| s.label.is_some()).collect();
        let total_ticks = DEFAULT_GRID.ticks_per_measure(&measure.time_signature());
        compute_label_xs(&selected, measure, layout, rect, total_ticks)
    }

    #[test]
    fn cursor_on_another_measures_layout_is_none() {
        // The staff's upper voice laid out with eight eighths, asked about a
        // bar of ten notes (the counter rhythm): no cursor, no panic.
        use grooph_measure::notation::parse_score;
        let eighths = parse_score("4/4 e e e e e e e e").unwrap().measures.remove(0);
        let tens = parse_score("4/4 qt8 qt8 qt8 qt8 qt8 qt8 qt8 qt8 qt8 qt8").unwrap();
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let layout = build_measure_layout(&eighths, &opts_for(rect));
        let bar_ticks = DEFAULT_GRID.ticks_per_measure(&TimeSignature::FOUR_FOUR) as f64;
        let tick = 0.85 * bar_ticks;
        assert_eq!(playback_cursor_x(&tens.measures[0], &layout, rect, tick, 0.5), None);
    }

    #[test]
    fn beat_anchors_sit_on_rests_and_inside_held_notes() {
        use grooph_measure::notation::parse_score;
        let m = parse_score("4/4 r:e e q h").unwrap().measures.remove(0);
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let layout = build_measure_layout(&m, &opts_for(rect));
        let quarter = DEFAULT_GRID.ticks_of(&q()).unwrap() as f64;
        let x = |beat: f64| beat_anchor_x(&m, &layout, rect, beat * quarter).unwrap();
        let heads: Vec<f32> = layout.notes.iter().map(|n| n.center.x).collect();
        // Beat 1 on the eighth rest, 2 and 3 on their notes.
        assert_eq!(x(0.0), heads[0]);
        assert_eq!(x(1.0), heads[2]);
        assert_eq!(x(2.0), heads[3]);
        // Beat 4 halfway from the half note to the barline.
        assert!((x(3.0) - (heads[3] + rect.right()) / 2.0).abs() < 1e-3);
    }

    #[test]
    fn two_quarter_rests_two_four_ands_stay_inside_the_measure() {
        // 2/4 + two quarter rests + "Ands" subdivision. `1` and `2` sit over
        // the rests (at their onsets with proportional spacing). The first `&`
        // is halfway between the rests; the trailing `&` halfway between the
        // last rest and the barline, not on it (#13).
        let mut m = Measure::new(TimeSignature::TWO_FOUR);
        m.set_beat(0, Beat::rest(q())).unwrap();
        m.set_beat(1, Beat::rest(q())).unwrap();

        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let opts = opts_for(rect);
        let layout = build_measure_layout(&m, &opts);

        let xs = label_xs(&m, &layout, rect, &ands_config());
        assert_eq!(xs.len(), 4, "expected 1 & 2 &");

        let (r0, r1) = (layout.notes[0].center.x, layout.notes[1].center.x);
        let expected = [r0, (r0 + r1) / 2.0, r1, (r1 + rect.right()) / 2.0];
        for (i, (got, exp)) in xs.iter().zip(expected.iter()).enumerate() {
            assert!((got - exp).abs() < 0.5, "label {i}: got {got}, expected {exp}",);
        }
    }

    #[test]
    fn sixteenth_labels_are_evenly_spaced_up_to_the_barline() {
        // Quarter, two eighths, quarter, quarter: notes sit at their onsets,
        // so every sixteenth label is the same distance from the next, also
        // after the first quarter and in the last beat before the barline.
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.set_beat(0, Beat::note(q())).unwrap();
        m.set_beat(1, Beat::note(e())).unwrap();
        m.set_beat(2, Beat::note(e())).unwrap();
        m.set_beat(3, Beat::note(q())).unwrap();
        m.set_beat(4, Beat::note(q())).unwrap();

        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let opts = opts_for(rect);
        let layout = build_measure_layout(&m, &opts);

        let xs = label_xs(&m, &layout, rect, &counting(Subdivision::Quarters));
        assert_eq!(xs.len(), 16);
        let step = (rect.right() - xs[0]) / 16.0;
        for (i, w) in xs.windows(2).enumerate() {
            let gap = w[1] - w[0];
            assert!((gap - step).abs() < 0.5, "gap after label {i}: {gap}, expected {step}");
        }
    }

    #[test]
    fn four_eighth_notes_align_with_note_centers() {
        // When notes happen to sit on slot tick centers, the proportional
        // formula reproduces the note centers — labels align with notes.
        let mut m = Measure::new(TimeSignature::TWO_FOUR);
        for i in 0..4 {
            m.set_beat(i, Beat::note(e())).unwrap();
        }

        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let opts = opts_for(rect);
        let layout = build_measure_layout(&m, &opts);

        let xs = label_xs(&m, &layout, rect, &ands_config());
        assert_eq!(xs.len(), 4);
        for (i, x) in xs.iter().enumerate() {
            let note_x = layout.notes[i].center.x;
            assert!(
                (x - note_x).abs() < 0.5,
                "label {i} at {x} should coincide with note at {note_x}",
            );
        }
    }

    #[test]
    fn beat_labels_of_compound_beats_sit_over_their_downbeats() {
        // 6/8 in eighth rests, counted in beats: each `1` and `2` spans a
        // dotted quarter and sits over the first rest of its beat.
        let mut m = Measure::new(TimeSignature::SIX_EIGHT);
        for i in 0..6 {
            m.set_beat(i, Beat::rest(e())).unwrap();
        }

        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let opts = opts_for(rect);
        let layout = build_measure_layout(&m, &opts);

        let xs = label_xs(&m, &layout, rect, &counting(Subdivision::Beats));
        assert_eq!(xs.len(), 2);
        assert!((xs[0] - layout.notes[0].center.x).abs() < 0.5);
        assert!((xs[1] - layout.notes[3].center.x).abs() < 0.5);
    }

    #[test]
    fn sixteenths_then_eighths_anchors_each_label_to_first_onset() {
        // 2/4: four 16ths on beat 1 + two 8ths on beat 2, with "Ands"
        // labels. Beat 1's `1` slot contains two 16th onsets (the 1st and
        // 2nd 16th) — `1` must anchor to the *first* of them so the label
        // sits over the 16th that musically *is* beat 1. Same for `&`,
        // which anchors to the 3rd 16th (the one that musically *is* the
        // `&` of beat 1).
        let mut m = Measure::new(TimeSignature::TWO_FOUR);
        for i in 0..4 {
            m.set_beat(i, Beat::note(s())).unwrap();
        }
        m.set_beat(4, Beat::note(e())).unwrap();
        m.set_beat(5, Beat::note(e())).unwrap();

        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let opts = opts_for(rect);
        let layout = build_measure_layout(&m, &opts);

        let xs = label_xs(&m, &layout, rect, &ands_config());
        assert_eq!(xs.len(), 4);

        // `1` over the 1st 16th, `&` over the 3rd 16th.
        assert!(
            (xs[0] - layout.notes[0].center.x).abs() < 0.5,
            "`1` should sit over the 1st 16th: got {}, expected {}",
            xs[0],
            layout.notes[0].center.x,
        );
        assert!(
            (xs[1] - layout.notes[2].center.x).abs() < 0.5,
            "`&` should sit over the 3rd 16th: got {}, expected {}",
            xs[1],
            layout.notes[2].center.x,
        );
        // `2` and last `&` over the two 8ths.
        assert!(
            (xs[2] - layout.notes[4].center.x).abs() < 0.5,
            "`2` should sit over the 1st 8th: got {}, expected {}",
            xs[2],
            layout.notes[4].center.x,
        );
        assert!(
            (xs[3] - layout.notes[5].center.x).abs() < 0.5,
            "last `&` should sit over the 2nd 8th: got {}, expected {}",
            xs[3],
            layout.notes[5].center.x,
        );
    }

    #[test]
    fn mixed_quarter_then_two_eighths_anchors_each_label() {
        // 2/4: quarter rest + 8th note + 8th note, with `1 & 2 &`.
        // `1` anchors to the quarter rest (W/4), `2` and the trailing `&`
        // anchor to the two 8ths (5W/8 and 7W/8). The first `&` is unanchored
        // and interpolates between `1` and `2` — landing at the visual
        // midpoint of the quarter rest and the first 8th.
        let mut m = Measure::new(TimeSignature::TWO_FOUR);
        m.set_beat(0, Beat::rest(q())).unwrap();
        m.set_beat(1, Beat::note(e())).unwrap();
        m.set_beat(2, Beat::note(e())).unwrap();

        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let opts = opts_for(rect);
        let layout = build_measure_layout(&m, &opts);

        let xs = label_xs(&m, &layout, rect, &ands_config());
        assert_eq!(xs.len(), 4);

        // `1` over the quarter rest, `2` and last `&` over the two 8ths.
        assert!((xs[0] - layout.notes[0].center.x).abs() < 0.5, "`1` should sit over quarter rest");
        assert!((xs[2] - layout.notes[1].center.x).abs() < 0.5, "`2` should sit over first 8th");
        assert!(
            (xs[3] - layout.notes[2].center.x).abs() < 0.5,
            "last `&` should sit over second 8th"
        );

        // The first `&` lies between `1` and `2` (interpolated, not anchored).
        assert!(xs[1] > xs[0] && xs[1] < xs[2], "first `&` between `1` and `2`: {:?}", xs);
    }

    #[test]
    fn three_quarter_rests_three_four_ands_between_rests() {
        // 3/4 with three quarter rests + Ands → "1 & 2 & 3 &". Numbers over
        // the rests, every `&` halfway to the next rest or the barline.
        let mut m = Measure::new(TimeSignature::THREE_FOUR);
        for i in 0..3 {
            m.set_beat(i, Beat::rest(q())).unwrap();
        }

        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(600.0, 100.0));
        let opts = opts_for(rect);
        let layout = build_measure_layout(&m, &opts);

        let xs = label_xs(&m, &layout, rect, &ands_config());
        assert_eq!(xs.len(), 6);
        let rests: Vec<f32> = layout.notes.iter().map(|n| n.center.x).collect();
        let ends = [rests[1], rests[2], rect.right()];
        for beat in 0..3 {
            assert!((xs[2 * beat] - rests[beat]).abs() < 0.5, "{xs:?}");
            let and = (rests[beat] + ends[beat]) / 2.0;
            assert!((xs[2 * beat + 1] - and).abs() < 0.5, "{xs:?}");
        }
    }

    #[test]
    fn closing_rest_keeps_its_and_before_the_barline() {
        // Regression for #13: 8ths are laid out tighter than quarter rests, so
        // extrapolating the spacing of the 8ths pushed the `&` of a closing
        // quarter rest past the barline, right in front of the next `1`.
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.set_beat(0, Beat::note(e())).unwrap();
        m.set_beat(1, Beat::note(e())).unwrap();
        m.set_beat(2, Beat::rest(q())).unwrap();
        m.set_beat(3, Beat::note(e())).unwrap();
        m.set_beat(4, Beat::note(e())).unwrap();
        m.set_beat(5, Beat::rest(q())).unwrap();

        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(500.0, 100.0));
        let opts = opts_for(rect);
        let mut layout = build_measure_layout(&m, &opts);
        // Squeeze the last rest towards the barline, as the stable staff
        // layout does with a short measure tail.
        let last = layout.notes.len() - 1;
        layout.notes[last].center.x = rect.right() - 30.0;

        let xs = label_xs(&m, &layout, rect, &ands_config());
        assert_eq!(xs.len(), 8);
        assert!(xs.windows(2).all(|w| w[0] < w[1]), "labels out of order: {xs:?}");
        let rest_x = layout.notes[last].center.x;
        assert!(xs[7] > rest_x && xs[7] < rect.right(), "trailing `&` at {}: {xs:?}", xs[7]);
    }

    #[test]
    fn and_inside_a_dotted_eighth_is_not_put_on_the_sixteenth() {
        // Dotted 8th + 16th: the `&` falls inside the dotted 8th. It belongs
        // between the two notes, not over the 16th (which is the "a").
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        m.set_beat(0, Beat::note(Duration::Dotted { base: NoteValue::Eighth, dots: 1 })).unwrap();
        m.set_beat(1, Beat::note(s())).unwrap();

        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(300.0, 100.0));
        let opts = opts_for(rect);
        let layout = build_measure_layout(&m, &opts);

        let xs = label_xs(&m, &layout, rect, &ands_config());
        assert_eq!(xs.len(), 2);
        let (x0, x1) = (layout.notes[0].center.x, layout.notes[1].center.x);
        let expected = x0 + (x1 - x0) * 2.0 / 3.0;
        assert!((xs[1] - expected).abs() < 0.5, "got {}, expected {expected}", xs[1]);
    }

    #[test]
    fn and_over_eighth_triplets_falls_between_second_and_third_note() {
        // The `&` (half a beat) lies inside the 2nd triplet eighth (1/3..2/3),
        // halfway between the 2nd and 3rd note heads; it is not the 3rd note.
        let mut m = Measure::new(TimeSignature::ONE_FOUR);
        for i in 0..3 {
            m.set_beat(i, Beat::note(t8())).unwrap();
        }

        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(300.0, 100.0));
        let opts = opts_for(rect);
        let layout = build_measure_layout(&m, &opts);

        let xs = label_xs(&m, &layout, rect, &ands_config());
        assert_eq!(xs.len(), 2);
        assert!((xs[0] - layout.notes[0].center.x).abs() < 0.5);
        let (x1, x2) = (layout.notes[1].center.x, layout.notes[2].center.x);
        let expected = (x1 + x2) / 2.0;
        assert!((xs[1] - expected).abs() < 0.5, "got {}, expected {expected}", xs[1]);
    }
}
