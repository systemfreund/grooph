#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) enum VisibilityEvent {
    Hidden,
    Visible,
    PageShow,
}

#[cfg(target_arch = "wasm32")]
pub(crate) use crate::web::PlatformRuntime;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct PlatformRuntime;

#[cfg(not(target_arch = "wasm32"))]
impl PlatformRuntime {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn install_listeners(&self, _ctx: eframe::egui::Context) {}

    pub(crate) fn take_visibility_events(&self) -> Vec<VisibilityEvent> {
        Vec::new()
    }

    pub(crate) fn acquire_wake_lock(&self) {}

    pub(crate) fn release_wake_lock(&self) {}
}

/// The part of the screen the user sees, in points: on the web the visual
/// viewport, which an on-screen keyboard shortens; elsewhere the whole
/// content area.
pub(crate) fn visible_rect(ctx: &eframe::egui::Context) -> eframe::egui::Rect {
    let content = ctx.content_rect();
    #[cfg(target_arch = "wasm32")]
    if let Some(visible) = crate::web::visible_canvas_rect(ctx.zoom_factor()) {
        let visible = visible.intersect(content);
        if visible.is_positive() {
            return visible;
        }
    }
    content
}

/// What produced a tap, for looking up its precise event time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) enum TapKind {
    /// A key, as lowercased `KeyboardEvent.key` (e.g. `"j"`).
    Key(&'static str),
    Pointer,
}

/// Time of a tap on the [`crate::clock`] timeline. The web uses the DOM event
/// timestamp; natively egui exposes no event times, so the tap is stamped when
/// the frame handles it (up to one frame late).
pub(crate) fn tap_time(kind: TapKind) -> f64 {
    let now = crate::clock::now_seconds();
    #[cfg(target_arch = "wasm32")]
    if let Some(time) = crate::web::take_press_time(kind, now) {
        return time.min(now);
    }
    let _ = kind;
    now
}

/// Seed for the rhythm generator. Uses the browser's crypto RNG on the web
/// and the system clock natively.
#[cfg(target_arch = "wasm32")]
pub(crate) fn random_seed() -> u64 {
    getrandom::u64().unwrap_or(0x5EED)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn random_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x5EED)
}

/// Query string of the page URL (e.g. `?bpm=90&r=...`), if any. Web only.
#[cfg(target_arch = "wasm32")]
pub(crate) fn link_query() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    (!search.is_empty()).then_some(search)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn link_query() -> Option<String> {
    None
}

/// Remove the query string from the address bar without reloading.
#[cfg(target_arch = "wasm32")]
pub(crate) fn clear_link_query() {
    let Some(window) = web_sys::window() else { return };
    let location = window.location();
    let path = location.pathname().unwrap_or_else(|_| "/".into());
    let hash = location.hash().unwrap_or_default();
    if let Ok(history) = window.history() {
        let _ = history.replace_state_with_url(
            &web_sys::wasm_bindgen::JsValue::NULL,
            "",
            Some(&format!("{path}{hash}")),
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn clear_link_query() {}

/// Base URL for shared links: the current page on the web (the app's own
/// page when embedded, see [`crate::embed`]), grooph.app natively.
#[cfg(target_arch = "wasm32")]
pub(crate) fn share_base_url() -> String {
    if let Some(url) = crate::web::host_string("appUrl") {
        return url;
    }
    web_sys::window()
        .and_then(|w| {
            let location = w.location();
            Some(format!("{}{}", location.origin().ok()?, location.pathname().ok()?))
        })
        .unwrap_or_else(|| "https://grooph.app/".into())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn share_base_url() -> String {
    "https://grooph.app/".into()
}

/// Open the system share sheet for `url` (Web Share API). Returns `false`
/// where there is none (most desktop browsers, native), so the caller can
/// copy the link instead.
#[cfg(target_arch = "wasm32")]
pub(crate) fn open_share_sheet(url: &str) -> bool {
    use web_sys::js_sys::{Function, Object, Promise, Reflect};
    use web_sys::wasm_bindgen::{JsCast as _, JsValue};

    let Some(window) = web_sys::window() else { return false };
    let navigator: JsValue = window.navigator().into();
    let Some(share) =
        Reflect::get(&navigator, &"share".into()).ok().and_then(|f| f.dyn_into::<Function>().ok())
    else {
        return false;
    };
    let data = Object::new();
    if Reflect::set(&data, &"url".into(), &url.into()).is_err() {
        return false;
    }
    match share.call1(&navigator, &data).map(|p| p.dyn_into::<Promise>()) {
        Ok(Ok(promise)) => {
            // Closing the sheet without sharing rejects the promise; nothing to do.
            wasm_bindgen_futures::spawn_local(async move {
                if let Err(err) = wasm_bindgen_futures::JsFuture::from(promise).await {
                    log::debug!("Share sheet closed: {err:?}");
                }
            });
            true
        }
        _ => false,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn open_share_sheet(_url: &str) -> bool {
    false
}

/// The theme the MCP App host shows, as the embedding view passes it on
/// (`window.groophHost.theme`). `None` outside embed mode.
#[cfg(target_arch = "wasm32")]
pub(crate) fn host_theme() -> Option<eframe::egui::Theme> {
    match crate::web::host_string("theme")?.as_str() {
        "light" => Some(eframe::egui::Theme::Light),
        "dark" => Some(eframe::egui::Theme::Dark),
        _ => None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn host_theme() -> Option<eframe::egui::Theme> {
    None
}

/// The chat host's colors as the embedding view resolved them
/// (`window.groophHost.colors`, see [`crate::embed::HostColors`]).
#[cfg(target_arch = "wasm32")]
pub(crate) fn host_colors() -> Option<String> {
    crate::web::host_string("colors").filter(|c| !c.is_empty())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn host_colors() -> Option<String> {
    None
}

/// Let the view's play button for screen readers drive the app
/// (`window.groophHost.togglePlay`).
#[cfg(target_arch = "wasm32")]
pub(crate) fn install_host_toggle_play(ctx: eframe::egui::Context) {
    crate::web::install_host_toggle_play(ctx);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_host_toggle_play(_ctx: eframe::egui::Context) {}

/// Whether the view's play button was pressed since the last call.
#[cfg(target_arch = "wasm32")]
pub(crate) fn take_toggle_play() -> bool {
    crate::web::take_toggle_play()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn take_toggle_play() -> bool {
    false
}

/// Let the embedding view ask for a frame (`window.groophHost.repaint()`),
/// e.g. after the host switched its theme.
#[cfg(target_arch = "wasm32")]
pub(crate) fn install_host_repaint(ctx: eframe::egui::Context) {
    crate::web::install_host_repaint(ctx);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_host_repaint(_ctx: eframe::egui::Context) {}

/// Notice `pointercancel`, which eframe does not forward: the page took over
/// a touch to scroll (see [`crate::embed::cancel_scrolled_touch`]).
#[cfg(target_arch = "wasm32")]
pub(crate) fn install_pointer_cancel_listener(ctx: eframe::egui::Context) {
    crate::web::install_pointer_cancel_listener(ctx);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_pointer_cancel_listener(_ctx: eframe::egui::Context) {}

/// Whether a `pointercancel` came since the last call.
#[cfg(target_arch = "wasm32")]
pub(crate) fn take_pointer_cancel() -> bool {
    crate::web::take_pointer_cancel()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn take_pointer_cancel() -> bool {
    false
}

/// Report the metronome's settings to the model through the MCP App host.
/// `false` if the view could not take them (no host, not connected).
#[cfg(target_arch = "wasm32")]
pub(crate) fn host_update_model_context(context: &crate::embed::ModelContext) -> bool {
    crate::web::host_update_model_context(context)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn host_update_model_context(_context: &crate::embed::ModelContext) -> bool {
    false
}

/// Ask the MCP App host to open `url`. `false` without a host.
#[cfg(target_arch = "wasm32")]
pub(crate) fn host_open_link(url: &str) -> bool {
    crate::web::host_call("openLink", &[url.into()]).is_some()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn host_open_link(_url: &str) -> bool {
    false
}
