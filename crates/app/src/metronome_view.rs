//! Metronome tab: its own meter, whose click bar (see
//! [`grooph_measure::click`]) is shown as a one-line staff above a ring.
//!
//! While the tab is open the click bar takes the working score's place, so
//! the staff, the tool palette and undo edit it like any rhythm; the rhythm is
//! put aside and comes back when the tab closes. Every frame the bar is
//! written back to the click bars of its meter. What plays is a silent bar of
//! the meter: only the clicks sound.
//!
//! The ring has one segment per note or rest of the click bar, as long as it.
//! The segment's thickness shows how loud it clicks; a tap moves it to the
//! next level. While playing, the click being played lights up and the middle
//! shows the bar number.

use crate::undo::{DEFAULT_UNDO_LIMIT, UndoHistory};
use crate::{Grooph, TransportState};
use eframe::egui;
use eframe::egui::epaint::PathShape;
use eframe::egui::{Align2, FontId, Pos2, Sense, Shape, Stroke, Ui, vec2};
use grooph_measure::click::{ClickLevel, default_click_bar, subdivided_click_bar};
use grooph_measure::duration::Duration;
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::score::Score;
use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{Cursor, Measure, TimeSignature};
use std::f32::consts::{FRAC_PI_2, TAU};

/// Gap between two segments, as a share of the full turn.
const SEGMENT_GAP: f32 = 0.012;
/// Points per radian on the ring's arcs, so they look round at any size.
const ARC_STEP: f32 = 0.05;

/// The rhythm the metronome tab put aside, back when it closes.
pub(crate) struct RhythmStash {
    pub(crate) score: Score,
    pub(crate) cursor: Cursor,
    history: UndoHistory,
    pub(crate) dirty: bool,
    pub(crate) active_pattern_id: Option<u64>,
}

pub(crate) struct MetronomeState {
    /// Meter the metronome clicks, separate from the rhythm's.
    pub(crate) meter: TimeSignature,
    /// `Some` while the tab is open.
    stash: Option<RhythmStash>,
    /// Undo of the click bar edits while the tab is closed.
    history: UndoHistory,
}

impl MetronomeState {
    pub(crate) fn new(meter: TimeSignature) -> Self {
        Self { meter, stash: None, history: UndoHistory::new(DEFAULT_UNDO_LIMIT) }
    }
}

/// Thickness of a segment of `level`, as a share of the ring radius. Off is
/// an outline of the medium thickness.
fn thickness(level: ClickLevel) -> f32 {
    match level {
        ClickLevel::Strong => 0.24,
        ClickLevel::Medium | ClickLevel::Off => 0.15,
        ClickLevel::Soft => 0.08,
    }
}

/// Where each beat of `bar` sits on the ring: start and end as a share of
/// the full turn, from the top clockwise, proportional to its length.
fn beat_spans(bar: &Measure) -> Vec<(f32, f32)> {
    let ts = bar.time_signature();
    let total = DEFAULT_GRID.ticks_per_measure(&ts).max(1) as f32;
    let onsets = DEFAULT_GRID.compute_onset_ticks(bar.beats());
    let mut ends: Vec<f32> = onsets.iter().skip(1).map(|&t| t as f32 / total).collect();
    ends.push(1.0);
    onsets.iter().zip(ends).map(|(&start, end)| (start as f32 / total, end)).collect()
}

/// Index of the beat at `turn` (share of the full turn from the top).
fn beat_at_turn(spans: &[(f32, f32)], turn: f32) -> Option<usize> {
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

/// Ready-made click bars of `ts`: the default, then every beat unit split
/// into 1 to 4 clicks, each with a name. Bars alike are offered once.
pub(crate) fn click_fills(ts: TimeSignature) -> Vec<(Measure, String)> {
    let mut fills = vec![(default_click_bar(ts), "Main beats".to_owned())];
    for parts in 1..=4 {
        let Some(bar) = subdivided_click_bar(ts, parts) else {
            continue;
        };
        if fills.iter().any(|(m, _)| same_bar(m, &bar)) {
            continue;
        }
        let name = bar.beats().first().map_or_else(String::new, |b| fill_name(b.duration));
        fills.push((bar, name));
    }
    fills
}

/// Click bar a meter picked in the metronome starts with: x/4 clicks its
/// beats, x/8 and x/16 every eighth or sixteenth.
pub(crate) fn meter_click_bar(ts: TimeSignature) -> Measure {
    if ts.beat_unit > 4 {
        subdivided_click_bar(ts, 1).unwrap_or_else(|| default_click_bar(ts))
    } else {
        default_click_bar(ts)
    }
}

fn fill_name(d: Duration) -> String {
    let base = d.base_note().name();
    if d.is_tuplet() { format!("{base} triplets") } else { format!("{base}s") }
}

/// Whether two bars hold the same meter and beats.
pub(crate) fn same_bar(a: &Measure, b: &Measure) -> bool {
    a.time_signature() == b.time_signature() && a.beats() == b.beats()
}

impl Grooph {
    /// Whether the metronome tab is open: the click bar is being edited and
    /// the rhythm is silent.
    pub(crate) fn metronome_mode(&self) -> bool {
        self.editor.metronome.stash.is_some()
    }

    /// The rhythm put aside while the metronome tab is open.
    pub(crate) fn metronome_stash(&self) -> Option<&RhythmStash> {
        self.editor.metronome.stash.as_ref()
    }

    /// Meter the metronome tab clicks.
    pub(crate) fn metronome_meter(&self) -> TimeSignature {
        self.editor.metronome.meter
    }

    /// The silent bar the metronome plays: only its clicks sound.
    pub(crate) fn metronome_score(&self) -> Score {
        Score::single(Measure::new(self.metronome_meter()))
    }

    /// Bars in the score being played, for the tempo ramp's loops.
    pub(crate) fn played_bars(&self) -> usize {
        if self.metronome_mode() { 1 } else { self.editor.score.len() }
    }

    /// Put the rhythm aside and edit the click bar of the metronome meter.
    pub(crate) fn open_metronome(&mut self) {
        if self.metronome_mode() {
            return;
        }
        let bar = self.playback_ctl.audio_cfg.click.for_meter(self.metronome_meter());
        let editor = &mut self.editor;
        let history = std::mem::replace(
            &mut editor.history,
            std::mem::replace(&mut editor.metronome.history, UndoHistory::new(0)),
        );
        editor.metronome.stash = Some(RhythmStash {
            score: std::mem::replace(&mut editor.score, Score::single(bar)),
            cursor: std::mem::replace(&mut editor.cursor, Cursor::start()),
            history,
            dirty: editor.dirty,
            active_pattern_id: editor.active_pattern_id,
        });
    }

    /// Store the click bar and bring the rhythm back.
    pub(crate) fn close_metronome(&mut self) {
        self.sync_click_bar();
        let editor = &mut self.editor;
        let Some(stash) = editor.metronome.stash.take() else {
            return;
        };
        editor.metronome.history = std::mem::replace(&mut editor.history, stash.history);
        editor.score = stash.score;
        editor.cursor = stash.cursor;
        editor.dirty = stash.dirty;
        editor.active_pattern_id = stash.active_pattern_id;
    }

    /// While the tab is open: the edited bar is the click bar of its meter,
    /// and its meter the metronome's (undo can bring back another one).
    pub(crate) fn sync_click_bar(&mut self) {
        if !self.metronome_mode() {
            return;
        }
        let editor = &mut self.editor;
        editor.score.measures.truncate(1);
        editor.cursor.measure_idx = 0;
        let bar = &editor.score.measures[0];
        editor.cursor.beat_idx = editor.cursor.beat_idx.min(bar.beats().len().saturating_sub(1));
        editor.metronome.meter = bar.time_signature();
        if self.playback_ctl.audio_cfg.click.for_meter(bar.time_signature()).beats() != bar.beats()
        {
            self.playback_ctl.audio_cfg.click.set(bar.clone());
        }
    }

    /// Edit the click bar of `ts` from now on: its own one, or else
    /// `meter_click_bar`. Undoable, like a bar change.
    pub(crate) fn set_metronome_meter(&mut self, ts: TimeSignature) {
        if ts == self.metronome_meter() {
            return;
        }
        self.sync_click_bar();
        let click = &self.playback_ctl.audio_cfg.click;
        let bar = click.custom_for(ts).cloned().unwrap_or_else(|| meter_click_bar(ts));
        if !self.metronome_mode() {
            self.editor.metronome.meter = ts;
            self.playback_ctl.audio_cfg.click.set(bar);
            return;
        }
        self.replace_click_bar(bar);
    }

    /// A meter picked for the rhythm (Practice or Edit) becomes the
    /// metronome's too, with the click bar that meter already has. Not the
    /// other way round: the metronome's meter leaves the rhythm alone.
    pub(crate) fn follow_meter(&mut self, ts: TimeSignature) {
        if !self.metronome_mode() {
            self.editor.metronome.meter = ts;
        }
    }

    /// Replace the click bar being edited, undoably.
    pub(crate) fn replace_click_bar(&mut self, bar: Measure) {
        if !self.metronome_mode() || same_bar(&bar, &self.editor.score.measures[0]) {
            return;
        }
        self.with_undo_snapshot(|app| {
            app.editor.score = Score::single(bar);
            app.editor.cursor = Cursor::start();
            true
        });
        self.sync_click_bar();
    }

    /// Name of the click bar being edited if it is a ready-made one, else
    /// "Custom".
    pub(crate) fn click_fill_label(&self) -> String {
        let bar = &self.editor.score.measures[0];
        click_fills(bar.time_signature())
            .into_iter()
            .find(|(m, _)| same_bar(m, bar))
            .map_or_else(|| "Custom".to_owned(), |(_, name)| name)
    }

    /// Move the click at `idx` to its next level.
    fn cycle_click_level(&mut self, idx: usize) {
        self.with_undo_snapshot(|app| {
            let bar = &mut app.editor.score.measures[0];
            let Some(level) = bar.beats().get(idx).map(ClickLevel::of) else {
                return false;
            };
            bar.set_click_level(idx, level.next());
            app.editor.cursor = Cursor::at(0, idx);
            true
        });
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

    /// The ring with the click bar, `height` tall at the bottom of the space
    /// left between the panels.
    pub(crate) fn metronome_ring(&mut self, ui: &mut Ui, height: f32) {
        egui::Panel::bottom("metronome_ring")
            .exact_size(height)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(ui.visuals().panel_fill))
            .show_inside(ui, |ui| self.ring(ui));
    }

    fn ring(&mut self, ui: &mut Ui) {
        let bar = self.editor.score.measures[0].clone();
        let ts = bar.time_signature();
        let spans = beat_spans(&bar);
        let tick = self.metronome_tick(&ts);
        let bar_ticks = DEFAULT_GRID.ticks_per_measure(&ts).max(1) as f64;
        let current_turn = tick.map(|t| (t / bar_ticks) as f32);
        let current = current_turn.and_then(|t| beat_at_turn(&spans, t));
        let bar_state = match self.playback_ctl.audio.as_mut() {
            Some(audio) if tick.is_some() => audio.bar_state(),
            _ => None,
        };
        if tick.is_some() {
            ui.ctx().request_repaint();
        }

        let area = ui.available_rect_before_wrap();
        let (rect, resp) = ui.allocate_exact_size(area.size(), Sense::click());
        let radius = 0.44 * rect.width().min(rect.height());
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

        // Tapped or hovered click: from the pointer's angle, if it is on the ring.
        let on_ring = |pos: Pos2| {
            let d = pos - center;
            let band = (0.3 * radius).max(24.0);
            if (d.length() - radius).abs() > band {
                return None;
            }
            let turn = ((d.y.atan2(d.x) + FRAC_PI_2) / TAU).rem_euclid(1.0);
            beat_at_turn(&spans, turn)
        };
        let hovered = resp.hover_pos().and_then(on_ring);

        for (idx, (&(start, end), beat)) in spans.iter().zip(bar.beats()).enumerate() {
            let level = ClickLevel::of(beat);
            let gap = SEGMENT_GAP.min(0.3 * (end - start)) / 2.0;
            let (start, end) = (start + gap, end - gap);
            let width = thickness(level) * radius;
            let color = if current == Some(idx) && !muted { lit } else { base };
            match level {
                ClickLevel::Off => {
                    let mut points = arc(center, radius + width / 2.0, start, end);
                    points.extend(arc(center, radius - width / 2.0, start, end).into_iter().rev());
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
            if hovered == Some(idx) {
                let outer = radius + thickness(ClickLevel::Strong) * radius / 2.0 + 6.0;
                painter.add(Shape::line(
                    arc(center, outer, start, end),
                    Stroke::new(2.0_f32, visuals.weak_text_color()),
                ));
            }
        }

        // A dot for every beat unit inside the ring, bigger ones where a beat
        // group starts.
        let units = ts.beats.max(1) as usize;
        let mut group_starts = vec![0usize];
        for &g in ts.groups().iter() {
            group_starts.push(group_starts.last().copied().unwrap_or(0) + g as usize);
        }
        let dot_radius = (0.0375 * radius).max(3.75);
        let dot_now = current_turn.map(|t| (t * units as f32) as usize);
        for dot in 0..units {
            let pos = at_turn(center, 0.72 * radius, dot as f32 / units as f32);
            let color =
                if dot_now == Some(dot) && !muted { lit } else { visuals.weak_text_color() };
            let r = if group_starts.contains(&dot) { dot_radius } else { 0.6 * dot_radius };
            painter.circle_filled(pos, r, color);
        }

        // Middle: the bar number while playing, else the meter.
        let (big, small) = match bar_state {
            Some(b) => ((b.bar + 1).to_string(), if muted { "muted" } else { "bar" }.to_owned()),
            None => (format!("{}/{}", ts.beats, ts.beat_unit), meter_caption(&ts)),
        };
        painter.text(
            center - vec2(0.0, 0.08 * radius),
            Align2::CENTER_CENTER,
            big,
            FontId::proportional(0.42 * radius),
            visuals.strong_text_color(),
        );
        painter.text(
            center + vec2(0.0, 0.26 * radius),
            Align2::CENTER_CENTER,
            small,
            FontId::proportional((0.14 * radius).clamp(13.0, 22.0)),
            visuals.weak_text_color(),
        );

        if let Some(idx) = hovered {
            let level = bar.beats().get(idx).map_or(ClickLevel::Medium, ClickLevel::of);
            resp.clone().on_hover_text(format!("Click: {} (tap to change)", level_label(level)));
        }
        if resp.clicked()
            && let Some(idx) = resp.interact_pointer_pos().and_then(on_ring)
        {
            self.cycle_click_level(idx);
        }
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
    use grooph_measure::notation::parse_score;

    fn bar(text: &str) -> Measure {
        parse_score(text).unwrap().measures.remove(0)
    }

    #[test]
    fn segments_are_as_long_as_their_clicks() {
        // 7/8 as 2+3+2.
        let spans = beat_spans(&default_click_bar(TimeSignature::SEVEN_EIGHT));
        assert_eq!(spans.len(), 3);
        assert!((spans[0].1 - 2.0 / 7.0).abs() < 1e-6);
        assert!((spans[1].1 - 5.0 / 7.0).abs() < 1e-6);
        assert!((spans[2].1 - 1.0).abs() < 1e-6);
        // A triplet without its middle click still has three segments.
        let spans = beat_spans(&bar("1/4 t8 r:t8 t8"));
        assert_eq!(spans.len(), 3);
        assert!((spans[1].0 - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn beat_at_turn_finds_the_segment() {
        let spans = beat_spans(&default_click_bar(TimeSignature::FOUR_FOUR));
        assert_eq!(beat_at_turn(&spans, 0.0), Some(0));
        assert_eq!(beat_at_turn(&spans, 0.3), Some(1));
        assert_eq!(beat_at_turn(&spans, 0.99), Some(3));
    }

    #[test]
    fn fills_name_each_split_once() {
        let names = |ts| click_fills(ts).into_iter().map(|(_, n)| n).collect::<Vec<_>>();
        // One click per quarter is the default bar already.
        assert_eq!(
            names(TimeSignature::FOUR_FOUR),
            ["Main beats", "Eighths", "Eighth triplets", "Sixteenths"]
        );
        assert_eq!(
            names(TimeSignature::SIX_EIGHT),
            ["Main beats", "Eighths", "Sixteenths", "Sixteenth triplets", "Thirty-seconds"]
        );
    }

    #[test]
    fn picked_meters_click_their_beat_unit() {
        let ts = TimeSignature::FOUR_FOUR;
        assert!(same_bar(&meter_click_bar(ts), &default_click_bar(ts)));
        let ts = TimeSignature::SEVEN_EIGHT;
        assert!(same_bar(&meter_click_bar(ts), &bar("7/8 >e (e) e (e) (e) e (e)")));
        let ts: TimeSignature = "5/16".parse().unwrap();
        assert_eq!(meter_click_bar(ts).beats().len(), 5);
        // Its name in the Clicks picker.
        let fills = click_fills(TimeSignature::SEVEN_EIGHT);
        let name =
            fills.iter().find(|(m, _)| same_bar(m, &meter_click_bar(TimeSignature::SEVEN_EIGHT)));
        assert_eq!(name.map(|(_, n)| n.as_str()), Some("Eighths"));
    }

    #[test]
    fn the_metronome_plays_one_silent_bar() {
        let m = Measure::new(TimeSignature::SEVEN_EIGHT);
        assert!(m.beats().iter().all(|b| b.kind == grooph_measure::BeatKind::Rest));
    }
}
