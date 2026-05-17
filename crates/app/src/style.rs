use crate::Grooph;
use eframe::egui;
use eframe::egui::{Margin, Vec2};

impl Grooph {
    pub(super) fn apply_style(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx();
        let is_dark = ctx.global_style().visuals.dark_mode;
        ctx.global_style_mut(|style| {
            let baseline_opt = if is_dark {
                &mut self.ui.baseline_dark
            } else {
                &mut self.ui.baseline_light
            };
            let font_bump = self.ui.font_bump;

            // Capture baseline for the current theme if not yet recorded
            let baseline = baseline_opt.get_or_insert_with(|| {
                style
                    .text_styles
                    .iter()
                    .map(|(ts, font)| (ts.clone(), font.size + font_bump))
                    .collect()
            });

            // Apply font bump to current style based on recorded baseline
            for (ts, font) in style.text_styles.iter_mut() {
                if let Some((_, base_sz)) = baseline.iter().find(|(t, _)| t == ts) {
                    font.size = *base_sz;
                }
            }

            style.spacing.button_padding = Vec2::new(15.0, 15.0);
            style.spacing.window_margin = Margin::same(10);
        });
    }
}
