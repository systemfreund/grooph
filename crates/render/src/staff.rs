//! Multi-measure rendering: walks a [`StaffLayout`] and renders each measure
//! with the existing per-measure renderer. Adds cross-measure visuals
//! (continuous staff line, barlines) and orchestrates the playback cursor
//! wrap animation across measure boundaries.

use crate::measure::{
    beat_anchor_x, draw_beat_anchor, draw_count_layer, draw_edit_cursor, draw_notes,
    highlight_color, last_note_entering_frac, last_note_split, playback_cursor_entering_x,
    playback_cursor_x, render_measure_at,
};
use eframe::egui;
use eframe::egui::{Color32, Rangef, Stroke};
use grooph_layout::glyphs;
use grooph_layout::staff_layout::{PlacedMeasure, StaffLayout, StaffOpts};
use grooph_measure::counting::CountConfig;
use grooph_measure::{Cursor, MeasureIdx, Score, StaffKind, VoiceIdx};

/// How the staff marks the playback position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlaybackMarks {
    /// The moving playback line; the count label being played is lit.
    Line,
    /// No line: a guide anchor `(measure_idx, local_tick, intensity)` lights
    /// its beat and its count label while it fades (`None` between anchors).
    Anchor(Option<(MeasureIdx, f64, f32)>),
}

/// Render a full score described by `staff` to `ui`.
///
/// `cursor` highlights the edit position in its voice; only the measure
/// matching `cursor.measure_idx` gets a blinking cursor. `playback` is
/// `Some((measure_idx, local_tick))` referring to the **active** measure (the
/// one currently being audibly played). The *visual* cursor may live in a
/// different measure during the wrap animation — see [`current_cursor_x`].
/// `marks` says whether the line or a guide anchor shows it. `lit` draws
/// the note it points at in the highlight colour.
#[expect(clippy::too_many_arguments)]
pub fn draw_staff(
    ui: &mut egui::Ui,
    score: &Score,
    staff: &StaffLayout,
    cursor: Option<Cursor>,
    lit: Option<Cursor>,
    playback: Option<(MeasureIdx, f64)>,
    marks: PlaybackMarks,
    count_config: Option<&CountConfig>,
    staff_opts: &StaffOpts,
) {
    let color = ui.visuals().text_color();
    let highlight = highlight_color(ui.visuals());
    // The lit note of `voice` in the measure `measure_idx`, if any.
    let lit_in = |voice: VoiceIdx, measure_idx: MeasureIdx| {
        lit.filter(|c| c.voice == voice && c.measure_idx == measure_idx)
            .map(|c| (c.beat_idx, highlight))
    };

    // The count label to light: the one being played under the line, the
    // anchor's while it fades in from the label colour.
    let (playback_line, count_tick) = match marks {
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
    let visual_cursor =
        playback.filter(|_| playback_line).and_then(|p| current_cursor_x(score, staff, p));

    for system in &staff.systems {
        // Continuous staff lines per system, so adjacent measures look
        // joined: one, or the kit's five around it.
        let lines = match system.measures.first().map(|p| p.staff) {
            Some(StaffKind::Kit) => -2..=2,
            _ => 0..=0,
        };
        for line in lines {
            ui.painter().hline(
                Rangef::new(system.rect.left(), system.rect.right()),
                system.y_baseline + line as f32 * staff_opts.staff_space(),
                Stroke::new(0.02 * staff_opts.em, color),
            );
        }

        for placed in &system.measures {
            let cursor_x =
                visual_cursor.filter(|(idx, _)| *idx == placed.measure_idx).map(|(_, x)| x);

            // The cursor of `voice` in this measure, if any.
            let cursor_in = |voice: VoiceIdx| {
                cursor.filter(|c| c.voice == voice && c.measure_idx == placed.measure_idx)
            };
            // The other voices' notes, below the shared line.
            for (voice, layout) in placed.voices.iter().enumerate().skip(1) {
                let voice_opts = staff_opts.voice_opts_for(placed, voice);
                let lit = lit_in(voice, placed.measure_idx);
                draw_notes(ui.painter(), layout, color, lit, &voice_opts);
                if let Some(c) = cursor_in(voice) {
                    draw_edit_cursor(ui, ui.painter(), layout, c.beat_idx, &voice_opts);
                }
            }

            // With a lower voice the count labels go under the whole staff.
            let (count_config, lower_counts) = match placed.voices.len() {
                1 => (count_config, None),
                _ => (None, count_config),
            };
            // Only the active measure lights a label, so a wrap-in cursor
            // doesn't light labels in the next measure before its turn.
            let measure_count_tick =
                count_tick.filter(|(idx, _, _)| *idx == placed.measure_idx).map(|(_, t, c)| (t, c));
            render_placed_measure(
                ui,
                score,
                placed,
                staff_opts,
                cursor_in(0),
                measure_count_tick,
                count_config,
                cursor_x,
                lit_in(0, placed.measure_idx),
            );
            if let Some(config) = lower_counts {
                draw_count_layer(
                    ui.painter(),
                    &score.measures()[placed.measure_idx],
                    &placed.voices,
                    &staff_opts.voice_opts_for(placed, 0),
                    placed.rect.bottom(),
                    config,
                    color,
                    measure_count_tick,
                );
            }
        }

        // Barlines between measures: vertical stroke at each measure boundary
        // except after the last one (the right edge is the score end).
        draw_barlines(ui, system, staff_opts, color);
    }

    if let PlaybackMarks::Anchor(Some((idx, tick, intensity))) = marks
        && playback.is_some()
        && intensity > 0.0
        && let Some(pos) = beat_anchor_pos(score, staff, idx, tick)
    {
        draw_beat_anchor(ui.painter(), pos, staff_opts.em, highlight, intensity);
    }
}

/// Where a guide anchor at `tick` of measure `measure_idx` sits: on the
/// staff line, at [`beat_anchor_x`].
pub fn beat_anchor_pos(
    score: &Score,
    staff: &StaffLayout,
    measure_idx: MeasureIdx,
    tick: f64,
) -> Option<egui::Pos2> {
    let measure = score.measures().get(measure_idx)?;
    let placed = staff.placed(measure_idx)?;
    let x = beat_anchor_x(measure, &placed.voices, placed.rect, tick)?;
    Some(egui::pos2(x, placed.rect.center().y + placed.y_offset))
}

/// Compute the X position of the playback cursor across the whole staff.
///
/// Implements the two-phase wrap animation:
/// - Phase 1: cursor lives in the currently playing measure
///   (`playback.0`). Returned as `(playback.0, x)`.
/// - Phase 2 (end of the last note, see [`last_note_split`]): cursor lives in
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

    let active_measure = score.measures().get(play_idx)?;
    let active_placed = staff.placed(play_idx)?;
    let next_idx = (play_idx + 1) % score.len();
    let next_placed = staff.placed(next_idx)?;
    let split = last_note_split(&active_placed.voices, active_placed.rect, &next_placed.voices);

    // Phase 2 first: if we're past the split of the active measure's last
    // note, the cursor visually belongs to the *next* measure.
    if let Some(entry_frac) = last_note_entering_frac(active_measure, local_tick, split) {
        let x = playback_cursor_entering_x(next_placed.layout(), next_placed.rect, entry_frac)?;
        return Some((next_idx, x));
    }

    // Phase 1: cursor lives in the active measure.
    let x = playback_cursor_x(
        active_measure,
        &active_placed.voices,
        active_placed.rect,
        local_tick,
        split,
    )?;
    Some((play_idx, x))
}

#[expect(clippy::too_many_arguments)]
fn render_placed_measure(
    ui: &mut egui::Ui,
    score: &Score,
    placed: &PlacedMeasure,
    staff_opts: &StaffOpts,
    cursor: Option<Cursor>,
    count_tick: Option<(f64, Color32)>,
    count_config: Option<&CountConfig>,
    cursor_x: Option<f32>,
    lit: Option<(usize, Color32)>,
) {
    let cursor_idx = cursor.filter(|c| c.measure_idx == placed.measure_idx).map(|c| c.beat_idx);

    // Build a per-measure LayoutOpts from the placed rect/flags so the cursor
    // and playback drawing get the right viewport.
    let per_opts = staff_opts.voice_opts_for(placed, 0);

    render_measure_at(
        ui,
        &score.measures()[placed.measure_idx],
        placed.layout(),
        &per_opts,
        cursor_idx,
        count_tick,
        count_config,
        /* draw_staff_line = */ false,
        cursor_x,
        lit,
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
            lower_label_room_em: 0.0,
            bar_rests: false,
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
            let marks = PlaybackMarks::Line;
            draw_staff(ui, &both, &staff, None, Some(lit), None, marks, None, &opts);
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
        let mut staff = build_staff_layout(&score, &opts());
        let lower = &mut staff.systems[0].measures[0].voices[1].notes;
        for note in &mut lower[1..5] {
            note.center.x += 3.0;
        }
        (score, staff)
    }

    #[test]
    fn the_playback_line_passes_a_lower_voices_notes() {
        let (score, staff) = busy_kick();
        let placed = staff.placed(0).unwrap();
        let lower = &placed.voices[1].notes;
        let th = grooph_measure::grid::DEFAULT_GRID
            .ticks_of(&grooph_measure::duration::th())
            .unwrap() as f64;
        let x = |tick: f64| current_cursor_x(&score, &staff, (0, tick)).unwrap().1;
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
    fn count_labels_sit_over_a_lower_voices_notes() {
        use grooph_measure::counting::{CountConfig, Subdivision};
        let (score, staff) = busy_kick();
        let placed = staff.placed(0).unwrap();
        let config = CountConfig { subdivision: Subdivision::Halves, system: Default::default() };
        let ctx = egui::Context::default();
        let marks = PlaybackMarks::Line;
        let opts = opts();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            draw_staff(ui, &score, &staff, None, None, None, marks, Some(&config), &opts);
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
