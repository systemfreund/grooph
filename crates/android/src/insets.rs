//! Size of the status bar and navigation bar, read from the activity's window
//! through JNI.

use crate::activity::{decor_view, with_activity};
use eframe::egui::SafeAreaInsets;
use eframe::epaint::MarginF32;
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
                Some(px) => {
                    if px != self.px {
                        log::info!("System bar insets (left, top, right, bottom): {px:?} px");
                    }
                    self.px = px;
                }
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
