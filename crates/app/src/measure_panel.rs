use crate::Grooph;
use crate::accuracy::{AccuracyMark, HitGrade, clamp_diff_to_beat_window};
use crate::documents::{Document, Documents};
use crate::guide::{Guide, anchor_intensity};
use crate::metronome_view::{MetronomeState, MetronomeVoice};
use crate::play_along::PlayAlongState;
use crate::playback_clock::{advance_tick, next_count_in};
use crate::state::PlaybackState;
use crate::tokens;
use crate::{Mode, TransportState};
use eframe::egui;
use eframe::egui::{FontId, Frame, Rect, Response, Stroke};
use grooph_layout::pixel_layout::compute_em;
use grooph_layout::staff_layout::{
    LEGIBILITY_FLOOR_EM, PlacedMeasure, ROOM_BELOW_EM, StaffLayout, StaffOpts, hit_test_head,
    hit_test_voice,
};
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::tempo::ScoreTiming;
use grooph_measure::{Cursor, Score, TimeSignature};
use grooph_render::staff::{PlaybackMarks, RenderContext, draw_staff};
use std::rc::Rc;

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
        let width_cap_factor = if self.ui.staff.narrow_screen {
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
        let font_id = FontId::new(em, self.ui.fonts.music_font_id.family.clone());

        // Tighter spacing on narrow screens makes the glyphs
        // bigger: the staff shrinks its em until a measure
        // full of sixteenths fits the width.
        let narrow = self.ui.staff.narrow_screen;
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
            // The compact card in a chat has no meter in a card: the staff
            // shows it. So does the score tab's, whatever its meters.
            time_signature_first: self.compact_card() || self.tab() == crate::shell::Tab::Score,
            room_below_em: ROOM_BELOW_EM,
            // The count labels and the stickings, each on its side.
            text_rows: self.text_rows(),
            // In the metronome every click is its own to tap. The editor
            // shows the whole rest too: its beats keep their places for the
            // cursor and the taps.
            bar_rests: !self.metronome_mode(),
            // The rests the score leaves out stay to be picked and replaced.
            veil_hidden_rests: self.ui.mode == Mode::Edit,
        }
    }

    /// Height of the metronome's staff panel at `width` (the room beside
    /// the panel's margins), margins included: what it needs this frame.
    pub(crate) fn metronome_staff_height(&self, ui: &egui::Ui, width: f32) -> f32 {
        let margins = staff_margins(ui.style());
        let rect = Rect::from_min_size(ui.cursor().min, egui::vec2(width - margins.x, 1.0e4));
        let opts = self.staff_opts(ui, rect);
        crate::staff_cache::staff_layout(self.editor.doc().score(), &opts).total_size.y + margins.y
    }

    /// The staff in its scroll area, one frame. The steps, in order:
    ///
    /// 0. [`staff_scroll_area`]: the scroll area, before the steps in it.
    /// 1. [`Self::panel_frame`]: playback moves on, the guide's anchor.
    /// 2. [`ShownScore::of`] and [`fit_to_height`]: what is shown, laid out.
    /// 3. [`follow_playback`] and [`follow_edit_cursor`]: auto-scroll.
    /// 4. [`hover_with_ring`]: the hover handoff with the metronome's ring.
    /// 5. [`Self::draw_staff_view`]: the staff, its flash and the ghost bar.
    /// 6. [`Self::bar_menu`]: the bar menu, which returns a [`BarAction`].
    /// 7. [`Self::draw_playing_marks`]: a muted bar, the accuracy marks.
    /// 8. [`Self::staff_input`], then [`Self::apply_bar_action`].
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
                let playing = self.playback_ctl.transport_state == TransportState::Playing;
                let playback = &mut self.playback_ctl.playback;
                let scroll_area =
                    staff_scroll_area(ui, playback, playing, self.ui.staff.touch_seen);
                scroll_area.show(ui, |ui| self.staff_view(ui, available));
            });
        });
    }

    /// The steps of [`Self::measure_panel`] inside the scroll area.
    ///
    /// The shown score is borrowed from the documents (or the calibration)
    /// field by field, so the steps that change other parts of the app (the
    /// playback state, the follow state, the hover) take just those parts.
    /// The steps that change the score or the cursor come last, once
    /// nothing borrows it any more.
    fn staff_view(&mut self, ui: &mut egui::Ui, available: egui::Vec2) {
        // `origin`, unlike `available`, must come from the
        // *inner* ui: egui bakes the current scroll offset
        // into its cursor position, which is what makes
        // content laid out from here actually move as the
        // user scrolls (the outer ui's cursor is fixed).
        let view = ScrollView { origin: ui.cursor().min, clip: ui.clip_rect() };
        let frame = self.panel_frame(ui);

        let edit = self.ui.mode == Mode::Edit;
        let metronome = self.metronome_mode();
        let calibrating = self.calibrating();
        let staff_opts = self.staff_opts(ui, Rect::from_min_size(view.origin, available));
        let shown = ShownScore::of(
            &self.editor.docs,
            self.editor.document,
            &self.playback_ctl.play_along,
            edit && !metronome && !calibrating,
        );
        let (staff_opts, staff) = fit_to_height(shown.score(), staff_opts, available.y);
        // Rows may render at a different em than the baseline `staff_opts`
        // (grow-to-fill / shrink-toward-floor) — rescale font/metrics to
        // match before drawing, so glyph size agrees with `staff`'s positions.
        let render_opts = staff_opts.rescaled(staff.scale);
        self.ui.staff.em = render_opts.em;

        follow_playback(ui, &mut self.playback_ctl.playback, &shown, &staff, &frame, view);
        let cursor = self.editor.doc().cursor();
        let editing = edit && self.playback_ctl.transport_state == TransportState::Stopped;
        follow_edit_cursor(ui, &staff, cursor, editing, view, &mut self.ui.staff.followed);

        let (rect, resp) = ui.allocate_exact_size(
            staff.total_size,
            staff_sense(ui, &mut self.ui.staff.touch_seen, &mut self.ui.staff.touching),
        );
        let ghost_rect = shown.ghost_rect(&staff);
        let with_ring = metronome && !edit;
        let lit = hover_with_ring(ui, &resp, &staff, with_ring, &mut self.editor.metronome);
        let staff_view = StaffView { shown: &shown, staff: &staff, render_opts: &render_opts };
        self.draw_staff_view(ui, &staff_view, &frame, rect, &resp, lit, ghost_rect);
        let bar_action = (edit && !calibrating)
            .then(|| self.bar_menu(ui, &resp, &staff, cursor.measure_idx))
            .flatten();
        self.draw_playing_marks(ui, &staff_view, &frame);

        self.staff_input(ui, resp, &staff, with_ring, ghost_rect);
        self.apply_bar_action(bar_action);
    }

    /// Step 1: move playback on to this frame and note the guide's anchor.
    /// It changes much of the playback state (the cursor, the count-in, the
    /// flash, the accuracy clock), so it takes the app as a whole.
    fn panel_frame(&mut self, ui: &egui::Ui) -> PanelFrame {
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
        let playing =
            playback_tick_to_draw.map(|global| timing.to_local(timing.written_global_tick(global)));
        let now = ui.input(|i| i.time);
        self.update_guide_anchor(playing, now);
        PanelFrame { timing, playing, guide: self.active_guide(), now }
    }

    /// Step 5: the staff, the flash over it and the ghost bar. `rect` and
    /// `resp` are the staff's place and response; `lit` the note the
    /// metronome's ring lights.
    #[expect(clippy::too_many_arguments)]
    fn draw_staff_view(
        &self,
        ui: &mut egui::Ui,
        view: &StaffView,
        frame: &PanelFrame,
        rect: Rect,
        resp: &Response,
        lit: Option<Cursor>,
        ghost_rect: Option<Rect>,
    ) {
        let playback = &self.playback_ctl.playback;
        // A silent bar shows no playback position, like the
        // metronome's ring: no line, no lit beat or count.
        let marks =
            if playback.bar_muted {
                PlaybackMarks::Anchor(None)
            } else if frame.guide == Guide::Line {
                PlaybackMarks::Line
            } else {
                PlaybackMarks::Anchor(playback.anchor.map(|a| {
                    (a.measure_idx, f64::from(a.tick), anchor_intensity(frame.now - a.at))
                }))
            };
        let cursor = (self.ui.mode == Mode::Edit).then_some(self.editor.doc().cursor());
        let count_config = self.build_count_config();
        let ctx = RenderContext {
            count_config: count_config.as_ref(),
            cursor,
            lit,
            playback: frame.playing,
            loop_bars: 0..view.shown.played.len(),
            marks,
            ..RenderContext::new(view.shown.score(), view.staff, view.render_opts)
        };
        draw_staff(ui, &ctx);

        // Draw flash overlay (white on dark, black on light) with decay
        let flash = playback.flash_shown();
        if flash > 0.01 {
            let dark = ui.visuals().dark_mode;
            let base = tokens::FLASH.get(dark);
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
    }

    /// Step 6: the bar menu, a "⋯" on the cursor's bar (`cursor_bar`) or a
    /// long press (right click) on any bar. Returns what was picked, which
    /// [`Self::apply_bar_action`] does once the staff is drawn.
    fn bar_menu(
        &self,
        ui: &mut egui::Ui,
        resp: &Response,
        staff: &StaffLayout,
        cursor_bar: usize,
    ) -> Option<BarAction> {
        let mut bar_action = None;
        let bars = self.editor.doc().score().len();
        let metronome = self.metronome_mode();
        // Respelling the bar only while a menu shows it.
        let clean_up = || !metronome && self.can_clean_up_current_measure();
        let menu = BarMenu { metronome, bars, clean_up: &clean_up };
        if (resp.secondary_clicked() || resp.long_touched())
            && let Some(pos) = resp.interact_pointer_pos()
            && let Some(idx) =
                (0..bars).find(|&idx| staff.placed(idx).is_some_and(|p| p.rect.contains(pos)))
        {
            bar_action = Some(BarAction::Select(idx));
        }
        resp.context_menu(|ui| menu.items(ui, &mut bar_action));
        if let Some(placed) = staff.placed(cursor_bar) {
            let chip = bar_menu_chip(ui, placed.rect);
            egui::Popup::menu(&chip).show(|ui| menu.items(ui, &mut bar_action));
        }
        bar_action
    }

    /// Step 7: the marks that go with playing, drawn over the bar menu's
    /// chip: the veil on a bar the gap click silences, and the accuracy
    /// marks.
    fn draw_playing_marks(&self, ui: &egui::Ui, view: &StaffView, frame: &PanelFrame) {
        if self.playback_ctl.playback.bar_muted
            && let Some((m_idx, _)) = frame.playing
            && let Some(placed) = view.staff.placed(m_idx)
        {
            draw_muted_bar(ui, placed.rect);
        }
        self.draw_accuracy_markers(
            ui,
            view.shown.played,
            view.staff,
            &frame.timing,
            view.render_opts.staff_space(),
        );
    }

    /// Step 8: what a pointer does on the staff. The first finger's drag,
    /// before the staff leaves its drags to the scroll area, scrolls by
    /// hand; anything else goes to [`Self::handle_input`].
    fn staff_input(
        &mut self,
        ui: &egui::Ui,
        resp: Response,
        staff: &StaffLayout,
        with_ring: bool,
        ghost_rect: Option<Rect>,
    ) {
        if resp.dragged() && ui.input(|i| i.any_touches()) {
            ui.scroll_with_delta_animation(
                resp.drag_delta(),
                eframe::egui::style::ScrollAnimation::none(),
            );
        } else {
            self.handle_input(resp, staff, with_ring, ghost_rect);
        }
    }

    /// Step 8, last: do what was picked in the bar menu, through the edit
    /// paths of the bar's own commands.
    fn apply_bar_action(&mut self, action: Option<BarAction>) {
        match action {
            Some(BarAction::Select(idx)) => {
                let doc = self.editor.doc_mut();
                if doc.cursor().measure_idx != idx {
                    let cursor = Cursor { measure_idx: idx, beat_idx: 0, ..doc.cursor() };
                    *doc.cursor_mut() = doc.score().clamp_cursor(cursor);
                }
            }
            Some(BarAction::Duplicate) => self.duplicate_current_measure(),
            Some(BarAction::CleanUp) => self.clean_up_current_measure(),
            Some(BarAction::Clear) => self.clear_current_measure(),
            Some(BarAction::Delete) => self.remove_current_measure(),
            None => {}
        }
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
        let latency_s = self.playback_ctl.devices.latency_s();
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
        let flash_enabled = self.playback_ctl.feedback.flash_enabled;
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
    /// accuracy on in Practice. The tracker keeps the marks of
    /// the last practice run, keyed by onset, so they would land on the
    /// click bar's notes.
    pub(crate) fn shows_accuracy_marks(&self) -> bool {
        self.calibration_source().is_some() || self.accuracy_active()
    }

    /// Marks on the notes of `score`, the score `staff` was laid out from
    /// (maybe with the ghost bar after it).
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
        let global_beats = crate::accuracy::global_beat_onsets(score, timing);
        let global_onsets_u64: Vec<u64> = global_beats.iter().map(|gb| gb.onset_tick).collect();

        for system in &staff.systems {
            // Not the ghost bar after the score's last.
            for placed in system.measures.iter().filter(|p| p.measure_idx < score.len()) {
                self.draw_accuracy_markers_for_measure(
                    ui,
                    placed,
                    timing,
                    &global_onsets_u64,
                    total_loop_ticks as f64,
                    staff_space,
                );
            }
        }
    }

    fn draw_accuracy_markers_for_measure(
        &self,
        ui: &egui::Ui,
        placed: &PlacedMeasure,
        timing: &ScoreTiming,
        global_onsets_u64: &[u64],
        total_loop_ticks: f64,
        staff_space: f32,
    ) {
        // Every onset of any voice, at the head standing for it.
        let columns = placed.ticks.columns();
        let measure_start = timing.measure_start_tick(placed.measure_idx);
        let dark = ui.visuals().dark_mode;
        for (idx, column) in columns.iter().enumerate() {
            let note_layout = &placed.voices[column.voice].notes[column.beat_idx];
            if note_layout.kind != grooph_measure::BeatKind::Note {
                continue;
            }
            // A note head is one staff space high.
            let y = note_layout.center.y + staff_space * 0.65;
            let h = staff_space * 0.8;
            let width = (staff_space * MARK_WIDTH_PER_HEAD).max(MARK_MIN_WIDTH);
            let note_x = column.x;
            ui.painter().line_segment(
                [egui::pos2(note_x, y), egui::pos2(note_x, y + h)],
                Stroke::new(width * 0.5, reference_color(dark)),
            );
            let global_onset = measure_start + column.tick as u64;
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
                    let Some(px_per_tick) = placed.ticks.px_per_tick(idx) else {
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
        if (resp.clicked() || resp.dragged())
            && let Some(pos) = resp.interact_pointer_pos()
            && let Some(hit) = hit_test_head(staff, pos, self.editor.doc().score())
            // Not the ghost bar's rests.
            && hit.measure_idx < self.editor.doc().score().len()
        {
            // A tap on another voice edits that voice from now on: in the
            // metronome tab the click bar or its counter rhythm. On the
            // kit's staff it also picks the instrument whose head it is on.
            *self.editor.doc_mut().cursor_mut() =
                Cursor::in_voice(hit.measure_idx, hit.voice, hit.beat_idx);
            if !self.staff_model().one_line() {
                self.editor.instrument = hit.instrument;
            }
        }
    }
}

/// The scroll area the staff sits in. It takes the playback tween's offset
/// right away when endless mode just removed rows above. `touch_seen`: see
/// [`staff_sense`].
fn staff_scroll_area(
    ui: &mut egui::Ui,
    playback: &mut PlaybackState,
    playing: bool,
    touch_seen: bool,
) -> egui::ScrollArea {
    // No scroll bar: the wheel, a finger or the cursor scroll
    // the score, and a bar fading in over the notes distracts.
    // Nor egui's shading at an edge with more beyond it: as
    // the cursor scrolls a staff a little wider than the
    // screen, it would move from one side to the other.
    ui.spacing_mut().scroll.fade.strength = 0.0;
    let mut scroll_area = egui::ScrollArea::both()
        // While playing, the tween in `follow_playback` is the only
        // animation: egui's own would keep heading for a target from an
        // earlier frame, e.g. one before endless mode removed a row.
        .animated(!playing)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        // egui turns drag-to-scroll on only after it has seen a touch, so
        // the very first swipe would be lost on a phone. Where a finger is
        // known to be the pointer the scroll area takes drags from the start;
        // otherwise the staff's own click-and-drag sense wins them anyway.
        .scroll_source(egui::scroll_area::ScrollSource {
            drag: if touch_seen {
                egui::scroll_area::DragScroll::Always
            } else {
                egui::scroll_area::DragScroll::OnTouch
            },
            ..Default::default()
        });
    if std::mem::take(&mut playback.rows_removed)
        && let Some(offset) = playback.scroll.value_at(ui.input(|i| i.time))
    {
        scroll_area = scroll_area.vertical_scroll_offset(offset);
    }
    scroll_area
}

/// What playback is at this frame, worked out before anything is drawn
/// (see [`Grooph::panel_frame`]).
struct PanelFrame {
    timing: ScoreTiming,
    /// Measure and local tick (at its written position) the playback
    /// cursor is at, while playing.
    playing: Option<(usize, f64)>,
    guide: Guide,
    /// The frame's time (s).
    now: f64,
}

/// The score the staff shows.
struct ShownScore<'a> {
    /// The bars that play: a calibration's click bar while it runs, else
    /// the document. Timing marks and the playback cursor go by them: the
    /// cursor never enters the ghost bar.
    played: &'a Score,
    /// `played` with an empty "ghost" bar after the last one, in a rhythm's
    /// editor: a tap there adds a bar. Laid out with the score, drawn
    /// veiled.
    ghost: Option<Score>,
}

impl<'a> ShownScore<'a> {
    /// Step 2: the score shown, `ghost` with the ghost bar. Borrowed from
    /// the parts of the app that hold it, not the app as a whole, so that
    /// the steps after it can change the others.
    fn of(
        docs: &'a Documents,
        document: Document,
        play_along: &'a PlayAlongState,
        ghost: bool,
    ) -> Self {
        let played = crate::calibration::played_score(docs, document, play_along);
        let ghost = ghost.then(|| {
            let mut shown = played.clone();
            let last = &played.measures()[played.len() - 1];
            shown.push(last.empty_like(last.time_signature()));
            shown
        });
        Self { played, ghost }
    }

    /// The score laid out: the ghost bar's, if any.
    fn score(&self) -> &Score {
        self.ghost.as_ref().unwrap_or(self.played)
    }

    /// Where the ghost bar is on `staff`.
    fn ghost_rect(&self, staff: &StaffLayout) -> Option<Rect> {
        self.ghost.as_ref().and_then(|g| staff.placed(g.len() - 1)).map(|p| p.rect)
    }
}

/// What the drawing steps share: the score shown, its layout and the
/// options it is drawn with.
struct StaffView<'a> {
    shown: &'a ShownScore<'a>,
    staff: &'a StaffLayout,
    render_opts: &'a StaffOpts,
}

/// Step 2: `score` laid out with `opts`, and the options it was laid out
/// with. When one row is taller than `height` (the view's), the staff
/// shrinks toward the legibility floor to fit it and is laid out again:
/// nothing above or below the staff (accents, count labels) is then cut off
/// on a short, wide panel.
fn fit_to_height(score: &Score, opts: StaffOpts, height: f32) -> (StaffOpts, Rc<StaffLayout>) {
    let staff = crate::staff_cache::staff_layout(score, &opts);
    let row_height = staff.room.height() * opts.em;
    if row_height > height && height > 0.0 {
        let floor = LEGIBILITY_FLOOR_EM / opts.em;
        let opts = opts.rescaled((height / row_height).max(floor.min(1.0)));
        let staff = crate::staff_cache::staff_layout(score, &opts);
        (opts, staff)
    } else {
        (opts, staff)
    }
}

/// The scroll area's view this frame, on screen: where its content starts
/// (`origin`, which moves as it scrolls) and the part of it shown (`clip`).
#[derive(Clone, Copy, Debug)]
struct ScrollView {
    origin: egui::Pos2,
    clip: Rect,
}

/// The vertical scroll offset to head for while the playing row is `row`
/// (on screen), in a staff `content_height` high.
///
/// The row is centred vertically. A row already in view stays where it
/// is: starting to play must not scroll the staff to the ghost bar. The
/// offset is one egui will not clamp, so the tween toward it never stalls
/// at the top or bottom of the score.
fn playing_row_offset(row: Rect, content_height: f32, view: ScrollView) -> f32 {
    let view_height = view.clip.height();
    let max_offset = (content_height - view_height).max(0.0);
    let centred = (row.center().y - view.origin.y - view_height / 2.0).clamp(0.0, max_offset);
    let shown = (view.clip.top() - view.origin.y).clamp(0.0, max_offset);
    let in_view =
        row.top() - view.origin.y >= shown && row.bottom() - view.origin.y <= shown + view_height;
    if in_view { shown } else { centred }
}

/// Step 3, while playing: scroll to the playing row (see
/// [`playing_row_offset`]). The offset eases to it, so the rows glide up
/// when the cursor wraps rather than jump; horizontally the view follows
/// the cursor at once. Notes the rows above the view for endless mode.
fn follow_playback(
    ui: &egui::Ui,
    playback: &mut PlaybackState,
    shown: &ShownScore,
    staff: &StaffLayout,
    frame: &PanelFrame,
    view: ScrollView,
) {
    // Auto-scroll follows the same visual cursor logic as the
    // renderer (incl. cross-measure wrap animation), so we use
    // the shared helper to find the cursor's actual home measure
    // and X for this frame.
    let Some(playing) = frame.playing else { return };
    let Some((m_idx, x)) =
        grooph_render::staff::current_cursor_x(staff, playing, 0..shown.played.len())
    else {
        return;
    };
    let Some(placed) = staff.placed(m_idx) else { return };
    let row_offset = playing_row_offset(placed.rect, staff.total_size.y, view);
    let offset = playback.scroll.update(row_offset, frame.now);
    playback.row_pitch = staff
        .systems
        .get(1)
        .zip(staff.systems.first())
        .map_or(0.0, |(b, a)| b.rect.top() - a.rect.top());
    playback.rows_above_view =
        if playback.row_pitch > 0.0 { (offset / playback.row_pitch).floor() as usize } else { 0 };
    // Centring this rect scrolls to exactly `offset`;
    // horizontally it follows the cursor at once.
    let view_height = view.clip.height();
    let cursor_rect = egui::Rect::from_center_size(
        egui::pos2(x, view.origin.y + offset + view_height / 2.0),
        egui::vec2(1.0, placed.rect.height()),
    );
    ui.scroll_to_rect_animation(
        cursor_rect,
        Some(egui::Align::Center),
        eframe::egui::style::ScrollAnimation::none(),
    );
}

/// The edit cursor the staff follows, and the view's height then (see
/// [`edit_cursor_target`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct EditFollow {
    /// The edit cursor while editing and stopped, else `None`.
    pub(crate) cursor: Option<Cursor>,
    pub(crate) view_height: f32,
}

/// The rect to scroll into view (just enough to show it) for the edit
/// cursor `cursor`, if the view follows it this frame: when the cursor
/// followed changed from `last` to `now`, or the view's height changed
/// while it is followed (the tool grid opening under it), as the bar must
/// still be whole in it. Following only on a change leaves the user free
/// to scroll away from it.
fn edit_cursor_target(
    staff: &StaffLayout,
    cursor: Cursor,
    last: EditFollow,
    now: EditFollow,
) -> Option<Rect> {
    let resized = now.cursor.is_some()
        && now.view_height > 1.0
        && (now.view_height - last.view_height).abs() > 0.5;
    if now.cursor == last.cursor && !resized {
        return None;
    }
    let placed = staff.placed(cursor.measure_idx)?;
    let note = placed.voices.get(cursor.voice)?.notes.get(cursor.beat_idx)?;
    Some(Rect::from_min_size(
        egui::pos2(note.center.x, placed.rect.top()),
        egui::vec2(1.0, placed.rect.height()),
    ))
}

/// Step 3, while editing: bring the edit cursor into view (see
/// [`edit_cursor_target`]). Followed only while `editing` (and stopped), so
/// it doesn't clobber [`follow_playback`]'s re-centering.
fn follow_edit_cursor(
    ui: &egui::Ui,
    staff: &StaffLayout,
    cursor: Cursor,
    editing: bool,
    view: ScrollView,
    followed: &mut EditFollow,
) {
    let now = EditFollow { cursor: editing.then_some(cursor), view_height: view.clip.height() };
    if let Some(rect) = edit_cursor_target(staff, cursor, *followed, now) {
        ui.scroll_to_rect_animation(rect, None, eframe::egui::style::ScrollAnimation::none());
    }
    *followed = now;
}

/// Step 4: under the metronome's ring (`with_ring`) the staff and the ring
/// light each other's hovered note and segment. Notes the staff's hovered
/// click for the ring, and returns the ring's, to light on the staff.
fn hover_with_ring(
    ui: &egui::Ui,
    resp: &Response,
    staff: &StaffLayout,
    with_ring: bool,
    metronome: &mut MetronomeState,
) -> Option<Cursor> {
    let lit = with_ring
        .then_some(metronome.ring_hover)
        .flatten()
        .map(|(voice, beat_idx)| Cursor::in_voice(0, voice.idx(), beat_idx));
    let staff_hover = resp
        .hover_pos()
        .filter(|_| with_ring)
        .and_then(|pos| hit_test_voice(staff, pos))
        .and_then(|(voice, _, beat_idx)| Some((MetronomeVoice::of(voice)?, beat_idx)));
    if staff_hover != metronome.staff_hover {
        // The ring was drawn before the staff: show it there next frame.
        metronome.staff_hover = staff_hover;
        ui.ctx().request_repaint();
    }
    lit
}

/// What the staff reacts to: a tap everywhere, and a mouse also its drag
/// (it moves the edit cursor). Once a finger has touched the screen a drag
/// is left to the scroll area, which scrolls with its momentum and its give
/// at the ends, as the tool grid does. A phone or tablet is known to have
/// a finger as its pointer from the start (`platform::finger_is_the_pointer`).
/// Otherwise the hit test, which uses the previous frame's widgets, leaves
/// it to the touches seen before, not the ones of this frame: the first
/// finger's drag is scrolled by hand.
fn staff_sense(ui: &egui::Ui, touch_seen: &mut bool, touching: &mut bool) -> egui::Sense {
    // Not changed under a finger that is dragging: a gesture keeps the
    // sense it started with.
    let now = ui.input(|i| i.any_touches());
    *touch_seen |= *touching && !now;
    *touching = now;
    match *touch_seen {
        true => egui::Sense::click(),
        false => egui::Sense::click_and_drag(),
    }
}

/// What the bar menu does to the cursor's bar, after the staff is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
enum BarAction {
    /// A long press on a bar: the menu is for that bar.
    Select(usize),
    Duplicate,
    /// Respell the bar for reading (see `Grooph::clean_up_current_measure`).
    CleanUp,
    Clear,
    Delete,
}

/// The bar menu's entries: in the metronome's one bar only "Clear".
#[derive(Clone, Copy)]
struct BarMenu<'a> {
    metronome: bool,
    bars: usize,
    /// Whether "Clean up bar" would change the bar; greyed out otherwise.
    /// Asked only while the menu is shown.
    clean_up: &'a dyn Fn() -> bool,
}

impl BarMenu<'_> {
    fn items(self, ui: &mut egui::Ui, action: &mut Option<BarAction>) {
        // Rows as tall as a finger's touch, as wide as the menu.
        let row = |ui: &mut egui::Ui, text: egui::RichText, enabled: bool| {
            let size = egui::vec2(ui.available_width().max(BAR_MENU_WIDTH), BAR_MENU_ROW);
            ui.add_enabled(enabled, egui::Button::new(text).min_size(size)).clicked()
        };
        ui.set_min_width(BAR_MENU_WIDTH);
        if !self.metronome && row(ui, "Clean up bar".into(), (self.clean_up)()) {
            *action = Some(BarAction::CleanUp);
        }
        if !self.metronome && row(ui, "Duplicate bar".into(), true) {
            *action = Some(BarAction::Duplicate);
        }
        if row(ui, (if self.metronome { "Clear" } else { "Clear bar" }).into(), true) {
            *action = Some(BarAction::Clear);
        }
        if !self.metronome {
            let delete = egui::RichText::new("Delete bar").color(ui.visuals().error_fg_color);
            if row(ui, delete, self.bars > 1) {
                *action = Some(BarAction::Delete);
            }
        }
    }
}

/// The bar menu's width and the height of its rows, a finger's touch.
const BAR_MENU_WIDTH: f32 = 160.0;
const BAR_MENU_ROW: f32 = 44.0;

/// The "⋯" in the top right corner of the cursor's bar (`bar`), which
/// opens the bar menu.
fn bar_menu_chip(ui: &mut egui::Ui, bar: Rect) -> Response {
    let size = egui::vec2(24.0, 16.0);
    let rect = Rect::from_min_size(bar.right_top() + egui::vec2(-size.x - 1.0, 0.0), size);
    let resp = ui
        // Drawn small, so it stays clear of marks above the last beat; the
        // finger gets more room.
        .interact(rect.expand(6.0), ui.scope_id().with("bar_menu"), egui::Sense::click())
        .on_hover_text("Bar menu");
    let visuals = ui.style().interact(&resp);
    let painter = ui.painter();
    painter.rect(
        rect,
        visuals.corner_radius,
        visuals.weak_bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    for dx in [-5.0, 0.0, 5.0] {
        painter.circle_filled(rect.center() + egui::vec2(dx, 0.0), 1.6, visuals.fg_stroke.color);
    }
    resp
}

/// How much of the ghost bar's rests the veil hides (of 255).
const GHOST_VEIL: u8 = 230;

/// The editor's ghost bar in `rect`: its rests veiled, a dashed frame and a
/// "+" in the middle, brighter under the pointer.
fn draw_ghost_bar(ui: &egui::Ui, rect: Rect, hovered: bool) {
    let visuals = ui.visuals();
    let bg = visuals.panel_fill;
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        4.0,
        egui::Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), GHOST_VEIL),
    );
    // The frame and "+" in the veiled staff's color, a little brighter
    // under the pointer.
    let veil = if hovered { 0.6 } else { f32::from(GHOST_VEIL) / 255.0 };
    let color = visuals.text_color().lerp_to_gamma(bg, veil);
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
    // Drawn as two bars, thicker than the font's "+".
    let arm = (0.2 * frame.height()).clamp(7.0, 20.0);
    let thick = egui::Stroke::new((arm / 5.0).max(2.5), color);
    let c = rect.center();
    painter.line_segment([c - egui::vec2(arm, 0.0), c + egui::vec2(arm, 0.0)], thick);
    painter.line_segment([c - egui::vec2(0.0, arm), c + egui::vec2(0.0, arm)], thick);
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

/// Marker colour of a grade (misses use `Off`, see [`tokens::GRADE_PERFECT`]).
pub(crate) fn grade_color(grade: HitGrade, dark: bool) -> egui::Color32 {
    match grade {
        HitGrade::Perfect => tokens::GRADE_PERFECT,
        HitGrade::Good => tokens::GRADE_GOOD,
        HitGrade::Off => tokens::GRADE_OFF,
    }
    .get(dark)
}

/// The note's zero line and the connector to the hit.
pub(crate) fn reference_color(dark: bool) -> egui::Color32 {
    tokens::GRADE_REFERENCE.get(dark)
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

    fn opts(em: f32, rect: Rect) -> StaffOpts {
        StaffOpts {
            rect,
            font_id: FontId::new(em, egui::FontFamily::Proportional),
            pixels_per_point: 1.0,
            em,
            y_offset: 0.0,
            stem_length_factor: 3.5,
            stem_thickness_factor: 0.04,
            accent_displacement: 0.07,
            min_measure_width_em: 6.0,
            note_width_em: 0.6,
            system_spacing_em: 0.2,
            layout_clef_first: false,
            time_signature_first: false,
            room_below_em: ROOM_BELOW_EM,
            text_rows: Default::default(),
            bar_rests: false,
            veil_hidden_rests: false,
        }
    }

    /// `n` bars of 4/4 quarters.
    fn bars(n: usize) -> Score {
        use grooph_measure::duration::q;
        use grooph_measure::{Beat, Measure};
        let bar = || {
            let mut m = Measure::new(TimeSignature::FOUR_FOUR);
            for i in 0..4 {
                m.first_voice_mut().set_beat(i, Beat::note(q())).unwrap();
            }
            m
        };
        Score::new((0..n).map(|_| bar()).collect())
    }

    fn wide() -> Rect {
        Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 1.0e4))
    }

    #[test]
    fn a_row_taller_than_the_view_shrinks_to_fit_it() {
        let score = bars(1);
        let (roomy, staff) = fit_to_height(&score, opts(40.0, wide()), 1.0e4);
        assert_eq!(roomy.em, 40.0, "a row that fits keeps its size");
        let row = staff.room.height() * 40.0;

        let (fitted, _) = fit_to_height(&score, opts(40.0, wide()), row / 2.0);
        assert!((fitted.em - 20.0).abs() < 1e-3, "{}", fitted.em);
        assert!((fitted.font_id.size - 20.0).abs() < 1e-3, "the font shrinks with it");

        // Not below the legibility floor, however short the view.
        let (floor, _) = fit_to_height(&score, opts(40.0, wide()), 1.0);
        assert!((floor.em - LEGIBILITY_FLOOR_EM).abs() < 1e-3, "{}", floor.em);
        // Nor a staff already smaller than the floor.
        let (small, _) = fit_to_height(&score, opts(LEGIBILITY_FLOOR_EM / 2.0, wide()), 1.0);
        assert_eq!(small.em, LEGIBILITY_FLOOR_EM / 2.0);
        // An empty view (before the first layout) changes nothing.
        let (empty, _) = fit_to_height(&score, opts(40.0, wide()), 0.0);
        assert_eq!(empty.em, 40.0);
    }

    /// Rows 70 high, 80 apart, starting at the content's top (`origin`).
    fn row(origin: egui::Pos2, idx: usize) -> Rect {
        let top = origin.y + 80.0 * idx as f32;
        Rect::from_min_max(egui::pos2(0.0, top), egui::pos2(400.0, top + 70.0))
    }

    /// The view shows `height` of the content, scrolled down by `offset`.
    fn view(offset: f32, height: f32) -> ScrollView {
        let clip = Rect::from_min_size(egui::pos2(0.0, 100.0), egui::vec2(400.0, height));
        ScrollView { origin: clip.min - egui::vec2(0.0, offset), clip }
    }

    const FOUR_ROWS: f32 = 310.0;

    #[test]
    fn a_playing_row_in_view_does_not_scroll() {
        let v = view(30.0, 200.0);
        // Rows 1 (80..150) and 0 at the top as well, scrolled partly away.
        assert_eq!(playing_row_offset(row(v.origin, 1), FOUR_ROWS, v), 30.0);
        let v = view(0.0, 200.0);
        assert_eq!(playing_row_offset(row(v.origin, 0), FOUR_ROWS, v), 0.0);
    }

    #[test]
    fn a_playing_row_wrapping_to_the_next_is_centred() {
        // Row 0 is in view; row 1 (80..150) is not whole in it.
        let v = view(0.0, 100.0);
        assert_eq!(playing_row_offset(row(v.origin, 0), FOUR_ROWS, v), 0.0);
        assert_eq!(playing_row_offset(row(v.origin, 1), FOUR_ROWS, v), 65.0);
        // The last row's centre would scroll past the end: as far as it goes.
        let v = view(65.0, 100.0);
        assert_eq!(playing_row_offset(row(v.origin, 3), FOUR_ROWS, v), FOUR_ROWS - 100.0);
        // Back to the top on a loop.
        assert_eq!(playing_row_offset(row(v.origin, 0), FOUR_ROWS, v), 0.0);
    }

    /// Four bars a row each.
    fn four_rows() -> Rc<StaffLayout> {
        let narrow = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 1.0e4));
        crate::staff_cache::staff_layout(&bars(4), &opts(30.0, narrow))
    }

    fn followed(cursor: Cursor, view_height: f32) -> EditFollow {
        EditFollow { cursor: Some(cursor), view_height }
    }

    #[test]
    fn the_view_follows_the_edit_cursor_off_screen() {
        let staff = four_rows();
        let (first, last) = (Cursor::in_voice(0, 0, 0), Cursor::in_voice(3, 0, 2));
        let bar = staff.placed(3).unwrap();
        assert!(bar.rect.top() > 100.0, "below a view 100 high");

        let target =
            edit_cursor_target(&staff, last, followed(first, 100.0), followed(last, 100.0));
        let note = &bar.voices[0].notes[2];
        assert_eq!(
            target,
            Some(Rect::from_min_size(
                egui::pos2(note.center.x, bar.rect.top()),
                egui::vec2(1.0, bar.rect.height()),
            ))
        );
        // Once followed, the user may scroll away from it.
        assert_eq!(
            edit_cursor_target(&staff, last, followed(last, 100.0), followed(last, 100.0)),
            None
        );
    }

    #[test]
    fn a_resize_brings_the_edit_cursor_back_into_view() {
        let staff = four_rows();
        let cursor = Cursor::in_voice(1, 0, 0);
        let bar = staff.placed(1).unwrap();
        // The tool grid opens under the view.
        let target =
            edit_cursor_target(&staff, cursor, followed(cursor, 300.0), followed(cursor, 200.0));
        assert_eq!(
            target.map(|r| (r.top(), r.height())),
            Some((bar.rect.top(), bar.rect.height()))
        );
        // Less than a pixel is no resize.
        let target =
            edit_cursor_target(&staff, cursor, followed(cursor, 300.0), followed(cursor, 300.3));
        assert_eq!(target, None);
        // Nor while not editing.
        let still = EditFollow { cursor: None, view_height: 200.0 };
        let target =
            edit_cursor_target(&staff, cursor, EditFollow { view_height: 300.0, ..still }, still);
        assert_eq!(target, None);
    }
}
