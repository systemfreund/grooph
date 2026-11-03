//! The drift chart under "Your run": the run's deviations over time, drawn
//! with the painter.

use crate::accuracy::{Drift, HitGrade, PERFECT_WINDOW_MS};
use crate::measure_panel::{grade_color, reference_color};
use eframe::egui::{self, Pos2, Rect, Stroke, pos2, vec2};

const HEIGHT: f32 = 84.0;
const MAX_WIDTH: f32 = 340.0;
const PAD: f32 = 4.0;

/// The deviation range shown up and down from the click: the run's largest,
/// in steps of 10 ms, from 40 to 150 ms. Larger ones sit on the edge.
pub(crate) fn range_ms(drift: &Drift) -> f64 {
    let max = drift.hits.iter().map(|h| h.delta_ms.abs()).fold(0.0, f64::max);
    ((max / 10.0).ceil() * 10.0).clamp(40.0, 150.0)
}

/// Late hits above the click's line, early ones below; a line for the
/// average over about a bar, the perfect window as a band, silent bars
/// shaded and missed notes as crosses on the bottom edge.
pub(crate) fn drift_chart(ui: &mut egui::Ui, drift: &Drift) {
    let width = ui.available_width().min(MAX_WIDTH);
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEIGHT), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    let dark = visuals.dark_mode;
    let range = range_ms(drift);
    let plot = rect.shrink(PAD);
    let x = |t: f64| plot.left() + (t / drift.length_s) as f32 * plot.width();
    let y =
        |ms: f64| plot.center().y - (ms.clamp(-range, range) / range) as f32 * plot.height() / 2.0;

    painter.rect_filled(rect, 4.0, visuals.extreme_bg_color);
    let shade = visuals.widgets.inactive.bg_fill;
    for &(start, end) in &drift.silent {
        let span = Rect::from_x_y_ranges(x(start)..=x(end), rect.y_range());
        painter.rect_filled(span, 0.0, shade);
    }
    let band = Rect::from_x_y_ranges(plot.x_range(), y(PERFECT_WINDOW_MS)..=y(-PERFECT_WINDOW_MS));
    painter.rect_filled(band, 0.0, grade_color(HitGrade::Perfect, dark).gamma_multiply(0.15));
    painter.hline(plot.x_range(), y(0.0), Stroke::new(1.0, reference_color(dark)));

    for hit in &drift.hits {
        painter.circle_filled(pos2(x(hit.t), y(hit.delta_ms)), 2.0, grade_color(hit.grade, dark));
    }
    let trend: Vec<Pos2> = drift.trend.iter().map(|&(t, ms)| pos2(x(t), y(ms))).collect();
    painter.add(egui::Shape::line(trend, Stroke::new(2.0, visuals.strong_text_color())));
    let off = Stroke::new(1.5, grade_color(HitGrade::Off, dark));
    for &t in &drift.misses {
        let c = pos2(x(t), plot.bottom() - 3.0);
        painter.line_segment([c + vec2(-3.0, -3.0), c + vec2(3.0, 3.0)], off);
        painter.line_segment([c + vec2(-3.0, 3.0), c + vec2(3.0, -3.0)], off);
    }

    response.on_hover_text(
        "Each dot is a hit, the line your average over about a bar: \
         drifting off the middle line means getting ahead of or behind \
         the click. The band is the perfect window of notes that are not \
         fast; fast notes get a narrower one, so a dot in it can still be \
         good. Shaded: silent bars. Crosses: missed notes",
    );
    // Below the chart, where it hides no dot or cross.
    ui.label(
        egui::RichText::new(format!("Late above, early below, up to {range:.0} ms")).small().weak(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::DriftHit;

    fn drift_of(deltas: &[f64]) -> Drift {
        Drift {
            hits: deltas
                .iter()
                .enumerate()
                .map(|(i, &delta_ms)| DriftHit { t: i as f64, delta_ms, grade: HitGrade::Good })
                .collect(),
            trend: Vec::new(),
            misses: Vec::new(),
            silent: Vec::new(),
            length_s: deltas.len() as f64,
        }
    }

    #[test]
    fn range_fits_the_largest_deviation_within_bounds() {
        assert_eq!(range_ms(&drift_of(&[5.0, -12.0])), 40.0);
        assert_eq!(range_ms(&drift_of(&[5.0, -61.0])), 70.0);
        assert_eq!(range_ms(&drift_of(&[400.0])), 150.0);
    }
}
