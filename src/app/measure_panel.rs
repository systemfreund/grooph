use crate::Grooph;
use crate::app::tools::{Modifier, ToolKind, all_tools};
use crate::app::{Mode, PlayerState};
use crate::layout::pixel_layout::{LayoutOpts, MeasureLayout, compute_em, GlyphMetrics};
use crate::measure::grid::DEFAULT_GRID;
use crate::render::glyphs;
use crate::render::measure::draw_measure;
use eframe::egui;
use eframe::egui::{Context, FontId, Frame, Rect, Response};

impl Grooph {
    pub(super) fn measure_panel(&mut self, ctx: &Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            Frame::canvas(ui.style())
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::NONE)
                .show(ui, |ui| {
                    let size = ui.available_size();
                    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());

                    // Update playback smoothing & primary-beat flash state
                    let playback_tick_to_draw = match self.player_state {
                        PlayerState::Playing => {
                            let ts = self.measure.time_signature();
                            let ticks_per_measure = DEFAULT_GRID.ticks_per_measure(&ts) as f64;
                            let ticks_per_beat = DEFAULT_GRID.ticks_per_beat(&ts) as f64;
                            let ticks_per_sec = (self.bpm as f64 / 60.0) * ticks_per_beat;

                            let now = ui.input(|i| i.time);
                            let last = self.playback_last_update.unwrap_or(now);
                            let dt = now - last;
                            self.playback_last_update = Some(now);

                            // Advance predictor
                            let mut next_tick = self.playback_smooth_tick + ticks_per_sec * dt;

                            // Sync with audio if available
                            if let Some(audio) = &self.audio
                                && let Some((raw_audio_tick, audio_total)) =
                                    audio.playback_position()
                            {
                                let total = audio_total as f64;
                                if total > 0.0 {
                                    // Adjust for user-configured audio offset (latency) if enabled
                                    let offset_ticks = if self.audio_latency_enabled {
                                        self.audio_offset as f64 * ticks_per_sec
                                    } else {
                                        0.0
                                    };
                                    let audio_tick =
                                        (raw_audio_tick - offset_ticks).rem_euclid(total);

                                    let mut diff = audio_tick - next_tick;
                                    // Handle wrap-around (shortest path)
                                    if diff > total * 0.5 {
                                        diff -= total;
                                    } else if diff < -total * 0.5 {
                                        diff += total;
                                    }

                                    // Snap if far off (e.g. startup/seek), else smooth nudge
                                    if diff.abs() > ticks_per_beat * 0.5 {
                                        next_tick = audio_tick;
                                    } else {
                                        next_tick += diff * 0.1;
                                    }
                                }
                            }

                            // Wrap
                            if ticks_per_measure > 0.0 {
                                next_tick = next_tick.rem_euclid(ticks_per_measure);
                            }
                            self.playback_smooth_tick = next_tick;

                            // Flash: trigger on primary beat change
                            let current_primary_beat = (next_tick / ticks_per_beat).floor() as u32;
                            if self.last_primary_beat != Some(current_primary_beat) {
                                self.flash_intensity = 1.0;
                                self.last_primary_beat = Some(current_primary_beat);
                            }

                            // Exponential decay towards 0
                            let decay_per_sec = 10.0; // larger -> faster fade
                            let decay = (-decay_per_sec * dt).exp();
                            self.flash_intensity *= decay as f32;

                            // Keep animation loop running
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));

                            Some(self.playback_smooth_tick)
                        }
                        PlayerState::Stopped => {
                            self.playback_last_update = None;
                            self.playback_smooth_tick = 0.0;
                            self.flash_intensity = 0.0;
                            self.last_primary_beat = None;
                            None
                        }
                    };

                    let em = compute_em(&rect, self.layout_width_cap_factor, ui);
                    let font_id = FontId::new(em, self.music_font_id.family.clone());

                    let metrics = GlyphMetrics::measure(ui, &font_id);

                    let opts = LayoutOpts {
                        rect,
                        font_id: font_id.clone(),
                        pixels_per_point: ui.ctx().pixels_per_point(),
                        em,
                        layout_clef: true,
                        layout_time_signature: true,
                        y_offset: 0.0,
                        stem_length_factor: self.layout_stem_length_factor,
                        stem_thickness_factor: 0.04,
                        accent_displacement: 0.07,
                        accent_below: self.layout_accent_below,
                        proportional_spacing: self.layout_proportional_spacing,
                        debug_bbox: self.layout_debug_bbox,
                        metrics,
                    };

                    let count_config = self.build_count_config();
                    let layout = draw_measure(
                        ui,
                        &self.measure,
                        &opts,
                        if self.mode == Mode::Edit { Some(self.cursor_idx) } else { None },
                        playback_tick_to_draw,
                        count_config.as_ref(),
                    );

                    // Draw flash overlay (white on dark, black on light) with decay
                    if self.flash_intensity > 0.01 {
                        let dark = ui.visuals().dark_mode;
                        let base = if dark { egui::Color32::GREEN } else { egui::Color32::BLUE };
                        let alpha = (0.8 * self.flash_intensity).clamp(0.0, 0.8);
                        let color = egui::Color32::from_rgba_unmultiplied(
                            base.r(),
                            base.g(),
                            base.b(),
                            (alpha * 255.0) as u8,
                        );
                        ui.painter().rect_filled(rect, 0.0, color);
                        ui.ctx().request_repaint();
                    }

                    self.handle_input(rect, resp, layout);
                });
        });
    }

    fn handle_input(&mut self, rect: Rect, resp: Response, layout: MeasureLayout) {
        if !matches!(self.mode, Mode::TimeSignature { .. })
            && (resp.clicked() || resp.dragged())
            && let Some(pos) = resp.interact_pointer_pos()
            && !layout.notes.is_empty()
        {
            let target_x = pos.x;
            let idx = if target_x <= rect.left() {
                0
            } else if target_x >= rect.right() {
                layout.notes.len() - 1
            } else {
                let mut best_i = 0usize;
                let mut best_d = f32::MAX;
                for (i, nl) in layout.notes.iter().enumerate() {
                    let d = (nl.center.x - target_x).abs();
                    if d < best_d {
                        best_d = d;
                        best_i = i;
                    }
                }
                best_i
            };
            self.cursor_idx = idx;

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
