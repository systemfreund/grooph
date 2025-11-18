use crate::layout::render_plan::build_measure_layout_px;
use crate::measure::Measure;
use crate::render::beat::draw_note_from_layout;
use crate::render::glyphs;
use eframe::egui;
use eframe::egui::{Align2, Color32, FontId, Rangef, Rect, Stroke, pos2};

pub(crate) fn draw_measure(
    ui: &mut egui::Ui,
    font_id: &FontId,
    measure: &Measure,
    rect: Rect,
    cursor_idx: Option<usize>,
) {
    let color: Color32 = if ui.visuals().dark_mode { Color32::WHITE } else { Color32::BLACK };
    let painter = ui.painter();
    let y = rect.center().y;
    // staff line
    painter.hline(Rangef::new(rect.left(), rect.right()), y, Stroke::new(1.0, color));

    // Build pixel layout (Phase A: x-centers + beam segments)
    let layout_px = build_measure_layout_px(measure, rect, font_id, ui.ctx().pixels_per_point());
    let inner_rect = layout_px.inner_rect;
    let em = layout_px.em;
    let font_id = layout_px.font_id.clone();

    // Left block: Clef + stacked time signature from layout (Phase C)
    if let Some(clef_pos) = layout_px.clef_pos {
        painter.text(
            clef_pos,
            Align2::CENTER_CENTER,
            glyphs::GLYPH_CLEF_PERCUSSION.to_string(),
            font_id.clone(),
            color,
        );
    }
    let ts = measure.time_signature();
    let top_digits = glyphs::ts_glyphs(ts.beats as u32);
    let bot_digits = glyphs::ts_glyphs(ts.beat_unit as u32);
    for (p, ch) in layout_px.time_sig_top.iter().zip(top_digits.iter()) {
        painter.text(*p, Align2::CENTER_CENTER, ch.to_string(), font_id.clone(), color);
    }
    for (p, ch) in layout_px.time_sig_bottom.iter().zip(bot_digits.iter()) {
        painter.text(*p, Align2::CENTER_CENTER, ch.to_string(), font_id.clone(), color);
    }

    // Absolute x-centers provided by layout
    let x_centers = layout_px.x_centers.clone();

    // 3) Draw beats using precomputed layout geometry (Phase B)
    for note in &layout_px.notes {
        draw_note_from_layout(painter, note, &font_id, color);
    }

    // 4) Draw beams from layout (horizontal rectangles at given y with thickness)
    for seg in &layout_px.beams {
        let left = seg.p1.x.min(seg.p2.x);
        let right = seg.p1.x.max(seg.p2.x);
        let yb = seg.p1.y; // bottom edge
        let top = yb - seg.thickness;
        let rect = Rect::from_min_max(pos2(left, top), pos2(right, yb));
        painter.rect_filled(rect, 0.0, color);
    }

    // 4c) Tuplets: draw from precomputed layout (Phase C)
    if !layout_px.tuplets.is_empty() {
        for t in &layout_px.tuplets {
            // draw bracket segments
            for seg in &t.bracket {
                painter.line_segment([seg.p1, seg.p2], Stroke::new(seg.thickness, color));
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
        && let Some(&x) = x_centers.get(idx)
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
        let top = inner_rect.top() + 0.5 * em;
        let bottom = inner_rect.bottom() - 0.5 * em;
        let base = if ui.visuals().dark_mode { Color32::WHITE } else { Color32::BLACK };
        let cursor_color = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha);
        painter.vline(x, Rangef::new(top, bottom), Stroke::new(2.0, cursor_color));
        // Ensure animation progresses even without input
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
}
