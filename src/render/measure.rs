use crate::layout::pixel_layout::{LayoutOpts, MeasureLayout, build_measure_layout};
use crate::measure::Measure;
use crate::render::beat::draw_beat;
use crate::render::glyphs;
use eframe::egui;
use eframe::egui::{Align2, Color32, FontId, Rangef, Rect, Stroke, pos2};

pub(crate) fn draw_measure(
    ui: &mut egui::Ui,
    font_id: &FontId,
    measure: &Measure,
    rect: Rect,
    cursor_idx: Option<usize>,
) -> MeasureLayout {
    let color: Color32 = if ui.visuals().dark_mode { Color32::WHITE } else { Color32::BLACK };
    let painter = ui.painter();
    let y = rect.center().y;
    // staff line
    painter.hline(Rangef::new(rect.left(), rect.right()), y, Stroke::new(1.0, color));

    let min_size = 24.0 * ui.ctx().pixels_per_point();

    // Derive font size mainly from the available height, modulated by width caps
    let width_cap = (rect.width() * 0.1).max(min_size);
    let max_size = (rect.height() * 0.80).max(min_size);
    let target_size = min_size.max(max_size.min(width_cap));
    let font_id = FontId::new(target_size, font_id.family.clone());

    let opts = LayoutOpts {
        rect,
        font_id: font_id.clone(),
        min_size,
        em: target_size,
        layout_clef: true,
        layout_time_signature: true,
        y_offset: 0.0,
        stem_length_factor: 1.0,
    };
    let measure_layout = build_measure_layout(measure, &opts);

    // Left block: Clef + stacked time signature from layout (Phase C)
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
        let top_digits = glyphs::ts_glyphs(ts.beats as u32);
        let bot_digits = glyphs::ts_glyphs(ts.beat_unit as u32);
        for (p, ch) in ts_layout.beats.iter().zip(top_digits.iter()) {
            painter.text(*p, Align2::CENTER_CENTER, ch.to_string(), font_id.clone(), color);
        }
        for (p, ch) in ts_layout.beat_unit.iter().zip(bot_digits.iter()) {
            painter.text(*p, Align2::CENTER_CENTER, ch.to_string(), font_id.clone(), color);
        }
    }

    // 3) Draw beats using precomputed layout geometry (Phase B)
    for note in &measure_layout.notes {
        draw_beat(painter, note, &opts, color);
    }

    // 4) Draw beams from layout (horizontal rectangles at given y with thickness)
    for seg in &measure_layout.beams {
        let left = seg.p1.x.min(seg.p2.x);
        let right = seg.p1.x.max(seg.p2.x);
        let yb = seg.p1.y; // bottom edge
        let top = yb - opts.beam_thickness();
        let rect = Rect::from_min_max(pos2(left, top), pos2(right, yb));
        painter.rect_filled(rect, 0.0, color);
    }

    // 4c) Tuplets: draw from precomputed layout (Phase C)
    if !measure_layout.tuplets.is_empty() {
        for t in &measure_layout.tuplets {
            // draw bracket segments
            for seg in &t.bracket {
                painter
                    .line_segment([seg.p1, seg.p2], Stroke::new(opts.bracket_thickness(), color));
            }
            // draw tuplet number at center
            let digits = glyphs::tuplet_glyphs(t.count);
            painter.text(
                t.number_center,
                Align2::CENTER_CENTER,
                digits,
                t.number_font.clone(),
                color,
            );
        }
    }

    // 5) Cursor at current beat index (does not consume width) — blink over time
    if let Some(idx) = cursor_idx
        && let Some(nl) = measure_layout.notes.get(idx)
    {
        // Blink parameters
        let blink_period = 1.0_f64; // seconds for a full on+off cycle
        let duty = 0.5_f64; // visible fraction of the period
        let t = ui.input(|i| i.time);
        let phase = (t % blink_period) / blink_period; // 0..1
        let visible = phase < duty;
        // Smooth fade near edges optional; for now a simple square wave with two alpha levels
        let alpha_on = 220u8;
        let alpha_off = 40u8; // faint but still present; set to 0 to hide completely
        let alpha = if visible { alpha_on } else { alpha_off };
        let c = measure_layout.notes[idx].center;
        let top = c.y + 0.5 * opts.em;
        let bottom = c.y - 0.5 * opts.em;
        let base = if ui.visuals().dark_mode { Color32::YELLOW } else { Color32::BLUE };
        let cursor_color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha);
        painter.vline(nl.center.x, Rangef::new(top, bottom), Stroke::new(2.0, cursor_color));
        // Ensure animation progresses even without input
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }

    measure_layout
}
