use crate::Grooph;
use eframe::egui;
use eframe::egui::{Color32, Margin, Vec2, Visuals};

/// Text that reads at WCAG AA (4.5:1) on cards and panels: egui's dark text
/// (3.8:1 on a card) and its weak text in both themes (2.3 to 2.9:1) are
/// too faint.
pub(crate) fn readable_text(visuals: &mut Visuals) {
    let (text, weak) = if visuals.dark_mode {
        (Some(Color32::from_gray(200)), Color32::from_gray(170))
    } else {
        // egui's light text (gray 80, 6.5:1) is fine.
        (None, Color32::from_gray(95))
    };
    if let Some(text) = text {
        let widgets = &mut visuals.widgets;
        for widget in [&mut widgets.noninteractive, &mut widgets.inactive] {
            widget.fg_stroke.color = text;
        }
    }
    visuals.weak_text_color = Some(weak);
}

impl Grooph {
    pub(super) fn apply_style(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx();
        let is_dark = ctx.global_style().visuals.dark_mode;
        ctx.global_style_mut(|style| {
            let baseline_opt =
                if is_dark { &mut self.ui.baseline_dark } else { &mut self.ui.baseline_light };
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

            readable_text(&mut style.visuals);
            // Embedded in a chat: its card colors (see `embed::HostColors`).
            if let Some(embed) = &self.ui.embed {
                embed.colors.apply(&mut style.visuals);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::readable_text;
    use eframe::egui::{Color32, Visuals};

    /// WCAG contrast ratio of two opaque colors.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let luminance = |c: Color32| {
            let channel = |v: u8| {
                let v = f32::from(v) / 255.0;
                if v <= 0.039_28 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
        };
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_reads_at_wcag_aa_on_cards_and_panels() {
        for mut visuals in [Visuals::light(), Visuals::dark()] {
            readable_text(&mut visuals);
            let card = visuals.widgets.inactive.weak_bg_fill;
            for background in [card, visuals.panel_fill] {
                for text in [visuals.text_color(), visuals.weak_text_color()] {
                    let ratio = contrast(text, background);
                    assert!(ratio >= 4.5, "{text:?} on {background:?}: {ratio:.2}");
                }
            }
            // Weak text still reads weaker than the rest.
            assert!(
                contrast(visuals.weak_text_color(), card) < contrast(visuals.text_color(), card)
            );
        }
    }
}
