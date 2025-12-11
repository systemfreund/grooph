use crate::Grooph;
use crate::app::{PlayerState, web};
use eframe::egui::Context;
use log::{debug, error, info};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};
use wasm_bindgen_futures::JsFuture;
use web_sys::{WakeLockSentinel, WakeLockType};
use web_sys::wasm_bindgen::JsCast as _;
use web_sys::wasm_bindgen::closure::Closure;

// 0 = None, 1 = Hidden, 2 = Visible, 3 = PageShow
static PENDING: AtomicU8 = AtomicU8::new(0);
static IS_MOBILE: OnceLock<bool> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisibilityEvent {
    Hidden,
    Visible,
    PageShow,
}

fn take_pending_visibility_event() -> Option<VisibilityEvent> {
    match PENDING.swap(0, Ordering::SeqCst) {
        1 => Some(VisibilityEvent::Hidden),
        2 => Some(VisibilityEvent::Visible),
        3 => Some(VisibilityEvent::PageShow),
        _ => None,
    }
}

pub fn install_visibility_listeners(ctx: Context) {
    // Only install visibility listeners on mobile browsers. On desktop, keep audio running.
    let is_mobile = is_mobile_browser();
    if !is_mobile {
        debug!("Desktop browser detected – skipping visibility pause handlers");
        return;
    }
    if let Some(win) = web_sys::window()
        && let Some(doc) = win.document()
    {
        // visibilitychange -> Hidden or Visible
        let ctx1 = ctx.clone();
        let document_clone = doc.clone();
        let on_vis = Closure::wrap(Box::new(move || {
            // Read document.visibilityState via JS to avoid gated web-sys features
            use web_sys::js_sys::Reflect;
            use web_sys::wasm_bindgen::JsValue;
            let doc_js: &JsValue = document_clone.as_ref();
            let visible_now = Reflect::get(doc_js, &JsValue::from_str("visibilityState"))
                .ok()
                .and_then(|v: JsValue| v.as_string())
                .map(|s| s == "visible")
                .unwrap_or(true);
            PENDING.store(if visible_now { 2 } else { 1 }, Ordering::SeqCst);
            debug!("visibility change: {}", visible_now);
            ctx1.request_repaint();
        }) as Box<dyn FnMut()>);
        doc.add_event_listener_with_callback("visibilitychange", on_vis.as_ref().unchecked_ref())
            .ok();
        on_vis.forget();

        // pageshow -> PageShow (covers bfcache restores)
        let ctx2 = ctx.clone();
        let on_pageshow = Closure::wrap(Box::new(move || {
            PENDING.store(3, Ordering::SeqCst);
            ctx2.request_repaint();
        }) as Box<dyn FnMut()>);
        doc.add_event_listener_with_callback("pageshow", on_pageshow.as_ref().unchecked_ref()).ok();
        on_pageshow.forget();
    }
}

impl Grooph {
    pub(super) fn handle_visibility_change(&mut self) {
        if !is_mobile_browser() {
            return;
        }
        if let Some(ev) = take_pending_visibility_event() {
            match ev {
                VisibilityEvent::Hidden => {
                    // going hidden
                    self.player_state = PlayerState::Stopped;
                    self.audio = None;
                }
                VisibilityEvent::Visible | VisibilityEvent::PageShow => {
                    // returning visible: drop audio to force clean re-init
                    self.audio = None;
                }
            }
        }
    }

    pub(super) fn acquire_wake_lock(&self) {
        let wake_lock_store = self.wake_lock.clone();
        if wake_lock_store.borrow().is_some() {
            return;
        }

        wasm_bindgen_futures::spawn_local(async move {
            let window = match web_sys::window() {
                Some(w) => w,
                None => return,
            };
            let navigator = window.navigator();
            let wake_lock = navigator.wake_lock();

            match JsFuture::from(wake_lock.request(WakeLockType::Screen)).await {
                Ok(sentinel_js) => {
                    use wasm_bindgen::JsCast;
                    let sentinel: WakeLockSentinel = sentinel_js.unchecked_into();
                    *wake_lock_store.borrow_mut() = Some(sentinel);
                    debug!("Wake lock acquired");
                }
                Err(e) => {
                    error!("Failed to request wake lock: {:?}", e);
                }
            }
        });
    }

    pub(super) fn release_wake_lock(&self) {
        let wake_lock_store = self.wake_lock.clone();
        if let Some(sentinel) = wake_lock_store.borrow_mut().take() {
            wasm_bindgen_futures::spawn_local(async move {
                let _ = JsFuture::from(sentinel.release()).await;
                debug!("Wake lock released");
            });
        }
    }
}

fn is_mobile_browser() -> bool {
    // Cached detection result
    *IS_MOBILE.get_or_init(|| {
        let Some(win) = web_sys::window() else { return false };
        let nav = win.navigator();

        // User-Agent heuristic
        let ua = nav.user_agent().unwrap_or_default().to_lowercase();
        let is_ios = ua.contains("iphone") || ua.contains("ipad") || ua.contains("ipod");
        let is_android = ua.contains("android");

        // Touch capability heuristic: navigator.maxTouchPoints > 0
        let has_touch = {
            use web_sys::js_sys::Reflect;
            use web_sys::wasm_bindgen::JsValue;
            let nav_js: &JsValue = nav.as_ref();
            Reflect::get(nav_js, &JsValue::from_str("maxTouchPoints"))
                .ok()
                .and_then(|v| v.as_f64())
                .map(|n| n > 0.0)
                .unwrap_or(false)
        };

        // Consider mobile if UA says iOS/Android, or if it has touch and UA contains "mobile"
        // This avoids flagging touch-enabled desktop devices as mobile.
        let is_mobile_token = ua.contains("mobile");
        is_ios || is_android || (has_touch && is_mobile_token)
    })
}
