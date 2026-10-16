//! The click row under each measure: one dot per beat group showing how loud
//! the metronome clicks it (see [`grooph_measure::click`]). A tap moves the
//! group to the next level; the change applies to every measure in that
//! meter and is undoable. While playing, the group being played lights up.

use crate::Grooph;
use eframe::egui;
use eframe::egui::{Color32, Id, Pos2, Rect, Sense, Stroke, pos2, vec2};
use grooph_layout::staff_layout::{PlacedMeasure, StaffLayout};
use grooph_measure::click::{ClickLevel, ClickPattern};
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::{Measure, MeasureIdx, TimeSignature};
use grooph_render::measure::playback_cursor_x;

/// Extra room below the staff line for the click row, in em.
pub(crate) const CLICK_ROW_EM: f32 = 0.5;

/// Smallest tap target of a dot (points), so it stays tappable on phones.
const MIN_TAP_SIZE: f32 = 32.0;

/// One dot of the click row.
struct ClickDot {
    center: Pos2,
    level: ClickLevel,
    group: usize,
}

/// Radius of a dot of `level` (points) at staff size `em`: a share of the em,
/// but never so small that the three sizes blur together on a phone. Off is
/// an outline of the medium size.
fn dot_radius(level: ClickLevel, em: f32) -> f32 {
    let (share, min) = match level {
        ClickLevel::Strong => (0.11, 6.0),
        ClickLevel::Medium | ClickLevel::Off => (0.075, 4.5),
        ClickLevel::Soft => (0.05, 3.0),
    };
    (share * em).max(min)
}

fn level_label(level: ClickLevel) -> &'static str {
    match level {
        ClickLevel::Strong => "strong",
        ClickLevel::Medium => "medium",
        ClickLevel::Soft => "soft",
        ClickLevel::Off => "off",
    }
}

/// Tick at which each beat group of `ts` starts: the downbeat and the primary
/// beats, the same ticks the metronome clicks.
fn group_start_ticks(ts: &TimeSignature) -> Vec<u32> {
    std::iter::once(0).chain(DEFAULT_GRID.primary_boundaries(ts)).collect()
}

/// Index of the group that `tick` falls in.
fn group_at(starts: &[u32], tick: f64) -> usize {
    starts.iter().take_while(|&&s| s as f64 <= tick).count().saturating_sub(1)
}

/// X of `tick` in a placed measure: on the note that starts there, or between
/// the notes around it. Falls back to a share of the note area.
fn x_of_tick(measure: &Measure, placed: &PlacedMeasure, tick: u32) -> f32 {
    playback_cursor_x(measure, &placed.layout, placed.rect, tick as f64, 1.0).unwrap_or_else(|| {
        let total = DEFAULT_GRID.ticks_per_measure(&measure.time_signature()).max(1) as f32;
        let left = placed.layout.notes_left_edge;
        left + (placed.rect.right() - left) * tick as f32 / total
    })
}

impl Grooph {
    /// Draw the click row of every measure and handle taps on it. `playing`
    /// is the measure and local tick being played, if any.
    pub(crate) fn click_row(
        &mut self,
        ui: &egui::Ui,
        staff: &StaffLayout,
        em: f32,
        playing: Option<(MeasureIdx, f64)>,
    ) {
        // While playing along, a tap on the staff is a hit, not an edit.
        let interactive = !self.tap_capture_active();
        let dark = ui.visuals().dark_mode;
        let color = ui.visuals().weak_text_color();
        let highlight = ui.visuals().selection.stroke.color;
        let mut tapped: Option<(TimeSignature, usize)> = None;

        for system in &staff.systems {
            for placed in &system.measures {
                let Some(measure) = self.editor.score.measures.get(placed.measure_idx) else {
                    continue;
                };
                let ts = measure.time_signature();
                let pattern = self.playback_ctl.audio_cfg.click.for_meter(ts);
                let starts = group_start_ticks(&ts);
                let y = system.rect.bottom() - 0.5 * CLICK_ROW_EM * em;
                let dots: Vec<ClickDot> = starts
                    .iter()
                    .zip(pattern.levels())
                    .enumerate()
                    .map(|(group, (&tick, &level))| ClickDot {
                        center: pos2(x_of_tick(measure, placed, tick), y),
                        level,
                        group,
                    })
                    .collect();
                let current = playing
                    .filter(|(m_idx, tick)| *m_idx == placed.measure_idx && *tick >= 0.0)
                    .map(|(_, tick)| group_at(&starts, tick));

                for (i, dot) in dots.iter().enumerate() {
                    let lit = current == Some(dot.group);
                    paint_dot(ui.painter(), dot, em, if lit { highlight } else { color }, dark);
                    if !interactive {
                        continue;
                    }
                    // Tap target: half way to the neighbouring dots, at least MIN_TAP_SIZE.
                    let half_gap = |other: Option<&ClickDot>| {
                        other.map_or(f32::INFINITY, |o| (o.center.x - dot.center.x).abs() / 2.0)
                    };
                    let half_width = half_gap(i.checked_sub(1).and_then(|j| dots.get(j)))
                        .min(half_gap(dots.get(i + 1)))
                        .min(0.4 * em)
                        .max(MIN_TAP_SIZE / 2.0);
                    let height = (CLICK_ROW_EM * em).max(MIN_TAP_SIZE);
                    let target = Rect::from_center_size(dot.center, vec2(2.0 * half_width, height));
                    let id = Id::new(("click_dot", placed.measure_idx, dot.group));
                    let resp = ui.interact(target, id, Sense::click()).on_hover_text(format!(
                        "Click: {} (tap to change)",
                        level_label(dot.level)
                    ));
                    if resp.hovered() {
                        ui.painter().circle_stroke(
                            dot.center,
                            dot_radius(ClickLevel::Strong, em) * 1.6,
                            Stroke::new(1.0_f32, color),
                        );
                    }
                    if resp.clicked() {
                        tapped = Some((ts, dot.group));
                    }
                }
            }
        }

        if let Some((ts, group)) = tapped {
            self.cycle_click_level(ts, group);
        }
    }

    /// Move one group of `ts` to its next click level, undoably.
    pub(crate) fn cycle_click_level(&mut self, ts: TimeSignature, group: usize) {
        let mut levels = self.playback_ctl.audio_cfg.click.for_meter(ts).levels().to_vec();
        let Some(level) = levels.get_mut(group) else {
            return;
        };
        *level = level.next();
        let snapshot = self.current_snapshot();
        if self.playback_ctl.audio_cfg.click.set(ts, ClickPattern(levels)) {
            self.editor.history.push(snapshot);
        }
    }
}

fn paint_dot(painter: &egui::Painter, dot: &ClickDot, em: f32, color: Color32, dark: bool) {
    let radius = dot_radius(dot.level, em);
    match dot.level {
        ClickLevel::Off => {
            painter.circle_stroke(dot.center, radius, Stroke::new((0.015 * em).max(1.0), color));
        }
        ClickLevel::Soft => {
            // Lighter than the others, so three sizes read as three volumes.
            let soft = color.gamma_multiply(if dark { 0.7 } else { 0.6 });
            painter.circle_filled(dot.center, radius, soft);
        }
        ClickLevel::Strong | ClickLevel::Medium => {
            painter.circle_filled(dot.center, radius, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_start_on_the_metronome_clicks() {
        let eighth = DEFAULT_GRID.ticks_per_beat(&TimeSignature::SEVEN_EIGHT);
        // 7/8 as 2+3+2.
        assert_eq!(group_start_ticks(&TimeSignature::SEVEN_EIGHT), [0, 2 * eighth, 5 * eighth]);
        assert_eq!(group_start_ticks(&TimeSignature::FOUR_FOUR).len(), 4);
    }

    #[test]
    fn group_at_finds_the_group_being_played() {
        let starts = [0, 100, 250];
        assert_eq!(group_at(&starts, 0.0), 0);
        assert_eq!(group_at(&starts, 99.9), 0);
        assert_eq!(group_at(&starts, 100.0), 1);
        assert_eq!(group_at(&starts, 300.0), 2);
    }
}
