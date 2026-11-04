//! What the tests need from eframe to run the app without a window.
//!
//! eframe offers it only as `_new_kittest`, hidden from its docs and outside
//! its stable API, so it is called here alone: when an eframe release
//! changes or drops it, this is the one place to adapt (or to switch to
//! `egui_kittest`).

/// A creation context for [`crate::Grooph::new`] on `ctx`.
pub(crate) fn creation_context(ctx: egui::Context) -> eframe::CreationContext<'static> {
    eframe::CreationContext::_new_kittest(ctx)
}
