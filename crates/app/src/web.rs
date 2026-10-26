use crate::platform::TapKind;
use crate::platform::VisibilityEvent;
use eframe::egui::{Context, Rect, pos2, vec2};
use log::{debug, error};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::OnceLock;
use wasm_bindgen_futures::JsFuture;
use web_sys::wasm_bindgen::JsCast as _;
use web_sys::wasm_bindgen::closure::Closure;
use web_sys::{WakeLockSentinel, WakeLockType};

static IS_MOBILE: OnceLock<bool> = OnceLock::new();

thread_local! {
    /// Visibility events since the last frame, oldest first: a hide and a
    /// show between two frames must both be handled.
    static PENDING_VISIBILITY: RefCell<Vec<VisibilityEvent>> = const { RefCell::new(Vec::new()) };
}

fn push_visibility_event(event: VisibilityEvent) {
    PENDING_VISIBILITY.with(|q| q.borrow_mut().push(event));
}

fn take_pending_visibility_events() -> Vec<VisibilityEvent> {
    PENDING_VISIBILITY.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

/// The visual viewport, which an on-screen keyboard shortens, over the
/// canvas, in points (CSS pixels divided by egui's `zoom`).
pub(crate) fn visible_canvas_rect(zoom: f32) -> Option<Rect> {
    let window = web_sys::window()?;
    let viewport = window.visual_viewport()?;
    let canvas = window.document()?.get_element_by_id("grooph-canvas")?.get_bounding_client_rect();
    let min = pos2(
        (viewport.offset_left() - canvas.left()) as f32,
        (viewport.offset_top() - canvas.top()) as f32,
    );
    let size = vec2(viewport.width() as f32, viewport.height() as f32);
    Some(Rect::from_min_size(min / zoom, size / zoom))
}

/// Paint again when the visual viewport changes, as when the on-screen
/// keyboard comes up: egui sees no event of its own then.
fn install_viewport_listeners(ctx: Context) {
    let Some(viewport) = web_sys::window().and_then(|w| w.visual_viewport()) else {
        return;
    };
    let on_change = Closure::wrap(Box::new(move || ctx.request_repaint()) as Box<dyn FnMut()>);
    for event in ["resize", "scroll"] {
        viewport.add_event_listener_with_callback(event, on_change.as_ref().unchecked_ref()).ok();
    }
    on_change.forget();
}

fn install_visibility_listeners(ctx: Context) {
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
            push_visibility_event(if visible_now {
                VisibilityEvent::Visible
            } else {
                VisibilityEvent::Hidden
            });
            debug!("visibility change: {}", visible_now);
            ctx1.request_repaint();
        }) as Box<dyn FnMut()>);
        doc.add_event_listener_with_callback("visibilitychange", on_vis.as_ref().unchecked_ref())
            .ok();
        on_vis.forget();

        // pageshow -> PageShow (covers bfcache restores)
        let ctx2 = ctx.clone();
        let on_pageshow = Closure::wrap(Box::new(move || {
            push_visibility_event(VisibilityEvent::PageShow);
            ctx2.request_repaint();
        }) as Box<dyn FnMut()>);
        doc.add_event_listener_with_callback("pageshow", on_pageshow.as_ref().unchecked_ref()).ok();
        on_pageshow.forget();
    }
}

/// A raw key or pointer press with its DOM timestamp (seconds, same timeline
/// as `performance.now()`).
struct RawPress {
    /// Lowercased `KeyboardEvent.key`; `None` for pointer presses.
    key: Option<String>,
    time: f64,
}

/// Presses older than this are dropped; egui sees a press within a frame or two.
const RAW_PRESS_MAX_AGE_S: f64 = 0.5;

thread_local! {
    static RAW_PRESSES: RefCell<VecDeque<RawPress>> = const { RefCell::new(VecDeque::new()) };
}

fn push_raw_press(key: Option<String>, time: f64) {
    RAW_PRESSES.with(|q| {
        let mut q = q.borrow_mut();
        q.push_back(RawPress { key, time });
        while q.len() > 32 {
            q.pop_front();
        }
    });
}

/// Record key and pointer presses with their event timestamps. egui only sees
/// them at the next frame, which would add up to a frame of jitter to taps.
fn install_press_listeners() {
    let Some(win) = web_sys::window() else { return };
    let on_key = Closure::wrap(Box::new(move |e: web_sys::KeyboardEvent| {
        if !e.repeat() {
            push_raw_press(Some(e.key().to_lowercase()), e.time_stamp() / 1000.0);
        }
    }) as Box<dyn FnMut(_)>);
    win.add_event_listener_with_callback("keydown", on_key.as_ref().unchecked_ref()).ok();
    on_key.forget();

    let on_pointer = Closure::wrap(Box::new(move |e: web_sys::Event| {
        push_raw_press(None, e.time_stamp() / 1000.0);
    }) as Box<dyn FnMut(_)>);
    win.add_event_listener_with_callback("pointerdown", on_pointer.as_ref().unchecked_ref()).ok();
    on_pointer.forget();
}

/// DOM timestamp of the latest press matching `kind`; older presses of that
/// kind are consumed with it.
pub(crate) fn take_press_time(kind: TapKind, now: f64) -> Option<f64> {
    RAW_PRESSES.with(|q| {
        let mut q = q.borrow_mut();
        q.retain(|p| now - p.time <= RAW_PRESS_MAX_AGE_S);
        let matches = |p: &RawPress| match kind {
            TapKind::Key(key) => p.key.as_deref() == Some(key),
            TapKind::Pointer => p.key.is_none(),
        };
        let time = q.iter().rev().find(|p| matches(p)).map(|p| p.time)?;
        q.retain(|p| !matches(p));
        Some(time)
    })
}

pub(crate) struct PlatformRuntime {
    wake_lock: Rc<RefCell<Option<WakeLockSentinel>>>,
}

impl PlatformRuntime {
    pub(crate) fn new() -> Self {
        Self { wake_lock: Rc::new(RefCell::new(None)) }
    }

    pub(crate) fn install_listeners(&self, ctx: Context) {
        install_viewport_listeners(ctx.clone());
        install_visibility_listeners(ctx);
        install_press_listeners();
    }

    /// Visibility events since the last call, oldest first.
    pub(crate) fn take_visibility_events(&self) -> Vec<VisibilityEvent> {
        if !is_mobile_browser() {
            return Vec::new();
        }
        take_pending_visibility_events()
    }

    pub(crate) fn acquire_wake_lock(&self) {
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

    pub(crate) fn release_wake_lock(&self) {
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
