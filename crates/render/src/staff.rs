//! Multi-measure rendering: walks a [`StaffLayout`] and renders each measure
//! with the existing per-measure renderer. Adds cross-measure visuals
//! (continuous staff line, barlines) and orchestrates the playback cursor
//! wrap animation across measure boundaries.

use crate::measure::{
    last_note_entering_frac, playback_cursor_entering_x, playback_cursor_x, render_measure_at,
};
use eframe::egui;
use eframe::egui::{Color32, Rangef, Stroke};
use grooph_layout::staff_layout::{PlacedMeasure, StaffLayout, StaffOpts};
use grooph_measure::counting::CountConfig;
use grooph_measure::{Cursor, MeasureIdx, Score};

/// Render a full score described by `staff` to `ui`.
///
/// `cursor` highlights the edit position; only the measure matching
/// `cursor.measure_idx` gets a blinking cursor. `playback` is
/// `Some((measure_idx, local_tick))` referring to the **active** measure (the
/// one currently being audibly played). The *visual* cursor may live in a
/// different measure during the wrap animation — see [`current_cursor_x`].
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

    // Resolve the *visual* cursor location once per frame. Phase 1 of a
    // measure's last note keeps the cursor in that measure; Phase 2 hands it
    // off to the next one (wrapping at the score end).
    let visual_cursor = playback.and_then(|p| current_cursor_x(score, staff, p));

    for system in &staff.systems {
        // One continuous staff line per system, so adjacent measures look joined.
        ui.painter().hline(
            Rangef::new(system.rect.left(), system.rect.right()),
            system.y_baseline,
            Stroke::new(0.02 * staff_opts.em, color),
        );

        for placed in &system.measures {
            let cursor_x =
                visual_cursor.filter(|(idx, _)| *idx == placed.measure_idx).map(|(_, x)| x);

            render_placed_measure(
                ui,
                score,
                placed,
                staff_opts,
                cursor,
                playback,
                count_config,
                cursor_x,
            );
        }

        // Barlines between measures: vertical stroke at each measure boundary
        // except after the last one (the right edge is the score end).
        draw_barlines(ui, system, staff_opts, color);
    }
}

/// Compute the X position of the playback cursor across the whole staff.
///
/// Implements the two-phase wrap animation:
/// - Phase 1: cursor lives in the currently playing measure
///   (`playback.0`). Returned as `(playback.0, x)`.
/// - Phase 2 (last-note second half): cursor lives in
///   `(playback.0 + 1) % score.len()`, entering from the left. Returned as
///   that measure's index plus its entering X.
///
/// Returns `None` if there is no visible cursor (e.g. score empty, layout
/// empty, or the relevant measure has no notes).
pub fn current_cursor_x(
    score: &Score,
    staff: &StaffLayout,
    playback: (MeasureIdx, f64),
) -> Option<(MeasureIdx, f32)> {
    let (play_idx, local_tick) = playback;
    if score.is_empty() {
        return None;
    }

    let active_measure = score.measures.get(play_idx)?;

    // Phase 2 first: if we're in the second half of the active measure's last
    // note, the cursor visually belongs to the *next* measure.
    if let Some(entry_frac) = last_note_entering_frac(active_measure, local_tick) {
        let next_idx = (play_idx + 1) % score.len();
        let next_placed = staff.placed(next_idx)?;
        let x = playback_cursor_entering_x(&next_placed.layout, next_placed.rect, entry_frac)?;
        return Some((next_idx, x));
    }

    // Phase 1: cursor lives in the active measure.
    let active_placed = staff.placed(play_idx)?;
    let x =
        playback_cursor_x(active_measure, &active_placed.layout, active_placed.rect, local_tick)?;
    Some((play_idx, x))
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
    cursor_x: Option<f32>,
) {
    let cursor_idx = cursor.filter(|c| c.measure_idx == placed.measure_idx).map(|c| c.beat_idx);
    // `playback_tick` drives count-label highlighting (and only that, since
    // the visual cursor is now passed as `cursor_x`). Restrict to the active
    // measure so a wrap-in cursor doesn't light up labels in the next measure
    // before its turn.
    let playback_tick = playback.filter(|(idx, _)| *idx == placed.measure_idx).map(|(_, t)| t);

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
        cursor_x,
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
