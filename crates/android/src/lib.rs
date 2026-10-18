//! Android entry point: NativeActivity calls `android_main`, which runs the
//! same eframe app as the desktop build. The window is a phone, so the app
//! picks the compact layout on its own.

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
    let options =
        eframe::NativeOptions { android_app: Some(app), persistence_path, ..Default::default() };
    if let Err(err) =
        eframe::run_native("grooph", options, Box::new(|cc| Ok(Box::new(Grooph::new(cc)))))
    {
        log::error!("grooph stopped: {err}");
    }
}
