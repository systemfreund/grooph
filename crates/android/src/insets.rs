//! Size of the status bar and navigation bar, read from the activity's window
//! through JNI.

use crate::activity::{decor_view, with_activity};
use eframe::egui::{Context, SafeAreaInsets};
use eframe::epaint::MarginF32;
use std::time::{Duration, Instant};
use winit::platform::android::activity::AndroidApp;

/// How often the insets are read again (rotation, bars shown or hidden).
const REFRESH: Duration = Duration::from_secs(1);

/// How long the insets are read on every frame after the keyboard is asked
/// to open or close.
const KEYBOARD_ANIMATION: Duration = Duration::from_millis(800);

/// System bar insets in physical pixels, refreshed now and then.
pub(crate) struct SystemBars {
    app: AndroidApp,
    /// Left, top, right, bottom.
    px: [i32; 4],
    read_at: Option<Instant>,
    /// Read on every frame until then: the keyboard is sliding in or out.
    follow_until: Option<Instant>,
}

impl SystemBars {
    pub(crate) fn new(app: AndroidApp) -> Self {
        Self { app, px: [0; 4], read_at: None, follow_until: None }
    }

    /// Follow the insets closely for a moment: the on-screen keyboard is
    /// opening or closing, and the insets include it.
    pub(crate) fn follow_keyboard(&mut self) {
        self.follow_until = Some(Instant::now() + KEYBOARD_ANIMATION);
    }

    /// The insets in points.
    pub(crate) fn safe_area(&mut self, ctx: &Context) -> SafeAreaInsets {
        let following = self.follow_until.is_some_and(|until| Instant::now() < until);
        if following {
            ctx.request_repaint();
        }
        if following || self.read_at.is_none_or(|at| at.elapsed() >= REFRESH) {
            self.read_at = Some(Instant::now());
            match read_insets(&self.app) {
                Some(px) => {
                    if px != self.px {
                        log::info!("System bar insets (left, top, right, bottom): {px:?} px");
                    }
                    self.px = px;
                }
                None => log::warn!("Could not read the system bar insets"),
            }
        }
        let pixels_per_point = ctx.pixels_per_point();
        let [left, top, right, bottom] = self.px.map(|px| px as f32 / pixels_per_point);
        SafeAreaInsets(MarginF32 { left, right, top, bottom })
    }
}

/// `activity.getWindow().getDecorView().getRootWindowInsets()` as
/// `[left, top, right, bottom]`; `None` before the window is laid out.
fn read_insets(app: &AndroidApp) -> Option<[i32; 4]> {
    with_activity(app, |env, activity| {
        let decor = decor_view(env, activity)?;
        let insets = env
            .call_method(&decor, "getRootWindowInsets", "()Landroid/view/WindowInsets;", &[])?
            .l()?;
        if insets.is_null() {
            return Ok(None);
        }
        // The system window insets cover the status bar, the navigation bar
        // and a display cutout next to them (deprecated in API 30, still
        // reported).
        let mut side = |name: &str| env.call_method(&insets, name, "()I", &[])?.i();
        Ok(Some([
            side("getSystemWindowInsetLeft")?,
            side("getSystemWindowInsetTop")?,
            side("getSystemWindowInsetRight")?,
            side("getSystemWindowInsetBottom")?,
        ]))
    })
    .flatten()
}
