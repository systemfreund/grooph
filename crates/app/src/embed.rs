//! Embed mode: the metronome alone, inside a chat as an MCP App (see
//! `mcp/` and ARCHITECTURE.md). The view the host shows starts the web build
//! with `?embed=mcp` (and `theme=light|dark`) next to the tool's link and
//! defines `window.groophHost`, through which the app reports its settings to
//! the model and opens links (see [`crate::platform`]).
//!
//! An embedded app starts from the link only: it reads and writes no storage,
//! shows no tabs, menus or library, and leaves out the update check, the
//! wake lock, MIDI, the microphone and calibration.

use crate::metronome_view::ratio_label;
use crate::pickers::silent_bars_label;
use crate::{Grooph, TransportState, platform};
use eframe::egui::{Context, Theme, Ui};
use grooph_link::format_bar;

/// How long the settings stay unchanged before they are reported, so a
/// tempo dragged on the slider is reported once.
const REPORT_DELAY_S: f64 = 1.0;

/// What the view's query asks of an embedded app.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EmbedParams {
    /// The host's theme at start.
    pub(crate) theme: Option<Theme>,
}

/// `Some` if `query` (with or without `?`) starts the app embedded:
/// `embed=mcp`.
pub(crate) fn embed_params(query: &str) -> Option<EmbedParams> {
    let mut embedded = false;
    let mut theme = None;
    for pair in query.trim_start_matches('?').split('&') {
        match pair.split_once('=').unwrap_or((pair, "")) {
            ("embed", "mcp") => embedded = true,
            ("theme", "light") => theme = Some(Theme::Light),
            ("theme", "dark") => theme = Some(Theme::Dark),
            _ => {}
        }
    }
    embedded.then_some(EmbedParams { theme })
}

/// A value of the settings reported to the model, as JSON would hold it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Field {
    Text(String),
    Number(f64),
    Bool(bool),
}

/// The metronome's settings as reported to the model: a line of text and the
/// same as fields.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModelContext {
    pub(crate) text: String,
    pub(crate) fields: Vec<(&'static str, Field)>,
}

/// State of an embedded app. Not persisted.
pub(crate) struct Embed {
    /// Theme last applied: from the query, then from the host.
    theme: Option<Theme>,
    /// Settings last reported to the model.
    reported: Option<ModelContext>,
    /// Settings that differ from the reported ones, and since when.
    changed: Option<(ModelContext, f64)>,
}

impl Embed {
    pub(crate) fn new(ctx: &Context, params: EmbedParams) -> Self {
        if let Some(theme) = params.theme {
            ctx.set_theme(theme);
        }
        platform::install_host_repaint(ctx.clone());
        Self { theme: params.theme, reported: None, changed: None }
    }
}

impl Grooph {
    /// Whether the app runs embedded in a chat (see [`crate::embed`]).
    pub(crate) fn embedded(&self) -> bool {
        self.ui.embed.is_some()
    }

    /// The layout of an embedded app: the metronome tab without the tab bar.
    pub(crate) fn embed_ui(&mut self, ui: &mut Ui) {
        self.follow_host_theme(ui.ctx());
        self.metronome_page(ui);
        self.measure_panel(ui);
        self.settings_picker(ui);
        self.report_to_model(ui);
    }

    /// Switch to the host's theme when it changes.
    fn follow_host_theme(&mut self, ctx: &Context) {
        let Some(embed) = self.ui.embed.as_mut() else { return };
        if let Some(theme) = platform::host_theme()
            && embed.theme != Some(theme)
        {
            ctx.set_theme(theme);
            embed.theme = Some(theme);
        }
    }

    /// The metronome's settings, for the model.
    pub(crate) fn model_context(&self) -> ModelContext {
        let ts = self.metronome_meter();
        let bpm = self.playback_ctl.bpm;
        let playing = self.playback_ctl.transport_state == TransportState::Playing;
        let cfg = &self.playback_ctl.audio_cfg;
        let click = format_bar(&self.click_bar()).unwrap_or_default();
        let subdivision = self.click_fill_label();
        let mut text = format!(
            "Metronome: {ts} at {bpm} BPM (quarter notes), click bar {click} ({subdivision})"
        );
        let mut fields = vec![
            ("ts", Field::Text(ts.to_string())),
            ("bpm", Field::Number(bpm.into())),
            ("click", Field::Text(click)),
            ("subdivision", Field::Text(subdivision)),
        ];
        if let Some(bar) = self.counter_bar() {
            let counter = format_bar(bar).unwrap_or_default();
            let name = self.counter_ratio().map_or_else(|| "custom".to_owned(), ratio_label);
            text += &format!(", polyrhythm {name} ({counter})");
            fields.push(("polyrhythm", Field::Text(name)));
            fields.push(("counter", Field::Text(counter)));
        }
        if !cfg.swing.is_straight() {
            text += &format!(", swing {}% {}", cfg.swing.percent, cfg.swing.unit.label());
            fields.push(("swing", Field::Number(cfg.swing.percent.into())));
            fields.push(("su", Field::Text(cfg.swing.unit.label().to_owned())));
        }
        if cfg.count_in {
            text += ", count-in";
            fields.push(("countIn", Field::Bool(true)));
        }
        if cfg.gap.enabled {
            let silent = silent_bars_label(&cfg.gap);
            text += &format!(", silent bars {silent}");
            fields.push(("silentBars", Field::Text(silent)));
        }
        text += if playing { ", playing." } else { ", stopped." };
        fields.push(("playing", Field::Bool(playing)));
        if let Some(link) = self.share_link_url() {
            text += &format!(" Link: {link}");
            fields.push(("link", Field::Text(link)));
        }
        ModelContext { text, fields }
    }

    /// Tell the model what the view is set to, once the settings have stayed
    /// the same for [`REPORT_DELAY_S`]. The first report goes out at once.
    fn report_to_model(&mut self, ui: &Ui) {
        let context = self.model_context();
        let now = ui.input(|i| i.time);
        let Some(embed) = self.ui.embed.as_mut() else { return };
        if embed.reported.as_ref() == Some(&context) {
            embed.changed = None;
            return;
        }
        let since = match &embed.changed {
            Some((pending, since)) if *pending == context => *since,
            _ if embed.reported.is_none() => now - REPORT_DELAY_S,
            _ => now,
        };
        if now - since < REPORT_DELAY_S {
            embed.changed = Some((context, since));
            ui.ctx().request_repaint_after_secs((since + REPORT_DELAY_S - now) as f32);
            return;
        }
        platform::host_update_model_context(&context);
        embed.reported = Some(context);
        embed.changed = None;
    }

    /// Open the metronome as it is set on grooph.app, through the host.
    pub(crate) fn open_in_grooph(&self, ui: &Ui) {
        let Some(url) = self.share_link_url() else { return };
        if !platform::host_open_link(&url) {
            ui.ctx().open_url(eframe::egui::OpenUrl::new_tab(url));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EmbedParams, Field, embed_params};
    use crate::Grooph;
    use eframe::egui::Theme;

    #[test]
    fn embed_mode_comes_from_the_query() {
        assert_eq!(embed_params(""), None);
        assert_eq!(embed_params("?v=2&bpm=90&ts=7/8"), None);
        assert_eq!(embed_params("?embed=other"), None);
        assert_eq!(embed_params("?v=2&ts=7/8&embed=mcp"), Some(EmbedParams { theme: None }));
        assert_eq!(
            embed_params("embed=mcp&theme=dark"),
            Some(EmbedParams { theme: Some(Theme::Dark) })
        );
        assert_eq!(
            embed_params("?theme=light&embed=mcp"),
            Some(EmbedParams { theme: Some(Theme::Light) })
        );
        assert_eq!(embed_params("?embed=mcp&theme=sepia"), Some(EmbedParams { theme: None }));
    }

    fn embedded_app(query: &str) -> (eframe::egui::Context, Grooph) {
        let ctx = eframe::egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let app = Grooph::new_with(&cc, embed_params(query), Some(query.to_owned()));
        (ctx, app)
    }

    #[test]
    fn an_embedded_app_opens_the_link_in_the_metronome() {
        let (ctx, app) = embedded_app("?v=2&bpm=140&ts=7/8&click=>q+r:q.+(q)&embed=mcp&theme=dark");
        assert!(app.embedded());
        assert!(app.metronome_mode());
        assert_eq!(app.playback_ctl.bpm, 140);
        assert_eq!(app.metronome_meter().to_string(), "7/8");
        assert_eq!(ctx.options(|o| o.theme_preference), Theme::Dark.into());
        let context = app.model_context();
        assert!(context.text.starts_with("Metronome: 7/8 at 140 BPM"), "{}", context.text);
        assert!(context.fields.contains(&("bpm", Field::Number(140.0))));
        assert!(context.fields.contains(&("click", Field::Text(">q r:q. (q)".into()))));
        assert!(context.fields.contains(&("playing", Field::Bool(false))));
        assert!(app.ui.link_error.is_none());
    }

    #[test]
    fn a_wrong_link_shows_its_error_in_the_view() {
        let (_, app) = embedded_app("?v=2&ts=7/8&counter=t4+t4+t4&embed=mcp");
        assert!(app.ui.link_error.is_some());
        assert!(app.metronome_mode());
    }

    #[test]
    fn the_model_hears_of_the_polyrhythm_and_swing() {
        let (_, app) =
            embedded_app("?v=2&bpm=90&ts=4/4&counter=t4+t4+t4+t4+t4+t4&swing=66&embed=mcp");
        let context = app.model_context();
        assert!(context.text.contains("polyrhythm 3:2 (t4 t4 t4 t4 t4 t4)"), "{}", context.text);
        assert!(context.text.contains("swing 66% 8ths"), "{}", context.text);
        let link = context.fields.iter().find(|(k, _)| *k == "link").map(|(_, v)| v.clone());
        let Some(Field::Text(link)) = link else { panic!("no link in {:?}", context.fields) };
        assert!(link.contains("ts=4/4") && link.contains("counter=") && link.contains("swing=66"));
        assert!(!link.contains("embed"), "{link}");
    }

    #[test]
    fn an_embedded_app_stores_nothing() {
        use eframe::App as _;
        let (_, mut app) = embedded_app("?v=2&bpm=100&ts=3/4&embed=mcp");
        assert!(!app.persist_egui_memory());
        let mut storage = MemoryStorage::default();
        app.save(&mut storage);
        assert!(storage.0.is_empty());
    }

    #[test]
    fn the_embedded_view_renders_at_chat_sizes() {
        use eframe::egui::{Pos2, RawInput, Rect, vec2};
        for (width, height) in [(360.0, 520.0), (720.0, 520.0), (360.0, 400.0)] {
            let (ctx, mut app) =
                embedded_app("?v=2&bpm=90&ts=4/4&counter=t4+t4+t4+t4+t4+t4&embed=mcp");
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(width, height));
            for picker in [None, Some(crate::pickers::Picker::MetronomeMeter)] {
                app.ui.picker = picker;
                let input = RawInput { screen_rect: Some(screen), ..Default::default() };
                let mut output = ctx.run_ui(input, |ui| {
                    app.apply_style(ui);
                    app.embed_ui(ui);
                });
                output.textures_delta.clear();
            }
        }
    }

    #[derive(Default)]
    struct MemoryStorage(std::collections::HashMap<String, String>);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }
}
