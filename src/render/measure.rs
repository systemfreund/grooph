use crate::layout::pixel_layout::{LayoutOpts, MeasureLayout, build_measure_layout};
use crate::measure::Measure;
use crate::measure::grid::DEFAULT_GRID;
use crate::render::beat::draw_beat;
use crate::render::glyphs;
use eframe::egui;
use eframe::egui::{Align2, Color32, FontId, Painter, Rangef, Rect, Stroke, pos2};

pub(crate) fn draw_measure(
    ui: &mut egui::Ui,
    measure: &Measure,
    opts: &LayoutOpts,
    cursor_idx: Option<usize>,
    playback_tick: Option<f64>,
) -> MeasureLayout {
    let color = ui.visuals().text_color();
    let painter = ui.painter();
    let rect = opts.rect;

    // staff line
    painter.hline(Rangef::new(rect.left(), rect.right()), rect.center().y, Stroke::new(0.02 * opts.em, color));

    let font_id = &opts.font_id;
    let measure_layout = build_measure_layout(measure, opts);

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
