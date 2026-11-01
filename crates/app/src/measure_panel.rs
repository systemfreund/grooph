use crate::Grooph;
use crate::accuracy::{AccuracyMark, HitGrade, clamp_diff_to_beat_window};
use crate::guide::{Guide, anchor_intensity};
use crate::metronome_view::MetronomeVoice;
use crate::playback_clock::{advance_tick, next_count_in};
use crate::tools::{Modifier, ToolKind, all_tools};
use crate::{Mode, TransportState};
use eframe::egui;
use eframe::egui::{FontId, Frame, Rect, Response, Stroke};
use grooph_layout::pixel_layout::{NoteLayout, compute_em, onset_head};
use grooph_layout::staff_layout::{
    LEGIBILITY_FLOOR_EM, LOWER_VOICE_LABEL_ROOM_EM, PlacedMeasure, ROOM_BELOW_EM,
    ROOM_BELOW_WITH_LABELS_EM, StaffLayout, StaffOpts, build_staff_layout, hit_test_head,
    hit_test_voice,
};
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{Cursor, Score, TimeSignature};
use grooph_render::staff::{PlaybackMarks, draw_staff};

/// The frames around the staff: the central panel's, and the canvas inside
/// it (transparent, no stroke).
fn staff_frames(style: &egui::Style) -> (Frame, Frame) {
    let canvas = Frame::canvas(style).fill(egui::Color32::TRANSPARENT).stroke(egui::Stroke::NONE);
    (Frame::central_panel(style), canvas)
}

/// What the frames around the staff take of the room it is given.
pub(crate) fn staff_margins(style: &egui::Style) -> egui::Vec2 {
    let (panel, canvas) = staff_frames(style);
    panel.total_margin().sum() + canvas.total_margin().sum()
}

impl Grooph {
    /// The staff's options for `viewport_rect`. The metronome's staff takes
    /// its size from the width alone: its height then decides how much room
    /// the ring below gets (see `Grooph::staff_and_ring`), which must not in
    /// turn decide its size.
    pub(crate) fn staff_opts(&self, ui: &egui::Ui, viewport_rect: Rect) -> StaffOpts {
        let width_cap_factor = if self.ui.narrow_screen {
            // Let the measure width alone decide the size.
            self.ui.layout.width_cap_factor.max(0.2)
        } else {
            self.ui.layout.width_cap_factor
        };
        let em = if self.metronome_mode() {
            let tall = viewport_rect.with_max_y(viewport_rect.top() + 1.0e4);
            compute_em(&tall, width_cap_factor, ui)
        } else {
            compute_em(&viewport_rect, width_cap_factor, ui)
        };
        let font_id = FontId::new(em, self.ui.music_font_id.family.clone());
        let count_config = self.build_count_config();

        // Tighter spacing on narrow screens makes the glyphs
        // bigger: the staff shrinks its em until a measure
        // full of sixteenths fits the width.
        let narrow = self.ui.narrow_screen;
        let (min_measure_width_em, note_width_em) = if narrow { (2.0, 0.36) } else { (6.0, 0.6) };
        StaffOpts {
            rect: viewport_rect,
            font_id: font_id.clone(),
            pixels_per_point: ui.ctx().pixels_per_point(),
            em,
            y_offset: 0.0,
            stem_length_factor: self.ui.layout.stem_length_factor,
            stem_thickness_factor: 0.04,
            accent_displacement: 0.07,
            min_measure_width_em,
            note_width_em,
            system_spacing_em: 0.2,
            // No percussion clef: on a single line it says nothing
            // the line doesn't, and every row starts alike.
            layout_clef_first: false,
            // The compact card in a chat has no meter in
            // a card: the staff shows it.
            time_signature_first: self.compact_card(),
            room_below_em: if count_config.is_some() {
                ROOM_BELOW_WITH_LABELS_EM
            } else {
                ROOM_BELOW_EM
            },
            // With a lower voice, count labels go under it.
            lower_label_room_em: if count_config.is_some() {
                LOWER_VOICE_LABEL_ROOM_EM
            } else {
                0.0
            },
            // While editing every rest stays to be picked, and
            // in the metronome every click its own to tap.
            bar_rests: self.ui.mode != Mode::Edit && !self.metronome_mode(),
        }
    }

    /// Height of the metronome's staff panel at `width` (the room beside
    /// the panel's margins), margins included: what it needs this frame.
    pub(crate) fn metronome_staff_height(&self, ui: &egui::Ui, width: f32) -> f32 {
        let margins = staff_margins(ui.style());
        let rect = Rect::from_min_size(ui.cursor().min, egui::vec2(width - margins.x, 1.0e4));
        let opts = self.staff_opts(ui, rect);
        build_staff_layout(&self.editor.doc().score, &opts).total_size.y + margins.y
    }

    pub(super) fn measure_panel(&mut self, ui: &mut egui::Ui) {
        let (panel, canvas) = staff_frames(ui.style());
        egui::CentralPanel::default().frame(panel).show(ui, |ui| {
            canvas.show(ui, |ui| {
                // Size measured on the frame's own `ui`, *before* entering
                // the `ScrollArea`: because the area scrolls both axes,
                // its inner `ui.available_size()` reports an unbounded
                // (scrollable) extent rather than the actual visible
                // viewport, which would make the staff size itself (and
                // pick its em) against a phantom width — overflowing the
                // real, narrower screen (e.g. on mobile) instead of
                // shrinking a measure to fit it fully in one row.
                let available = ui.available_size();

                // While playing, the tween below is the only animation:
                // egui's own would keep heading for a target from an
                // earlier frame, e.g. one before endless mode removed a
                // row.
                let playing = self.playback_ctl.transport_state == TransportState::Playing;
                let mut scroll_area = egui::ScrollArea::both().animated(!playing);
                let playback = &mut self.playback_ctl.playback;
                if std::mem::take(&mut playback.rows_removed)
                    && let Some(offset) = playback.scroll.value_at(ui.input(|i| i.time))
                {
                    scroll_area = scroll_area.vertical_scroll_offset(offset);
                }
                scroll_area.show(ui, |ui| {
                    // `origin`, unlike `available`, must come from the
                    // *inner* ui: egui bakes the current scroll offset
                    // into its cursor position, which is what makes
                    // content laid out from here actually move as the
                    // user scrolls (the outer ui's cursor is fixed).
                    let origin = ui.cursor().min;
                    let viewport_rect = Rect::from_min_size(origin, available);

                    // Build the multi-measure tempo backbone once per frame.
                    // O(score.len()) — cheap for realistic score sizes.
                    let timing = self.score_timing();

                    // Update playback smoothing & primary-beat flash state
                    let playback_tick_to_draw = match self.playback_ctl.transport_state {
                        TransportState::Playing => Some(self.advance_playback(ui, &timing)),
                        TransportState::Stopped => {
                            self.playback_ctl.playback.reset();
                            None
                        }
                    };

                    // From the global tick, derive (playing_measure_idx, local_tick).
                    // The playback cursor renders / auto-scrolls based on this — it
                    // wanders through all measures, independent of cursor.measure_idx.
                    // With swing the cursor follows the written positions, so it
                    // reaches a swung note exactly when it sounds.
                    let playback_local = playback_tick_to_draw
                        .map(|global| timing.to_local(timing.written_global_tick(global)));
                    let now = ui.input(|i| i.time);
                    self.update_guide_anchor(playback_local, now);
                    let guide = self.active_guide();

                    let staff_opts = self.staff_opts(ui, viewport_rect);
                    let em = staff_opts.em;
                    let count_config = self.build_count_config();

                    // A calibration's click bar while it runs, else the
                    // document (borrowed apart from the hover marks
                    // below, which change).
                    let calibration = self.calibrating().then(|| self.played_score().clone());
                    let score = calibration
                        .as_ref()
                        .unwrap_or(&self.editor.docs.get(self.editor.document).score);
                    // A rhythm's editor shows an empty "ghost" bar after the
                    // last one: a tap there adds a bar. Laid out with the
                    // score, drawn veiled.
                    let ghost = (self.ui.mode == Mode::Edit
                        && !self.metronome_mode()
                        && calibration.is_none())
                    .then(|| {
                        let mut shown = score.clone();
                        let last = &score.measures()[score.len() - 1];
                        shown.push(last.empty_like(last.time_signature()));
                        shown
                    });
                    let score = ghost.as_ref().unwrap_or(score);
                    let staff = build_staff_layout(score, &staff_opts);
                    // One row fits the height, so nothing above or below
                    // the staff (accents, count labels) is cut off on a
                    // short, wide panel.
                    let row_height = staff.room.height() * em;
                    let (staff_opts, staff) = if row_height > available.y && available.y > 0.0 {
                        let floor = LEGIBILITY_FLOOR_EM / em;
                        let staff_opts =
                            staff_opts.rescaled((available.y / row_height).max(floor.min(1.0)));
                        let staff = build_staff_layout(score, &staff_opts);
                        (staff_opts, staff)
                    } else {
                        (staff_opts, staff)
                    };
                    // Rows may render at a different em than the baseline `staff_opts`
                    // (grow-to-fill / shrink-toward-floor) — rescale font/metrics to
                    // match before drawing, so glyph size agrees with `staff`'s positions.
                    let render_opts = staff_opts.rescaled(staff.scale);
                    self.ui.staff_em = render_opts.em;

                    // Auto-scroll follows the same visual cursor logic as the
                    // renderer (incl. cross-measure wrap animation), so we use
                    // the shared helper to find the cursor's actual home measure
                    // and X for this frame.
                    if let Some(playback) = playback_local
                        && let Some((m_idx, x)) =
                            grooph_render::staff::current_cursor_x(score, &staff, playback)
                        && let Some(placed) = staff.placed(m_idx)
                    {
                        // The row is centred vertically, but the offset
                        // eases to it, so the rows glide up when the cursor
                        // wraps rather than jump. The tween runs on the
                        // offset egui will clamp to, so it never stalls at
                        // the top or bottom of the score.
                        let view_height = ui.clip_rect().height();
                        let max_offset = (staff.total_size.y - view_height).max(0.0);
                        let row_offset = (placed.rect.center().y - origin.y - view_height / 2.0)
                            .clamp(0.0, max_offset);
                        let now = ui.input(|i| i.time);
                        let playback = &mut self.playback_ctl.playback;
                        let offset = playback.scroll.update(row_offset, now);
                        playback.row_pitch = staff
                            .systems
                            .get(1)
                            .zip(staff.systems.first())
                            .map_or(0.0, |(b, a)| b.rect.top() - a.rect.top());
                        playback.rows_above_view = if playback.row_pitch > 0.0 {
                            (offset / playback.row_pitch).floor() as usize
                        } else {
                            0
                        };
                        // Centring this rect scrolls to exactly `offset`;
                        // horizontally it follows the cursor at once.
                        let cursor_rect = egui::Rect::from_center_size(
                            egui::pos2(x, origin.y + offset + view_height / 2.0),
                            egui::vec2(1.0, placed.rect.height()),
                        );
                        ui.scroll_to_rect_animation(
                            cursor_rect,
                            Some(egui::Align::Center),
                            eframe::egui::style::ScrollAnimation::none(),
                        );
                    }

                    // Bring the edit cursor into view when it moves. `Align::None`
                    // means "scroll just enough to make visible"; following only
                    // on a move leaves the user free to scroll away from it.
                    // Gated on Stopped so it doesn't clobber the playback
                    // re-centering above.
                    let editing = self.ui.mode == Mode::Edit
                        && self.playback_ctl.transport_state == TransportState::Stopped;
                    let staff_cursor = self.editor.doc().cursor;
                    let followed = editing.then_some(staff_cursor);
                    if followed != self.ui.followed_cursor
                        && let Some(placed) = staff.placed(staff_cursor.measure_idx)
                        && let Some(layout) = placed.voices.get(staff_cursor.voice)
                        && let Some(note) = layout.notes.get(staff_cursor.beat_idx)
                    {
                        let cursor_rect = egui::Rect::from_min_size(
                            egui::pos2(note.center.x, placed.rect.top()),
                            egui::vec2(1.0, placed.rect.height()),
                        );
                        ui.scroll_to_rect_animation(
                            cursor_rect,
                            None,
                            eframe::egui::style::ScrollAnimation::none(),
                        );
                    }
                    self.ui.followed_cursor = followed;

                    let (rect, resp) =
                        ui.allocate_exact_size(staff.total_size, egui::Sense::click_and_drag());
                    let ghost_rect =
                        ghost.as_ref().and_then(|g| staff.placed(g.len() - 1)).map(|p| p.rect);

                    let cursor = (self.ui.mode == Mode::Edit).then_some(staff_cursor);
                    let playback = playback_local;
                    // A silent bar shows no playback position, like the
                    // metronome's ring: no line, no lit beat or count.
                    let marks = if self.playback_ctl.playback.bar_muted {
                        PlaybackMarks::Anchor(None)
                    } else if guide == Guide::Line {
                        PlaybackMarks::Line
                    } else {
                        PlaybackMarks::Anchor(self.playback_ctl.playback.anchor.map(|a| {
                            (a.measure_idx, f64::from(a.tick), anchor_intensity(now - a.at))
                        }))
                    };

                    // Under the metronome's ring the staff and the ring
                    // light each other's hovered note and segment.
                    let with_ring = self.metronome_mode() && self.ui.mode != Mode::Edit;
                    let lit = with_ring
                        .then_some(self.editor.metronome.ring_hover)
                        .flatten()
                        .map(|(voice, beat_idx)| Cursor::in_voice(0, voice.idx(), beat_idx));
                    let staff_hover = resp
                        .hover_pos()
                        .filter(|_| with_ring)
                        .and_then(|pos| hit_test_voice(&staff, pos))
                        .and_then(|(voice, _, beat_idx)| {
                            Some((MetronomeVoice::of(voice)?, beat_idx))
                        });
                    if staff_hover != self.editor.metronome.staff_hover {
                        // The ring was drawn before the staff: show it there next frame.
                        self.editor.metronome.staff_hover = staff_hover;
                        ui.ctx().request_repaint();
                    }

                    draw_staff(
                        ui,
                        score,
                        &staff,
                        cursor,
                        lit,
                        playback,
                        marks,
                        count_config.as_ref(),
                        &render_opts,
                    );

                    // Draw flash overlay (white on dark, black on light) with decay
                    let flash = self.playback_ctl.playback.flash_shown();
                    if flash > 0.01 {
                        let dark = ui.visuals().dark_mode;
                        let base = if dark { egui::Color32::GREEN } else { egui::Color32::BLUE };
                        let alpha = (0.8 * flash).clamp(0.0, 0.8);
                        let color = egui::Color32::from_rgba_unmultiplied(
                            base.r(),
                            base.g(),
                            base.b(),
                            (alpha * 255.0) as u8,
                        );
                        ui.painter().rect_filled(rect, 0.0, color);
                        ui.ctx().request_repaint();
                    }

                    if let Some(ghost) = ghost_rect {
                        let hovered = resp.hover_pos().is_some_and(|p| ghost.contains(p));
                        draw_ghost_bar(ui, ghost, hovered);
                    }

                    if self.playback_ctl.playback.bar_muted
                        && let Some((m_idx, _)) = playback_local
                        && let Some(placed) = staff.placed(m_idx)
                    {
                        draw_muted_bar(ui, placed.rect);
                    }

                    self.draw_accuracy_markers(
                        ui,
                        score,
                        &staff,
                        &timing,
                        render_opts.staff_space(),
                    );
                    // A finger dragging over the notes scrolls the score,
                    // as on the scroll bar; a mouse drags the edit cursor.
                    if resp.dragged() && ui.input(|i| i.any_touches()) {
                        ui.scroll_with_delta_animation(
                            resp.drag_delta(),
                            eframe::egui::style::ScrollAnimation::none(),
                        );
                    } else {
                        self.handle_input(resp, &staff, with_ring, ghost_rect);
                    }
                });
            });
        });
    }

    /// Keep the playback going while no staff is drawn (the More screen):
    /// endless mode, play along and the count-in follow its cursor.
    pub(super) fn advance_unseen_playback(&mut self, ui: &egui::Ui) {
        match self.playback_ctl.transport_state {
            TransportState::Playing => {
                let timing = self.score_timing();
                let tick = self.advance_playback(ui, &timing);
                // Nothing is on screen: every bar before the playing one is
                // out of view, so endless mode drops them as it goes.
                self.playback_ctl.playback.rows_above_view = timing.to_local(tick).0;
            }
            TransportState::Stopped => self.playback_ctl.playback.reset(),
        }
    }

    /// Move the playback cursor to this frame (see [`advance_tick`]), track
    /// the count-in and fire the primary-beat flash. Returns the cursor's
    /// global tick.
    fn advance_playback(&mut self, ui: &egui::Ui, timing: &ScoreTiming) -> f64 {
        let now = ui.input(|i| i.time);
        let latency_s = self.playback_ctl.audio_cfg.latency_s();
        let audio = self.playback_ctl.audio.as_ref();
        let playback = &mut self.playback_ctl.playback;
        // The output is still waking up: the cursor waits at the start for
        // the first click instead of running ahead of it.
        if audio.is_some_and(grooph_audio::Audio::is_starting) {
            playback.last_update = None;
            playback.waited_for_output = true;
            // Frames go on, so the wait ends (and the watchdog runs).
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
            return playback.smooth_tick;
        }
        let audio_pos = audio.and_then(|audio| audio.playback_position());
        let dt = now - playback.last_update.unwrap_or(now);
        playback.last_update = Some(now);

        let was_counting_in = playback.counting_in();
        playback.count_in = next_count_in(playback.count_in, now, audio_pos, latency_s, timing);
        let counting_in = playback.counting_in();
        let tick =
            advance_tick(playback.smooth_tick, dt, audio_pos, latency_s, counting_in, timing);
        playback.smooth_tick = tick;
        // Its own clock: taking over `tick` after the count-in would jump
        // back, as `tick` starts the score from the downbeat.
        playback.heard_tick =
            advance_tick(playback.heard_tick, dt, audio_pos, latency_s, false, timing);
        let waited = std::mem::take(&mut playback.waited_for_output);
        if (was_counting_in || waited) && !counting_in {
            self.realign_accuracy_clock();
        }

        // Flash once per beat of the playing measure.
        let (m_idx, local_tick) = timing.to_local(tick);
        let primary_beat_in_measure = self
            .played_score()
            .measures()
            .get(m_idx)
            .map_or(0, |m| primary_beat_index(&m.time_signature(), local_tick));
        let flash_enabled = self.ui.flash_enabled;
        self.playback_ctl
            .playback
            .reach_flash_beat((m_idx, primary_beat_in_measure), flash_enabled);
        let playback = &mut self.playback_ctl.playback;
        // Exponential decay towards 0; larger -> faster fade.
        let decay_per_sec = 10.0;
        playback.flash_intensity *= (-decay_per_sec * dt).exp() as f32;

        // Keep the animation running.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
        tick
    }

    /// Whether the staff shows accuracy marks: while calibrating, and with
    /// accuracy on outside the metronome tab. The tracker keeps the marks of
    /// the last practice run, keyed by onset, so they would land on the
    /// click bar's notes.
    pub(crate) fn shows_accuracy_marks(&self) -> bool {
        self.calibration_source().is_some()
            || (self.playback_ctl.accuracy.enabled && !self.metronome_mode())
    }

    /// Marks on the notes of `score`, the score `staff` was laid out from.
    fn draw_accuracy_markers(
        &self,
        ui: &egui::Ui,
        score: &Score,
        staff: &StaffLayout,
        timing: &ScoreTiming,
        staff_space: f32,
    ) {
        if !self.shows_accuracy_marks() {
            return;
        }
        // Marks stay on the staff after stopping, until the next run or an edit.
        if !self.playback_ctl.accuracy.tracker.has_marks() {
            return;
        }
        let total_loop_ticks = timing.total_loop_ticks();
        if total_loop_ticks == 0 {
            return;
        }
        // One flat list of global onsets across the whole score. Used by
        // `clamp_diff_to_beat_window`, which needs the global neighbours of
        // each note.
        let global_beats = crate::accuracy::compute_global_beat_onsets(score, timing);
        let global_onsets_u64: Vec<u64> = global_beats.iter().map(|gb| gb.onset_tick).collect();

        for system in &staff.systems {
            for placed in &system.measures {
                self.draw_accuracy_markers_for_measure(
                    ui,
                    score,
                    placed,
                    timing,
                    &global_onsets_u64,
                    total_loop_ticks as f64,
                    staff_space,
                );
            }
        }
    }

    #[expect(clippy::too_many_arguments)]
    fn draw_accuracy_markers_for_measure(
        &self,
        ui: &egui::Ui,
        score: &Score,
        placed: &PlacedMeasure,
        timing: &ScoreTiming,
        global_onsets_u64: &[u64],
        total_loop_ticks: f64,
        staff_space: f32,
    ) {
        let measure = &score.measures()[placed.measure_idx];
        // Every onset of any voice, at the head standing for it.
        let columns: Vec<(u32, &NoteLayout)> = measure
            .onsets()
            .iter()
            .filter_map(|o| Some((o.tick, onset_head(&placed.voices, o)?)))
            .collect();
        let measure_start = timing.measure_start_tick(placed.measure_idx);
        let dark = ui.visuals().dark_mode;
        for (idx, &(local_onset, note_layout)) in columns.iter().enumerate() {
            if note_layout.kind != grooph_measure::BeatKind::Note {
                continue;
            }
            // A note head is one staff space high.
            let y = note_layout.center.y + staff_space * 0.65;
            let h = staff_space * 0.8;
            let width = (staff_space * MARK_WIDTH_PER_HEAD).max(MARK_MIN_WIDTH);
            let note_x = note_layout.center.x;
            ui.painter().line_segment(
                [egui::pos2(note_x, y), egui::pos2(note_x, y + h)],
                Stroke::new(width * 0.5, reference_color(dark)),
            );
            let global_onset = measure_start + local_onset as u64;
            let Some(mark) = self.playback_ctl.accuracy.tracker.mark_for_onset(global_onset) else {
                continue;
            };
            match mark {
                AccuracyMark::Hit { diff_ticks, grade } => {
                    let global_idx = global_onsets_u64.binary_search(&global_onset).unwrap_or(0);
                    let diff_ticks = clamp_diff_to_beat_window(
                        diff_ticks,
                        global_idx,
                        global_onsets_u64,
                        total_loop_ticks,
                    );
                    let Some(px_per_tick) = local_pixels_per_tick(&columns, idx) else {
                        continue;
                    };
                    let x = note_x + (diff_ticks * px_per_tick) as f32;
                    paint_hit_mark(ui.painter(), note_x, x, y, h, width, grade, dark);
                }
                AccuracyMark::Miss => paint_miss_mark(ui.painter(), note_x, y, h, width, dark),
            }
        }
    }

    /// `with_ring`: under the metronome's ring a tap changes the tapped
    /// click's level, as on the ring.
    /// `ghost`: where the editor's ghost bar is, which a tap turns into a
    /// bar at the end.
    fn handle_input(
        &mut self,
        resp: Response,
        staff: &StaffLayout,
        with_ring: bool,
        ghost: Option<Rect>,
    ) {
        // While playing along, a touch or click on the staff is a hit, not an edit.
        if self.tap_capture_active() {
            if resp.contains_pointer() && resp.ctx.input(|i| i.pointer.any_pressed()) {
                self.push_pointer_tap();
            }
            return;
        }
        // A tap there changes the click's level, which also picks the voice
        // and moves the cursor to it.
        if with_ring && resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos()
                && let Some((voice, _, b_idx)) = hit_test_voice(staff, pos)
                && let Some(voice) = MetronomeVoice::of(voice)
            {
                self.tap_staff_note(voice, b_idx);
            }
            return;
        }
        if resp.clicked()
            && let Some(ghost) = ghost
            && resp.interact_pointer_pos().is_some_and(|p| ghost.contains(p))
        {
            self.push_measure();
            return;
        }
        if !matches!(self.ui.mode, Mode::TimeSignature(_))
            && (resp.clicked() || resp.dragged())
            && let Some(pos) = resp.interact_pointer_pos()
            && let Some(hit) = hit_test_head(staff, pos)
            // Not the ghost bar's rests.
            && hit.measure_idx < self.editor.doc().score.len()
        {
            // A tap on another voice edits that voice from now on: in the
            // metronome tab the click bar or its counter rhythm. On the
            // kit's staff it also picks the instrument whose head it is on.
            self.editor.doc_mut().cursor =
                Cursor::in_voice(hit.measure_idx, hit.voice, hit.beat_idx);
            if !self.staff_model().one_line() {
                self.editor.instrument = hit.instrument;
            }

            if resp.double_clicked()
                && let Some(tool) = all_tools()
                    .iter()
                    .find(|t| matches!(t.kind, ToolKind::Modify(Modifier::ToggleRestNote)))
            {
                self.apply_tool(tool);
            }
        }
    }
}

/// The editor's ghost bar in `rect`: its rests veiled, a dashed frame and a
/// "+" in the middle, brighter under the pointer.
fn draw_ghost_bar(ui: &egui::Ui, rect: Rect, hovered: bool) {
    let visuals = ui.visuals();
    let bg = visuals.panel_fill;
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        4.0,
        egui::Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), 230),
    );
    let color = if hovered { visuals.text_color() } else { visuals.weak_text_color() };
    let frame = rect.shrink(4.0);
    let stroke = egui::Stroke::new(1.5, color);
    let corners = [frame.left_top(), frame.right_top(), frame.right_bottom(), frame.left_bottom()];
    for i in 0..4 {
        painter.extend(egui::Shape::dashed_line(
            &[corners[i], corners[(i + 1) % 4]],
            stroke,
            6.0,
            4.0,
        ));
    }
    let size = (0.5 * frame.height()).clamp(16.0, 48.0);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "+",
        FontId::proportional(size),
        color,
    );
}

/// Dim a bar silenced by the gap click, so the silence reads as intended.
fn draw_muted_bar(ui: &egui::Ui, rect: Rect) {
    let bg = ui.visuals().panel_fill;
    let veil = egui::Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), 150);
    ui.painter().rect_filled(rect, 4.0, veil);
    ui.painter().text(
        rect.right_top() + egui::vec2(-6.0, 6.0),
        egui::Align2::RIGHT_TOP,
        "🔇",
        FontId::proportional(16.0),
        ui.visuals().weak_text_color(),
    );
}

/// Index of the primary beat (the metronome's click group) that
/// `local_tick` falls in, e.g. 0..=2 in 7/8 (2+2+3), not one per eighth.
fn primary_beat_index(ts: &TimeSignature, local_tick: f64) -> u32 {
    DEFAULT_GRID.primary_boundaries(ts).iter().take_while(|&&b| b as f64 <= local_tick).count()
        as u32
}

/// Accuracy marks scale with the note head, but never get thinner than this (px).
const MARK_MIN_WIDTH: f32 = 3.0;
const MARK_WIDTH_PER_HEAD: f32 = 0.25;

/// Marker colour of a grade (misses use `Off`): blue, orange/yellow and
/// purple stay apart for red-green and blue-yellow colour vision deficiencies
/// and differ in lightness. No red/green pair. The grade is shown by shape as
/// well (off hits get an arrowhead, misses a cross), so colour is never the
/// only cue.
fn grade_color(grade: HitGrade, dark: bool) -> egui::Color32 {
    match (grade, dark) {
        (HitGrade::Perfect, false) => egui::Color32::from_rgb(51, 102, 204),
        (HitGrade::Perfect, true) => egui::Color32::from_rgb(102, 204, 238),
        (HitGrade::Good, false) => egui::Color32::from_rgb(215, 140, 0),
        (HitGrade::Good, true) => egui::Color32::from_rgb(240, 228, 66),
        (HitGrade::Off, false) => egui::Color32::from_rgb(153, 0, 102),
        (HitGrade::Off, true) => egui::Color32::from_rgb(200, 80, 255),
    }
}

/// The note's zero line and the connector to the hit.
fn reference_color(dark: bool) -> egui::Color32 {
    if dark { egui::Color32::from_gray(150) } else { egui::Color32::from_gray(120) }
}

/// A hit: a bar where it landed, joined to the note's zero line. Perfect and
/// good are plain bars; an off hit also gets an arrowhead pointing the way it
/// was off (left = early, right = late).
#[expect(clippy::too_many_arguments)]
fn paint_hit_mark(
    painter: &egui::Painter,
    note_x: f32,
    x: f32,
    y: f32,
    h: f32,
    width: f32,
    grade: HitGrade,
    dark: bool,
) {
    let color = grade_color(grade, dark);
    let mid_y = y + h * 0.5;
    painter.line_segment(
        [egui::pos2(note_x, mid_y), egui::pos2(x, mid_y)],
        Stroke::new(width * 0.5, reference_color(dark)),
    );
    painter.line_segment([egui::pos2(x, y), egui::pos2(x, y + h)], Stroke::new(width, color));
    if grade == HitGrade::Off {
        let dir = if x < note_x { -1.0 } else { 1.0 };
        let tip_x = x - dir * width * 0.5;
        let base_x = tip_x - dir * width * 2.0;
        let half = width * 1.4;
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(tip_x, mid_y),
                egui::pos2(base_x, mid_y - half),
                egui::pos2(base_x, mid_y + half),
            ],
            color,
            Stroke::NONE,
        ));
    }
}

/// A missed note: a cross on the zero line.
fn paint_miss_mark(painter: &egui::Painter, note_x: f32, y: f32, h: f32, width: f32, dark: bool) {
    let stroke = Stroke::new(width, grade_color(HitGrade::Off, dark));
    let mid_y = y + h * 0.5;
    let half = h * 0.35;
    let (left, right, top, bottom) = (note_x - half, note_x + half, mid_y - half, mid_y + half);
    painter.line_segment([egui::pos2(left, top), egui::pos2(right, bottom)], stroke);
    painter.line_segment([egui::pos2(left, bottom), egui::pos2(right, top)], stroke);
}

/// Pixels per tick around column `idx` of a measure's `columns` (onset tick
/// and head), toward the next column or, for the last, from the one before.
fn local_pixels_per_tick(columns: &[(u32, &NoteLayout)], idx: usize) -> Option<f64> {
    let slope = |a: usize, b: usize| {
        let (ta, na) = columns.get(a)?;
        let (tb, nb) = columns.get(b)?;
        let dt = (*tb as i64 - *ta as i64) as f64;
        (dt != 0.0).then(|| (nb.center.x - na.center.x) as f64 / dt)
    };
    slope(idx, idx + 1).or_else(|| idx.checked_sub(1).and_then(|prev| slope(prev, idx)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flashes(ts: TimeSignature) -> Vec<u32> {
        let tpm = DEFAULT_GRID.ticks_per_measure(&ts);
        let step = DEFAULT_GRID.ticks_per_beat(&ts);
        let mut keys: Vec<u32> =
            (0..tpm).step_by(step as usize).map(|t| primary_beat_index(&ts, t as f64)).collect();
        keys.dedup();
        keys
    }

    #[test]
    fn primary_beat_index_follows_metronome_groups() {
        assert_eq!(flashes(TimeSignature::FOUR_FOUR), [0, 1, 2, 3]);
        assert_eq!(flashes(TimeSignature::SIX_EIGHT), [0, 1]);
        assert_eq!(flashes(TimeSignature::new(7, 8)), [0, 1, 2]);
        // 3+2+2: the second flash comes on the fourth eighth.
        let ts = TimeSignature::SEVEN_EIGHT.with_grouping(2);
        let tick = |eighth: u32| (eighth * DEFAULT_GRID.ticks_per_beat(&ts)) as f64;
        assert_eq!(primary_beat_index(&ts, tick(2)), 0);
        assert_eq!(primary_beat_index(&ts, tick(3)), 1);
    }
}
