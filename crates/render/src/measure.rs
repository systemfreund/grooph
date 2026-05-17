use crate::beat::draw_beat;
use eframe::egui;
use eframe::egui::{Align2, Color32, FontFamily, FontId, Painter, Rangef, Rect, Stroke, pos2};
use grooph_layout::glyphs;
use grooph_layout::pixel_layout::{
    BeamLayout, LayoutOpts, MeasureLayout, NoteLayout, TimeSignatureLayout, TupletLayout,
    build_measure_layout,
};
use grooph_measure::Measure;
use grooph_measure::counting::{ColorId, CountConfig, CountSlot, build_count_slots};
use grooph_measure::grid::DEFAULT_GRID;
use std::collections::HashMap;

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
        playback_tick,
        count_config,
        true,
        None,
    );
    measure_layout
}

/// Render a measure given a pre-built `MeasureLayout`.
///
/// `draw_staff_line` controls whether the horizontal staff line is drawn —
/// the multi-measure renderer turns it off and draws one continuous line per
/// system instead. All cursor/playback drawing is scoped to this measure; the
/// caller decides whether to forward `cursor_idx` / `playback_tick` based on
/// the active measure.
///
/// `cursor_x` is the pre-computed playback cursor X (in the same coordinate
/// space as `measure_layout`). The cursor's home measure depends on the
/// playback phase: during the first half of a measure's last note the cursor
/// lives in *this* measure, during the second half it visually moves into the
/// following measure. The caller (`draw_staff`) decides via `current_cursor_x`
/// and passes `Some(x)` only to the measure that actually hosts the cursor.
///
/// The function orchestrates independent decoration phases — counting layer,
/// staff line, clef + time signature, notes/beams/tuplets, edit cursor,
/// playback cursor. Each phase is a small helper; reorder or extend by
/// editing this function.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_measure_at(
    ui: &mut egui::Ui,
    measure: &Measure,
    measure_layout: &MeasureLayout,
    opts: &LayoutOpts,
    cursor_idx: Option<usize>,
    playback_tick: Option<f64>,
    count_config: Option<&CountConfig>,
    draw_staff_line: bool,
    cursor_x: Option<f32>,
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
            config,
            ui.visuals().dark_mode,
            ui.visuals().text_color(),
            ui.visuals().selection.stroke.color,
            playback_tick,
        );
    }

    if draw_staff_line {
        draw_staff_line_segment(painter, rect, opts.em, color);
    }

    draw_clef(painter, measure_layout.clef_pos, &opts.font_id, color);
    draw_time_signature(
        painter,
        measure_layout.time_signature.as_ref(),
        measure,
        &opts.font_id,
        color,
    );

    draw_notes(painter, measure_layout, color, opts);

    if let Some(idx) = cursor_idx {
        draw_edit_cursor(ui, painter, measure_layout, idx, opts);
    }

    if let Some(x) = cursor_x {
        draw_playback_cursor(painter, rect, x, opts.em, ui.visuals().selection.stroke.color);
    }
}

fn draw_staff_line_segment(painter: &Painter, rect: Rect, em: f32, color: Color32) {
    painter.hline(
        Rangef::new(rect.left(), rect.right()),
        rect.center().y,
        Stroke::new(0.02 * em, color),
    );
}

fn draw_clef(painter: &Painter, clef_pos: Option<egui::Pos2>, font_id: &FontId, color: Color32) {
    if let Some(pos) = clef_pos {
        painter.text(
            pos,
            Align2::CENTER_CENTER,
            glyphs::GLYPH_CLEF_PERCUSSION.to_string(),
            font_id.clone(),
            color,
        );
    }
}

fn draw_time_signature(
    painter: &Painter,
    ts_layout: Option<&TimeSignatureLayout>,
    measure: &Measure,
    font_id: &FontId,
    color: Color32,
) {
    let Some(ts_layout) = ts_layout else { return };
    let ts = measure.time_signature();
    let top_digits = glyphs::ts_glyphs(ts.beats);
    let bot_digits = glyphs::ts_glyphs(ts.beat_unit);
    for (p, ch) in ts_layout.beats.iter().zip(top_digits.iter()) {
        painter.text(*p, Align2::CENTER_CENTER, ch.to_string(), font_id.clone(), color);
    }
    for (p, ch) in ts_layout.beat_unit.iter().zip(bot_digits.iter()) {
        painter.text(*p, Align2::CENTER_CENTER, ch.to_string(), font_id.clone(), color);
    }
}

fn draw_edit_cursor(
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

fn draw_playback_cursor(painter: &Painter, rect: Rect, x: f32, em: f32, base: Color32) {
    let top = rect.center().y + 0.7 * em;
    let bottom = rect.center().y - 0.7 * em;
    let cursor_color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 100);
    painter.vline(x, Rangef::new(top, bottom), Stroke::new(0.1 * em, cursor_color));
}

#[allow(clippy::too_many_arguments)]
fn draw_count_layer(
    painter: &Painter,
    measure: &Measure,
    measure_layout: &MeasureLayout,
    opts: &LayoutOpts,
    config: &CountConfig,
    dark_mode: bool,
    text_color: Color32,
    highlight_color: Color32,
    playback_tick: Option<f64>,
) {
    let slots = build_count_slots(measure, config);
    if slots.is_empty() {
        return;
    }
    let total_ticks = DEFAULT_GRID.ticks_per_measure(&measure.time_signature());
    draw_count_underlay(painter, measure, measure_layout, opts.rect, opts.em, dark_mode, &slots);
    draw_count_labels(
        painter,
        measure,
        measure_layout,
        opts.rect,
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
/// - Last note, `frac < 0.5`: linear from `notes[last].center.x` to
///   `rect.right()` (cursor extends past the last note head, fading out).
/// - Last note, `frac >= 0.5`: returns `None`. The cursor visually moves into
///   the *following* measure — call [`playback_cursor_entering_x`] there.
/// - Trailing gap (tick past every beat's end, i.e. an incomplete measure):
///   `Some(rect.right())`. Cursor parks at the right edge.
///
/// Returns `None` if the measure has no notes or zero total ticks.
pub fn playback_cursor_x(
    measure: &Measure,
    measure_layout: &MeasureLayout,
    rect: Rect,
    tick: f64,
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
            let x0 = measure_layout.notes[i].center.x;

            if i + 1 < measure_layout.notes.len() {
                // Normal note: interpolate to the next note in this measure.
                let x1 = measure_layout.notes[i + 1].center.x;
                return Some(x0 + ((x1 - x0) * frac as f32));
            }

            // Last note: split into two phases. Phase 1 covers frac in [0, 0.5)
            // and walks from the note head to the measure's right edge over
            // *half* of the note duration. Phase 2 (frac >= 0.5) renders in
            // the following measure via `playback_cursor_entering_x`.
            if frac < 0.5 {
                let phase1_frac = (frac * 2.0) as f32;
                return Some(x0 + (rect.right() - x0) * phase1_frac);
            }
            return None;
        }
    }

    // Trailing gap: tick lies past the last beat's end (incomplete measure).
    // Hold the cursor at the right edge — visually consistent with "done".
    Some(rect.right())
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

/// If `local_tick` is in the last note's second half (frac >= 0.5), returns
/// the entry fraction mapped into `[0, 1]` for [`playback_cursor_entering_x`].
/// Returns `None` for everything else (cursor stays in the current measure or
/// hasn't reached the wrap point yet).
pub fn last_note_entering_frac(measure: &Measure, local_tick: f64) -> Option<f64> {
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
    if frac < 0.5 {
        return None;
    }
    Some((frac - 0.5) * 2.0)
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
pub fn draw_notes(
    painter: &Painter,
    measure_layout: &MeasureLayout,
    color: Color32,
    opts: &LayoutOpts,
) {
    draw_note_glyphs(painter, &measure_layout.notes, opts, color);
    draw_beams(painter, &measure_layout.beams, color);
    draw_tuplets(painter, &measure_layout.tuplets, opts, color);
}

/// Per-note glyphs: head, stem, flag, dots, accent, optional debug boxes.
pub fn draw_note_glyphs(
    painter: &Painter,
    notes: &[NoteLayout],
    opts: &LayoutOpts,
    color: Color32,
) {
    for note in notes {
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
        for seg in &t.bracket {
            painter.line_segment([seg.p1, seg.p2], Stroke::new(opts.bracket_thickness(), color));
        }
        let digits = glyphs::tuplet_glyphs(t.count);
        painter.text(t.number_center, Align2::CENTER_CENTER, digits, t.number_font.clone(), color);
    }
}

struct TickMapper {
    onsets: Vec<u32>,
    boundary_x: Vec<f32>,
    total_ticks: u32,
}

impl TickMapper {
    fn tick_to_boundary_x(&self, tick: u32) -> f32 { self.interpolate(&self.boundary_x, tick) }

    fn interpolate(&self, anchors: &[f32], tick: u32) -> f32 {
        let t = tick.min(self.total_ticks);
        let mut i = 0usize;
        while i + 1 < self.onsets.len() && t >= self.onsets[i + 1] {
            i += 1;
        }
        let start_tick = self.onsets.get(i).copied().unwrap_or(0);
        let end_tick =
            if i + 1 < self.onsets.len() { self.onsets[i + 1] } else { self.total_ticks };
        let x0 = *anchors.get(i).unwrap_or(&anchors[0]);
        let x1 = *anchors.get(i + 1).unwrap_or(&anchors[anchors.len() - 1]);
        let span = end_tick.saturating_sub(start_tick);
        if span == 0 {
            return x0;
        }
        let frac = (t - start_tick) as f32 / span as f32;
        x0 + (x1 - x0) * frac
    }
}

fn build_tick_mapper(measure: &Measure, layout: &MeasureLayout, rect: Rect) -> Option<TickMapper> {
    let beats = measure.beats();
    if beats.is_empty() || layout.notes.is_empty() {
        return None;
    }
    let onsets = DEFAULT_GRID.compute_onset_ticks(beats);
    let total_ticks = DEFAULT_GRID.ticks_per_measure(&measure.time_signature());
    if total_ticks == 0 {
        return None;
    }
    let mut boundary_x = Vec::with_capacity(layout.notes.len() + 1);
    boundary_x.push(layout.notes_left_edge);
    for i in 1..layout.notes.len() {
        let x_prev = layout.notes[i - 1].center.x;
        let x_cur = layout.notes[i].center.x;
        boundary_x.push((x_prev + x_cur) * 0.5);
    }
    boundary_x.push(rect.right());

    Some(TickMapper { onsets, boundary_x, total_ticks })
}

fn draw_count_underlay(
    painter: &Painter,
    measure: &Measure,
    layout: &MeasureLayout,
    rect: Rect,
    em: f32,
    dark_mode: bool,
    slots: &[CountSlot],
) {
    let mapper = match build_tick_mapper(measure, layout, rect) {
        Some(mapper) => mapper,
        None => return,
    };
    let mut color_slots: Vec<&CountSlot> = slots.iter().filter(|s| s.color.is_some()).collect();
    if color_slots.is_empty() {
        return;
    }
    color_slots.sort_by(|a, b| a.priority.cmp(&b.priority));

    let y0 = rect.center().y - 0.85 * em;
    let y1 = rect.center().y + 0.85 * em;
    let alpha = if dark_mode { 85 } else { 65 };

    for slot in color_slots {
        let Some(color_id) = slot.color else { continue };
        let x0 = mapper.tick_to_boundary_x(slot.start_tick);
        let x1 = mapper.tick_to_boundary_x(slot.end_tick);
        if x1 <= x0 + 0.5 {
            continue;
        }
        let color = count_color(color_id, alpha);
        let r = Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        painter.rect_filled(r, 0.0, color);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_count_labels(
    painter: &Painter,
    measure: &Measure,
    layout: &MeasureLayout,
    rect: Rect,
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
    let label_slots: Vec<&CountSlot> = slots.iter().filter(|s| s.label.is_some()).collect();
    if label_slots.is_empty() {
        return;
    }
    let selected = select_label_slots(&label_slots);
    let active_start = active_label_start(&selected, playback_tick, measure);
    let xs = compute_label_xs(&selected, measure, layout, rect, total_ticks);

    let font = FontId::new(em * 0.4, FontFamily::Proportional);
    let label_y = (rect.center().y + 1.35 * em).min(rect.bottom() - 0.2 * em);
    let label_color =
        Color32::from_rgba_unmultiplied(base_color.r(), base_color.g(), base_color.b(), 200);
    let highlight = Color32::from_rgba_unmultiplied(
        highlight_color.r(),
        highlight_color.g(),
        highlight_color.b(),
        255,
    );

    for (slot, &x) in selected.iter().zip(xs.iter()) {
        let Some(label) = &slot.label else { continue };
        let color = if active_start == Some(slot.start_tick) { highlight } else { label_color };
        painter.text(pos2(x, label_y), Align2::CENTER_CENTER, label, font.clone(), color);
    }
}

fn select_label_slots<'a>(slots: &'a [&'a CountSlot]) -> Vec<&'a CountSlot> {
    let mut best: HashMap<u32, usize> = HashMap::new();
    for (i, slot) in slots.iter().enumerate() {
        best.entry(slot.start_tick)
            .and_modify(|idx| {
                let current = slots[*idx];
                if slot.priority > current.priority {
                    *idx = i;
                } else if slot.priority == current.priority {
                    let cur_span = current.end_tick.saturating_sub(current.start_tick);
                    let new_span = slot.end_tick.saturating_sub(slot.start_tick);
                    if new_span < cur_span {
                        *idx = i;
                    }
                }
            })
            .or_insert(i);
    }
    let mut out: Vec<&CountSlot> = best.values().map(|idx| slots[*idx]).collect();
    out.sort_by(|a, b| a.start_tick.cmp(&b.start_tick));
    out
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

    let mut best: Option<&CountSlot> = None;
    for slot in slots {
        let start = slot.start_tick as f64;
        let end = slot.end_tick as f64;
        if t >= start && t < end {
            best = match best {
                None => Some(*slot),
                Some(cur) => {
                    if slot.priority > cur.priority {
                        Some(*slot)
                    } else if slot.priority == cur.priority {
                        let cur_span = cur.end_tick.saturating_sub(cur.start_tick);
                        let new_span = slot.end_tick.saturating_sub(slot.start_tick);
                        if new_span < cur_span { Some(*slot) } else { Some(cur) }
                    } else {
                        Some(cur)
                    }
                }
            };
        }
    }
    best.map(|s| s.start_tick)
}

/// Compute X positions for all selected count labels.
///
/// **Anchor rule.** A slot is *anchored* to the **first** note whose onset
/// falls in `[slot.start_tick, slot.end_tick)`. The anchor's X is that note's
/// `center.x` — so the label sits directly above the note that begins on the
/// slot's musical position, regardless of the spacing scheme (proportional or
/// uniform). Slots that no onset touches (covered by a longer sustaining
/// note) are left unanchored. Sub-slot subdivisions like 16ths inside an `&`
/// slot still anchor to the first 16th in the slot — which is the one that
/// musically *is* the `&`.
///
/// **Filling the gaps.** With ≥2 anchors we treat them as keypoints in a
/// piecewise-linear map from `slot.center_tick` to X, and:
/// - interpolate unanchored slots between surrounding anchors,
/// - extrapolate unanchored slots beyond the last anchor using the slope of
///   the last two anchors. In the common 2/4 + two-quarter-rests + ands case
///   this puts the trailing `&` exactly at the barline — visually the
///   midpoint between `2` of this measure and `1` of the next.
///
/// **Fallback.** With fewer than 2 anchors (e.g. a whole-note slot covering
/// the entire measure) we cannot determine a meaningful slope, so we fall
/// back to a uniform proportional mapping (`slot_center_tick / total_ticks`).
fn compute_label_xs(
    selected: &[&CountSlot],
    measure: &Measure,
    layout: &MeasureLayout,
    rect: Rect,
    total_ticks: u32,
) -> Vec<f32> {
    let content_w = rect.right() - layout.notes_left_edge;
    let slot_center = |s: &CountSlot| (s.start_tick + s.end_tick) as f32 * 0.5;
    let proportional =
        |center: f32| layout.notes_left_edge + center / total_ticks as f32 * content_w;

    let fallback =
        || -> Vec<f32> { selected.iter().map(|s| proportional(slot_center(s))).collect() };

    if total_ticks == 0 || selected.is_empty() {
        return fallback();
    }
    let beats = measure.beats();
    if beats.is_empty() || layout.notes.is_empty() {
        return fallback();
    }
    let onsets = DEFAULT_GRID.compute_onset_ticks(beats);

    // (slot_index_in_selected, slot_center_tick, anchor_x)
    let mut anchors: Vec<(usize, f32, f32)> = Vec::new();
    for (i, slot) in selected.iter().enumerate() {
        for (note_i, &onset) in onsets.iter().enumerate() {
            if onset >= slot.end_tick {
                break;
            }
            if onset >= slot.start_tick && note_i < layout.notes.len() {
                anchors.push((i, slot_center(slot), layout.notes[note_i].center.x));
                break;
            }
        }
    }

    if anchors.len() < 2 {
        return fallback();
    }

    selected
        .iter()
        .enumerate()
        .map(|(i, slot)| {
            let sc = slot_center(slot);
            if let Some(&(_, _, x)) = anchors.iter().find(|(idx, _, _)| *idx == i) {
                return x;
            }
            let before = anchors.iter().rev().find(|(_, t, _)| *t < sc).copied();
            let after = anchors.iter().find(|(_, t, _)| *t > sc).copied();
            match (before, after) {
                (Some((_, t1, x1)), Some((_, t2, x2))) => {
                    let frac = (sc - t1) / (t2 - t1);
                    x1 + (x2 - x1) * frac
                }
                (Some((_, t1, x1)), None) => {
                    let n = anchors.len();
                    let (_, ta, xa) = anchors[n - 2];
                    let (_, tb, xb) = anchors[n - 1];
                    let slope = (xb - xa) / (tb - ta);
                    x1 + slope * (sc - t1)
                }
                (None, Some((_, t1, x1))) => {
                    let (_, ta, xa) = anchors[0];
                    let (_, tb, xb) = anchors[1];
                    let slope = (xb - xa) / (tb - ta);
                    x1 - slope * (t1 - sc)
                }
                (None, None) => proportional(sc),
            }
        })
        .collect()
}

fn count_color(id: ColorId, alpha: u8) -> Color32 {
    let palette = [
        Color32::from_rgb(255, 231, 173),
        Color32::from_rgb(205, 233, 255),
        Color32::from_rgb(206, 245, 220),
        Color32::from_rgb(255, 215, 202),
        Color32::from_rgb(230, 240, 210),
        Color32::from_rgb(235, 235, 235),
    ];
    let base = palette[id.0 as usize % palette.len()];
    Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{FontFamily, FontId, Pos2};
    use grooph_layout::pixel_layout::GlyphMetrics;
    use grooph_measure::counting::{CountLayer, CountScope, LabelPattern, LabelToken, Subdiv};
    use grooph_measure::duration::{e, q, s};
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
            accent_below: false,
            proportional_spacing: true,
            debug_bbox: false,
            metrics: GlyphMetrics::debug(em),
        }
    }

    fn ands_config() -> CountConfig {
        let mut layer = CountLayer::new(0, CountScope::BeatUnit, Subdiv::Fixed(2));
        layer.labels = Some(LabelPattern::ands());
        CountConfig::new(vec![layer])
    }

    fn primary_config() -> CountConfig {
        let mut layer = CountLayer::new(0, CountScope::Measure, Subdiv::Fixed(1));
        layer.labels = Some(LabelPattern::single(LabelToken::BeatNum));
        CountConfig::new(vec![layer])
    }

    /// Collect per-label x positions in slot-start-tick order, using the same
    /// pipeline as `draw_count_labels` (with `select_label_slots`).
    fn label_xs(
        measure: &Measure,
        layout: &MeasureLayout,
        rect: Rect,
        config: &CountConfig,
    ) -> Vec<f32> {
        let slots = build_count_slots(measure, config);
        let label_slots: Vec<&CountSlot> = slots.iter().filter(|s| s.label.is_some()).collect();
        let selected = select_label_slots(&label_slots);
        let total_ticks = DEFAULT_GRID.ticks_per_measure(&measure.time_signature());
        compute_label_xs(&selected, measure, layout, rect, total_ticks)
    }

    #[test]
    fn two_quarter_rests_two_four_ands_anchor_to_notes() {
        // 2/4 + two quarter rests + "Ands" subdivision.
        // The numeric labels `1` and `2` are anchored to the quarter-rest
        // centers (W/4 and 3W/4 with proportional spacing). The `&`s land
        // halfway between adjacent anchors — the trailing `&` extrapolates
        // to the barline, which is the visual midpoint between `2` of this
        // measure and `1` of the next.
        let mut m = Measure::new(TimeSignature::TWO_FOUR);
        m.set_beat(0, Beat::rest(q())).unwrap();
        m.set_beat(1, Beat::rest(q())).unwrap();

        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let opts = opts_for(rect);
        let layout = build_measure_layout(&m, &opts);

        let xs = label_xs(&m, &layout, rect, &ands_config());
        assert_eq!(xs.len(), 4, "expected 1 & 2 &");

        let w = rect.right() - layout.notes_left_edge;
        let l = layout.notes_left_edge;
        let expected = [l + w / 4.0, l + w / 2.0, l + 3.0 * w / 4.0, l + w];
        for (i, (got, exp)) in xs.iter().zip(expected.iter()).enumerate() {
            assert!((got - exp).abs() < 0.5, "label {i}: got {got}, expected {exp}",);
        }

        // `1` and `2` align with the rest centers.
        assert!((xs[0] - layout.notes[0].center.x).abs() < 0.5);
        assert!((xs[2] - layout.notes[1].center.x).abs() < 0.5);

        // Uniform spacing across the full set.
        let gaps: Vec<f32> = xs.windows(2).map(|w| w[1] - w[0]).collect();
        for g in &gaps {
            assert!((g - gaps[0]).abs() < 0.5, "non-uniform spacing: {:?}", gaps);
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
    fn whole_measure_primary_label_at_midpoint() {
        // 4/4 with a single `1` label spanning the whole measure — slot
        // center tick = T/2 → label at the midpoint of the content area.
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        for i in 0..4 {
            m.set_beat(i, Beat::rest(q())).unwrap();
        }

        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 100.0));
        let opts = opts_for(rect);
        let layout = build_measure_layout(&m, &opts);

        let xs = label_xs(&m, &layout, rect, &primary_config());
        assert_eq!(xs.len(), 1);
        let expected = (layout.notes_left_edge + rect.right()) * 0.5;
        assert!((xs[0] - expected).abs() < 0.5);
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
    fn three_quarter_rests_three_four_ands_uniform() {
        // 3/4 with three quarter rests + Ands → six labels "1 & 2 & 3 &",
        // uniform W/6 spacing. Same bug pattern as 2/4.
        let mut m = Measure::new(TimeSignature::THREE_FOUR);
        for i in 0..3 {
            m.set_beat(i, Beat::rest(q())).unwrap();
        }

        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(600.0, 100.0));
        let opts = opts_for(rect);
        let layout = build_measure_layout(&m, &opts);

        let xs = label_xs(&m, &layout, rect, &ands_config());
        assert_eq!(xs.len(), 6);

        let gaps: Vec<f32> = xs.windows(2).map(|w| w[1] - w[0]).collect();
        for g in &gaps {
            assert!((g - gaps[0]).abs() < 0.5, "non-uniform spacing in 3/4: {:?}", gaps);
        }
    }
}
