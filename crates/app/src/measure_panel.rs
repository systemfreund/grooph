use crate::Grooph;
use crate::accuracy::{AccuracyMark, AccuracyTracker};
use crate::tools::{Modifier, ToolKind, all_tools};
use crate::{Mode, TransportState};
use eframe::egui;
use eframe::egui::{FontId, Frame, Rect, Response, Stroke};
use grooph_layout::pixel_layout::{GlyphMetrics, MeasureLayout, compute_em};
use grooph_layout::staff_layout::{PlacedMeasure, StaffLayout, StaffOpts, build_staff_layout};
use grooph_measure::grid::DEFAULT_GRID;
use grooph_measure::tempo::ScoreTiming;
use grooph_render::staff::draw_staff;

impl Grooph {
    pub(super) fn measure_panel(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            Frame::canvas(ui.style())
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::NONE)
                .show(ui, |ui| {
                    egui::ScrollArea::horizontal().show(ui, |ui| {
                    let available = ui.available_size();
                    let origin = ui.cursor().min;
                    let viewport_rect = Rect::from_min_size(origin, available);

                    // Build the multi-measure tempo backbone once per frame.
                    // O(score.len()) — cheap for realistic score sizes.
                    let timing = ScoreTiming::from_score(&self.score, self.bpm);

                    // Update playback smoothing & primary-beat flash state
                    let playback_tick_to_draw = match self.transport_state {
                        TransportState::Playing => {
                            let now = ui.input(|i| i.time);
                            let last = self.playback.last_update.unwrap_or(now);
                            let dt = now - last;
                            self.playback.last_update = Some(now);

                            let total = timing.total_loop_ticks() as f64;

                            // Advance predictor at the current measure's rate.
                            // smooth_tick is now a global tick across the whole loop.
                            let current_m =
                                timing.measure_at_global_tick(self.playback.smooth_tick);
                            let current_tps = timing.ticks_per_sec_in_measure(current_m);
                            let mut next_tick =
                                self.playback.smooth_tick + current_tps * dt;

                            // Sync with audio if available
                            if let Some(audio) = &self.audio
                                && let Some((raw_audio_tick, audio_total)) =
                                    audio.playback_position()
                            {
                                let audio_total_f = audio_total as f64;
                                if audio_total_f > 0.0 {
                                    // Adjust for user-configured audio offset (latency).
                                    // Use the playing measure's rate (small offset, sub-tick
                                    // approximation acceptable across TS boundaries).
                                    let audio_m =
                                        timing.measure_at_global_tick(raw_audio_tick);
                                    let audio_tps =
                                        timing.ticks_per_sec_in_measure(audio_m);
                                    let offset_ticks = if self.audio_cfg.latency_enabled {
                                        self.audio_cfg.offset as f64 * audio_tps
                                    } else {
                                        0.0
                                    };
                                    let audio_tick =
                                        (raw_audio_tick - offset_ticks).rem_euclid(audio_total_f);

                                    let mut diff = audio_tick - next_tick;
                                    // Handle wrap-around (shortest path)
                                    if diff > audio_total_f * 0.5 {
                                        diff -= audio_total_f;
                                    } else if diff < -audio_total_f * 0.5 {
                                        diff += audio_total_f;
                                    }

                                    // Snap if far off, else smooth nudge. "Far" uses the
                                    // playing measure's ticks_per_beat as a unit.
                                    let tpb =
                                        timing.ticks_per_beat_in_measure(audio_m) as f64;
                                    if diff.abs() > tpb * 0.5 {
                                        next_tick = audio_tick;
                                    } else {
                                        next_tick += diff * 0.1;
                                    }
                                }
                            }

                            // Wrap at score end (not at any measure boundary).
                            if total > 0.0 {
                                next_tick = next_tick.rem_euclid(total);
                            }
                            self.playback.smooth_tick = next_tick;

                            // Flash: trigger on primary-beat change in the currently
                            // playing measure. Key is (measure_idx, primary_beat_in_measure).
                            let (m_idx, local_tick) = timing.to_local(next_tick);
                            let tpb_m = timing.ticks_per_beat_in_measure(m_idx) as f64;
                            let primary_beat_in_measure = if tpb_m > 0.0 {
                                (local_tick / tpb_m).floor() as u32
                            } else {
                                0
                            };
                            let key = (m_idx, primary_beat_in_measure);
                            if self.playback.last_primary_beat != Some(key) {
                                self.playback.flash_intensity = 1.0;
                                self.playback.last_primary_beat = Some(key);
                            }

                            // Exponential decay towards 0
                            let decay_per_sec = 10.0; // larger -> faster fade
                            let decay = (-decay_per_sec * dt).exp();
                            self.playback.flash_intensity *= decay as f32;

                            // Keep animation loop running
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));

                            Some(self.playback.smooth_tick)
                        }
                        TransportState::Stopped => {
                            self.playback.reset();
                            None
                        }
                    };

                    let em = compute_em(&viewport_rect, self.layout.width_cap_factor, ui);
                    let font_id = FontId::new(em, self.music_font_id.family.clone());

                    let metrics = GlyphMetrics::measure(ui, &font_id);

                    let staff_opts = StaffOpts {
                        rect: viewport_rect,
                        font_id: font_id.clone(),
                        pixels_per_point: ui.ctx().pixels_per_point(),
                        em,
                        y_offset: 0.0,
                        stem_length_factor: self.layout.stem_length_factor,
                        stem_thickness_factor: 0.04,
                        accent_displacement: 0.07,
                        accent_below: self.layout.accent_below,
                        proportional_spacing: self.layout.proportional_spacing,
                        debug_bbox: self.layout.debug_bbox,
                        metrics,
                        min_measure_width_em: 6.0,
                        note_width_em: 0.6,
                        system_spacing_em: 4.0,
                        layout_clef_first: true,
                    };

                    let staff = build_staff_layout(&self.score, &staff_opts);

                    // From the global tick, derive (playing_measure_idx, local_tick).
                    // The playback cursor renders / auto-scrolls based on this — it
                    // wanders through all measures, independent of cursor.measure_idx.
                    let playback_local =
                        playback_tick_to_draw.map(|global| timing.to_local(global));

                    // Auto-scroll: keep the playback cursor centered in the viewport.
                    // Mirror the renderer's cross-measure anchor so the auto-scroll
                    // target matches what's drawn (especially during the last note
                    // of a measure, when the cursor glides into the next one).
                    if let Some((play_m, local_t)) = playback_local
                        && let Some(placed) = staff.placed(play_m)
                    {
                        let next_anchor_x = staff
                            .systems
                            .iter()
                            .flat_map(|s| s.measures.iter())
                            .find(|p| p.measure_idx == play_m + 1)
                            .and_then(|p| p.layout.notes.first().map(|n| n.center.x));

                        if let Some(x) = grooph_render::measure::playback_cursor_x(
                            &self.score.measures[placed.measure_idx],
                            &placed.layout,
                            placed.rect,
                            local_t,
                            next_anchor_x,
                        ) {
                            let cursor_rect = egui::Rect::from_min_size(
                                egui::pos2(x, placed.rect.top()),
                                egui::vec2(1.0, placed.rect.height()),
                            );
                            ui.scroll_to_rect_animation(
                                cursor_rect,
                                Some(egui::Align::Center),
                                eframe::egui::style::ScrollAnimation::none(),
                            );
                        }
                    }

                    let (rect, resp) = ui.allocate_exact_size(
                        staff.total_size,
                        egui::Sense::click_and_drag(),
                    );

                    let count_config = self.build_count_config();
                    let cursor = if self.mode == Mode::Edit { Some(self.cursor) } else { None };
                    let playback = playback_local;

                    draw_staff(
                        ui,
                        &self.score,
                        &staff,
                        cursor,
                        playback,
                        count_config.as_ref(),
                        &staff_opts,
                    );

                    // Draw flash overlay (white on dark, black on light) with decay
                    if self.playback.flash_intensity > 0.01 {
                        let dark = ui.visuals().dark_mode;
                        let base = if dark { egui::Color32::GREEN } else { egui::Color32::BLUE };
                        let alpha = (0.8 * self.playback.flash_intensity).clamp(0.0, 0.8);
                        let color = egui::Color32::from_rgba_unmultiplied(
                            base.r(),
                            base.g(),
                            base.b(),
                            (alpha * 255.0) as u8,
                        );
                        ui.painter().rect_filled(rect, 0.0, color);
                        ui.ctx().request_repaint();
                    }

                    // TODO(midi-multi-measure): markers are drawn only for the
                    // active (cursor-selected) measure. When MIDI becomes
                    // multi-measure, iterate over *all* PlacedMeasures and
                    // look up marks via the *global* onset tick
                    // (`timing.measure_start_tick(placed.measure_idx) + onsets[i]`).
                    if let Some(active) = staff.placed(self.cursor.measure_idx) {
                        self.draw_accuracy_marker(ui, active, &staff_opts.metrics);
                    }
                    self.handle_input(resp, &staff);
                    });
                });
        });
    }

    fn draw_accuracy_marker(
        &self,
        ui: &egui::Ui,
        placed: &PlacedMeasure,
        metrics: &GlyphMetrics,
    ) {
        if !self.accuracy.enabled {
            return;
        }
        let Some(midi_input) = self.midi.input.as_ref() else {
            return;
        };
        if !midi_input.is_connected() {
            return;
        }
        if self.transport_state != TransportState::Playing {
            return;
        }
        let measure = &self.score.measures[placed.measure_idx];
        let beats = measure.beats();
        if beats.is_empty() {
            return;
        }
        let onsets = DEFAULT_GRID.compute_onset_ticks(beats);
        let total_ticks = DEFAULT_GRID.ticks_per_measure(&measure.time_signature()) as f64;
        if total_ticks <= 0.0 {
            return;
        }
        let layout = &placed.layout;
        for (idx, note_layout) in layout.notes.iter().enumerate() {
            if note_layout.kind != grooph_measure::BeatKind::Note {
                continue;
            }
            let y = note_layout.center.y + metrics.head_size.y * 0.65;
            let h = metrics.head_size.y * 0.8;
            let zero_start = egui::Pos2::new(note_layout.center.x, y);
            let zero_end = egui::Pos2::new(note_layout.center.x, y + h);
            ui.painter().line_segment(
                [zero_start, zero_end],
                Stroke::new(2.0, egui::Color32::from_gray(140)),
            );
            let Some(onset_tick) = onsets.get(idx) else {
                continue;
            };
            let Some(mark) = self.accuracy.tracker.mark_for_onset(*onset_tick) else {
                continue;
            };
            match mark {
                AccuracyMark::Hit(diff_ticks) => {
                    let diff_ticks = AccuracyTracker::clamp_diff_to_beat_window(
                        diff_ticks,
                        idx,
                        &onsets,
                        total_ticks,
                    );
                    let Some(px_per_tick) = self.local_pixels_per_tick(&onsets, layout, idx) else {
                        continue;
                    };
                    let x = note_layout.center.x + (diff_ticks * px_per_tick) as f32;
                    let mid_y = y + h * 0.5;
                    let h_start = egui::Pos2::new(note_layout.center.x, mid_y);
                    let h_end = egui::Pos2::new(x, mid_y);
                    ui.painter().line_segment(
                        [h_start, h_end],
                        Stroke::new(2.0, egui::Color32::from_gray(140)),
                    );
                    let start = egui::Pos2::new(x, y);
                    let end = egui::Pos2::new(x, y + h);
                    ui.painter().line_segment([start, end], Stroke::new(2.0, egui::Color32::RED));
                }
                AccuracyMark::Miss => {
                    let mid_y = y + h * 0.5;
                    let half = h * 0.25;
                    let left = note_layout.center.x - half;
                    let right = note_layout.center.x + half;
                    let top = mid_y - half;
                    let bottom = mid_y + half;
                    let stroke = Stroke::new(2.0, egui::Color32::RED);
                    ui.painter().line_segment(
                        [egui::Pos2::new(left, top), egui::Pos2::new(right, bottom)],
                        stroke,
                    );
                    ui.painter().line_segment(
                        [egui::Pos2::new(left, bottom), egui::Pos2::new(right, top)],
                        stroke,
                    );
                }
            }
        }
    }

    fn local_pixels_per_tick(
        &self,
        onsets: &[u32],
        layout: &MeasureLayout,
        idx: usize,
    ) -> Option<f64> {
        if idx + 1 < onsets.len() {
            let dt = (onsets[idx + 1] as i64 - onsets[idx] as i64) as f64;
            if dt != 0.0 {
                let x0 = layout.notes.get(idx)?.center.x as f64;
                let x1 = layout.notes.get(idx + 1)?.center.x as f64;
                return Some((x1 - x0) / dt);
            }
        }
        if idx > 0 {
            let dt = (onsets[idx] as i64 - onsets[idx - 1] as i64) as f64;
            if dt != 0.0 {
                let x0 = layout.notes.get(idx - 1)?.center.x as f64;
                let x1 = layout.notes.get(idx)?.center.x as f64;
                return Some((x1 - x0) / dt);
            }
        }
        None
    }

    fn handle_input(&mut self, resp: Response, staff: &StaffLayout) {
        if !matches!(self.mode, Mode::TimeSignature { .. })
            && (resp.clicked() || resp.dragged())
            && let Some(pos) = resp.interact_pointer_pos()
            && let Some((m_idx, b_idx)) =
                grooph_layout::staff_layout::hit_test_staff(staff, pos.x)
        {
            self.cursor.measure_idx = m_idx;
            self.cursor.beat_idx = b_idx;

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
