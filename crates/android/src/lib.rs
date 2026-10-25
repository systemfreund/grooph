//! Android entry point: NativeActivity calls `android_main`, which runs the
//! same eframe app as the desktop build. The window is a phone, so the app
//! packs the staff for a narrow screen on its own.

#[cfg(target_os = "android")]
mod activity;
#[cfg(target_os = "android")]
mod insets;

#[cfg(target_os = "android")]
#[expect(unsafe_code, reason = "NativeActivity looks the entry point up by name")]
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
    let activity = activity::Activity { app: app.clone() };
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
        Box::new(|cc| {
            let mut app = Grooph::new(cc);
            app.set_host(Box::new(activity.clone()));
            Ok(Box::new(AndroidApp { app, insets, activity, typing: false }))
        }),
    );
    if let Err(err) = result {
        log::error!("grooph stopped: {err}");
    }
}

/// The app plus what only Android needs: keeping clear of the system bars,
/// the back button and going to the background.
#[cfg(target_os = "android")]
struct AndroidApp {
    app: grooph_app::Grooph,
    insets: insets::SystemBars,
    activity: activity::Activity,
    /// A text field had the focus last frame, so the keyboard is open.
    typing: bool,
}

#[cfg(target_os = "android")]
impl eframe::App for AndroidApp {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        use eframe::egui::{Key, Modifiers};

        // The back button arrives as BrowserBack. It closes what is open in
        // the app; with nothing open it leaves the app, as elsewhere.
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::BrowserBack)) && !self.app.back() {
            self.activity.move_to_back();
        }
        // The root Ui spans the whole screen; keep the app inside the part the
        // system bars leave free (the safe area set in `raw_input_hook`).
        let content = ui.ctx().content_rect();
        ui.scope_builder(eframe::egui::UiBuilder::new().max_rect(content), |ui| {
            self.app.ui(ui, frame);
        });

        // winit's request for the keyboard does nothing; open it when a text
        // field gets the focus, and again on a tap into it after the
        // keyboard was closed with the back button.
        let typing = ui.ctx().output(|o| o.ime.is_some());
        let tapped = ui.input(|i| i.pointer.any_released());
        if typing != self.typing || typing && tapped {
            self.typing = typing;
            self.activity.soft_keyboard(typing);
            self.insets.follow_keyboard();
        }
    }

    /// eframe saves when the window goes away, the only notice it passes on
    /// that the app went to the background; it also saves every 30 s.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.activity.window_hidden() {
            self.app.hidden();
        }
        self.app.save(storage);
    }

    /// Fills the strips behind the system bars like the app's panels.
    fn clear_color(&self, visuals: &eframe::egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    /// NativeActivity draws under the status bar and the navigation bar;
    /// their size goes to egui as safe-area insets, which shrink
    /// `content_rect`.
    fn raw_input_hook(
        &mut self,
        ctx: &eframe::egui::Context,
        raw_input: &mut eframe::egui::RawInput,
    ) {
        raw_input.safe_area_insets = Some(self.insets.safe_area(ctx));
    }
}
