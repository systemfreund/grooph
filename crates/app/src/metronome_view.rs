//! Metronome tab: the rhythm is silent and a ring shows the click pattern of
//! the meter (see [`grooph_measure::click`]), one segment per beat group, as
//! long as the group. The segment's thickness shows how loud it clicks; a tap
//! moves it to the next level. While playing, the group being played lights
//! up and the middle shows the bar number.

use crate::{Grooph, Mode, TransportState};
use eframe::egui;
use eframe::egui::epaint::PathShape;
use eframe::egui::{Align2, FontId, Pos2, Sense, Shape, Stroke, Ui, vec2};
use grooph_measure::click::{ClickLevel, ClickPattern};
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::score::Score;
use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{Measure, TimeSignature};
use std::f32::consts::{FRAC_PI_2, TAU};

/// Gap between two segments, as a share of the full turn.
const SEGMENT_GAP: f32 = 0.012;
/// Points per radian on the ring's arcs, so they look round at any size.
const ARC_STEP: f32 = 0.05;

/// Thickness of a segment of `level`, as a share of the ring radius. Off is
/// an outline of the medium thickness.
fn thickness(level: ClickLevel) -> f32 {
    match level {
        ClickLevel::Strong => 0.24,
        ClickLevel::Medium | ClickLevel::Off => 0.15,
        ClickLevel::Soft => 0.08,
    }
}

/// Where each beat group of `ts` sits on the ring: start and end as a share
/// of the full turn, from the top clockwise, proportional to its beats.
fn group_spans(ts: &TimeSignature) -> Vec<(f32, f32)> {
    let groups = ts.groups();
    let total = groups.iter().map(|&g| g as f32).sum::<f32>().max(1.0);
    let mut start = 0.0;
    groups
        .iter()
        .map(|&g| {
            let end = start + g as f32 / total;
            let span = (start, end);
            start = end;
            span
        })
        .collect()
}

/// Index of the group at `turn` (share of the full turn from the top).
fn group_at_turn(spans: &[(f32, f32)], turn: f32) -> Option<usize> {
    spans.iter().position(|&(start, end)| (start..end).contains(&turn))
}

/// Point on a circle around `center` at `turn` (share of the full turn,
/// from the top clockwise).
fn at_turn(center: Pos2, radius: f32, turn: f32) -> Pos2 {
    let angle = turn * TAU - FRAC_PI_2;
    center + radius * vec2(angle.cos(), angle.sin())
}

/// Points of an arc from `start` to `end` (shares of the full turn).
fn arc(center: Pos2, radius: f32, start: f32, end: f32) -> Vec<Pos2> {
    let steps = (((end - start) * TAU) / ARC_STEP).ceil().max(1.0) as usize;
    (0..=steps)
        .map(|i| at_turn(center, radius, start + (end - start) * i as f32 / steps as f32))
        .collect()
}

impl Grooph {
    /// Whether the metronome tab is open: the rhythm is silent then.
    pub(crate) fn metronome_mode(&self) -> bool {
        self.ui.mode == Mode::Metronome
    }

    /// Meter the metronome clicks: the one the rhythms are generated in.
    pub(crate) fn metronome_meter(&self) -> TimeSignature {
        self.editor.generator.settings.time_signature
    }

    /// The silent bar the metronome plays: only its clicks sound.
    pub(crate) fn metronome_score(&self) -> Score {
        Score::single(Measure::new(self.metronome_meter()))
    }

    /// Bars in the score being played, for the tempo ramp's loops.
    pub(crate) fn played_bars(&self) -> usize {
        if self.metronome_mode() { 1 } else { self.editor.score.len() }
    }

    /// Move one group of `ts` to its next click level.
    fn cycle_click_level(&mut self, ts: TimeSignature, group: usize) {
        let mut levels = self.playback_ctl.audio_cfg.click.for_meter(ts).levels().to_vec();
        let Some(level) = levels.get_mut(group) else {
            return;
        };
        *level = level.next();
        self.playback_ctl.audio_cfg.click.set(ts, ClickPattern(levels));
    }

    /// Tick within the bar being heard (latency taken off), if playing.
    fn metronome_tick(&self, ts: &TimeSignature) -> Option<f64> {
        if self.playback_ctl.transport_state != TransportState::Playing {
            return None;
        }
        let (tick, _) = self.playback_ctl.audio.as_ref()?.playback_position()?;
        let bar_ticks = DEFAULT_GRID.ticks_per_measure(ts) as f64;
        let ticks_per_sec = ScoreTiming::from_score(&self.metronome_score(), self.current_bpm())
            .ticks_per_sec_in_measure(0);
        let heard = tick - self.playback_ctl.audio_cfg.latency_s() * ticks_per_sec;
        Some(heard.rem_euclid(bar_ticks.max(1.0)))
    }

    /// The ring with the click pattern of the metronome meter, filling the
    /// space left between the panels.
    pub(crate) fn metronome_panel(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let ts = self.metronome_meter();
            let pattern = self.playback_ctl.audio_cfg.click.for_meter(ts);
            let spans = group_spans(&ts);
            let tick = self.metronome_tick(&ts);
            let bar_ticks = DEFAULT_GRID.ticks_per_measure(&ts).max(1) as f64;
            let current_turn = tick.map(|t| (t / bar_ticks) as f32);
            let current = current_turn.and_then(|t| group_at_turn(&spans, t));
            let bar = match self.playback_ctl.audio.as_mut() {
                Some(audio) if tick.is_some() => audio.bar_state(),
                _ => None,
            };
            if tick.is_some() {
                ui.ctx().request_repaint();
            }

            let area = ui.available_rect_before_wrap();
            let (rect, resp) = ui.allocate_exact_size(area.size(), Sense::click());
            let radius = 0.4 * rect.width().min(rect.height());
            if radius < 20.0 {
                return;
            }
            let center = rect.center();

            let visuals = ui.visuals();
            let muted = self.playback_ctl.playback.bar_muted;
            let base = if muted { visuals.weak_text_color() } else { visuals.text_color() };
            let lit = visuals.selection.stroke.color;
            let soft_dim = if visuals.dark_mode { 0.6 } else { 0.5 };
            let painter = ui.painter();

            // Tapped or hovered group: from the pointer's angle, if it is on the ring.
            let on_ring = |pos: Pos2| {
                let d = pos - center;
                let band = (0.3 * radius).max(24.0);
                if (d.length() - radius).abs() > band {
                    return None;
                }
                let turn = ((d.y.atan2(d.x) + FRAC_PI_2) / TAU).rem_euclid(1.0);
                group_at_turn(&spans, turn)
            };
            let hovered = resp.hover_pos().and_then(on_ring);

            for (group, (&(start, end), &level)) in spans.iter().zip(pattern.levels()).enumerate() {
                let gap = SEGMENT_GAP / 2.0;
                let (start, end) = (start + gap, end - gap);
                let width = thickness(level) * radius;
                let color = if current == Some(group) && !muted { lit } else { base };
                match level {
                    ClickLevel::Off => {
                        let mut points = arc(center, radius + width / 2.0, start, end);
                        points.extend(
                            arc(center, radius - width / 2.0, start, end).into_iter().rev(),
                        );
                        painter.add(PathShape::closed_line(points, Stroke::new(2.0_f32, color)));
                    }
                    _ => {
                        let color = if level == ClickLevel::Soft {
                            color.gamma_multiply(soft_dim)
                        } else {
                            color
                        };
                        painter.add(Shape::line(
                            arc(center, radius, start, end),
                            Stroke::new(width, color),
                        ));
                    }
                }
                if hovered == Some(group) {
                    let outer = radius + thickness(ClickLevel::Strong) * radius / 2.0 + 6.0;
                    painter.add(Shape::line(
                        arc(center, outer, start, end),
                        Stroke::new(2.0_f32, visuals.weak_text_color()),
                    ));
                }
            }

            // A dot for every counted beat, inside the ring.
            let beats = ts.groups().iter().map(|&g| g as usize).sum::<usize>().max(1);
            let dot_radius = (0.025 * radius).max(2.5);
            let beat_now = current_turn.map(|t| (t * beats as f32) as usize);
            for beat in 0..beats {
                let pos = at_turn(center, 0.72 * radius, beat as f32 / beats as f32);
                let color =
                    if beat_now == Some(beat) && !muted { lit } else { visuals.weak_text_color() };
                painter.circle_filled(pos, dot_radius, color);
            }

            // Middle: the bar number while playing, else the meter.
            let (big, small) = match bar {
                Some(b) => {
                    ((b.bar + 1).to_string(), if muted { "muted" } else { "bar" }.to_owned())
                }
                None => (format!("{}/{}", ts.beats, ts.beat_unit), meter_caption(&ts)),
            };
            let big_size = 0.42 * radius;
            painter.text(
                center - vec2(0.0, 0.08 * radius),
                Align2::CENTER_CENTER,
                big,
                FontId::proportional(big_size),
                visuals.strong_text_color(),
            );
            painter.text(
                center + vec2(0.0, 0.26 * radius),
                Align2::CENTER_CENTER,
                small,
                FontId::proportional((0.14 * radius).clamp(13.0, 22.0)),
                visuals.weak_text_color(),
            );

            if let Some(group) = hovered {
                let level = pattern.levels().get(group).copied().unwrap_or(ClickLevel::Medium);
                resp.clone()
                    .on_hover_text(format!("Click: {} (tap to change)", level_label(level)));
            }
            if resp.clicked()
                && let Some(group) = resp.interact_pointer_pos().and_then(on_ring)
            {
                self.cycle_click_level(ts, group);
            }
        });
    }
}

/// Caption under the meter: its beat groups when there are several.
fn meter_caption(ts: &TimeSignature) -> String {
    if ts.beat_unit != 4 && ts.groups().len() > 1 { ts.groups_label() } else { String::new() }
}

fn level_label(level: ClickLevel) -> &'static str {
    match level {
        ClickLevel::Strong => "strong",
        ClickLevel::Medium => "medium",
        ClickLevel::Soft => "soft",
        ClickLevel::Off => "off",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_are_as_long_as_their_groups() {
        // 7/8 as 2+3+2.
        let spans = group_spans(&TimeSignature::SEVEN_EIGHT);
        assert_eq!(spans.len(), 3);
        assert!((spans[0].1 - 2.0 / 7.0).abs() < 1e-6);
        assert!((spans[1].1 - 5.0 / 7.0).abs() < 1e-6);
        assert!((spans[2].1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn group_at_turn_finds_the_segment() {
        let spans = group_spans(&TimeSignature::FOUR_FOUR);
        assert_eq!(group_at_turn(&spans, 0.0), Some(0));
        assert_eq!(group_at_turn(&spans, 0.3), Some(1));
        assert_eq!(group_at_turn(&spans, 0.99), Some(3));
    }

    #[test]
    fn the_metronome_plays_one_silent_bar() {
        let m = Measure::new(TimeSignature::SEVEN_EIGHT);
        assert!(m.beats().iter().all(|b| b.kind == grooph_measure::BeatKind::Rest));
    }
}
