//! Embed mode: the metronome alone, inside a chat as an MCP App (see
//! `mcp/` and ARCHITECTURE.md). The view the host shows starts the web build
//! with `?embed=mcp` (and `theme=light|dark`) next to the tool's link and
//! defines `window.groophHost`, through which the app reports its settings to
//! the model and opens links (see [`crate::platform`]).
//!
//! An embedded app starts from the link only: it reads and writes no storage,
//! shows no tabs, menus or library, and leaves out the update check, the
//! wake lock, MIDI, the microphone and calibration.

use crate::pickers::silent_bars_label;
use crate::{Grooph, TransportState, platform};
use eframe::egui::{
    self, Align, Button, Color32, Context, Event, FontId, Frame, Layout, Margin, PointerButton,
    RawInput, Rect, RichText, SafeAreaInsets, Sense, Stroke, Theme, TouchPhase, Ui, Visuals, pos2,
    vec2,
};
use eframe::epaint::MarginF32;
use grooph_link::format_bar;
use grooph_measure::swing::SwingUnit;
use grooph_measure::tempo::{MAX_BPM, MIN_BPM};

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

/// Whether the view is in fullscreen, read by eframe's event handlers (see
/// [`chat_takes`]): there no chat scrolls behind it.
static FULLSCREEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the page may handle `event` too (no `preventDefault`): inline,
/// pointer, touch and wheel events, so the chat scrolls on a vertical drag
/// (the view's canvas has `touch-action: pan-y` then) or the wheel. In
/// fullscreen vertical gestures are the app's again (pickers scroll, the
/// options sheet folds on a flick), as Claude's guidelines allow.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn chat_takes(event: &Event) -> bool {
    !FULLSCREEN.load(std::sync::atomic::Ordering::Relaxed) && leaves_to_page(event)
}

/// Whether the page may handle `event` too when embedded inline: pointer,
/// touch and wheel events.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn leaves_to_page(event: &Event) -> bool {
    matches!(
        event,
        Event::PointerButton { .. }
            | Event::PointerMoved(_)
            | Event::PointerGone
            | Event::Touch { .. }
            | Event::MouseWheel { .. }
    )
}

/// A touch the page took over to scroll the chat (`touchcancel`, or a
/// `pointercancel` seen by [`platform::take_pointer_cancel`]) must not end
/// as a tap: the view moves with the page, so to egui the finger seems to
/// stay where it pressed. The pointer moves far off before it is released,
/// which egui takes as a drag, not a click.
pub(crate) fn cancel_scrolled_touch(ctx: &Context, raw: &mut RawInput) {
    // In fullscreen the app keeps its touches (see `chat_takes`).
    if FULLSCREEN.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let touch_cancelled =
        raw.events.iter().any(|e| matches!(e, Event::Touch { phase: TouchPhase::Cancel, .. }));
    if !(platform::take_pointer_cancel() || touch_cancelled) {
        return;
    }
    // The press may have come in with this frame's events.
    let pressed_now = raw.events.iter().rev().find_map(|e| match e {
        Event::PointerButton { pos, pressed: true, .. } => Some(*pos),
        _ => None,
    });
    let Some(origin) = pressed_now.or_else(|| ctx.input(|i| i.pointer.press_origin())) else {
        return;
    };
    let away = origin + vec2(0.0, 10_000.0);
    raw.events.push(Event::PointerMoved(away));
    raw.events.push(Event::PointerButton {
        pos: away,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    raw.events.push(Event::PointerGone);
}

/// What the chat host covers at the view's edges, as egui's safe area:
/// `insets` (`top,right,bottom,left` in CSS pixels) in points.
pub(crate) fn safe_area_of(insets: &str, zoom: f32) -> Option<SafeAreaInsets> {
    let sides: Vec<f32> =
        insets.split(',').map(|v| v.trim().parse().ok()).collect::<Option<_>>()?;
    let [top, right, bottom, left] = sides[..] else { return None };
    let point = |px: f32| px.max(0.0) / zoom.max(0.1);
    Some(SafeAreaInsets(MarginF32 {
        left: point(left),
        right: point(right),
        top: point(top),
        bottom: point(bottom),
    }))
}

/// The host's safe area for this frame (see [`safe_area_of`]).
pub(crate) fn host_safe_area(ctx: &Context) -> Option<SafeAreaInsets> {
    safe_area_of(&platform::host_insets()?, ctx.zoom_factor())
}

/// The chat host's own colors (`hostContext.styles.variables`, resolved by
/// the view as `background=#rrggbbaa;card=…`), so the view's background,
/// cards and text match the chat around it. Accents (play, the ring) stay
/// grooph's. Each is `None` where the host gave none: egui's own then.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HostColors {
    background: Option<Color32>,
    card: Option<Color32>,
    text: Option<Color32>,
    text_weak: Option<Color32>,
    border: Option<Color32>,
}

impl HostColors {
    pub(crate) fn parse(text: &str) -> Self {
        let mut colors = Self::default();
        for (name, hex) in text.split(';').filter_map(|pair| pair.split_once('=')) {
            let Ok(color) = Color32::from_hex(hex.trim()) else { continue };
            let slot = match name.trim() {
                "background" => &mut colors.background,
                "card" => &mut colors.card,
                "text" => &mut colors.text,
                "textWeak" => &mut colors.text_weak,
                "border" => &mut colors.border,
                _ => continue,
            };
            *slot = Some(color);
        }
        colors
    }

    /// Paint egui's panels, cards (and buttons, which share their fill) and
    /// text in these colors.
    pub(crate) fn apply(&self, visuals: &mut Visuals) {
        if let Some(background) = self.background {
            visuals.panel_fill = background;
            visuals.window_fill = background;
        }
        let widgets = &mut visuals.widgets;
        if let Some(card) = self.card {
            widgets.inactive.weak_bg_fill = card;
            // Hovered and pressed cards turn a little towards the text.
            let toward = self.text.unwrap_or(widgets.inactive.fg_stroke.color);
            widgets.hovered.weak_bg_fill = card.lerp_to_gamma(toward, 0.08);
            widgets.active.weak_bg_fill = card.lerp_to_gamma(toward, 0.16);
        }
        if let Some(text) = self.text {
            for widget in [&mut widgets.noninteractive, &mut widgets.inactive, &mut widgets.hovered]
            {
                widget.fg_stroke.color = text;
            }
        }
        if let Some(border) = self.border {
            widgets.noninteractive.bg_stroke.color = border;
            visuals.window_stroke.color = border;
        }
        if self.text_weak.is_some() {
            visuals.weak_text_color = self.text_weak;
        }
    }
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
    /// The host shows the view in fullscreen, and whether it offers that:
    /// inline it is then the compact card (see [`Grooph::compact_card`]).
    pub(crate) fullscreen: bool,
    pub(crate) fullscreen_offered: bool,
    /// The host's colors, and the text they were read from.
    pub(crate) colors: HostColors,
    colors_text: Option<String>,
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
        platform::install_pointer_cancel_listener(ctx.clone());
        platform::install_host_toggle_play(ctx.clone());
        Self {
            theme: params.theme,
            fullscreen: false,
            fullscreen_offered: false,
            colors: HostColors::default(),
            colors_text: None,
            reported: None,
            changed: None,
        }
    }
}

impl Grooph {
    /// Whether the app runs embedded in a chat (see [`crate::embed`]).
    pub(crate) fn embedded(&self) -> bool {
        self.ui.embed.is_some()
    }

    /// Whether a chat scrolls behind the view: embedded and inline. Vertical
    /// gestures are then the chat's (see [`chat_takes`]).
    pub(crate) fn chat_scrolls(&self) -> bool {
        self.ui.embed.as_ref().is_some_and(|e| !e.fullscreen)
    }

    /// Whether the view is the compact card: inline, where the host offers
    /// fullscreen for the rest (options, pickers, "Open in grooph").
    /// Otherwise the metronome tab without the tab bar.
    pub(crate) fn compact_card(&self) -> bool {
        self.ui.embed.as_ref().is_some_and(|e| e.fullscreen_offered && !e.fullscreen)
    }

    /// The layout of an embedded app: the compact card inline, the
    /// metronome tab without the tab bar in fullscreen (or where the host
    /// offers no fullscreen).
    pub(crate) fn embed_ui(&mut self, ui: &mut Ui) {
        self.follow_host_theme(ui.ctx());
        // The view's play button, for screen readers.
        if platform::take_toggle_play() {
            self.toggle_playback();
        }
        if self.compact_card() {
            self.tempo_row(ui);
            self.staff_and_ring(ui, true);
        } else {
            self.metronome_page(ui);
        }
        self.measure_panel(ui);
        self.settings_picker(ui);
        self.report_to_model(ui);
    }

    /// Switch to the host's theme and colors when they change.
    fn follow_host_theme(&mut self, ctx: &Context) {
        let Some(embed) = self.ui.embed.as_mut() else { return };
        if let Some(theme) = platform::host_theme()
            && embed.theme != Some(theme)
        {
            ctx.set_theme(theme);
            embed.theme = Some(theme);
        }
        if let Some((fullscreen, offered)) = platform::host_display()
            && (fullscreen, offered) != (embed.fullscreen, embed.fullscreen_offered)
        {
            embed.fullscreen = fullscreen;
            embed.fullscreen_offered = offered;
            FULLSCREEN.store(fullscreen, std::sync::atomic::Ordering::Relaxed);
            ctx.request_repaint();
        }
        let colors = platform::host_colors();
        if colors != embed.colors_text {
            embed.colors = colors.as_deref().map(HostColors::parse).unwrap_or_default();
            embed.colors_text = colors;
            ctx.request_repaint();
        }
    }

    /// The compact card's top row: the tempo as notation writes it
    /// (`♩ = 140`) with − and + beside it, and the fullscreen button.
    fn tempo_row(&mut self, ui: &mut Ui) {
        let music = self.ui.music_font_id.family.clone();
        let bpm = self.playback_ctl.playback.ramp_bpm.unwrap_or(self.playback_ctl.bpm);
        let mut step = 0;
        let mut fullscreen = false;
        let frame = Frame::new().fill(ui.visuals().panel_fill).inner_margin(Margin {
            left: 12,
            right: 8,
            top: 2,
            bottom: 0,
        });
        egui::Panel::top("embed_tempo").frame(frame).show_separator_line(false).show(ui, |ui| {
            ui.horizontal(|ui| {
                // Buttons as tall as their 44 pt, not the app's roomy padding.
                ui.spacing_mut().button_padding = vec2(8.0, 4.0);
                tempo_mark(ui, bpm, music);
                ui.add_space(4.0);
                for (label, delta, hint) in [("−", -1, "Slower"), ("+", 1, "Faster")] {
                    let button =
                        Button::new(RichText::new(label).size(22.0)).min_size(vec2(TARGET, TARGET));
                    if ui.add(button).on_hover_text(hint).clicked() {
                        step = delta;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    fullscreen = fullscreen_button(ui).on_hover_text("Fullscreen").clicked();
                });
            });
        });
        if step != 0 {
            self.step_bpm(step);
        }
        if fullscreen {
            platform::host_request_fullscreen();
        }
    }

    /// One BPM faster (`delta` 1) or slower (-1), within the tempo range.
    fn step_bpm(&mut self, delta: i64) {
        let bpm = (i64::from(self.playback_ctl.bpm) + delta).clamp(MIN_BPM.into(), MAX_BPM.into());
        self.playback_ctl.bpm = bpm as u32;
        self.handle_bpm_change();
    }

    /// The metronome's settings, for the model.
    pub(crate) fn model_context(&self) -> ModelContext {
        let ts = self.metronome_meter();
        let bpm = self.playback_ctl.bpm;
        let playing = self.playback_ctl.transport_state == TransportState::Playing;
        let cfg = &self.playback_ctl.audio_cfg;
        let click = format_bar(&self.click_bar()).unwrap_or_default();
        let subdivision = self.click_fill_label();
        // The same description as the tool's result (grooph_link::describe):
        // the meter's grouping, where the clicks fall, the polyrhythm, swing.
        let mut text = format!("Metronome: {}", grooph_link::describe(&self.shared_link()));
        let mut fields = vec![
            // The meter alone, as the tool's ts; the grouping is groups.
            ("ts", Field::Text(format!("{}/{}", ts.beats, ts.beat_unit))),
            ("bpm", Field::Number(bpm.into())),
            ("click", Field::Text(click)),
            ("subdivision", Field::Text(subdivision)),
        ];
        let groups = ts.groups();
        if groups.len() > 1 && groups.iter().any(|&g| g > 1) {
            fields.push(("groups", Field::Text(ts.groups_label())));
        }
        if self.counter_bar().is_some() {
            // The fields use the tool's arguments only: the ratio as `poly`;
            // edited clicks are in the text and the link.
            if let Some((ratio, edited)) = self.counter_ratio() {
                fields.push(("poly", Field::Text(ratio.to_string())));
                if edited {
                    fields.push(("polyEdited", Field::Bool(true)));
                }
            } else {
                fields.push(("polyEdited", Field::Bool(true)));
            }
        }
        if !cfg.swing.is_straight() {
            fields.push(("swing", Field::Number(cfg.swing.percent.into())));
            let su = match cfg.swing.unit {
                SwingUnit::Eighths => 8,
                SwingUnit::Sixteenths => 16,
            };
            fields.push(("su", Field::Number(su.into())));
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
        // A link that could not be opened: the model can call again.
        if let Some(error) = &self.ui.link_error {
            text = format!("The tool's link could not be opened: {error}. {text}");
            fields.push(("error", Field::Text(error.clone())));
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
        // Not taken (no host yet): a later frame tries again.
        if !platform::host_update_model_context(&context) {
            embed.changed = Some((context, since));
            return;
        }
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

/// Smallest touch target, in points (Apple's and Claude's guidelines).
const TARGET: f32 = 44.0;
/// Size of the tempo mark's text.
const TEMPO_TEXT_SIZE: f32 = 22.0;

/// `♩ = 140` as a tempo mark, the note and the text on one baseline,
/// centered in a row as tall as the buttons beside it. Painted rather than
/// labelled: the music font's line is far taller than its note, and labels
/// could be selected.
fn tempo_mark(ui: &mut Ui, bpm: u32, music: egui::FontFamily) {
    let color = ui.visuals().strong_text_color();
    // SMuFL metNoteQuarterUp: a quarter note, as in a tempo mark.
    let note = ui.painter().layout_no_wrap("\u{ECA5}".into(), FontId::new(24.0, music), color);
    let text = ui.painter().layout_no_wrap(
        format!("= {bpm}"),
        FontId::proportional(TEMPO_TEXT_SIZE),
        color,
    );
    let gap = 6.0;
    let (rect, _) =
        ui.allocate_exact_size(vec2(note.size().x + gap + text.size().x, TARGET), Sense::hover());
    // Each galley's first glyph sits on its baseline; the middle of the
    // digits (cap height about 0.7 of the size) on the row's middle.
    let baseline_of = |galley: &egui::Galley| {
        galley.rows.first().and_then(|r| r.row.glyphs.first()).map_or(0.0, |g| g.pos.y)
    };
    let baseline = rect.center().y + 0.35 * TEMPO_TEXT_SIZE;
    let painter = ui.painter();
    let note_top = baseline - baseline_of(&note);
    painter.galley(pos2(rect.left(), note_top), note.clone(), color);
    let text_top = baseline - baseline_of(&text);
    painter.galley(pos2(rect.left() + note.size().x + gap, text_top), text, color);
}

/// The fullscreen button: four corners pointing outwards.
fn fullscreen_button(ui: &mut Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(TARGET, TARGET), Sense::click());
    let color =
        if resp.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
    let stroke = Stroke::new(2.0, color);
    let r = Rect::from_center_size(rect.center(), vec2(18.0, 18.0));
    let arm = 6.0;
    for (corner, dx, dy) in [
        (r.left_top(), 1.0, 1.0),
        (r.right_top(), -1.0, 1.0),
        (r.left_bottom(), 1.0, -1.0),
        (r.right_bottom(), -1.0, -1.0),
    ] {
        let painter = ui.painter();
        painter.line_segment([corner, pos2(corner.x + dx * arm, corner.y)], stroke);
        painter.line_segment([corner, pos2(corner.x, corner.y + dy * arm)], stroke);
    }
    resp
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
        assert!(context.text.starts_with("Metronome: 7/8 grouped 2+3+2"), "{}", context.text);
        assert!(context.text.contains("at 140 BPM"), "{}", context.text);
        assert!(context.text.contains("(strong on eighths 1; soft on 6)"), "{}", context.text);
        assert!(context.fields.contains(&("bpm", Field::Number(140.0))));
        assert!(context.fields.contains(&("click", Field::Text(">q r:q. (q)".into()))));
        assert!(context.fields.contains(&("playing", Field::Bool(false))));
        assert!(app.ui.link_error.is_none());
    }

    #[test]
    fn the_model_hears_the_grouping_as_the_tools_arguments() {
        // ts is the meter alone, as the tool takes it; the grouping is groups.
        let (_, app) = embedded_app("?v=2&ts=7/8&groups=3%2B2%2B2&embed=mcp");
        let context = app.model_context();
        assert!(
            context.fields.contains(&("ts", Field::Text("7/8".into()))),
            "{:?}",
            context.fields
        );
        assert!(context.fields.contains(&("groups", Field::Text("3+2+2".into()))));
        assert!(context.text.starts_with("Metronome: 7/8 grouped 3+2+2"), "{}", context.text);
    }

    #[test]
    fn a_wrong_link_shows_its_error_in_the_view() {
        let (_, app) = embedded_app("?v=2&ts=7/8&counter=t4+t4+t4&embed=mcp");
        assert!(app.ui.link_error.is_some());
        assert!(app.metronome_mode());
        let text = app.model_context().text;
        assert!(text.starts_with("The tool's link could not be opened: counter"), "{text}");
    }

    #[test]
    fn the_model_hears_of_the_polyrhythm_and_swing() {
        let (_, app) = embedded_app("?v=2&bpm=90&ts=4/4&poly=3:2&swing=66&embed=mcp");
        let context = app.model_context();
        assert!(context.text.contains("polyrhythm 3:2,"), "{}", context.text);
        assert!(context.text.contains("swing 66% 8ths"), "{}", context.text);
        assert!(context.fields.contains(&("poly", Field::Text("3:2".into()))));
        assert!(context.fields.contains(&("su", Field::Number(8.0))));
        let link = context.fields.iter().find(|(k, _)| *k == "link").map(|(_, v)| v.clone());
        let Some(Field::Text(link)) = link else { panic!("no link in {:?}", context.fields) };
        assert!(link.contains("ts=4/4") && link.contains("poly=3:2") && link.contains("swing=66"));
        assert!(!link.contains("embed"), "{link}");
        // Clicks written out that are a ratio's are reported as the ratio.
        let (_, app) = embedded_app("?v=2&ts=4/4&counter=t4+t4+t4+t4+t4+t4&embed=mcp");
        assert!(app.model_context().text.contains("polyrhythm 3:2,"));
        // Edited ones are written out.
        let (_, app) = embedded_app("?v=2&ts=4/4&counter=t4+(t4)+t4+t4+t4+t4&embed=mcp");
        let context = app.model_context();
        assert!(
            context.text.contains("polyrhythm 3:2 with edited clicks t4 (t4) t4"),
            "{}",
            context.text
        );
        assert!(context.fields.contains(&("poly", Field::Text("3:2".into()))));
    }

    #[test]
    fn a_report_the_view_did_not_take_is_tried_again() {
        use eframe::egui::RawInput;
        let (ctx, mut app) = embedded_app("?v=2&ts=4/4&embed=mcp");
        // Natively there is no view to take it, like a host not yet connected.
        let mut output = ctx.run_ui(RawInput::default(), |ui| app.embed_ui(ui));
        output.textures_delta.clear();
        let embed = app.ui.embed.as_ref().unwrap();
        assert!(embed.reported.is_none());
        assert!(embed.changed.is_some());
    }

    #[test]
    fn a_touch_the_page_scrolls_with_is_no_tap() {
        use super::cancel_scrolled_touch;
        use eframe::egui::{
            Button, Event, Modifiers, PointerButton, RawInput, TouchDeviceId, TouchId, TouchPhase,
            pos2,
        };
        let ctx = eframe::egui::Context::default();
        let at = pos2(50.0, 20.0);
        let press = Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::default(),
        };
        let cancel = Event::Touch {
            device_id: TouchDeviceId(0),
            id: TouchId(0),
            phase: TouchPhase::Cancel,
            pos: at,
            force: None,
        };
        // A button under the finger; returns whether it was clicked.
        let frame = |mut raw: RawInput| {
            cancel_scrolled_touch(&ctx, &mut raw);
            let mut clicked = false;
            let mut output = ctx.run_ui(raw, |ui| {
                clicked = ui.add_sized([200.0, 100.0], Button::new("play")).clicked();
            });
            output.textures_delta.clear();
            clicked
        };
        // Each touch starts with the finger's position, as eframe reports it.
        let events = |mut events: Vec<Event>| {
            if events
                .first()
                .is_some_and(|e| matches!(e, Event::PointerButton { pressed: true, .. }))
            {
                events.insert(0, Event::PointerMoved(at));
            }
            RawInput { events, ..Default::default() }
        };
        frame(events(vec![]));
        let release = Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::default(),
        };
        // Pressed, then the page took the touch over (in one frame or two);
        // the finger lifts where it pressed, as the view moved with the page.
        assert!(!frame(events(vec![press.clone(), cancel.clone()])));
        assert!(!frame(events(vec![release.clone()])));
        assert!(!frame(events(vec![press.clone()])));
        assert!(!frame(events(vec![cancel])));
        assert!(!frame(events(vec![release.clone()])));
        // A real tap still clicks.
        frame(events(vec![press]));
        assert!(frame(events(vec![release])));
    }

    #[test]
    fn host_colors_paint_panels_cards_and_text() {
        use super::HostColors;
        use eframe::egui::{Color32, Visuals};
        let colors = HostColors::parse(
            "background=#ffffffff;card=#f5f4efff;text=#141413ff;textWeak=#73726cff;bogus=#000000ff;border=nope",
        );
        let mut visuals = Visuals::light();
        let accent = visuals.selection.bg_fill;
        colors.apply(&mut visuals);
        assert_eq!(visuals.panel_fill, Color32::WHITE);
        assert_eq!(visuals.widgets.inactive.weak_bg_fill, Color32::from_rgb(0xf5, 0xf4, 0xef));
        assert_eq!(visuals.text_color(), Color32::from_rgb(0x14, 0x14, 0x13));
        assert_eq!(visuals.weak_text_color(), Color32::from_rgb(0x73, 0x72, 0x6c));
        // What the host does not give stays egui's; accents stay grooph's.
        assert_eq!(visuals.window_stroke, Visuals::light().window_stroke);
        assert_eq!(visuals.selection.bg_fill, accent);
        // No colors, no change.
        let mut plain = Visuals::dark();
        HostColors::parse("").apply(&mut plain);
        assert_eq!(plain.panel_fill, Visuals::dark().panel_fill);
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
        let sizes =
            [(320.0, 520.0), (360.0, 520.0), (720.0, 520.0), (360.0, 400.0), (320.0, 360.0)];
        // The whole metronome, and the compact card where fullscreen is offered.
        for ((width, height), compact) in sizes.into_iter().flat_map(|s| [(s, false), (s, true)]) {
            let (ctx, mut app) = embedded_app("?v=2&bpm=90&ts=7/8&groups=2+2+3&poly=3:7&embed=mcp");
            app.ui.embed.as_mut().unwrap().fullscreen_offered = compact;
            assert_eq!(app.compact_card(), compact);
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

    #[test]
    fn the_staff_keeps_its_size_and_follows_a_resize_at_once() {
        use eframe::egui::{Pos2, RawInput, Rect, vec2};
        let run = |app: &mut Grooph, ctx: &eframe::egui::Context, width: f32| {
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(width, 400.0));
            let input = RawInput { screen_rect: Some(screen), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                app.apply_style(ui);
                app.ui.narrow_screen = ui.available_width() < crate::shell::NARROW_MAX_WIDTH;
                app.embed_ui(ui);
            });
            output.textures_delta.clear();
            app.ui.staff_em
        };
        for compact in [true, false] {
            let fresh = |width| {
                let (ctx, mut app) = embedded_app("?v=2&bpm=129&ts=7/8&poly=4:7&embed=mcp");
                app.ui.embed.as_mut().unwrap().fullscreen_offered = compact;
                run(&mut app, &ctx, width)
            };
            let (ctx, mut app) = embedded_app("?v=2&bpm=129&ts=7/8&poly=4:7&embed=mcp");
            app.ui.embed.as_mut().unwrap().fullscreen_offered = compact;
            // The same size frame after frame: it does not shrink to the
            // room it was given.
            let sizes: Vec<f32> = (0..4).map(|_| run(&mut app, &ctx, 393.0)).collect();
            assert!(sizes.windows(2).all(|w| w[0] == w[1]) && sizes[0] > 20.0, "{sizes:?}");
            // A new width shows in the first frame, as in a fresh view.
            assert_eq!(run(&mut app, &ctx, 360.0), fresh(360.0), "{compact}");
        }
    }

    #[test]
    fn the_tempo_buttons_step_the_bpm_within_its_range() {
        let (_, mut app) = embedded_app("?v=2&bpm=21&ts=4/4&embed=mcp");
        app.step_bpm(1);
        assert_eq!(app.playback_ctl.bpm, 22);
        for _ in 0..5 {
            app.step_bpm(-1);
        }
        assert_eq!(app.playback_ctl.bpm, 20);
        app.playback_ctl.bpm = 300;
        app.step_bpm(1);
        assert_eq!(app.playback_ctl.bpm, 300);
    }

    #[test]
    fn the_middle_of_the_ring_is_the_play_button() {
        use crate::metronome_view::paint_play_middle;
        use eframe::egui::{Event, Modifiers, PointerButton, RawInput, Sense, pos2};
        let ctx = eframe::egui::Context::default();
        let center = pos2(100.0, 100.0);
        let tapped_at = |at| {
            let mut tapped = false;
            for events in [
                vec![Event::PointerMoved(at)],
                vec![Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::default(),
                }],
                vec![Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::default(),
                }],
            ] {
                let mut output = ctx.run_ui(RawInput { events, ..Default::default() }, |ui| {
                    let resp = ui.allocate_response(ui.available_size(), Sense::click());
                    tapped |= paint_play_middle(ui, &resp, center, 30.0, false, None);
                });
                output.textures_delta.clear();
            }
            tapped
        };
        assert!(tapped_at(center));
        assert!(tapped_at(pos2(120.0, 115.0)));
        assert!(!tapped_at(pos2(140.0, 100.0)), "outside the button: the ring's");
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
