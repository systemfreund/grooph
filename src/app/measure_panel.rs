use crate::Grooph;
use crate::app::tools::{Modifier, ToolKind, all_tools};
use crate::app::{Mode, PlayerState};
use crate::layout::pixel_layout::{LayoutOpts, compute_em};
use crate::measure::grid::DEFAULT_GRID;
use crate::render::measure::draw_measure;
use eframe::egui;
use eframe::egui::{Context, FontId, Frame};

impl Grooph {
    pub(super) fn measure_panel(&mut self, ctx: &Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            Frame::canvas(ui.style())
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::NONE)
                .show(ui, |ui| {
                    let size = ui.available_size();
                    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());

                    // Update playback smoothing
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
                                && let Some((audio_tick, audio_total)) = audio.playback_position()
                            {
                                let total = audio_total as f64;
                                if total > 0.0 {
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

                            // Keep animation loop running
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));

                            Some(self.playback_smooth_tick)
                        }
                        PlayerState::Stopped => {
                            self.playback_last_update = None;
                            self.playback_smooth_tick = 0.0;
                            None
                        }
                    };

                    let em = compute_em(&rect, 0.1, ui);
                    let font_id = FontId::new(em, self.music_font_id.family.clone());
                    let opts = LayoutOpts {
                        rect,
                        font_id: font_id.clone(),
                        pixels_per_point: ui.ctx().pixels_per_point(),
                        em,
                        layout_clef: true,
                        layout_time_signature: true,
                        y_offset: 0.0,
                        stem_length_factor: 0.9,
                        stem_thickness_factor: 0.04,
                        accent_displacement: 0.5,
                        accent_below: true,
                    };

                    let layout = draw_measure(
                        ui,
                        &self.measure,
                        &opts,
                        if self.mode == Mode::Edit { Some(self.cursor_idx) } else { None },
                        playback_tick_to_draw,
                    );

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
                            && let Some(tool) = all_tools().iter().find(|t| {
                                matches!(t.kind, ToolKind::Modify(Modifier::ToggleRestNote))
                            })
                        {
                            self.apply_tool(tool);
                        }
                    }
                });
        });
    }
}
