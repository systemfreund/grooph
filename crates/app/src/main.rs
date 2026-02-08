#![allow(dead_code)]

use grooph_app::Grooph;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use eframe::egui;
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native("grooph.app", options, Box::new(|cc| Ok(Box::new(Grooph::new(cc)))))
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    // Redirect `log` message to `console.log` and friends:
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window().expect("No window").document().expect("No document");

        let canvas = document
            .get_element_by_id("grooph-canvas")
            .expect("Failed to find canvas grooph-canvas")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("grooph-canvas was not a HtmlCanvasElement");

        let start_result = eframe::WebRunner::new()
            .start(canvas, web_options, Box::new(|cc| Ok(Box::new(Grooph::new(cc)))))
            .await;

        // Remove the loading text and spinner:
        if let Some(loading_text) = document.get_element_by_id("loading_text") {
            match start_result {
                Ok(_) => {
                    loading_text.remove();
                }
                Err(e) => {
                    loading_text.set_inner_html(
                        "<p> The app has crashed. See the developer console for details. </p>",
                    );
                    panic!("Failed to start eframe: {e:?}");
                }
            }
        }
    });
}
