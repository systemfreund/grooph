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

    pub(crate) fn take_visibility_event(&self) -> Option<VisibilityEvent> {
        None
    }

    pub(crate) fn acquire_wake_lock(&self) {}

    pub(crate) fn release_wake_lock(&self) {}
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
pub(crate) fn link_query() -> Option<String> { None }

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

/// Base URL for shared links: the current page on the web, grooph.app natively.
#[cfg(target_arch = "wasm32")]
pub(crate) fn share_base_url() -> String {
    web_sys::window()
        .and_then(|w| {
            let location = w.location();
            Some(format!("{}{}", location.origin().ok()?, location.pathname().ok()?))
        })
        .unwrap_or_else(|| "https://grooph.app/".into())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn share_base_url() -> String { "https://grooph.app/".into() }
