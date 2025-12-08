use crate::Grooph;
use crate::app::PlayerState;
use crate::measure::grid::DEFAULT_GRID;
use crate::render::measure::draw_measure;
use eframe::egui;
use eframe::egui::{Context, Frame};

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
                        PlayerState::Paused => {
                            // Paused: keep the last known playback position visible.
                            // Do not advance the predictor; just stop the clock and draw the last value.
                            self.playback_last_update = None;
                            Some(self.playback_smooth_tick)
                        }
                        PlayerState::Stopped => {
                            self.playback_last_update = None;
                            None
                        }
                    };

                    let layout = draw_measure(
                        ui,
                        &self.music_font_id,
                        &self.measure,
                        rect,
                        if self.edit_mode_enabled { Some(self.cursor_idx) } else { None },
                        playback_tick_to_draw,
                    );

                    // Block canvas interactions while the time signature dialog is open
                    if !self.show_ts_dialog
                        && (resp.clicked() || resp.dragged())
                        && let Some(pos) = resp.interact_pointer_pos()
                    {
                        // Falls keine Beats vorhanden sind, nichts tun
                        if !layout.notes.is_empty() {
                            // Außerhalb des Inhalts: zum nächstliegenden Rand clampen
                            let target_x = pos.x;
                            let idx = if target_x <= rect.left() {
                                0
                            } else if target_x >= rect.right() {
                                layout.notes.len() - 1
                            } else {
                                // Innerhalb: Index des nächstgelegenen x-Centers suchen
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
                        }
                    }
                });
        });
    }
}
