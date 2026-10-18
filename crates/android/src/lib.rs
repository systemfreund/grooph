//! Android entry point: NativeActivity calls `android_main`, which runs the
//! same eframe app as the desktop build. The window is a phone, so the app
//! picks the compact layout on its own.

#[cfg(target_os = "android")]
mod insets;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use grooph_app::Grooph;

    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("grooph"),
    );
    // eframe finds no settings folder on Android; keep them in the app's own
    // storage so presets, patterns and settings survive a restart.
    let persistence_path = app.internal_data_path().map(|dir| dir.join("grooph.ron"));
    let insets = insets::SystemBars::new(app.clone());
    let options = eframe::NativeOptions {
        android_app: Some(app),
        persistence_path,
        // OpenGL ES runs on every phone and in the emulator, where wgpu
        // fails to create a window surface (EGL BadAlloc).
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let result = eframe::run_native(
        "grooph",
        options,
        Box::new(|cc| Ok(Box::new(AndroidApp { app: Grooph::new(cc), insets }))),
    );
    if let Err(err) = result {
        log::error!("grooph stopped: {err}");
    }
}

/// The app plus what only Android needs: keeping clear of the system bars.
#[cfg(target_os = "android")]
struct AndroidApp {
    app: grooph_app::Grooph,
    insets: insets::SystemBars,
}

#[cfg(target_os = "android")]
impl eframe::App for AndroidApp {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.app.save(storage);
    }

    /// Fills the strips behind the system bars like the app's panels.
    fn clear_color(&self, visuals: &eframe::egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    /// NativeActivity draws under the status bar and the navigation bar;
    /// telling egui their size as safe-area insets keeps the UI out of them.
    fn raw_input_hook(
        &mut self,
        ctx: &eframe::egui::Context,
        raw_input: &mut eframe::egui::RawInput,
    ) {
        raw_input.safe_area_insets = Some(self.insets.safe_area(ctx.pixels_per_point()));
    }
}
