//! Multi-measure rendering: walks a [`StaffLayout`] and renders each measure
//! with the existing per-measure renderer. Adds cross-measure visuals
//! (continuous staff line, barlines).

use crate::measure::render_measure_at;
use eframe::egui;
use eframe::egui::{Color32, Rangef, Stroke};
use grooph_layout::staff_layout::{PlacedMeasure, StaffLayout, StaffOpts};
use grooph_measure::counting::CountConfig;
use grooph_measure::{Cursor, MeasureIdx, Score};

/// Render a full score described by `staff` to `ui`.
///
/// `cursor` highlights the edit position; only the measure matching
/// `cursor.measure_idx` gets a blinking cursor. `playback` is
/// `Some((measure_idx, smooth_tick))` and similarly only animates inside
/// its measure. `count_config` is applied per measure if set.
#[allow(clippy::too_many_arguments)]
pub fn draw_staff(
    ui: &mut egui::Ui,
    score: &Score,
    staff: &StaffLayout,
    cursor: Option<Cursor>,
    playback: Option<(MeasureIdx, f64)>,
    count_config: Option<&CountConfig>,
    staff_opts: &StaffOpts,
) {
    let color = ui.visuals().text_color();

    for system in &staff.systems {
        // One continuous staff line per system, so adjacent measures look joined.
        ui.painter().hline(
            Rangef::new(system.rect.left(), system.rect.right()),
            system.y_baseline,
            Stroke::new(0.02 * staff_opts.em, color),
        );

        for (i, placed) in system.measures.iter().enumerate() {
            // Anchor for cross-measure cursor interpolation: the first note X
            // of the *next* measure in this system. `None` for the last one —
            // the cursor then walks to the measure's right edge, and the
            // score-wrap is rendered as a step on the next frame.
            let next_anchor_x = system
                .measures
                .get(i + 1)
                .and_then(|next| next.layout.notes.first().map(|n| n.center.x));

            render_placed_measure(
                ui,
                score,
                placed,
                staff_opts,
                cursor,
                playback,
                count_config,
                next_anchor_x,
            );
        }

        // Barlines between measures: vertical stroke at each measure boundary
        // except after the last one (the right edge is the score end).
        draw_barlines(ui, system, staff_opts, color);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_placed_measure(
    ui: &mut egui::Ui,
    score: &Score,
    placed: &PlacedMeasure,
    staff_opts: &StaffOpts,
    cursor: Option<Cursor>,
    playback: Option<(MeasureIdx, f64)>,
    count_config: Option<&CountConfig>,
    next_anchor_x: Option<f32>,
) {
    let cursor_idx = cursor
        .filter(|c| c.measure_idx == placed.measure_idx)
        .map(|c| c.beat_idx);
    let playback_tick = playback
        .filter(|(idx, _)| *idx == placed.measure_idx)
        .map(|(_, t)| t);

    // Build a per-measure LayoutOpts from the placed rect/flags so the cursor
    // and playback drawing get the right viewport.
    let per_opts = staff_opts.measure_opts_for(placed);

    render_measure_at(
        ui,
        &score.measures[placed.measure_idx],
        &placed.layout,
        &per_opts,
        cursor_idx,
        playback_tick,
        count_config,
        /* draw_staff_line = */ false,
        next_anchor_x,
    );
}

fn draw_barlines(
    ui: &egui::Ui,
    system: &grooph_layout::staff_layout::SystemLayout,
    staff_opts: &StaffOpts,
    color: Color32,
) {
    let top = system.y_baseline - 0.5 * staff_opts.em;
    let bottom = system.y_baseline + 0.5 * staff_opts.em;
    let stroke = Stroke::new(0.04 * staff_opts.em, color);
    // Inner barlines: between adjacent measures.
    for w in system.measures.windows(2) {
        let x = w[0].rect.right();
        ui.painter().vline(x, Rangef::new(top, bottom), stroke);
    }
    // Final barline at the very end of the system.
    if let Some(last) = system.measures.last() {
        ui.painter().vline(last.rect.right(), Rangef::new(top, bottom), stroke);
    }
}
