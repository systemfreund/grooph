//! Metronome tab: its own meter, apart from the rhythm's, whose click bar (see
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
//! shows the bar number. The segment under the pointer (or a finger) and its
//! note on the staff light each other, from either side, and a tap on the
//! note changes its level like one on the segment.
//!
//! A meter can have a counter rhythm (see [`grooph_measure::counter`]), e.g.
//! three clicks against the four beats of 4/4. It plays in this tab only and
//! shows as a second ring inside the first, one segment per click, tapped
//! through the same levels. It is the staff's lower voice and can be edited
//! like the click bar: the editor holds both as one measure of two voices,
//! and the voice of the cursor, last tapped on the staff or the ring, is the
//! one edited (see [`MetronomeVoice`]).

use crate::documents::Document;
use crate::pickers::shown_groups;
use crate::shell::paint_meter;
use crate::state::CountInState;
use crate::{Grooph, TransportState};
use eframe::egui;
use eframe::egui::epaint::PathShape;
use eframe::egui::{Align2, Color32, FontId, Mesh, Pos2, Sense, Shape, Stroke, Ui, vec2};
use grooph_measure::click::{ClickLevel, meter_click_bar, same_bar, subdivisions};
use grooph_measure::counter::{
    CounterBars, Ratio, click_level, counter_bar, counter_clicks, counter_ratios, is_ratio_bar,
    named_ratio_of,
};
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::score::Score;
use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{Beat, Cursor, Measure, TimeSignature, VoiceIdx};
use grooph_render::{highlight_color, playback_cursor_stroke};
use std::cell::RefCell;
use std::f32::consts::{FRAC_PI_2, TAU};
use std::rc::Rc;

/// Gap between two segments, as a share of the full turn.
const SEGMENT_GAP: f32 = 0.012;
/// Distance of the hover mark from the outside of the thickest segments.
const HOVER_OFFSET: f32 = 6.0;
/// Share of the room from the view's center to its nearest edge that the
/// outer ring's radius takes at most.
const RING_ROOM: f32 = 0.88;
/// Width of the outline that smooths the filled segments' edges.
const FILL_OUTLINE: f32 = 1.0;
/// Width of the hover mark's line.
const HOVER_STROKE: f32 = 2.0;
/// Points per radian on the ring's arcs, so they look round at any size.
const ARC_STEP: f32 = 0.05;
/// Radius of the counter rhythm's ring, as a share of the outer ring's.
const COUNTER_RING: f32 = 0.6;
/// Thickness of the counter ring's segments, as a share of the outer ones.
const COUNTER_THICKNESS: f32 = 0.75;
/// Brightness of soft segments relative to the others.
const SOFT_DIM: f32 = 0.6;

/// A voice of the metronome's bar: the click bar, the first voice of the
/// staff, or the counter rhythm below it. The editor holds both as one
/// measure; the cursor's voice is the one being edited.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MetronomeVoice {
    #[default]
    Click,
    Counter,
}

impl MetronomeVoice {
    /// The bar's voice on the staff.
    pub(crate) const fn idx(self) -> VoiceIdx {
        match self {
            MetronomeVoice::Click => 0,
            MetronomeVoice::Counter => 1,
        }
    }

    /// The bar of the staff's voice `voice`, if it is one of the two.
    pub(crate) fn of(voice: VoiceIdx) -> Option<Self> {
        match voice {
            0 => Some(MetronomeVoice::Click),
            1 => Some(MetronomeVoice::Counter),
            _ => None,
        }
    }
}

pub(crate) struct MetronomeState {
    /// Meter the metronome clicks, separate from the rhythm's.
    pub(crate) meter: TimeSignature,
    /// The note of the ring's segment under the pointer (or finger), which
    /// the staff lights. Written by the ring every frame it is drawn.
    pub(crate) ring_hover: Option<(MetronomeVoice, usize)>,
    /// The note under the pointer on the staff, whose segment the ring
    /// marks. Written by the staff every frame it is drawn with the ring.
    pub(crate) staff_hover: Option<(MetronomeVoice, usize)>,
}

impl MetronomeState {
    pub(crate) fn new(meter: TimeSignature) -> Self {
        Self { meter, ring_hover: None, staff_hover: None }
    }

    /// Forget what the pointer was on, e.g. while the staff is not drawn.
    pub(crate) fn forget_hover(&mut self) {
        self.ring_hover = None;
        self.staff_hover = None;
    }
}

/// The ring's idle and lit colours. The idle segments sit between the
/// background and the lit one, and the lit one is a saturated blue, so the
/// click being played stands out by lightness and by hue.
fn ring_colors(dark_mode: bool) -> (Color32, Color32) {
    if dark_mode {
        (Color32::from_gray(90), Color32::from_rgb(100, 175, 255))
    } else {
        (Color32::from_gray(175), Color32::from_rgb(25, 118, 210))
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
    let onsets = DEFAULT_GRID.compute_onset_ticks(bar.first_voice().beats());
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

/// Distance from the center to the hover mark of a segment on the circle of
/// `radius`, thick as [`thickness`] of `unit`: just outside the thickest
/// segments.
fn hover_mark_radius(radius: f32, unit: f32) -> f32 {
    radius + thickness(ClickLevel::Strong) * unit / 2.0 + HOVER_OFFSET
}

/// How far the outer ring's hover mark reaches from the center: its line
/// and a point for the anti-aliasing.
fn hover_mark_reach(radius: f32) -> f32 {
    hover_mark_radius(radius, radius) + HOVER_STROKE / 2.0 + 1.0
}

/// Radius of the outer ring with `room` from the center to the nearest edge
/// of the view: [`RING_ROOM`] of it, or less so that the hover mark stays inside.
fn ring_radius(room: f32) -> f32 {
    // The mark's reach grows linearly with the radius.
    let base = hover_mark_reach(0.0);
    let slope = hover_mark_reach(1.0) - base;
    (RING_ROOM * room).min((room - base) / slope)
}

/// Points of an arc from `start` to `end` (shares of the full turn).
fn arc(center: Pos2, radius: f32, start: f32, end: f32) -> Vec<Pos2> {
    arc_of_steps(center, radius, start, end, arc_steps(start, end))
}

/// How many steps [`arc`] takes from `start` to `end`.
fn arc_steps(start: f32, end: f32) -> usize {
    (((end - start) * TAU) / ARC_STEP).ceil().max(1.0) as usize
}

/// Points of an arc in `steps` even steps.
fn arc_of_steps(center: Pos2, radius: f32, start: f32, end: f32, steps: usize) -> Vec<Pos2> {
    (0..=steps)
        .map(|i| at_turn(center, radius, start + (end - start) * i as f32 / steps as f32))
        .collect()
}

/// A ready-made click bar (see [`click_fills`]).
pub(crate) struct ClickFill {
    pub bar: Measure,
    pub name: String,
    /// Whether it splits the beat units: all but the main beats do.
    subdivides: bool,
}

/// How a [`ClickFill`] is shown: the main beats as "no subdivision" (in
/// 5/8 they are a dotted quarter and a quarter, no one note value), the
/// others as one beat unit of their notes.
pub(crate) enum FillLook {
    NoSubdivision,
    Notes(Measure),
}

impl ClickFill {
    /// How it is shown. Built on demand: the label needs only the names.
    pub(crate) fn look(&self) -> Option<FillLook> {
        if !self.subdivides {
            return Some(FillLook::NoSubdivision);
        }
        tile_of(&self.bar, 1).map(FillLook::Notes)
    }
}

/// Ready-made click bars of `ts`: the default, then every beat unit split
/// into 1 to 9 clicks where the meter allows, each with a name. Bars alike
/// are offered once. Kept per meter once built: the options sheet asks for
/// them every frame, and building the bars takes milliseconds.
pub(crate) fn click_fills(ts: TimeSignature) -> Rc<[ClickFill]> {
    thread_local! {
        static FILLS: RefCell<Vec<(TimeSignature, Rc<[ClickFill]>)>> = const { RefCell::new(Vec::new()) };
    }
    FILLS.with_borrow_mut(|cache| {
        if let Some((_, fills)) = cache.iter().find(|(t, _)| *t == ts) {
            return Rc::clone(fills);
        }
        let fills: Rc<[ClickFill]> = build_click_fills(ts).into();
        cache.push((ts, Rc::clone(&fills)));
        fills
    })
}

fn build_click_fills(ts: TimeSignature) -> Vec<ClickFill> {
    subdivisions(ts)
        .iter()
        .map(|o| ClickFill {
            subdivides: o.code != "none",
            name: o.name.clone(),
            bar: o.bar.clone(),
        })
        .collect()
}

/// The first `units` beat units of `bar` as a bar of their own, every
/// click medium: no accents or soft clicks.
fn tile_of(bar: &Measure, units: u8) -> Option<Measure> {
    let ts = bar.time_signature();
    let end = u32::from(units) * DEFAULT_GRID.ticks_per_beat(&ts);
    let onsets = DEFAULT_GRID.compute_onset_ticks(bar.first_voice().beats());
    let mut tile = Measure::new(TimeSignature::new(units, ts.beat_unit));
    for (idx, (b, &onset)) in bar.first_voice().beats().iter().zip(&onsets).enumerate() {
        if onset >= end {
            break;
        }
        tile.first_voice_mut().set_beat(idx, Beat::new(b.duration, b.kind())).ok()?;
    }
    Some(tile)
}

/// The ratio a counter bar plays, `named` if it was picked as that one, and
/// whether its clicks were edited since it was picked: louder, softer or
/// muted ones the ratio's own bar (every click medium) does not have.
pub(crate) fn counter_ratio(bar: &Measure, named: Option<Ratio>) -> Option<(Ratio, bool)> {
    let ratio = named_ratio_of(bar, named)?;
    let edited = !is_ratio_bar(bar, ratio);
    Some((ratio, edited))
}

/// "3:4", or "3:4 edited" once its clicks were changed.
pub(crate) fn ratio_label((ratio, edited): (Ratio, bool)) -> String {
    if edited { format!("{ratio} edited") } else { ratio.to_string() }
}

impl Grooph {
    /// Whether the metronome tab is open (also behind the More screen): its
    /// click bar is the document edited and the rhythm is silent.
    pub(crate) fn metronome_mode(&self) -> bool {
        self.editor.document == Document::Metronome
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
        if self.metronome_mode() { 1 } else { self.editor.doc().score.len() }
    }

    /// Open the metronome's document on the click bar of its meter, with
    /// its counter rhythm: links and pickers change them while it is closed.
    pub(crate) fn open_metronome(&mut self) {
        let bar = self.metronome_bar(self.click_bar(), self.counter_bar().as_ref());
        let doc = &mut self.editor.docs.metronome;
        doc.score = Score::single(bar);
        doc.cursor = Cursor::start();
        self.editor.metronome.forget_hover();
    }

    /// Store the click bar as the metronome's document closes.
    pub(crate) fn close_metronome(&mut self) {
        self.sync_click_bar();
        self.editor.metronome.forget_hover();
    }

    /// Close the metronome, if open, for something that needs a rhythm
    /// (a calibration, a preset, a link): shows the practice page.
    pub(crate) fn leave_metronome(&mut self) {
        if self.metronome_mode() {
            self.show_document(Document::Practice);
        }
    }

    /// While the tab is open: the edited bar is the click bar of its meter
    /// with the counter rhythm, if any, as its second voice, and its meter
    /// the metronome's (undo can bring back another one). The audio
    /// settings follow it.
    pub(crate) fn sync_click_bar(&mut self) {
        if !self.metronome_mode() {
            return;
        }
        let editor = &mut self.editor;
        let doc = &mut editor.docs.metronome;
        doc.score.truncate(1);
        doc.cursor = doc.score.clamp_cursor(doc.cursor);
        let bar = &doc.score.measures()[0];
        let ts = bar.time_signature();
        editor.metronome.meter = ts;
        let cfg = &mut self.playback_ctl.audio_cfg;
        // Compared voice by voice, copied only when changed: this runs
        // every frame.
        let click = MetronomeVoice::Click.idx();
        if cfg.click.for_meter(ts).first_voice().beats() != bar.first_voice().beats() {
            cfg.click.set(bar.single_voice(click).expect("a measure's first voice"));
        }
        let counter = MetronomeVoice::Counter.idx();
        let same_counter = match (cfg.counter.for_meter(ts), bar.voice(counter)) {
            (None, None) => true,
            (Some(a), Some(b)) => a.first_voice().beats() == b.beats(),
            _ => false,
        };
        if !same_counter {
            cfg.counter.set(ts, bar.single_voice(counter));
        }
    }

    /// The bar the metronome tab edits: `click` with `counter`, if any, as
    /// its second voice.
    fn metronome_bar(&self, click: Measure, counter: Option<&Measure>) -> Measure {
        let mut bar = click;
        if let Some(counter) = counter {
            bar.push_voices(counter).expect("a counter rhythm in its click bar's meter");
        }
        bar
    }

    /// The voice the cursor edits in the metronome tab.
    pub(crate) fn edited_voice(&self) -> MetronomeVoice {
        if !self.metronome_mode() {
            return MetronomeVoice::Click;
        }
        let cursor = self.editor.doc().score.clamp_cursor(self.editor.doc().cursor);
        MetronomeVoice::of(cursor.voice).unwrap_or_default()
    }

    /// Edit `voice` from now on: the click bar, or the counter rhythm if
    /// the meter has one. Moves the cursor to the voice's first beat; not
    /// an edit, so not undoable.
    pub(crate) fn select_voice(&mut self, voice: MetronomeVoice) {
        if !self.metronome_mode() {
            return;
        }
        let voice = if self.counter_bar().is_some() { voice } else { MetronomeVoice::Click };
        if voice != self.edited_voice() {
            self.editor.doc_mut().cursor = Cursor::in_voice(0, voice.idx(), 0);
        }
    }

    /// The click bar of the metronome meter, edited or not.
    pub(crate) fn click_bar(&self) -> Measure {
        match self.metronome_mode() {
            true => self.editor.doc().score.measures()[0]
                .single_voice(MetronomeVoice::Click.idx())
                .expect("a measure's first voice"),
            false => self.playback_ctl.audio_cfg.click.for_meter(self.metronome_meter()),
        }
    }

    /// Click `ts` from now on, starting over: its click bar is
    /// `meter_click_bar` and it has no counter rhythm, whatever it had when
    /// it was last picked. Undoable, like a bar change.
    pub(crate) fn set_metronome_meter(&mut self, ts: TimeSignature) {
        if ts == self.metronome_meter() {
            return;
        }
        let bar = meter_click_bar(ts);
        if !self.metronome_mode() {
            self.editor.metronome.meter = ts;
            let cfg = &mut self.playback_ctl.audio_cfg;
            cfg.click.set(bar);
            cfg.counter.set(ts, None);
            return;
        }
        self.with_undo_snapshot(|app| {
            app.playback_ctl.audio_cfg.counter.set(ts, None);
            app.editor.doc_mut().score = Score::single(bar);
            app.editor.doc_mut().cursor = Cursor::start();
            true
        });
        self.sync_click_bar();
    }

    /// Replace the click bar being edited, undoably.
    pub(crate) fn replace_click_bar(&mut self, bar: Measure) {
        if !self.metronome_mode() {
            return;
        }
        if same_bar(&bar, &self.click_bar()) {
            return;
        }
        let bar = self.metronome_bar(bar, self.counter_bar().as_ref());
        self.with_undo_snapshot(|app| {
            app.editor.doc_mut().score = Score::single(bar);
            app.editor.doc_mut().cursor = Cursor::start();
            true
        });
        self.sync_click_bar();
    }

    /// Name of the click bar being edited if it is a ready-made one, else
    /// "Custom".
    pub(crate) fn click_fill_label(&self) -> String {
        self.current_click_fill()
            .and_then(|i| click_fills(self.metronome_meter()).get(i).map(|f| f.name.clone()))
            .unwrap_or_else(|| "Custom".to_owned())
    }

    /// How the click bar being edited is shown if it is a ready-made one
    /// (see [`ClickFill::look`]).
    pub(crate) fn click_fill_look(&self) -> Option<FillLook> {
        click_fills(self.metronome_meter()).get(self.current_click_fill()?)?.look()
    }

    /// The counter rhythm of the metronome meter, if it has one.
    pub(crate) fn counter_bar(&self) -> Option<Measure> {
        match self.metronome_mode() {
            true => {
                self.editor.doc().score.measures()[0].single_voice(MetronomeVoice::Counter.idx())
            }
            false => self.playback_ctl.audio_cfg.counter.for_meter(self.metronome_meter()).cloned(),
        }
    }

    /// The counter rhythm the audio plays: the metronome meter's, in the
    /// metronome tab only, and not while calibrating.
    pub(crate) fn played_counter(&self) -> CounterBars {
        let mut counter = CounterBars::default();
        if self.metronome_mode() && self.calibration_stash().is_none() {
            let ts = self.metronome_meter();
            counter.set(ts, self.counter_bar());
        }
        counter
    }

    /// Ratios the metronome meter offers for a counter rhythm.
    pub(crate) fn counter_choices(&self) -> Vec<Ratio> {
        counter_ratios(self.metronome_meter())
    }

    /// The counter rhythm's ratio: "Off" without one, "Custom" for one that
    /// is no ratio's, marked "edited" once its clicks were changed.
    pub(crate) fn counter_label(&self) -> String {
        let Some(bar) = self.counter_bar() else {
            return "Off".to_owned();
        };
        let named = self.playback_ctl.audio_cfg.counter.name(self.metronome_meter());
        counter_ratio(&bar, named).map_or_else(|| "Custom".to_owned(), ratio_label)
    }

    /// The counter rhythm's ratio and whether its clicks were edited (see
    /// [`counter_ratio`]), if it has one.
    pub(crate) fn counter_ratio(&self) -> Option<(Ratio, bool)> {
        let named = self.playback_ctl.audio_cfg.counter.name(self.metronome_meter());
        counter_ratio(&self.counter_bar()?, named)
    }

    /// The counter rhythm the Polyrhythm switch turns on: the last one, if
    /// the meter has a counter bar for it, else the meter's first ratio.
    pub(crate) fn counter_to_restore(&self) -> Option<Ratio> {
        let ts = self.metronome_meter();
        self.ui
            .shell
            .last_counter
            .filter(|&r| counter_bar(ts, r).is_some())
            .or_else(|| self.counter_choices().first().copied())
    }

    /// Play `ratio` against the click bar from now on, or no counter rhythm.
    /// Undoable.
    pub(crate) fn set_counter_ratio(&mut self, ratio: Option<Ratio>) {
        let ts = self.metronome_meter();
        let bar = ratio.and_then(|r| counter_bar(ts, r));
        let unchanged = match (&self.counter_bar(), &bar) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                same_bar(a, b) && self.playback_ctl.audio_cfg.counter.name(ts) == ratio
            }
            _ => false,
        };
        if unchanged {
            return;
        }
        self.with_undo_snapshot(|app| {
            app.playback_ctl.audio_cfg.counter.set_ratio(ts, ratio.filter(|_| bar.is_some()));
            if app.metronome_mode() {
                let on_counter = app.edited_voice() == MetronomeVoice::Counter;
                let new_bar = app.metronome_bar(app.click_bar(), bar.as_ref());
                app.editor.doc_mut().score = Score::single(new_bar);
                if on_counter {
                    app.editor.doc_mut().cursor =
                        Cursor::in_voice(0, MetronomeVoice::Counter.idx(), 0);
                }
                let doc = app.editor.doc_mut();
                doc.cursor = doc.score.clamp_cursor(doc.cursor);
            }
            true
        });
    }

    /// A tap on note `idx` of `voice` on the staff, as on its ring segment.
    pub(crate) fn tap_staff_note(&mut self, voice: MetronomeVoice, idx: usize) {
        let spans = self.counter_bar().as_ref().map(counter_spans).unwrap_or_default();
        match segment_of_note(voice, idx, &spans) {
            Some(RingHit::Click(idx)) => self.cycle_click_level(idx),
            Some(RingHit::Counter(idx)) => self.cycle_counter_level(idx),
            None => {}
        }
    }

    /// Move the counter click starting at beat `idx` to its next level, and
    /// edit the counter rhythm from now on.
    fn cycle_counter_level(&mut self, idx: usize) {
        self.select_voice(MetronomeVoice::Counter);
        if self.edited_voice() != MetronomeVoice::Counter {
            return;
        }
        let Some(counter) = self.counter_bar() else {
            return;
        };
        let level = click_level(&counter, idx).next();
        let voice = MetronomeVoice::Counter.idx();
        self.with_undo_snapshot(|app| {
            let bar = &mut app.editor.doc_mut().score.measures_mut()[0];
            bar.voice_mut(voice).expect("checked above").set_click_level(idx, level);
            app.editor.doc_mut().cursor = Cursor::in_voice(0, voice, idx);
            true
        });
    }

    /// Move the click at `idx` to its next level, and edit the click bar
    /// from now on.
    fn cycle_click_level(&mut self, idx: usize) {
        self.select_voice(MetronomeVoice::Click);
        self.with_undo_snapshot(|app| {
            let bar = &mut app.editor.doc_mut().score.measures_mut()[0];
            let Some(level) = bar.first_voice().beats().get(idx).map(ClickLevel::of) else {
                return false;
            };
            bar.first_voice_mut().set_click_level(idx, level.next());
            app.editor.doc_mut().cursor = Cursor::in_voice(0, MetronomeVoice::Click.idx(), idx);
            true
        });
    }

    /// Tick within the bar being heard (latency taken off) at `now`, if
    /// playing. Follows the staff's smoothed playback cursor, carried on to
    /// this frame, so the ring moves as evenly as the cursor, also through
    /// the count-in; the audio position alone only moves once per audio
    /// buffer.
    fn metronome_tick(&self, ts: &TimeSignature, now: f64) -> Option<f64> {
        if self.playback_ctl.transport_state != TransportState::Playing {
            return None;
        }
        let audio = self.playback_ctl.audio.as_ref()?;
        // Waiting for the output, as the staff's cursor does: on the web the
        // position runs ahead by the buffers rendered before it plays.
        if audio.is_starting() {
            return Some(0.0);
        }
        let (tick, _) = audio.playback_position()?;
        let bar_ticks = DEFAULT_GRID.ticks_per_measure(ts) as f64;
        let ticks_per_sec = ScoreTiming::from_score(&self.metronome_score(), self.current_bpm())
            .ticks_per_sec_in_measure(0);
        let playback = &self.playback_ctl.playback;
        let heard = match playback.last_update {
            Some(last) => playback.heard_tick + (now - last).max(0.0) * ticks_per_sec,
            None => tick - self.playback_ctl.audio_cfg.latency_s() * ticks_per_sec,
        };
        Some(heard.rem_euclid(bar_ticks.max(1.0)))
    }

    /// Whether the count-in's first click has been heard yet. Until then,
    /// with output latency, the ring's turn still wraps to the bar's end.
    fn count_in_click_heard(&self, ts: &TimeSignature) -> bool {
        let Some((tick, _)) = self
            .playback_ctl
            .audio
            .as_ref()
            .filter(|a| !a.is_starting())
            .and_then(|a| a.playback_position())
        else {
            return false;
        };
        let bar_ticks = DEFAULT_GRID.ticks_per_measure(ts) as f64;
        let ticks_per_sec = ScoreTiming::from_score(&self.metronome_score(), self.current_bpm())
            .ticks_per_sec_in_measure(0);
        tick - self.playback_ctl.audio_cfg.latency_s() * ticks_per_sec >= -bar_ticks
    }

    /// The ring with the click bar, `height` tall at the bottom of the space
    /// left between the panels. With `play_in_middle` (the inline view in a
    /// chat) its middle is the play button.
    pub(crate) fn metronome_ring(&mut self, ui: &mut Ui, height: f32, play_in_middle: bool) {
        egui::Panel::bottom("metronome_ring")
            .exact_size(height)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(ui.visuals().panel_fill))
            .show(ui, |ui| self.ring(ui, play_in_middle));
    }

    fn ring(&mut self, ui: &mut Ui, play_in_middle: bool) {
        // Nothing hovered unless the ring is drawn below.
        self.editor.metronome.ring_hover = None;
        let bar = self.click_bar();
        let ts = bar.time_signature();
        let spans = beat_spans(&bar);
        let counter = self.counter_bar();
        let counter_spans = counter.as_ref().map(counter_spans).unwrap_or_default();
        let tick = self.metronome_tick(&ts, ui.input(|i| i.time));
        let bar_ticks = DEFAULT_GRID.ticks_per_measure(&ts).max(1) as f64;
        let current_turn = tick.map(|t| (t / bar_ticks) as f32);
        let current = current_turn.and_then(|t| beat_at_turn(&spans, t));
        let counter_now = current_turn.and_then(|t| {
            counter_spans.iter().position(|&(_, start, end)| (start..end).contains(&t))
        });
        let bar_state = match self.playback_ctl.audio.as_mut() {
            Some(audio) if tick.is_some() => audio.bar_state(),
            _ => None,
        };
        if tick.is_some() {
            ui.ctx().request_repaint();
        }

        let area = ui.available_rect_before_wrap();
        let (rect, resp) = ui.allocate_exact_size(area.size(), Sense::click());
        // Too small for a ring below 20 points at the full share of the room.
        let room = rect.width().min(rect.height()) / 2.0;
        if RING_ROOM * room < 20.0 {
            return;
        }
        let radius = ring_radius(room);
        let center = rect.center();
        let inner = COUNTER_RING * radius;

        let visuals = ui.visuals();
        let muted = self.playback_ctl.playback.bar_muted;
        let (idle, lit) = ring_colors(visuals.dark_mode);
        let base = if muted { idle.gamma_multiply(0.5) } else { idle };
        let painter = ui.painter();

        // Tapped or hovered click: from the pointer's angle and its distance
        // from the middle. With a counter rhythm the two rings split the
        // room halfway between them.
        let on_ring = |pos: Pos2| {
            let d = pos - center;
            let turn = ((d.y.atan2(d.x) + FRAC_PI_2) / TAU).rem_euclid(1.0);
            let band = (0.3 * radius).max(24.0);
            let dist = d.length();
            if counter.is_some() && dist < (radius + inner) / 2.0 {
                if dist < inner - band {
                    return None;
                }
                let idx = counter_spans.iter().position(|&(_, s, e)| (s..e).contains(&turn))?;
                return Some(RingHit::Counter(counter_spans[idx].0));
            }
            if (dist - radius).abs() > band {
                return None;
            }
            beat_at_turn(&spans, turn).map(RingHit::Click)
        };
        let hovered = resp.hover_pos().and_then(on_ring);
        let hover_color = visuals.weak_text_color();
        // The staff lights the hovered segment's note, and a note hovered on
        // the staff marks its segment here: a counter click spans its rests.
        self.editor.metronome.ring_hover = hovered.map(|hit| match hit {
            RingHit::Click(idx) => (MetronomeVoice::Click, idx),
            RingHit::Counter(idx) => (MetronomeVoice::Counter, idx),
        });
        let marked = marked_segment(hovered, self.editor.metronome.staff_hover, &counter_spans);

        for (idx, (&(start, end), beat)) in spans.iter().zip(bar.first_voice().beats()).enumerate()
        {
            let color = if current == Some(idx) && !muted { lit } else { base };
            let segment = Segment { start, end, level: ClickLevel::of(beat), color };
            segment.paint(painter, center, radius, radius);
            if marked == Some(RingHit::Click(idx)) {
                segment.paint_hover(painter, center, radius, radius, hover_color);
            }
        }

        // Where the hand starts, inside the innermost ring or dots.
        let hand_from = if let Some(counter) = &counter {
            for (i, &(idx, start, end)) in counter_spans.iter().enumerate() {
                let color = if counter_now == Some(i) && !muted { lit } else { base };
                let level = click_level(counter, idx);
                let segment = Segment { start, end, level, color };
                let unit = COUNTER_THICKNESS * radius;
                segment.paint(painter, center, inner, unit);
                if marked == Some(RingHit::Counter(idx)) {
                    segment.paint_hover(painter, center, inner, unit, hover_color);
                }
            }
            inner - thickness(ClickLevel::Strong) * COUNTER_THICKNESS * radius / 2.0 - 4.0
        } else {
            // A dot for every beat unit inside the ring, bigger ones where a
            // beat group starts. The counter ring takes their place.
            let units = ts.beats.max(1) as usize;
            let mut group_starts = vec![0usize];
            for &g in ts.groups().iter() {
                group_starts.push(group_starts.last().copied().unwrap_or(0) + g as usize);
            }
            let dot_radius = (0.0375 * radius).max(3.75);
            let dot_now = current_turn.map(|t| (t * units as f32) as usize);
            for dot in 0..units {
                let pos = at_turn(center, 0.72 * radius, dot as f32 / units as f32);
                let color = if dot_now == Some(dot) && !muted { lit } else { base };
                let r = if group_starts.contains(&dot) { dot_radius } else { 0.6 * dot_radius };
                painter.circle_filled(pos, r, color);
            }
            0.72 * radius - dot_radius - 4.0
        };

        // The play button in the middle, as big as the room inside the dots
        // or the counter ring leaves (at least a finger's width).
        let button = play_in_middle.then(|| (0.75 * hand_from).max(22.0).min(hand_from));
        let hand_from = button.map_or(hand_from, |r| r + 4.0);

        // A hand while playing, from inside the dots (or across both rings
        // with a counter rhythm, so it shows where the clicks of the two
        // rhythms fall against each other). Drawn like the staff's playback
        // cursor.
        if let Some(turn) = current_turn.filter(|_| !muted) {
            let to = radius + thickness(ClickLevel::Strong) * radius / 2.0 + 4.0;
            let stroke = playback_cursor_stroke(self.ui.staff_em, highlight_color(visuals));
            painter.line_segment(
                [at_turn(center, hand_from, turn), at_turn(center, to, turn)],
                stroke,
            );
        }

        // Middle: the beat being counted during the count-in, the bar number
        // while playing, else the meter (and the counter rhythm's ratio).
        let count_in_beat = current_turn
            .filter(|_| self.playback_ctl.playback.count_in == CountInState::Active)
            .map(|t| if self.count_in_click_heard(&ts) { count_in_beat(t, &ts) } else { 1 });
        if let Some(button) = button {
            let playing = self.playback_ctl.transport_state == TransportState::Playing;
            let tapped = paint_play_middle(ui, &resp, center, button, playing, count_in_beat);
            if tapped {
                self.toggle_playback();
            } else if resp.clicked() {
                match resp.interact_pointer_pos().and_then(on_ring) {
                    Some(RingHit::Click(idx)) => self.cycle_click_level(idx),
                    Some(RingHit::Counter(idx)) => self.cycle_counter_level(idx),
                    None => {}
                }
            }
            return;
        }
        let caption = match counter.as_ref().and(self.counter_ratio()) {
            Some(ratio) => ratio_label(ratio),
            None if counter.is_some() => "custom".to_owned(),
            // Under the meter: its beat groups when there are several.
            None => shown_groups(ts).unwrap_or_default(),
        };
        let (big, small) = match (count_in_beat, bar_state) {
            (Some(beat), _) => (Some(beat.to_string()), String::new()),
            (None, Some(b)) => {
                (Some((b.bar + 1).to_string()), if muted { "muted" } else { "bar" }.to_owned())
            }
            // The meter, stacked in the music font as in the settings card.
            (None, None) => (None, caption),
        };
        // Smaller inside the counter ring.
        let scale = if counter.is_some() { 0.8 } else { 1.0 };
        match big {
            Some(text) => {
                painter.text(
                    center - vec2(0.0, 0.08 * scale * radius),
                    Align2::CENTER_CENTER,
                    text,
                    FontId::proportional(0.42 * scale * radius),
                    visuals.strong_text_color(),
                );
            }
            None => {
                // Alone it sits in the middle; above a caption, smaller and higher.
                let (lift, em) = if small.is_empty() { (0.0, 0.5) } else { (0.12, 0.44) };
                paint_meter(
                    painter,
                    ts,
                    center - vec2(0.0, lift * scale * radius),
                    em * scale * radius,
                    1.1 * scale * radius,
                    &self.ui.music_font_id.family,
                    visuals.strong_text_color(),
                );
            }
        }
        painter.text(
            center + vec2(0.0, 0.26 * scale * radius),
            Align2::CENTER_CENTER,
            small,
            FontId::proportional((0.14 * scale * radius).clamp(13.0, 22.0)),
            visuals.weak_text_color(),
        );

        if resp.clicked() {
            match resp.interact_pointer_pos().and_then(on_ring) {
                Some(RingHit::Click(idx)) => self.cycle_click_level(idx),
                Some(RingHit::Counter(idx)) => self.cycle_counter_level(idx),
                None => {}
            }
        }
    }
}

/// The play button in the ring's middle, `radius` around `center`: play when
/// stopped; the beat being counted during the count-in, on a faded button;
/// stop while playing. Returns whether it was tapped.
pub(crate) fn paint_play_middle(
    ui: &Ui,
    resp: &egui::Response,
    center: Pos2,
    radius: f32,
    playing: bool,
    count_in_beat: Option<u32>,
) -> bool {
    let visuals = ui.visuals();
    let on_button = |pos: Pos2| (pos - center).length() <= radius;
    if resp.hover_pos().is_some_and(on_button) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let pressed =
        resp.is_pointer_button_down_on() && resp.interact_pointer_pos().is_some_and(on_button);
    let mut fill = visuals.selection.bg_fill;
    if count_in_beat.is_some() {
        fill = fill.gamma_multiply(0.35);
    }
    if pressed {
        fill = fill.gamma_multiply(0.8);
    }
    let painter = ui.painter();
    painter.circle_filled(center, radius, fill);
    let icon = visuals.selection.stroke.color;
    let icon_rect = egui::Rect::from_center_size(center, vec2(radius, radius) * 1.2);
    match count_in_beat {
        Some(beat) => {
            painter.text(
                center,
                Align2::CENTER_CENTER,
                beat.to_string(),
                FontId::proportional(1.1 * radius),
                visuals.strong_text_color(),
            );
        }
        None => {
            crate::shell::paint_play_icon(painter, icon_rect, icon, playing);
        }
    }
    resp.clicked() && resp.interact_pointer_pos().is_some_and(on_button)
}

/// What a tap on the ring hits: a beat of the click bar, or a click of the
/// counter rhythm (the index of its first beat).
#[derive(Clone, Copy, PartialEq, Eq)]
enum RingHit {
    Click(usize),
    Counter(usize),
}

/// One arc of the ring.
struct Segment {
    start: f32,
    end: f32,
    level: ClickLevel,
    color: Color32,
}

impl Segment {
    /// Paint on the circle of `radius`, as thick as [`thickness`] of `unit`.
    fn paint(&self, painter: &egui::Painter, center: Pos2, radius: f32, unit: f32) {
        let width = thickness(self.level) * unit;
        // Filled segments are inset by half their outline, so the outline
        // ends where the segment does and does not narrow the gap.
        let inset = if self.level == ClickLevel::Off { 0.0 } else { FILL_OUTLINE / 2.0 };
        let (outer, inner) = self.band(center, radius, width, inset);
        let mut outline = outer.clone();
        outline.extend(inner.iter().rev());
        match self.level {
            ClickLevel::Off => {
                painter.add(PathShape::closed_line(outline, Stroke::new(2.0_f32, self.color)));
            }
            _ => {
                let color = if self.level == ClickLevel::Soft {
                    self.color.gamma_multiply(SOFT_DIM)
                } else {
                    self.color
                };
                // Filled as a strip of quads between the two arcs: egui only
                // fills convex paths.
                let mut mesh = Mesh::default();
                for (&o, &i) in outer.iter().zip(&inner) {
                    mesh.colored_vertex(o, color);
                    mesh.colored_vertex(i, color);
                }
                for k in 0..outer.len().saturating_sub(1) as u32 {
                    let a = 2 * k;
                    mesh.add_triangle(a, a + 1, a + 2);
                    mesh.add_triangle(a + 1, a + 3, a + 2);
                }
                painter.add(mesh);
                // The mesh has hard edges; a thin outline smooths them.
                painter.add(PathShape::closed_line(outline, Stroke::new(FILL_OUTLINE, color)));
            }
        }
    }

    /// The outer and inner arc of the segment as a band `width` thick around
    /// `radius` and `inset` smaller on every side (at most down to no width),
    /// both clockwise and with as many points.
    fn band(&self, center: Pos2, radius: f32, width: f32, inset: f32) -> (Vec<Pos2>, Vec<Pos2>) {
        let inset = inset.min(width / 2.0);
        let (r_out, r_in) = (radius + width / 2.0 - inset, radius - width / 2.0 + inset);
        let (start, end) = self.trimmed(radius, r_out, inset);
        let steps = arc_steps(start, end);
        let edge = |r: f32| {
            let (start, end) = self.trimmed(radius, r, inset);
            arc_of_steps(center, r, start, end, steps)
        };
        (edge(r_out), edge(r_in))
    }

    /// The hover mark just outside the segment.
    fn paint_hover(
        &self,
        painter: &egui::Painter,
        center: Pos2,
        radius: f32,
        unit: f32,
        color: Color32,
    ) {
        let outer = hover_mark_radius(radius, unit);
        let (start, end) = self.trimmed(radius, outer, 0.0);
        painter.add(Shape::line(arc(center, outer, start, end), Stroke::new(HOVER_STROKE, color)));
    }

    /// Start and end at distance `r` from the center, with half the gap
    /// between segments taken off each. The gap is equally wide at any
    /// distance, so the ends of neighbouring segments run parallel; on the
    /// ring's `radius` it is [`SEGMENT_GAP`] of the turn. `inset` widens each
    /// half of the gap.
    fn trimmed(&self, radius: f32, r: f32, inset: f32) -> (f32, f32) {
        let half_gap = SEGMENT_GAP.min(0.3 * (self.end - self.start)) * TAU * radius / 2.0 + inset;
        let trim = (half_gap / r).min(1.0).asin() / TAU;
        let mid = (self.start + self.end) / 2.0;
        ((self.start + trim).min(mid), (self.end - trim).max(mid))
    }
}

/// Where each click of a counter bar sits on the ring: its first beat, start
/// and end (shares of the full turn). A click spans the rests it was written
/// with, see [`counter_clicks`].
fn counter_spans(bar: &Measure) -> Vec<(usize, f32, f32)> {
    let spans = beat_spans(bar);
    counter_clicks(bar)
        .into_iter()
        .filter_map(|r| Some((r.start, spans.get(r.start)?.0, spans.get(r.end - 1)?.1)))
        .collect()
}

/// The segment the ring marks: the one under the pointer, else the one of
/// the note hovered on the staff.
fn marked_segment(
    hovered: Option<RingHit>,
    staff_hover: Option<(MetronomeVoice, usize)>,
    counter_spans: &[(usize, f32, f32)],
) -> Option<RingHit> {
    hovered.or_else(|| {
        let (voice, idx) = staff_hover?;
        segment_of_note(voice, idx, counter_spans)
    })
}

/// The ring segment of note `idx` of `voice`: a click of the click bar, or
/// the counter click whose span (see [`counter_spans`]) holds it.
fn segment_of_note(
    voice: MetronomeVoice,
    idx: usize,
    counter_spans: &[(usize, f32, f32)],
) -> Option<RingHit> {
    match voice {
        MetronomeVoice::Click => Some(RingHit::Click(idx)),
        MetronomeVoice::Counter => counter_spans
            .iter()
            .rev()
            .find(|&&(start, _, _)| start <= idx)
            .map(|&(start, _, _)| RingHit::Counter(start)),
    }
}

/// Beat of the count-in bar (from 1) at `turn`: it clicks every beat unit.
fn count_in_beat(turn: f32, ts: &TimeSignature) -> u32 {
    let units = ts.beats.max(1) as u32;
    ((turn * units as f32) as u32).min(units - 1) + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::Tab;
    use grooph_measure::click::default_click_bar;
    use grooph_measure::notation::parse_score;

    fn bar(text: &str) -> Measure {
        parse_score(text).unwrap().into_measures().remove(0)
    }

    fn segment(start: f32, end: f32) -> Segment {
        Segment { start, end, level: ClickLevel::Strong, color: Color32::WHITE }
    }

    #[test]
    fn segment_gap_is_equally_wide_at_any_distance() {
        let radius = 100.0;
        let seg = segment(0.1, 0.3);
        // Distance of each end from the line through the center at the
        // segment's start and end: half the gap, inside as outside.
        let half_gap = SEGMENT_GAP * TAU * radius / 2.0;
        for r in [70.0, 100.0, 130.0] {
            let (start, end) = seg.trimmed(radius, r, 0.0);
            assert!((r * ((start - 0.1) * TAU).sin() - half_gap).abs() < 1e-3, "start at {r}");
            assert!((r * ((0.3 - end) * TAU).sin() - half_gap).abs() < 1e-3, "end at {r}");
        }
    }

    #[test]
    fn filled_band_is_inset_by_half_its_outline() {
        let (center, radius, width) = (Pos2::ZERO, 100.0, 20.0);
        let seg = segment(0.1, 0.3);
        let inset = FILL_OUTLINE / 2.0;
        let (outer, inner) = seg.band(center, radius, width, inset);
        assert_eq!(outer.len(), inner.len());
        for p in &outer {
            assert!((p.distance(center) - (radius + width / 2.0 - inset)).abs() < 1e-3);
        }
        for p in &inner {
            assert!((p.distance(center) - (radius - width / 2.0 + inset)).abs() < 1e-3);
        }
        // The outline's half outside the band ends where the gap begins.
        let half_gap = SEGMENT_GAP * TAU * radius / 2.0;
        for p in [outer[0], inner[0]] {
            let (start, _) = seg.trimmed(radius, p.distance(center), inset);
            let r = p.distance(center);
            assert!((r * ((start - 0.1) * TAU).sin() - inset - half_gap).abs() < 1e-3);
        }
        // Thinner than the outline: no width left, but not inverted.
        let (outer, inner) = seg.band(center, radius, 0.4, inset);
        assert!(outer[0].distance(center) >= inner[0].distance(center) - 1e-3);
    }

    #[test]
    fn short_segment_keeps_its_ends_in_order() {
        let seg = segment(0.5, 0.501);
        for r in [1.0, 50.0, 100.0] {
            let (start, end) = seg.trimmed(100.0, r, 0.0);
            assert!(0.5 <= start && start <= end && end <= 0.501, "at {r}: {start}..{end}");
        }
        // On the ring's radius the gap is capped at 30 % of the segment.
        let (start, end) = seg.trimmed(100.0, 100.0, 0.0);
        assert!(((0.501 - 0.5) - (end - start) - 0.3 * 0.001).abs() < 1e-5, "{start}..{end}");
    }

    #[test]
    fn ring_leaves_room_for_the_hover_mark() {
        for room in [23.0, 50.0, 150.0, 400.0, 2000.0] {
            let radius = ring_radius(room);
            assert!(radius <= RING_ROOM * room);
            assert!(hover_mark_reach(radius) <= room + 1e-3, "room {room}");
        }
        // Large views keep the usual 88 %.
        assert_eq!(ring_radius(2000.0), RING_ROOM * 2000.0);
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
    fn count_in_counts_every_beat_unit() {
        let four = TimeSignature::FOUR_FOUR;
        let counts: Vec<_> =
            [0.0, 0.24, 0.25, 0.6, 0.99, 1.0].map(|t| count_in_beat(t, &four)).into();
        assert_eq!(counts, [1, 1, 2, 3, 4, 4]);
        assert_eq!(count_in_beat(0.9, &TimeSignature::SEVEN_EIGHT), 7);
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
        let names = |ts| click_fills(ts).iter().map(|f| f.name.clone()).collect::<Vec<_>>();
        // One click per quarter is the default bar already.
        assert_eq!(
            names(TimeSignature::FOUR_FOUR),
            [
                "No subdivision",
                "Eighths",
                "Eighth triplets",
                "Sixteenths",
                "Quintuplets",
                "Sextuplets",
                "Septuplets",
                "Thirty-seconds",
                "Nonuplets",
            ]
        );
        assert_eq!(
            names(TimeSignature::SIX_EIGHT),
            [
                "No subdivision",
                "Eighths",
                "Sixteenths",
                "Sixteenth triplets",
                "Thirty-seconds",
                "Quintuplets",
                "Sextuplets",
                "Septuplets",
            ]
        );
    }

    #[test]
    fn fills_show_one_beat_unit_and_the_main_beats_no_subdivision() {
        // 5/8's main beats are a dotted quarter and a quarter: no one note.
        let fills = click_fills(TimeSignature::new(5, 8));
        assert!(matches!(fills[0].look(), Some(FillLook::NoSubdivision)));
        let fills = click_fills(TimeSignature::SIX_EIGHT);
        let tiles: Vec<_> = fills[1..]
            .iter()
            .map(|f| match f.look() {
                Some(FillLook::Notes(m)) => m,
                _ => panic!("{} has no notes", f.name),
            })
            .collect();
        // Eighths, sixteenths, sixteenth triplets: one eighth's worth.
        let counts: Vec<_> = tiles[..3].iter().map(|t| t.first_voice().beats().len()).collect();
        assert_eq!(counts, [1, 2, 3]);
        for tile in &tiles {
            assert_eq!(tile.time_signature(), TimeSignature::new(1, 8));
            assert!(
                tile.first_voice().beats().iter().all(|b| ClickLevel::of(b) == ClickLevel::Medium)
            );
        }
        for ts in crate::pickers::all_time_signatures() {
            assert!(click_fills(ts).iter().all(|f| f.look().is_some()), "{ts}");
        }
    }

    #[test]
    fn picked_meters_click_their_beat_unit() {
        let ts = TimeSignature::FOUR_FOUR;
        assert!(same_bar(&meter_click_bar(ts), &default_click_bar(ts)));
        let ts = TimeSignature::SEVEN_EIGHT;
        assert!(same_bar(&meter_click_bar(ts), &bar("7/8 >e (e) e (e) (e) e (e)")));
        let ts: TimeSignature = "5/16".parse().unwrap();
        assert_eq!(meter_click_bar(ts).first_voice().beats().len(), 5);
        // Its name in the Subdivision picker.
        let fills = click_fills(TimeSignature::SEVEN_EIGHT);
        let name =
            fills.iter().find(|f| same_bar(&f.bar, &meter_click_bar(TimeSignature::SEVEN_EIGHT)));
        assert_eq!(name.map(|f| f.name.as_str()), Some("Eighths"));
    }

    #[test]
    fn counter_segments_span_a_click_and_its_rests() {
        let ts = TimeSignature::THREE_FOUR;
        let mut bar = counter_bar(ts, Ratio { n: 5, span: 3 }).unwrap();
        bar.first_voice_mut().set_click_level(3, ClickLevel::Off);
        let spans = counter_spans(&bar);
        // Five clicks of a fifth of the bar each, every one a sixteenth
        // quintuplet and two rests.
        assert_eq!(spans.iter().map(|s| s.0).collect::<Vec<_>>(), [0, 3, 6, 9, 12]);
        for (i, &(_, start, end)) in spans.iter().enumerate() {
            assert!((start - i as f32 / 5.0).abs() < 1e-6);
            assert!((end - (i + 1) as f32 / 5.0).abs() < 1e-6);
        }
    }

    #[test]
    fn a_note_marks_the_segment_it_sounds_or_rests_in() {
        let bar = counter_bar(TimeSignature::THREE_FOUR, Ratio { n: 5, span: 3 }).unwrap();
        let spans = counter_spans(&bar);
        // The click and its two rests all belong to the click's segment.
        for (idx, segment) in [(0, 0), (2, 0), (3, 3), (5, 3), (14, 12)] {
            assert!(
                segment_of_note(MetronomeVoice::Counter, idx, &spans)
                    == Some(RingHit::Counter(segment))
            );
        }
        assert!(segment_of_note(MetronomeVoice::Click, 2, &spans) == Some(RingHit::Click(2)));
    }

    #[test]
    fn the_ring_marks_its_own_hover_else_the_staff_note() {
        let bar = counter_bar(TimeSignature::THREE_FOUR, Ratio { n: 5, span: 3 }).unwrap();
        let spans = counter_spans(&bar);
        let marked = |hovered, staff| marked_segment(hovered, staff, &spans);
        assert!(marked(None, Some((MetronomeVoice::Counter, 4))) == Some(RingHit::Counter(3)));
        assert!(marked(None, Some((MetronomeVoice::Click, 1))) == Some(RingHit::Click(1)));
        // A finger on the ring wins over what the staff last reported.
        let on_ring = Some(RingHit::Click(2));
        assert!(marked(on_ring, Some((MetronomeVoice::Counter, 4))) == on_ring);
        assert!(marked(None, None).is_none());
    }

    /// Runs frames of the whole app at phone size with the pointer at `pos`
    /// (hover follows a frame behind the pointer).
    fn hover(app: &mut Grooph, ctx: &egui::Context, pos: Pos2) {
        for _ in 0..3 {
            frame(app, ctx, vec![egui::Event::PointerMoved(pos)]);
        }
    }

    /// A tap at `pos`, then the pointer stays there.
    fn tap(app: &mut Grooph, ctx: &egui::Context, pos: Pos2) {
        hover(app, ctx, pos);
        for pressed in [true, false] {
            frame(app, ctx, vec![button(pos, pressed)]);
        }
        hover(app, ctx, pos);
    }

    /// One frame of the whole app at phone size.
    fn frame(app: &mut Grooph, ctx: &egui::Context, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(390.0, 844.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            app.apply_style(ui);
            app.ui.narrow_screen = true;
            app.shell_ui(ui);
            app.sync_click_bar();
        });
        output.textures_delta.clear();
    }

    /// What the ring and the staff report while the pointer moves up the
    /// screen left of the ring's middle: each change once.
    fn hovers_upwards(app: &mut Grooph, ctx: &egui::Context) -> Vec<HoverState> {
        hovers_along(app, ctx, (0..820).rev().step_by(4).map(|y| Pos2::new(180.0, y as f32)))
            .into_iter()
            .map(|(state, _)| state)
            .collect()
    }

    /// What the ring and the staff report at each of `points`, each change
    /// once with the first point it showed at.
    fn hovers_along(
        app: &mut Grooph,
        ctx: &egui::Context,
        points: impl Iterator<Item = Pos2>,
    ) -> Vec<(HoverState, Pos2)> {
        let mut seen: Vec<(HoverState, Pos2)> = Vec::new();
        for pos in points {
            hover(app, ctx, pos);
            let m = &app.editor.metronome;
            let now = (m.ring_hover, m.staff_hover);
            if seen.last().map(|s| s.0) != Some(now) {
                seen.push((now, pos));
            }
        }
        seen
    }

    /// The staff's notes under the pointer, left to right along the row at
    /// `y`, each with a point on it.
    fn staff_notes(
        app: &mut Grooph,
        ctx: &egui::Context,
        y: f32,
    ) -> Vec<(MetronomeVoice, usize, Pos2)> {
        hovers_along(app, ctx, (5..385).step_by(3).map(|x| Pos2::new(x as f32, y)))
            .into_iter()
            .filter_map(|((_, staff), pos)| staff.map(|(v, i)| (v, i, pos)))
            .collect()
    }

    /// A phone-sized app in the Metronome tab, clicking `meter`'s default bar.
    fn metronome_on(ctx: &egui::Context, meter: TimeSignature) -> Grooph {
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.select_tab(crate::shell::Tab::Metronome);
        app.set_metronome_meter(meter);
        app.replace_click_bar(default_click_bar(meter));
        app
    }

    /// A height on the staff where `voice` reports its notes, from the top.
    fn staff_row(app: &mut Grooph, ctx: &egui::Context, voice: MetronomeVoice) -> f32 {
        (100..500)
            .step_by(4)
            .map(|y| y as f32)
            .find(|&y| {
                hover(app, ctx, Pos2::new(195.0, y));
                app.editor.metronome.staff_hover.is_some_and(|(v, _)| v == voice)
            })
            .expect("a staff row")
    }

    fn indices(notes: &[(MetronomeVoice, usize, Pos2)]) -> Vec<usize> {
        notes.iter().map(|n| n.1).collect()
    }

    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn a_tap_on_the_staff_changes_the_level_as_on_the_ring() {
        let ctx = egui::Context::default();
        let mut app = metronome_on(&ctx, TimeSignature::FOUR_FOUR);
        let row = staff_row(&mut app, &ctx, MetronomeVoice::Click);
        let notes = staff_notes(&mut app, &ctx, row);
        assert_eq!(indices(&notes), [0, 1, 2, 3], "{notes:?}");

        // Every tap moves the note's click on, also two in a row.
        let (_, idx, pos) = notes[2];
        let level = |app: &Grooph| ClickLevel::of(&app.click_bar().first_voice().beats()[idx]);
        let before = level(&app);
        tap(&mut app, &ctx, pos);
        assert_eq!(level(&app), before.next());
        tap(&mut app, &ctx, pos);
        assert_eq!(level(&app), before.next().next());
    }

    #[test]
    fn a_silent_bar_keeps_a_rest_per_click_to_tap() {
        let ctx = egui::Context::default();
        let meters =
            [TimeSignature::FOUR_FOUR, TimeSignature::SIX_EIGHT, TimeSignature::SEVEN_EIGHT];
        for meter in meters {
            let mut app = metronome_on(&ctx, meter);
            let row = staff_row(&mut app, &ctx, MetronomeVoice::Click);
            let mut silent = app.click_bar();
            let clicks = silent.first_voice().beats().len();
            for i in 0..clicks {
                silent.first_voice_mut().set_click_level(i, ClickLevel::Off);
            }
            app.replace_click_bar(silent);
            let notes = staff_notes(&mut app, &ctx, row);
            assert_eq!(indices(&notes), (0..clicks).collect::<Vec<_>>(), "{meter:?}: {notes:?}");
            let last = clicks - 1;
            tap(&mut app, &ctx, notes[last].2);
            let level = ClickLevel::of(&app.click_bar().first_voice().beats()[last]);
            assert_eq!(level, ClickLevel::Off.next(), "{meter:?}");
        }
    }

    #[test]
    fn a_tap_on_a_counter_rest_changes_its_click() {
        let ctx = egui::Context::default();
        let mut app = metronome_on(&ctx, TimeSignature::THREE_FOUR);
        app.set_counter_ratio(Some(Ratio { n: 5, span: 3 }));
        let row = staff_row(&mut app, &ctx, MetronomeVoice::Counter);
        let notes = staff_notes(&mut app, &ctx, row);
        assert!(notes.iter().all(|n| n.0 == MetronomeVoice::Counter), "{notes:?}");
        // The second click is a note and two rests: tap its last rest.
        let start = counter_spans(&app.counter_bar().unwrap())[1].0;
        let (_, _, pos) = *notes.iter().find(|n| n.1 == start + 2).expect("the rest");
        let level = |app: &Grooph| click_level(&app.counter_bar().unwrap(), start);
        let before = level(&app);
        tap(&mut app, &ctx, pos);
        assert_eq!(level(&app), before.next());
    }

    #[test]
    fn a_held_finger_lights_like_a_hover() {
        let ctx = egui::Context::default();
        let mut app = metronome_on(&ctx, TimeSignature::FOUR_FOUR);
        let row = staff_row(&mut app, &ctx, MetronomeVoice::Click);
        let on_staff = staff_notes(&mut app, &ctx, row)[1].2;
        let up_the_ring = (500..820).rev().map(|y| Pos2::new(180.0, y as f32));
        let (_, on_ring) = hovers_along(&mut app, &ctx, up_the_ring)
            .into_iter()
            .find(|(state, _)| state.0.is_some())
            .expect("the ring");
        let held = [
            (on_staff, (None, Some((MetronomeVoice::Click, 1)))),
            (on_ring, (Some((MetronomeVoice::Click, 2)), None)),
        ];
        for (pos, want) in held {
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos), button(pos, true)]);
            for _ in 0..3 {
                frame(&mut app, &ctx, vec![]);
                let m = &app.editor.metronome;
                assert_eq!((m.ring_hover, m.staff_hover), want);
            }
            frame(&mut app, &ctx, vec![button(pos, false), egui::Event::PointerGone]);
            frame(&mut app, &ctx, vec![]);
        }
    }

    type HoverState = (Option<(MetronomeVoice, usize)>, Option<(MetronomeVoice, usize)>);

    #[test]
    fn ring_and_staff_report_the_hovered_note() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Grooph::new(&cc);
        app.select_tab(crate::shell::Tab::Metronome);
        app.set_metronome_meter(TimeSignature::FOUR_FOUR);
        app.replace_click_bar(default_click_bar(TimeSignature::FOUR_FOUR));
        app.set_counter_ratio(Some(Ratio { n: 3, span: 4 }));
        let counter = counter_spans(&app.counter_bar().unwrap());

        let seen = hovers_upwards(&mut app, &ctx);
        let rings: Vec<_> = seen.iter().filter_map(|s| s.0).collect();
        // Up the left half: the third quarter of the click bar, the counter's
        // second and third clicks, then the last quarter.
        let want = [
            (MetronomeVoice::Click, 2),
            (MetronomeVoice::Counter, counter[1].0),
            (MetronomeVoice::Counter, counter[2].0),
            (MetronomeVoice::Click, 3),
        ];
        assert_eq!(rings, want, "{seen:?}");
        // Never both at once; the staff above reports its notes.
        assert!(seen.iter().all(|s| s.0.is_none() || s.1.is_none()), "{seen:?}");
        assert!(seen.iter().any(|s| s.1.is_some()), "{seen:?}");

        // Editing the bar, the ring is gone and the staff lights nothing; the
        // ring's last hover is forgotten, so leaving the editor lights none.
        app.editor.metronome.ring_hover = Some((MetronomeVoice::Click, 1));
        app.ui.mode = crate::Mode::Edit;
        frame(&mut app, &ctx, vec![]);
        assert!(app.editor.metronome.ring_hover.is_none());
        let seen = hovers_upwards(&mut app, &ctx);
        assert!(seen.iter().all(|s| s.1.is_none()), "{seen:?}");
    }

    fn metronome_app() -> Grooph {
        let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        let mut app = Grooph::new(&cc);
        app.select_tab(Tab::Practice);
        app.editor.metronome.meter = TimeSignature::FOUR_FOUR;
        app.select_tab(Tab::Metronome);
        app
    }

    #[test]
    fn practice_accuracy_marks_stay_out_of_the_metronome() {
        let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        let mut app = Grooph::new(&cc);
        // A fresh app opens the metronome; start from practice.
        app.select_tab(Tab::Practice);
        app.playback_ctl.accuracy.enabled = true;
        assert!(app.shows_accuracy_marks());
        // The marks of a practice run would land on the click bar's notes.
        app.select_tab(Tab::Metronome);
        assert!(!app.shows_accuracy_marks());
        // Back in practice they would belong to the rhythm again (a page
        // change clears the last run's marks, see `show_document`).
        app.select_tab(Tab::Practice);
        assert!(app.shows_accuracy_marks());
    }

    #[test]
    fn the_cursors_voice_is_the_edited_bar() {
        let mut app = metronome_app();
        app.set_counter_ratio(Some(Ratio { n: 3, span: 4 }));
        // The editor holds click and counter as one bar of two voices.
        assert_eq!(app.editor.doc().score.measures()[0].voices().len(), 2);
        assert_eq!(app.edited_voice(), MetronomeVoice::Click);
        app.select_voice(MetronomeVoice::Counter);
        assert_eq!(app.editor.doc().cursor, Cursor::in_voice(0, 1, 0));
        assert_eq!(app.edited_voice(), MetronomeVoice::Counter);
        // A cursor moved into a voice edits it, as in a kit score.
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 0, 1);
        assert_eq!(app.edited_voice(), MetronomeVoice::Click);
        // A tool edits the counter rhythm in its voice, not the clicks.
        let click = app.click_bar();
        app.editor.doc_mut().cursor = Cursor::in_voice(0, 1, 0);
        app.with_undo_snapshot(|app| {
            app.current_voice_mut().set_click_level(0, ClickLevel::Off);
            true
        });
        app.sync_click_bar();
        assert!(same_bar(&app.click_bar(), &click));
        let counter = app.playback_ctl.audio_cfg.counter.for_meter(TimeSignature::FOUR_FOUR);
        assert_eq!(counter.map(|c| click_level(c, 0)), Some(ClickLevel::Off));
    }

    #[test]
    fn the_click_fill_is_found_with_a_counter_rhythm() {
        let mut app = metronome_app();
        let fill = app.current_click_fill();
        assert!(fill.is_some());
        app.set_counter_ratio(Some(Ratio { n: 3, span: 4 }));
        assert_eq!(app.current_click_fill(), fill);
    }

    #[test]
    fn the_counter_rhythm_is_edited_like_the_click_bar() {
        let mut app = metronome_app();
        let click = app.click_bar();
        app.set_counter_ratio(Some(Ratio { n: 3, span: 4 }));
        app.select_voice(MetronomeVoice::Counter);
        assert_eq!(app.edited_voice(), MetronomeVoice::Counter);
        // The cursor is in the counter's voice; an edit goes to the counter.
        app.with_undo_snapshot(|app| {
            app.current_voice_mut().set_click_level(2, ClickLevel::Strong);
            true
        });
        app.sync_click_bar();
        let counter = app.playback_ctl.audio_cfg.counter.for_meter(TimeSignature::FOUR_FOUR);
        assert_eq!(counter.map(|c| click_level(c, 2)), Some(ClickLevel::Strong));
        assert!(same_bar(&app.click_bar(), &click));
        // Back to the clicks: the edit stays, the click bar is the editor's.
        app.select_voice(MetronomeVoice::Click);
        assert!(same_bar(&app.click_bar(), &click));
        assert_eq!(app.counter_bar().map(|c| click_level(&c, 2)), Some(ClickLevel::Strong));
    }

    #[test]
    fn a_ratio_of_ones_own_with_a_common_factor_is_named_as_picked() {
        let mut app = metronome_app();
        // 6:4 plays 3:2's bar, but stays 6:4.
        app.set_counter_ratio(Some(Ratio { n: 6, span: 4 }));
        assert_eq!(app.counter_label(), "6:4");
        app.cycle_counter_level(2);
        assert_eq!(app.counter_label(), "6:4 edited");
        app.set_counter_ratio(Some(Ratio { n: 6, span: 4 }));
        // Picking 3:2 renames it, undoably.
        app.set_counter_ratio(Some(Ratio { n: 3, span: 2 }));
        assert_eq!(app.counter_label(), "3:2");
        app.undo();
        assert_eq!(app.counter_label(), "6:4");
    }

    #[test]
    fn an_edited_counter_rhythm_says_so_until_picked_again() {
        let mut app = metronome_app();
        let ratio = Ratio { n: 3, span: 4 };
        app.set_counter_ratio(Some(ratio));
        assert_eq!(app.counter_label(), "3:4");
        app.cycle_counter_level(2);
        assert_eq!(app.counter_label(), "3:4 edited");
        // A muted click still plays the ratio, edited.
        while app.counter_bar().map(|c| click_level(&c, 2)) != Some(ClickLevel::Off) {
            app.cycle_counter_level(2);
        }
        assert_eq!(app.counter_label(), "3:4 edited");
        // Picking the ratio again starts its clicks over.
        app.set_counter_ratio(Some(ratio));
        assert_eq!(app.counter_label(), "3:4");
    }

    #[test]
    fn undo_brings_back_the_voice_it_was_taken_in() {
        let mut app = metronome_app();
        app.set_counter_ratio(Some(Ratio { n: 3, span: 4 }));
        app.cycle_counter_level(2);
        assert_eq!(app.edited_voice(), MetronomeVoice::Counter);
        let counter_edited = app.counter_bar().unwrap();
        app.cycle_click_level(1);
        assert_eq!(app.edited_voice(), MetronomeVoice::Click);
        // Undoing the click edit leaves the counter alone...
        app.undo();
        app.sync_click_bar();
        assert_eq!(app.edited_voice(), MetronomeVoice::Click);
        assert!(same_bar(&app.click_bar(), &default_click_bar(TimeSignature::FOUR_FOUR)));
        assert!(same_bar(&app.counter_bar().unwrap(), &counter_edited));
        // ...and undoing the counter edit goes back to editing the counter,
        // never writing it into the click bar.
        app.undo();
        app.sync_click_bar();
        assert_eq!(app.edited_voice(), MetronomeVoice::Counter);
        let plain = counter_bar(TimeSignature::FOUR_FOUR, Ratio { n: 3, span: 4 }).unwrap();
        assert!(same_bar(&app.counter_bar().unwrap(), &plain));
        assert!(same_bar(&app.click_bar(), &default_click_bar(TimeSignature::FOUR_FOUR)));
    }

    #[test]
    fn redo_across_a_voice_switch_keeps_both_voices() {
        let mut app = metronome_app();
        app.set_counter_ratio(Some(Ratio { n: 3, span: 4 }));
        app.cycle_click_level(1);
        let click_edited = app.click_bar();
        app.select_voice(MetronomeVoice::Counter);
        app.undo();
        app.sync_click_bar();
        assert!(!same_bar(&app.click_bar(), &click_edited));
        app.redo();
        app.sync_click_bar();
        assert!(same_bar(&app.click_bar(), &click_edited));
        assert!(app.counter_bar().is_some());
    }

    #[test]
    fn turning_the_counter_off_goes_back_to_the_clicks() {
        let mut app = metronome_app();
        app.set_counter_ratio(Some(Ratio { n: 3, span: 4 }));
        app.select_voice(MetronomeVoice::Counter);
        app.set_counter_ratio(None);
        assert_eq!(app.edited_voice(), MetronomeVoice::Click);
        assert!(app.counter_bar().is_none());
        assert!(same_bar(
            &app.editor.doc().score.measures()[0],
            &default_click_bar(TimeSignature::FOUR_FOUR)
        ));
        // Without a counter there is no lower voice to pick.
        app.select_voice(MetronomeVoice::Counter);
        assert_eq!(app.edited_voice(), MetronomeVoice::Click);
        // Closing the tab while editing the counter keeps both bars.
        app.set_counter_ratio(Some(Ratio { n: 5, span: 4 }));
        app.select_voice(MetronomeVoice::Counter);
        app.select_tab(Tab::Practice);
        assert_eq!(app.edited_voice(), MetronomeVoice::Click);
        let counter = app.playback_ctl.audio_cfg.counter.for_meter(TimeSignature::FOUR_FOUR);
        assert_eq!(
            counter.and_then(grooph_measure::counter::ratio_of),
            Some(Ratio { n: 5, span: 4 })
        );
    }

    #[test]
    fn a_picked_meter_starts_over() {
        let mut app = metronome_app();
        app.set_counter_ratio(Some(Ratio { n: 3, span: 4 }));
        app.cycle_click_level(1);
        app.set_metronome_meter(TimeSignature::THREE_FOUR);
        assert!(app.counter_bar().is_none());
        // Back in 4/4: plain clicks and no counter rhythm, in the tab...
        app.set_metronome_meter(TimeSignature::FOUR_FOUR);
        assert_eq!(app.edited_voice(), MetronomeVoice::Click);
        assert!(app.counter_bar().is_none());
        assert!(same_bar(&app.click_bar(), &default_click_bar(TimeSignature::FOUR_FOUR)));
        // ...and with it closed.
        app.set_counter_ratio(Some(Ratio { n: 3, span: 4 }));
        app.select_tab(Tab::Practice);
        app.set_metronome_meter(TimeSignature::THREE_FOUR);
        app.set_metronome_meter(TimeSignature::FOUR_FOUR);
        assert!(app.playback_ctl.audio_cfg.counter.for_meter(TimeSignature::FOUR_FOUR).is_none());
        // Undo in the tab brings the counter rhythm back with its meter.
        app.select_tab(Tab::Metronome);
        app.set_counter_ratio(Some(Ratio { n: 3, span: 4 }));
        app.set_metronome_meter(TimeSignature::THREE_FOUR);
        app.undo();
        app.sync_click_bar();
        assert_eq!(app.metronome_meter(), TimeSignature::FOUR_FOUR);
        assert!(app.counter_bar().is_some());
    }

    #[test]
    fn the_metronome_plays_one_silent_bar() {
        let m = Measure::new(TimeSignature::SEVEN_EIGHT);
        assert!(m.first_voice().beats().iter().all(|b| b.kind() == grooph_measure::BeatKind::Rest));
    }
}
