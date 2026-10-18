//! Size of the status bar and navigation bar, read from the activity's window
//! through JNI.

use eframe::egui::SafeAreaInsets;
use eframe::epaint::MarginF32;
use jni::JNIEnv;
use jni::objects::JObject;
use std::time::{Duration, Instant};
use winit::platform::android::activity::AndroidApp;

/// How often the insets are read again (rotation, bars shown or hidden).
const REFRESH: Duration = Duration::from_secs(1);

/// System bar insets in physical pixels, refreshed now and then.
pub(crate) struct SystemBars {
    app: AndroidApp,
    /// Left, top, right, bottom.
    px: [i32; 4],
    read_at: Option<Instant>,
}

impl SystemBars {
    pub(crate) fn new(app: AndroidApp) -> Self {
        Self { app, px: [0; 4], read_at: None }
    }

    /// The insets in points.
    pub(crate) fn safe_area(&mut self, pixels_per_point: f32) -> SafeAreaInsets {
        if self.read_at.is_none_or(|at| at.elapsed() >= REFRESH) {
            self.read_at = Some(Instant::now());
            match read_insets(&self.app) {
                Some(px) => self.px = px,
                None => log::warn!("Could not read the system bar insets"),
            }
        }
        let [left, top, right, bottom] = self.px.map(|px| px as f32 / pixels_per_point);
        SafeAreaInsets(MarginF32 { left, right, top, bottom })
    }
}

/// `activity.getWindow().getDecorView().getRootWindowInsets()` as
/// `[left, top, right, bottom]`; `None` before the window is laid out.
fn read_insets(app: &AndroidApp) -> Option<[i32; 4]> {
    // SAFETY: both pointers come from android-activity and stay valid for as
    // long as the app runs; the activity is a global reference.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let insets = system_window_insets(&mut env, &activity);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    insets
}

fn system_window_insets(env: &mut JNIEnv, activity: &JObject) -> Option<[i32; 4]> {
    let window =
        env.call_method(activity, "getWindow", "()Landroid/view/Window;", &[]).ok()?.l().ok()?;
    let decor =
        env.call_method(&window, "getDecorView", "()Landroid/view/View;", &[]).ok()?.l().ok()?;
    let insets = env
        .call_method(&decor, "getRootWindowInsets", "()Landroid/view/WindowInsets;", &[])
        .ok()?
        .l()
        .ok()?;
    if insets.is_null() {
        return None;
    }
    // The system window insets cover the status bar, the navigation bar and
    // a display cutout next to them (deprecated in API 30, still reported).
    let mut side = |name: &str| env.call_method(&insets, name, "()I", &[]).ok()?.i().ok();
    Some([
        side("getSystemWindowInsetLeft")?,
        side("getSystemWindowInsetTop")?,
        side("getSystemWindowInsetRight")?,
        side("getSystemWindowInsetBottom")?,
    ])
}
