//! Notices when a newer build of the web app has been deployed. The CI web
//! build bakes the commit into the app (`GROOPH_BUILD`) and writes the same
//! value to `/version.txt` (crates/site). The app fetches that file at start
//! and every hour; when it differs, a notice offers to reload. Builds without
//! `GROOPH_BUILD` (local, native) never check.

use crate::Grooph;
use eframe::egui::{self, Ui};

/// The build this app was compiled from, if CI set it.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const BUILD: Option<&str> = option_env!("GROOPH_BUILD");

#[cfg(target_arch = "wasm32")]
mod web {
    use super::BUILD;
    use eframe::egui::Context;
    use std::cell::RefCell;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::wasm_bindgen::JsCast as _;
    use web_sys::wasm_bindgen::closure::Closure;
    use web_sys::{RequestCache, RequestInit, Response};

    const VERSION_URL: &str = "/version.txt";
    const CHECK_INTERVAL_MS: i32 = 60 * 60 * 1000;

    thread_local! {
        /// Build found on the server, when it differs from ours.
        static AVAILABLE: RefCell<Option<String>> = const { RefCell::new(None) };
        /// Build whose notice was dismissed; a later deploy shows it again.
        static DISMISSED: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    pub(crate) fn start(ctx: Context) {
        if BUILD.is_none() {
            return;
        }
        let Some(window) = web_sys::window() else { return };
        check(ctx.clone());
        let on_interval = Closure::wrap(Box::new(move || check(ctx.clone())) as Box<dyn FnMut()>);
        window
            .set_interval_with_callback_and_timeout_and_arguments_0(
                on_interval.as_ref().unchecked_ref(),
                CHECK_INTERVAL_MS,
            )
            .ok();
        on_interval.forget();
    }

    fn check(ctx: Context) {
        wasm_bindgen_futures::spawn_local(async move {
            let Some(remote) = fetch_build().await else { return };
            if Some(remote.as_str()) != BUILD {
                log::info!("New version deployed: {remote}");
                AVAILABLE.with(|a| *a.borrow_mut() = Some(remote));
                ctx.request_repaint();
            }
        });
    }

    /// Build id from the server; `None` if offline or the file is missing.
    async fn fetch_build() -> Option<String> {
        let window = web_sys::window()?;
        let init = RequestInit::new();
        init.set_cache(RequestCache::NoStore);
        let response = JsFuture::from(window.fetch_with_str_and_init(VERSION_URL, &init))
            .await
            .ok()?
            .dyn_into::<Response>()
            .ok()?;
        if !response.ok() {
            return None;
        }
        let text = JsFuture::from(response.text().ok()?).await.ok()?.as_string()?;
        let build = text.trim();
        (!build.is_empty()).then(|| build.to_owned())
    }

    pub(crate) fn available() -> bool {
        AVAILABLE.with(|a| {
            let a = a.borrow();
            a.is_some() && DISMISSED.with(|d| *d.borrow() != *a)
        })
    }

    pub(crate) fn dismiss() {
        let available = AVAILABLE.with(|a| a.borrow().clone());
        DISMISSED.with(|d| *d.borrow_mut() = available);
    }

    pub(crate) fn reload() {
        if let Some(window) = web_sys::window() {
            window.location().reload().ok();
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) use web::start;
#[cfg(target_arch = "wasm32")]
use web::{available, dismiss, reload};

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn start(_ctx: egui::Context) {}

#[cfg(not(target_arch = "wasm32"))]
fn available() -> bool {
    false
}

#[cfg(not(target_arch = "wasm32"))]
fn dismiss() {}

#[cfg(not(target_arch = "wasm32"))]
fn reload() {}

impl Grooph {
    /// "New version" notice with Reload and Later, once a newer build is deployed.
    pub(crate) fn update_notice(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        if !available() {
            return;
        }
        let (mut do_reload, mut later) = (false, false);
        egui::Window::new("update_notice")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 12.0))
            .show(ui.ctx(), |ui| {
                ui.label("A new version of grooph is available.");
                ui.horizontal(|ui| {
                    do_reload = ui.button("Reload").clicked();
                    later = ui.button("Later").clicked();
                });
            });
        if do_reload {
            self.stop_transport();
            // Keep the score and settings: eframe would not save before the reload.
            if let Some(storage) = frame.storage_mut() {
                eframe::App::save(self, storage);
                storage.flush();
            }
            reload();
        } else if later {
            dismiss();
        }
    }
}
