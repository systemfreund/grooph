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
    pub(crate) fn new() -> Self { Self }

    pub(crate) fn install_listeners(&self, _ctx: eframe::egui::Context) {}

    pub(crate) fn take_visibility_event(&self) -> Option<VisibilityEvent> { None }

    pub(crate) fn acquire_wake_lock(&self) {}

    pub(crate) fn release_wake_lock(&self) {}
}
