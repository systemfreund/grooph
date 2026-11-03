//! The app's own colours, named as in the design system's
//! `docs/design-system/tokens.json`. That file's other colours are egui's
//! (its light and dark `Visuals`, after [`crate::style::readable_text`]),
//! grooph-render's highlight, the guide pages' `site.css` and the app icon's;
//! a test keeps the file in step with all of them.

use eframe::egui::Color32;

/// A colour with a value for the light and one for the dark theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Themed {
    pub(crate) light: Color32,
    pub(crate) dark: Color32,
}

impl Themed {
    const fn same(color: Color32) -> Self {
        Self { light: color, dark: color }
    }

    pub(crate) fn get(self, dark: bool) -> Color32 {
        if dark { self.dark } else { self.light }
    }
}

/// Body text and row labels. egui's dark text (3.8:1 on a card) is too faint;
/// its light text (gray 80, 6.5:1) is fine.
pub(crate) const TEXT: Themed =
    Themed { light: Color32::from_gray(80), dark: Color32::from_gray(200) };
/// Text of a resting button: egui's in light, lifted like [`TEXT`] in dark.
pub(crate) const TEXT_BUTTON: Themed =
    Themed { light: Color32::from_gray(60), dark: Color32::from_gray(200) };
/// Weak text: egui's own (2.3 to 2.9:1) is too faint in both themes.
pub(crate) const TEXT_WEAK: Themed =
    Themed { light: Color32::from_gray(95), dark: Color32::from_gray(170) };

/// A badge such as "BETA", in a settings row or on a tab: orange, the same in
/// both themes.
pub(crate) const BADGE: Themed = Themed::same(Color32::from_rgb(0xD9, 0x6B, 0x1E));
pub(crate) const ON_BADGE: Themed = Themed::same(Color32::WHITE);

/// Track of a switch that is off. It has to differ from the white knob, so
/// the light theme gets a grey instead of its white field colour.
pub(crate) const SWITCH_OFF: Themed =
    Themed { light: Color32::from_gray(190), dark: Color32::from_gray(10) };
pub(crate) const SWITCH_KNOB: Themed = Themed::same(Color32::WHITE);

/// The metronome ring. The idle segments sit between the background and the
/// lit one, and the lit one is a saturated blue, so the click being played
/// stands out by lightness and by hue.
pub(crate) const RING_IDLE: Themed =
    Themed { light: Color32::from_gray(175), dark: Color32::from_gray(90) };
pub(crate) const RING_LIT: Themed =
    Themed { light: Color32::from_rgb(25, 118, 210), dark: Color32::from_rgb(100, 175, 255) };

/// Play-along marks of a grade (misses use `Off`): blue, orange/yellow and
/// purple stay apart for red-green and blue-yellow colour vision deficiencies
/// and differ in lightness. No red/green pair. The grade is shown by shape as
/// well (off hits get an arrowhead, misses a cross), so colour is never the
/// only cue.
pub(crate) const GRADE_PERFECT: Themed =
    Themed { light: Color32::from_rgb(51, 102, 204), dark: Color32::from_rgb(102, 204, 238) };
pub(crate) const GRADE_GOOD: Themed =
    Themed { light: Color32::from_rgb(215, 140, 0), dark: Color32::from_rgb(240, 228, 66) };
pub(crate) const GRADE_OFF: Themed =
    Themed { light: Color32::from_rgb(153, 0, 102), dark: Color32::from_rgb(200, 80, 255) };
/// A note's zero line and the connector from it to the hit.
pub(crate) const GRADE_REFERENCE: Themed =
    Themed { light: Color32::from_gray(120), dark: Color32::from_gray(150) };

/// The flash over the staff on each click, drawn see-through.
pub(crate) const FLASH: Themed = Themed { light: Color32::BLUE, dark: Color32::GREEN };

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::readable_text;
    use eframe::egui::Visuals;
    use serde_json::Value;
    use std::collections::BTreeMap;

    const TOKENS_JSON: &str = include_str!("../../../docs/design-system/tokens.json");
    const SITE_CSS: &str = include_str!("../../site/src/site.css");

    /// The tokens defined in this module, by their name in `tokens.json`.
    const OWN: [(&str, Themed); 14] = [
        ("text", TEXT),
        ("text-button", TEXT_BUTTON),
        ("text-weak", TEXT_WEAK),
        ("badge", BADGE),
        ("on-badge", ON_BADGE),
        ("switch-off", SWITCH_OFF),
        ("switch-knob", SWITCH_KNOB),
        ("ring-idle", RING_IDLE),
        ("ring-lit", RING_LIT),
        ("grade-perfect", GRADE_PERFECT),
        ("grade-good", GRADE_GOOD),
        ("grade-off", GRADE_OFF),
        ("grade-reference", GRADE_REFERENCE),
        ("flash", FLASH),
    ];

    /// Taken from the app icon; the app code does not draw them.
    const ICON: [&str; 3] = ["brand-blue", "brand-sky", "on-brand"];

    /// egui's colours as the app shows them, by their name in `tokens.json`.
    fn from_visuals(name: &str, v: &Visuals) -> Option<Color32> {
        let w = &v.widgets;
        Some(match name {
            "panel" => v.panel_fill,
            "card" => w.inactive.weak_bg_fill,
            "card-hover" => w.hovered.weak_bg_fill,
            "control-active" => w.active.weak_bg_fill,
            "field" => v.extreme_bg_color,
            "separator" => w.noninteractive.bg_stroke.color,
            "control-border-hover" => w.hovered.bg_stroke.color,
            "text-strong" => v.strong_text_color(),
            "accent" => v.selection.bg_fill,
            "accent-text" => v.selection.stroke.color,
            "highlight" => grooph_render::highlight_color(v),
            "link" => v.hyperlink_color,
            "warn" => v.warn_fg_color,
            "error" => v.error_fg_color,
            "code-bg" => v.code_bg_color,
            _ => return None,
        })
    }

    /// The `--name: #hex;` variables of a block of `site.css`.
    fn css_vars(block: &str) -> BTreeMap<String, Color32> {
        block
            .lines()
            .filter_map(|line| line.trim().strip_prefix("--")?.strip_suffix(';')?.split_once(':'))
            .map(|(name, hex)| (format!("site-{name}"), Color32::from_hex(hex.trim()).unwrap()))
            .collect()
    }

    fn hex(value: &Value) -> Color32 {
        Color32::from_hex(value.as_str().unwrap()).unwrap()
    }

    #[test]
    fn tokens_json_matches_the_code() {
        let tokens: Value = serde_json::from_str(TOKENS_JSON).unwrap();
        let (light_css, dark_css) =
            SITE_CSS.split_once("@media (prefers-color-scheme: dark)").unwrap();
        let (dark_css, _) = dark_css.split_once("\n}\n").unwrap();
        let (site_light, site_dark) = (css_vars(light_css), css_vars(dark_css));
        let mut light = Visuals::light();
        readable_text(&mut light);
        let mut dark = Visuals::dark();
        readable_text(&mut dark);

        let mut seen = Vec::new();
        for token in tokens["color"]["tokens"].as_array().unwrap() {
            let name = token["name"].as_str().unwrap();
            seen.push(name);
            if ICON.contains(&name) {
                continue;
            }
            let value = &token["value"];
            let file = if value.is_string() {
                Themed::same(hex(value))
            } else {
                Themed { light: hex(&value["light"]), dark: hex(&value["dark"]) }
            };
            let code = if let Some((_, own)) = OWN.iter().find(|(n, _)| *n == name) {
                *own
            } else if let Some(site) = site_light.get(name) {
                Themed { light: *site, dark: site_dark[name] }
            } else {
                let (Some(l), Some(d)) = (from_visuals(name, &light), from_visuals(name, &dark))
                else {
                    panic!("{name} in tokens.json is not in the code");
                };
                Themed { light: l, dark: d }
            };
            assert_eq!(file, code, "{name}");
        }
        for name in OWN.iter().map(|(n, _)| *n).chain(site_light.keys().map(String::as_str)) {
            assert!(seen.contains(&name), "{name} is missing from tokens.json");
        }
    }
}
