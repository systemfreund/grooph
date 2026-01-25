use crate::layout::pixel_layout::{LayoutOpts, MeasureLayout, build_measure_layout};
use crate::measure::counting::{build_count_slots, ColorId, CountConfig, CountSlot};
use crate::measure::Measure;
use crate::measure::grid::DEFAULT_GRID;
use crate::render::beat::draw_beat;
use crate::render::glyphs;
use eframe::egui;
use eframe::egui::{Align2, Color32, FontFamily, FontId, Painter, Rangef, Rect, Stroke, pos2};
use std::collections::HashMap;

pub(crate) fn draw_measure(
    ui: &mut egui::Ui,
    measure: &Measure,
    opts: &LayoutOpts,
    cursor_idx: Option<usize>,
    playback_tick: Option<f64>,
    count_config: Option<&CountConfig>,
) -> MeasureLayout {
    let color = ui.visuals().text_color();
    let painter = ui.painter();
    let rect = opts.rect;

    let font_id = &opts.font_id;
    let measure_layout = build_measure_layout(measure, opts);

    if let Some(config) = count_config {
        let slots = build_count_slots(measure, config);
        if !slots.is_empty() {
            draw_count_underlay(
                painter,
                measure,
                &measure_layout,
                rect,
                opts.em,
                ui.visuals().dark_mode,
                &slots,
            );
            draw_count_labels(
                painter,
                measure,
                &measure_layout,
                rect,
                opts.em,
                ui.visuals().text_color(),
                ui.visuals().selection.stroke.color,
                &slots,
                playback_tick,
            );
        }
    }

    // staff line
    painter.hline(Rangef::new(rect.left(), rect.right()), rect.center().y, Stroke::new(0.02 * opts.em, color));

    // Left block: Clef and stacked time signature from layout
    if let Some(clef_pos) = measure_layout.clef_pos {
        painter.text(
            clef_pos,
            Align2::CENTER_CENTER,
            glyphs::GLYPH_CLEF_PERCUSSION.to_string(),
            font_id.clone(),
            color,
        );
    }

    if let Some(ts_layout) = &measure_layout.time_signature {
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

    draw_notes(painter, &measure_layout, color, opts);

    // Edit cursor at current beat index
    if let Some(idx) = cursor_idx
        && let Some(nl) = measure_layout.notes.get(idx)
    {
        // Blink parameters
        let blink_period = 1.0_f64; // seconds for a full on+off cycle
        let duty = 0.5_f64; // visible fraction of the period
        let t = ui.input(|i| i.time);
        let phase = (t % blink_period) / blink_period; // 0..1
        let visible = phase < duty;
        let alpha_on = 220u8;
        let alpha_off = 40u8; // faint but still present; set to 0 to hide completely
        let alpha = if visible { alpha_on } else { alpha_off };
        let c = measure_layout.notes[idx].center;
        let top = c.y + 0.5 * opts.em;
        let bottom = c.y - 0.5 * opts.em;
        let base = if ui.visuals().dark_mode { Color32::YELLOW } else { Color32::BLUE };
        let cursor_color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha);
        painter.vline(nl.center.x, Rangef::new(top, bottom), Stroke::new(0.03 * opts.em, cursor_color));
        // Ensure animation progresses even without input
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }

    // Playback cursor
    if let Some(tick) = playback_tick {
        let ts = measure.time_signature();
        let total_ticks = DEFAULT_GRID.ticks_per_measure(&ts) as f64;
        if total_ticks > 0.0 && !measure_layout.notes.is_empty() {
            let t = if tick.is_sign_negative() {
                0.0
            } else {
                let m = tick % total_ticks;
                if m.is_nan() { 0.0 } else { m }
            };

            let onsets = DEFAULT_GRID.compute_onset_ticks(measure.beats());
            let mut x = measure_layout.notes[0].center.x;

            for (i, &onset) in onsets.iter().enumerate() {
                let start = onset as f64;
                let dur_ticks = DEFAULT_GRID
                    .ticks_of(&measure.beats()[i].duration)
                    .unwrap_or(0) as f64;
                let end = start + dur_ticks;
                if t >= start && t < end {
                    let x0 = measure_layout.notes[i].center.x;
                    let frac = if dur_ticks > 0.0 { (t - start) / dur_ticks } else { 0.0 };

                    if i + 1 < measure_layout.notes.len() {
                        let x1 = measure_layout.notes[i + 1].center.x;
                        x = x0 + ((x1 - x0) * (frac as f32));
                    } else {
                        // Smooth wrap: split travel between "after last note" and "before first note".
                        // First half of duration: travel right from last note.
                        // Second half of duration: travel right towards first note (from left edge).
                        let x_first = measure_layout.notes[0].center.x;
                        let gap_after_last = rect.right() - x0;
                        let gap_before_first = x_first - measure_layout.notes_left_edge;
                        let total_dist = gap_after_last + gap_before_first;

                        if frac < 0.5 {
                            x = x0 + total_dist * (frac as f32);
                        } else {
                            x = x_first - total_dist * ((1.0 - frac) as f32);
                        }
                    }
                    break;
                }
            }

            let top = rect.center().y + 0.7 * opts.em;
            let bottom = rect.center().y - 0.7 * opts.em;
            let base = ui.visuals().selection.stroke.color;
            let cursor_color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 100);
            painter.vline(x, Rangef::new(top, bottom), Stroke::new(0.1 * opts.em, cursor_color));
        }
    }

    measure_layout
}

pub(crate) fn draw_notes(
    painter: &Painter,
    measure_layout: &MeasureLayout,
    color: Color32,
    opts: &LayoutOpts,
) {
    // Beats/notes
    for note in &measure_layout.notes {
        draw_beat(painter, note, opts, color);
    }

    // Beams
    for seg in &measure_layout.beams {
        painter.rect_filled(seg.rect, 0.0, color);
    }

    // Tuplets
    for t in &measure_layout.tuplets {
        // draw bracket segments
        for seg in &t.bracket {
            painter.line_segment([seg.p1, seg.p2], Stroke::new(opts.bracket_thickness(), color));
        }
        // draw tuplet number at center
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
    fn tick_to_x(&self, tick: u32) -> f32 {
        let t = tick.min(self.total_ticks);
        let mut i = 0usize;
        while i + 1 < self.onsets.len() && t >= self.onsets[i + 1] {
            i += 1;
        }
        let start_tick = self.onsets.get(i).copied().unwrap_or(0);
        let end_tick = if i + 1 < self.onsets.len() {
            self.onsets[i + 1]
        } else {
            self.total_ticks
        };
        let x0 = *self.boundary_x.get(i).unwrap_or(&self.boundary_x[0]);
        let x1 = *self.boundary_x.get(i + 1).unwrap_or(&self.boundary_x[self.boundary_x.len() - 1]);
        let span = end_tick.saturating_sub(start_tick);
        if span == 0 {
            return x0;
        }
        let frac = (t - start_tick) as f32 / span as f32;
        x0 + (x1 - x0) * frac
    }
}

fn build_tick_mapper(
    measure: &Measure,
    layout: &MeasureLayout,
    rect: Rect,
) -> Option<TickMapper> {
    let beats = measure.beats();
    if beats.is_empty() || layout.notes.is_empty() {
        return None;
    }
    let onsets = DEFAULT_GRID.compute_onset_ticks(beats);
    let total_ticks = DEFAULT_GRID.ticks_per_measure(&measure.time_signature());
    if total_ticks == 0 {
        return None;
    }
    let mut boundary_x = Vec::with_capacity(onsets.len() + 1);
    boundary_x.push(layout.notes_left_edge);
    for i in 1..layout.notes.len() {
        let x_prev = layout.notes[i - 1].center.x;
        let x_cur = layout.notes[i].center.x;
        boundary_x.push((x_prev + x_cur) * 0.5);
    }
    boundary_x.push(rect.right());

    Some(TickMapper {
        onsets,
        boundary_x,
        total_ticks,
    })
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
        let x0 = mapper.tick_to_x(slot.start_tick);
        let x1 = mapper.tick_to_x(slot.end_tick);
        if x1 <= x0 + 0.5 {
            continue;
        }
        let color = count_color(color_id, alpha);
        let r = Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        painter.rect_filled(r, 0.0, color);
    }
}

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
) {
    let mapper = match build_tick_mapper(measure, layout, rect) {
        Some(mapper) => mapper,
        None => return,
    };
    let label_slots: Vec<&CountSlot> = slots.iter().filter(|s| s.label.is_some()).collect();
    if label_slots.is_empty() {
        return;
    }
    let selected = select_label_slots(&label_slots);
    let active_start = active_label_start(&selected, playback_tick, measure);

    let font = FontId::new(em * 0.4, FontFamily::Proportional);
    let label_y = (rect.center().y + 1.35 * em).min(rect.bottom() - 0.2 * em);
    let label_color = Color32::from_rgba_unmultiplied(base_color.r(), base_color.g(), base_color.b(), 200);
    let highlight = Color32::from_rgba_unmultiplied(
        highlight_color.r(),
        highlight_color.g(),
        highlight_color.b(),
        255,
    );

    for slot in selected {
        let Some(label) = &slot.label else { continue };
        let x = label_anchor_x(slot, &mapper, layout, rect);
        let color = if active_start == Some(slot.start_tick) {
            highlight
        } else {
            label_color
        };
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
        t = t % total_ticks;
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
                        if new_span < cur_span {
                            Some(*slot)
                        } else {
                            Some(cur)
                        }
                    } else {
                        Some(cur)
                    }
                }
            };
        }
    }
    best.map(|s| s.start_tick)
}

fn label_anchor_x(
    slot: &CountSlot,
    mapper: &TickMapper,
    layout: &MeasureLayout,
    rect: Rect,
) -> f32 {
    let max_idx = mapper.onsets.len().min(layout.notes.len());
    for i in 0..max_idx {
        let onset = mapper.onsets[i];
        if onset >= slot.start_tick && onset < slot.end_tick {
            return layout.notes[i].center.x;
        }
    }
    label_fallback_x(slot.start_tick, mapper, layout, rect)
}

fn label_fallback_x(tick: u32, mapper: &TickMapper, layout: &MeasureLayout, rect: Rect) -> f32 {
    let count = mapper.onsets.len().min(layout.notes.len());
    if count == 0 {
        return mapper.tick_to_x(tick);
    }
    let t = tick.min(mapper.total_ticks);
    let mut i = 0usize;
    while i + 1 < count && t >= mapper.onsets[i + 1] {
        i += 1;
    }
    let start = mapper.onsets[i];
    let x0 = layout.notes[i].center.x;
    let (end, x1) = if i + 1 < count {
        (mapper.onsets[i + 1], layout.notes[i + 1].center.x)
    } else {
        (mapper.total_ticks, rect.right())
    };
    let span = end.saturating_sub(start);
    if span == 0 {
        return x0;
    }
    let frac = (t - start) as f32 / span as f32;
    x0 + (x1 - x0) * frac
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
