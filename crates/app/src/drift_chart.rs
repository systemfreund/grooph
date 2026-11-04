//! The charts under "Your run", drawn with the painter: the run's
//! deviations over time, and its average per position in the beat.

use crate::accuracy::{
    BeatPositions, Drift, GOOD_WINDOW_MS, HitGrade, MIN_POSITION_HITS, PERFECT_WINDOW_MS,
    PositionTiming,
};
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

/// As high as the drift chart above it.
const STRIP_HEIGHT: f32 = HEIGHT;
/// From this many columns on, only the position that stands out keeps its
/// number under the strip; the others show theirs on hover.
const CROWDED_COLUMNS: usize = 6;

/// The deviation range of the strip: its largest average plus spread, in
/// steps of 10 ms, from 20 to 150 ms.
fn strip_range_ms(positions: &BeatPositions) -> f64 {
    let max = positions
        .positions
        .iter()
        .filter_map(|p| Some(p.mean_ms?.abs() + p.spread_ms.unwrap_or(0.0)))
        .fold(0.0, f64::max);
    ((max / 10.0).ceil() * 10.0).clamp(20.0, 150.0)
}

/// A signed deviation as people say it: "12 ms late", "4 ms early".
fn late_or_early(ms: f64) -> String {
    let side = if ms < 0.0 { "early" } else { "late" };
    format!("{:.0} ms {side}", ms.abs())
}

/// The sentence on the position that stands out, if one does.
fn notable_sentence(positions: &BeatPositions) -> Option<String> {
    let p = positions.positions.get(positions.notable?)?;
    let off = p.mean_ms? - positions.overall_ms;
    let way = if off < 0.0 { "earlier" } else { "later" };
    let at = if p.pos.parts <= 1 {
        "On the beat".to_string()
    } else {
        format!("On \u{201c}{}\u{201d}", p.label)
    };
    Some(format!("{at} you play {:.0} ms {way} than on average", off.abs()))
}

fn position_hover(p: &PositionTiming, label: &str) -> String {
    let mut text = format!("\u{201c}{label}\u{201d}: {} hits", p.hits);
    if p.misses > 0 {
        text += &format!(", {} missed", p.misses);
    }
    match (p.mean_ms, p.spread_ms) {
        (Some(mean), Some(spread)) => {
            text += &format!("\nAverage {}, spread {spread:.0} ms", late_or_early(mean));
        }
        (Some(mean), None) => text += &format!("\nAverage {}", late_or_early(mean)),
        _ => text += &format!("\nAn average needs {MIN_POSITION_HITS} hits"),
    }
    text
}

/// The run's average per position in the beat, one column each in the
/// order of the beat: a bar from the click's line to the average (late up),
/// a whisker for the spread, the perfect window as a band and the run's
/// average as a dashed line. Under it the positions as counted
/// and their averages, and a sentence on the one that stands out.
pub(crate) fn beat_position_strip(ui: &mut egui::Ui, positions: &BeatPositions) {
    let columns = positions.positions.len().max(1);
    let width = ui.available_width().min(MAX_WIDTH);
    let (rect, response) = ui.allocate_exact_size(vec2(width, STRIP_HEIGHT), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals().clone();
    let dark = visuals.dark_mode;
    let range = strip_range_ms(positions);
    let plot = rect.shrink(PAD);
    let y =
        |ms: f64| plot.center().y - (ms.clamp(-range, range) / range) as f32 * plot.height() / 2.0;
    let column_width = plot.width() / columns as f32;
    let column = |i: usize| {
        Rect::from_x_y_ranges(
            plot.left() + column_width * i as f32..=plot.left() + column_width * (i + 1) as f32,
            rect.y_range(),
        )
    };

    painter.rect_filled(rect, 4.0, visuals.extreme_bg_color);
    let band = Rect::from_x_y_ranges(plot.x_range(), y(PERFECT_WINDOW_MS)..=y(-PERFECT_WINDOW_MS));
    painter.rect_filled(band, 0.0, grade_color(HitGrade::Perfect, dark).gamma_multiply(0.15));
    painter.hline(plot.x_range(), y(0.0), Stroke::new(1.0, reference_color(dark)));
    let average = y(positions.overall_ms);
    painter.add(egui::Shape::dashed_line(
        &[pos2(plot.left(), average), pos2(plot.right(), average)],
        Stroke::new(1.0, visuals.weak_text_color()),
        4.0,
        3.0,
    ));

    let crowded = columns >= CROWDED_COLUMNS;
    let mut labels = Vec::with_capacity(columns);
    for (i, p) in positions.positions.iter().enumerate() {
        let col = column(i);
        let label = p.label.clone();
        let x = col.center().x;
        if let Some(mean) = p.mean_ms {
            let grade = if mean.abs() <= PERFECT_WINDOW_MS {
                HitGrade::Perfect
            } else if mean.abs() <= GOOD_WINDOW_MS {
                HitGrade::Good
            } else {
                HitGrade::Off
            };
            let bar_width = (column_width * 0.5).min(18.0);
            let bar = Rect::from_x_y_ranges(x - bar_width / 2.0..=x + bar_width / 2.0, {
                let (a, b) = (y(0.0), y(mean));
                a.min(b)..=a.max(b).max(a.min(b) + 1.0)
            });
            painter.rect_filled(bar, 1.0, grade_color(grade, dark));
            if let Some(spread) = p.spread_ms {
                let whisker = Stroke::new(1.0, visuals.strong_text_color());
                painter.vline(x, y(mean + spread)..=y(mean - spread), whisker);
            }
        }
        let shows_number = !crowded || positions.notable == Some(i);
        let number = match p.mean_ms {
            Some(mean) if shows_number => format!("{mean:+.0}"),
            Some(_) => String::new(),
            None => "\u{2013}".to_string(),
        };
        let hover = position_hover(p, &label);
        ui.interact(col, response.id.with(i), egui::Sense::hover()).on_hover_text(hover);
        labels.push((label, number));
    }

    // The positions and their averages, under their columns.
    let text_height = ui.text_style_height(&egui::TextStyle::Small);
    let (under, _) = ui.allocate_exact_size(vec2(width, text_height * 2.0), egui::Sense::hover());
    let painter = ui.painter_at(under);
    let small = egui::TextStyle::Small.resolve(ui.style());
    for (i, (label, number)) in labels.iter().enumerate() {
        let x = column(i).center().x;
        let color = if positions.notable == Some(i) {
            visuals.strong_text_color()
        } else {
            visuals.text_color()
        };
        painter.text(pos2(x, under.top()), egui::Align2::CENTER_TOP, label, small.clone(), color);
        painter.text(
            pos2(x, under.top() + text_height),
            egui::Align2::CENTER_TOP,
            number,
            small.clone(),
            visuals.weak_text_color(),
        );
    }
    if let Some(sentence) = notable_sentence(positions) {
        ui.label(sentence);
    }
    response.on_hover_text(
        "Your average per position in the beat, in ms, late above the \
         middle line and early below; the line through each bar is its \
         spread, the dashed line your average over the run",
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

    fn timing(part: u8, parts: u8, label: &str, mean_ms: Option<f64>) -> PositionTiming {
        PositionTiming {
            pos: grooph_measure::counting::CountPos {
                units: 1,
                uneven: false,
                beat: 1,
                part,
                parts,
            },
            label: label.to_string(),
            hits: 8,
            misses: 0,
            mean_ms,
            spread_ms: mean_ms.map(|_| 5.0),
        }
    }

    #[test]
    fn the_sentence_names_the_position_against_the_average() {
        let positions = BeatPositions {
            positions: vec![timing(0, 1, "1", Some(-20.0)), timing(1, 2, "&", Some(20.0))],
            overall_ms: 0.0,
            notable: Some(1),
        };
        assert_eq!(
            notable_sentence(&positions).as_deref(),
            Some("On \u{201c}&\u{201d} you play 20 ms later than on average")
        );
        let on_the_beat = BeatPositions { notable: Some(0), ..positions.clone() };
        assert_eq!(
            notable_sentence(&on_the_beat).as_deref(),
            Some("On the beat you play 20 ms earlier than on average")
        );
        assert_eq!(notable_sentence(&BeatPositions { notable: None, ..positions }), None);
        assert_eq!(strip_range_ms(&on_the_beat), 30.0);
    }
}
