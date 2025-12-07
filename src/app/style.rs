use crate::Grooph;
use eframe::egui::{Margin, Vec2};
use eframe::egui;

impl Grooph {
    pub(super) fn apply_style(&mut self, ctx: &egui::Context) {
        // Ensure the font-size bump applies for both dark and light themes by reapplying
        // an idempotent adjustment relative to each theme's baseline sizes.
        let is_dark = ctx.style().visuals.dark_mode;
        ctx.style_mut(|style| {
            // Capture baseline for current theme if not yet recorded
            if is_dark {
                if self.baseline_dark.is_none() {
                    let mut v = Vec::new();
                    for (ts, font) in style.text_styles.iter() {
                        v.push((ts.clone(), font.size));
                    }
                    self.baseline_dark = Some(v);
                }
                if let Some(base) = &self.baseline_dark {
                    for (ts, font) in style.text_styles.iter_mut() {
                        if let Some((_, sz)) = base.iter().find(|(t, _)| t == ts) {
                            font.size = *sz + self.font_bump;
                        }
                    }
                }
            } else {
                if self.baseline_light.is_none() {
                    let mut v = Vec::new();
                    for (ts, font) in style.text_styles.iter() {
                        v.push((ts.clone(), font.size));
                    }
                    self.baseline_light = Some(v);
                }
                if let Some(base) = &self.baseline_light {
                    for (ts, font) in style.text_styles.iter_mut() {
                        if let Some((_, sz)) = base.iter().find(|(t, _)| t == ts) {
                            font.size = *sz + self.font_bump;
                        }
                    }
                }
            }
        });

        // Global UI tweaks: increase button paddings across the app
        ctx.style_mut(|style| {
            style.spacing.button_padding = Vec2::new(15.0, 15.0);
            style.spacing.window_margin = Margin::same(10);
        });

    }
}
