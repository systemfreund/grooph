//! Multi-measure rendering: walks a [`StaffLayout`] and renders each measure
//! with the existing per-measure renderer. Adds cross-measure visuals
//! (continuous staff line, barlines) and orchestrates the playback cursor
//! wrap animation across measure boundaries.

use crate::measure::{
    draw_beat_anchor, draw_count_layer, draw_edit_cursor, draw_notes, draw_stickings,
    highlight_color, last_note_split, playback_cursor_entering_x, render_measure_at,
};
use eframe::egui;
use eframe::egui::{Color32, Rangef, Stroke};
use grooph_layout::glyphs;
use grooph_layout::staff::Staff;
use grooph_layout::staff_layout::{StaffLayout, StaffOpts};
use grooph_layout::text_rows::TextRowKind;
use grooph_measure::counting::CountConfig;
use grooph_measure::{Cursor, MeasureIdx, Score, StaffKind, VoiceIdx};
use std::ops::Range;

/// How the staff marks the playback position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlaybackMarks {
    /// The moving playback line; the count label being played is lit.
    Line,
    /// No line: a guide anchor `(measure_idx, local_tick, intensity)` lights
    /// its beat and its count label while it fades (`None` between anchors).
    Anchor(Option<(MeasureIdx, f64, f32)>),
}

/// What a staff is drawn from: the score, its layout and the options it
/// was laid out with, and what is marked on it this frame. Every layer of
/// the staff (notes, cursors, the rows of text, overlays) takes what it
/// needs from here; a bar's points in time from its
/// [`TickMap`](grooph_layout::tick_map::TickMap).
pub struct RenderContext<'a> {
    pub score: &'a Score,
    /// `score` laid out.
    pub staff: &'a StaffLayout,
    /// The options `staff` was laid out with, rescaled by its scale.
    pub opts: &'a StaffOpts,
    /// How the count labels count, if they are shown.
    pub count_config: Option<&'a CountConfig>,
    /// The edit position in its voice; only the measure matching
    /// `cursor.measure_idx` gets a blinking cursor.
    pub cursor: Option<Cursor>,
    /// The note drawn in the highlight colour.
    pub lit: Option<Cursor>,
    /// `(measure_idx, local_tick)` of the **active** measure (the one
    /// currently being audibly played). The *visual* cursor may live in a
    /// different measure during the wrap animation — see
    /// [`current_cursor_x`].
    pub playback: Option<(MeasureIdx, f64)>,
    /// The bars that play, in a loop: the cursor goes on from the last to
    /// the first of them, never into a bar after them (the editor's ghost
    /// bar).
    pub loop_bars: Range<MeasureIdx>,
    /// Whether the line or a guide anchor shows the playback position.
    pub marks: PlaybackMarks,
}

impl<'a> RenderContext<'a> {
    /// `score` laid out as `staff` with `opts`, nothing marked on it, every
    /// bar played.
    pub fn new(score: &'a Score, staff: &'a StaffLayout, opts: &'a StaffOpts) -> Self {
        RenderContext {
            score,
            staff,
            opts,
            count_config: None,
            cursor: None,
            lit: None,
            playback: None,
            loop_bars: 0..score.len(),
            marks: PlaybackMarks::Line,
        }
    }

    /// The edit cursor if it is in `voice` of the measure `measure_idx`.
    pub(crate) fn cursor_in(&self, voice: VoiceIdx, measure_idx: MeasureIdx) -> Option<Cursor> {
        self.cursor.filter(|c| c.voice == voice && c.measure_idx == measure_idx)
    }

    /// The lit note of `voice` in the measure `measure_idx`, if any.
    pub(crate) fn lit_in(&self, voice: VoiceIdx, measure_idx: MeasureIdx) -> Option<usize> {
        self.lit.filter(|c| c.voice == voice && c.measure_idx == measure_idx).map(|c| c.beat_idx)
    }
}

/// Render the staff of `ctx` to `ui`.
pub fn draw_staff(ui: &mut egui::Ui, ctx: &RenderContext) {
    let (staff, staff_opts, playback) = (ctx.staff, ctx.opts, ctx.playback);
    let color = ui.visuals().text_color();
    let highlight = highlight_color(ui.visuals());

    // The count label to light: the one being played under the line, the
    // anchor's while it fades in from the label colour.
    let (playback_line, count_tick) = match ctx.marks {
        PlaybackMarks::Line => (true, playback.map(|(idx, t)| (idx, t, highlight))),
        PlaybackMarks::Anchor(anchor) => (
            false,
            anchor.filter(|_| playback.is_some()).map(|(idx, t, intensity)| {
                let label = color.gamma_multiply_u8(200);
                (idx, t, label.lerp_to_gamma(highlight, intensity.clamp(0.0, 1.0)))
            }),
        ),
    };

    // Resolve the *visual* cursor location once per frame. Phase 1 of a
    // measure's last note keeps the cursor in that measure; Phase 2 hands it
    // off to the next one (wrapping at the score end).
    let visual_cursor = playback
        .filter(|_| playback_line)
        .and_then(|p| current_cursor_x(staff, p, ctx.loop_bars.clone()));

    for system in &staff.systems {
        // Continuous staff lines per system, so adjacent measures look
        // joined, from the top one down. A staff position is half a staff
        // space up from the middle line.
        let kind = system.measures.first().map_or(StaffKind::Line, |p| p.staff);
        for &pos in Staff::of(kind).lines.iter().rev() {
            ui.painter().hline(
                Rangef::new(system.rect.left(), system.rect.right()),
                system.y_baseline - f32::from(pos) * 0.5 * staff_opts.staff_space(),
                Stroke::new(0.02 * staff_opts.em, color),
            );
        }

        for placed in &system.measures {
            let cursor_x =
                visual_cursor.filter(|(idx, _)| *idx == placed.measure_idx).map(|(_, x)| x);

            // The other voices' notes, below the shared line.
            for (voice, layout) in placed.voices.iter().enumerate().skip(1) {
                let voice_opts = staff_opts.voice_opts_for(placed, voice);
                let lit = ctx.lit_in(voice, placed.measure_idx).map(|idx| (idx, highlight));
                draw_notes(ui.painter(), layout, color, lit, &voice_opts);
                if let Some(c) = ctx.cursor_in(voice, placed.measure_idx) {
                    draw_edit_cursor(ui, ui.painter(), layout, c.beat_idx, &voice_opts);
                }
            }

            // Only the active measure lights a label, so a wrap-in cursor
            // doesn't light labels in the next measure before its turn.
            let measure_count_tick =
                count_tick.filter(|(idx, _, _)| *idx == placed.measure_idx).map(|(_, t, c)| (t, c));
            render_measure_at(ui, ctx, placed, cursor_x);
            // The rows of text along the bar, where the layout put them:
            // clear of this bar's own notes, on their side of the staff.
            for row in &placed.text_rows {
                match row.kind {
                    TextRowKind::Count => {
                        draw_count_layer(ui.painter(), ctx, placed, row, color, measure_count_tick);
                    }
                    TextRowKind::Sticking => {
                        draw_stickings(ui.painter(), ctx, placed, row, color);
                    }
                }
            }
        }

        // Barlines between measures: vertical stroke at each measure boundary
        // except after the last one (the right edge is the score end).
        draw_barlines(ui, system, staff_opts, color);
    }

    if let PlaybackMarks::Anchor(Some((idx, tick, intensity))) = ctx.marks
        && playback.is_some()
        && intensity > 0.0
        && let Some(pos) = beat_anchor_pos(staff, idx, tick)
    {
        draw_beat_anchor(ui.painter(), pos, staff_opts.em, highlight, intensity);
    }
}

/// Where a guide anchor at `tick` of measure `measure_idx` sits: on the
/// staff line, at the tick's x ([`TickMap::x_at_tick`]): a note's head at
/// its onset, then on toward the next head (the barline after the last
/// one) as the note goes on, so beat 4 of a half note on 3 sits halfway.
///
/// [`TickMap::x_at_tick`]: grooph_layout::tick_map::TickMap::x_at_tick
pub fn beat_anchor_pos(
    staff: &StaffLayout,
    measure_idx: MeasureIdx,
    tick: f64,
) -> Option<egui::Pos2> {
    let placed = staff.placed(measure_idx)?;
    let x = placed.ticks.x_at_tick(tick);
    Some(egui::pos2(x, placed.rect.center().y + placed.y_offset))
}

/// Compute the X position of the playback cursor across the whole staff.
///
/// Implements the two-phase wrap animation:
/// - Phase 1: cursor lives in the currently playing measure
///   (`playback.0`). Returned as `(playback.0, x)`.
/// - Phase 2 (end of the last note, see [`last_note_split`]): cursor lives in
///   the next measure of `loop_bars`, the bars that play (after its last
///   one, its first), entering from the left. Returned as that measure's
///   index plus its entering X. A bar laid out past them (the editor's
///   ghost bar) is never entered.
///
/// Returns `None` if there is no visible cursor (e.g. score empty, layout
/// empty, or the relevant measure has no notes).
pub fn current_cursor_x(
    staff: &StaffLayout,
    playback: (MeasureIdx, f64),
    loop_bars: Range<MeasureIdx>,
) -> Option<(MeasureIdx, f32)> {
    let (play_idx, local_tick) = playback;
    if loop_bars.is_empty() {
        return None;
    }

    let active_placed = staff.placed(play_idx)?;
    let next_idx = match play_idx + 1 < loop_bars.end {
        true => play_idx + 1,
        false => loop_bars.start,
    };
    let next_placed = staff.placed(next_idx)?;
    let split = last_note_split(&active_placed.voices, active_placed.rect, &next_placed.voices);

    // Phase 2 first: if we're past the split of the active measure's last
    // note, the cursor visually belongs to the *next* measure.
    if let Some(entry_frac) = active_placed.ticks.entering_frac(local_tick, split) {
        let x = playback_cursor_entering_x(next_placed.layout(), next_placed.rect, entry_frac)?;
        return Some((next_idx, x));
    }

    // Phase 1: cursor lives in the active measure.
    let x = active_placed.ticks.cursor_x(local_tick, split)?;
    Some((play_idx, x))
}

fn draw_barlines(
    ui: &egui::Ui,
    system: &grooph_layout::staff_layout::SystemLayout,
    staff_opts: &StaffOpts,
    color: Color32,
) {
    let top = system.y_baseline - 0.5 * staff_opts.em;
    let bottom = system.y_baseline + 0.5 * staff_opts.em;
    let stroke = Stroke::new(glyphs::THIN_BARLINE_THICKNESS * staff_opts.staff_space(), color);
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

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{FontFamily, FontId, Pos2, Rect, vec2};
    use grooph_layout::staff_layout::{ROOM_BELOW_EM, build_staff_layout};
    use grooph_layout::text_rows::{Side, TextRows};
    use grooph_layout::tick_map::TickMap;
    use grooph_measure::notation::parse_score;

    fn opts() -> StaffOpts {
        let em = 20.0;
        StaffOpts {
            rect: Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0)),
            font_id: FontId::new(em, FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.1,
            accent_displacement: 0.0,
            min_measure_width_em: 6.0,
            note_width_em: 0.6,
            system_spacing_em: 0.5,
            layout_clef_first: false,
            time_signature_first: false,
            room_below_em: ROOM_BELOW_EM,
            text_rows: Default::default(),
            bar_rests: false,
            veil_hidden_rests: false,
        }
    }

    /// The voice and index of the note nearest (across) each shape drawn in
    /// the highlight colour, with `lit` lit on a two-voice staff whose notes
    /// all stand at different places.
    fn lit_notes(lit: Cursor) -> Vec<(VoiceIdx, usize)> {
        let upper = parse_score("4/4 q q q q").unwrap();
        let lower = parse_score("4/4 r:e q q q e").unwrap();
        let opts = opts();
        let mut both = upper.clone();
        for (m, l) in both.measures_mut().iter_mut().zip(lower.measures()) {
            m.push_voices(l).unwrap();
        }
        let staff = build_staff_layout(&both, &opts);
        let ctx = egui::Context::default();
        let mut color = Color32::PLACEHOLDER;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            color = highlight_color(ui.visuals());
            draw_staff(
                ui,
                &RenderContext { lit: Some(lit), ..RenderContext::new(&both, &staff, &opts) },
            );
        });
        output.textures_delta.clear();
        let placed = staff.placed(0).unwrap();
        let lower_notes = &placed.voices[1].notes;
        let notes: Vec<(VoiceIdx, usize, Pos2)> = (placed.layout().notes.iter().enumerate())
            .map(|(i, n)| (0, i, n.center))
            .chain(lower_notes.iter().enumerate().map(|(i, n)| (1, i, n.center)))
            .collect();
        output
            .shapes
            .iter()
            .filter(|c| match &c.shape {
                egui::Shape::LineSegment { stroke, .. } => stroke.color == color,
                egui::Shape::Text(text) => text.fallback_color == color,
                _ => false,
            })
            .map(|c| {
                let at = c.shape.visual_bounding_rect().center();
                let nearest = notes
                    .iter()
                    .min_by(|a, b| (a.2.x - at.x).abs().total_cmp(&(b.2.x - at.x).abs()))
                    .unwrap();
                (nearest.0, nearest.1)
            })
            .collect()
    }

    #[test]
    fn a_lit_note_lights_in_its_own_voice_only() {
        for voice in [0, 1] {
            let lit = lit_notes(Cursor::in_voice(0, voice, 1));
            assert!(lit.len() >= 2, "{voice:?}: {lit:?}");
            assert!(lit.iter().all(|&n| n == (voice, 1)), "{voice:?}: {lit:?}");
        }
        // A note of another measure lights nothing here.
        assert!(lit_notes(Cursor::at(1, 1)).is_empty());
    }

    /// A kit bar whose lower voice is busier than the upper one: halves
    /// above, four 32nd kicks and rests below. Its heads are moved right a
    /// little, as a layout giving them more room than their time would, so
    /// that following them differs from following the upper voice.
    fn busy_kick() -> (Score, StaffLayout) {
        let score = parse_score("4/4 H:h H:h ; K:th K:th K:th K:th r:e r:q r:h").unwrap();
        let rows = TextRows { count: Some(Side::Below), ..TextRows::default() };
        let mut staff = build_staff_layout(&score, &StaffOpts { text_rows: rows, ..opts() });
        let placed = &mut staff.systems[0].measures[0];
        for note in &mut placed.voices[1].notes[1..5] {
            note.center.x += 3.0;
        }
        placed.ticks = TickMap::new(&score.measures()[0], &placed.voices, placed.rect);
        (score, staff)
    }

    #[test]
    fn the_playback_line_passes_a_lower_voices_notes() {
        let (_, staff) = busy_kick();
        let placed = staff.placed(0).unwrap();
        let lower = &placed.voices[1].notes;
        let th = grooph_measure::grid::DEFAULT_GRID
            .ticks_of(&grooph_measure::duration::th())
            .unwrap() as f64;
        let x = |tick: f64| current_cursor_x(&staff, (0, tick), 0..1).unwrap().1;
        // On each kick the line is at its head, halfway between two of them
        // halfway between their heads.
        for (i, kick) in lower.iter().take(4).enumerate() {
            assert!((x(i as f64 * th) - kick.center.x).abs() < 1e-3, "kick {i}");
        }
        let between = (lower[2].center.x + lower[3].center.x) / 2.0;
        assert!((x(2.5 * th) - between).abs() < 1e-3);
        // The second half note, where no lower note starts, as before.
        let two = placed.layout().notes[1].center.x;
        assert!((x(16.0 * th) - two).abs() < 1e-3);
    }

    #[test]
    fn the_playback_line_goes_on_within_the_bars_that_play() {
        // Three bars laid out, the third the editor's ghost bar.
        let score = parse_score("4/4 q q q q | q q q q | q q q q").unwrap();
        let staff = build_staff_layout(&score, &opts());
        let ts = score.measures()[0].time_signature();
        let end = grooph_measure::grid::DEFAULT_GRID.ticks_per_measure(&ts) as f64 - 1.0;
        let next = |at, bars| current_cursor_x(&staff, (at, end), bars).unwrap().0;
        // At the end of the last bar that plays the line enters the first,
        // not the ghost bar.
        assert_eq!(next(1, 0..2), 0);
        assert_eq!(next(0, 0..2), 1);
        // Bars played on their own loop among themselves.
        assert_eq!(next(1, 1..2), 1);
        assert_eq!(next(2, 1..3), 1);
    }

    #[test]
    fn stickings_are_written_one_per_column_r_above_l() {
        let score = parse_score("4/4 S:q@R S:q@L S:q@R S:q ; FT:q r:q FT:q@L r:q").unwrap();
        for side in [Side::Below, Side::Above] {
            let rows = TextRows { sticking: Some(side), ..TextRows::default() };
            let opts = StaffOpts { text_rows: rows, ..opts() };
            let staff = build_staff_layout(&score, &opts);
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                draw_staff(ui, &RenderContext::new(&score, &staff, &opts));
            });
            output.textures_delta.clear();
            let mut letters: Vec<(String, f32, f32)> = output
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(text) if ["R", "L"].contains(&text.galley.text()) => {
                        let at = c.shape.visual_bounding_rect().center();
                        Some((text.galley.text().to_owned(), at.x, at.y))
                    }
                    _ => None,
                })
                .collect();
            // By column (the letters' own widths differ a little), then top first.
            letters.sort_by(|a, b| {
                (a.1 / 4.0).round().total_cmp(&(b.1 / 4.0).round()).then(a.2.total_cmp(&b.2))
            });
            let names: Vec<&str> = letters.iter().map(|l| l.0.as_str()).collect();
            // The third column has R above and L below it: both hands.
            assert_eq!(names, ["R", "L", "R", "L"], "{side:?}");
            let placed = staff.placed(0).unwrap();
            let heads: Vec<f32> = placed.voices[0].notes.iter().map(|n| n.center.x).collect();
            for (letter, column) in letters.iter().zip([0, 1, 2, 2]) {
                assert!((letter.1 - heads[column]).abs() < 1.0, "{letters:?}");
            }
            assert!(letters[2].2 + 1.0 < letters[3].2, "R above L");
            // Single letters share one line, the one next to the notes.
            assert!((letters[0].2 - letters[1].2).abs() < 1.0, "{side:?}: {letters:?}");
            let near = match side {
                Side::Below => letters[2].2,
                Side::Above => letters[3].2,
            };
            assert!((letters[0].2 - near).abs() < 1.0, "{side:?}: {letters:?}");
        }
    }

    #[test]
    fn count_labels_sit_over_a_lower_voices_notes() {
        use grooph_measure::counting::{CountConfig, Subdivision};
        let (score, staff) = busy_kick();
        let placed = staff.placed(0).unwrap();
        let config = CountConfig { subdivision: Subdivision::Halves, system: Default::default() };
        let ctx = egui::Context::default();
        let opts = opts();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let ctx = RenderContext {
                count_config: Some(&config),
                ..RenderContext::new(&score, &staff, &opts)
            };
            draw_staff(ui, &ctx);
        });
        output.textures_delta.clear();
        let ands: Vec<f32> = output
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(text) if text.galley.text() == "&" => {
                    Some(c.shape.visual_bounding_rect().center().x)
                }
                _ => None,
            })
            .collect();
        assert_eq!(ands.len(), 4, "{ands:?}");
        // The "&" of 1 over the lower voice's eighth rest there, not where
        // an eighth of the upper half note would be.
        let rest = placed.voices[1].notes[4].center.x;
        assert!((ands[0] - rest).abs() < 1.0, "{ands:?} vs {rest}");
    }
}
