//! What the tests need from eframe to run the app without a window.
//!
//! eframe offers it only as `_new_kittest`, hidden from its docs and outside
//! its stable API, so it is called here alone: when an eframe release
//! changes or drops it, this is the one place to adapt (or to switch to
//! `egui_kittest`).

/// A creation context for [`crate::Grooph::new`] on `ctx`.
pub(crate) fn creation_context(ctx: egui::Context) -> eframe::CreationContext<'static> {
    eframe::CreationContext::_new_kittest(ctx)
}

// AccessKit, which the helpers below read, is off on Android (see the
// app's Cargo.toml): the tests using them run elsewhere.

/// One frame of `ui` on a phone-sized screen at UI time `time`, with
/// `events`; returns the labeled widgets and text it showed, each with its rect, as
/// assistive technology reads them (AccessKit), to find what to tap next.
#[cfg(not(target_os = "android"))]
pub(crate) fn frame(
    ctx: &egui::Context,
    time: f64,
    events: Vec<egui::Event>,
    mut ui: impl FnMut(&mut egui::Ui),
) -> Vec<(String, egui::Rect)> {
    use egui::{Pos2, RawInput, Rect, pos2, vec2};
    ctx.enable_accesskit();
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(390.0, 844.0));
    let input =
        RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() };
    let mut output = ctx.run_ui(input, |root| ui(root));
    output.textures_delta.clear();
    let Some(update) = output.platform_output.accesskit_update else {
        return Vec::new();
    };
    update
        .nodes
        .iter()
        .filter_map(|(_, node)| {
            // Text shows as a value, a button by its label.
            let (label, b) = (node.label().or(node.value())?, node.bounds()?);
            let rect =
                Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32));
            Some((label.to_owned(), rect))
        })
        .collect()
}

/// The events of a tap on the widget labeled `label` among `widgets` (see
/// [`frame`]); panics if none shows.
#[cfg(not(target_os = "android"))]
pub(crate) fn tap(widgets: &[(String, egui::Rect)], label: &str) -> Vec<egui::Event> {
    use egui::{Event, Modifiers, PointerButton};
    let Some((_, rect)) = widgets.iter().find(|(l, _)| l == label) else {
        let labels: Vec<_> = widgets.iter().map(|(l, _)| l.as_str()).collect();
        panic!("no {label:?} among {labels:?}");
    };
    let pos = rect.center();
    let button = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    vec![Event::PointerMoved(pos), button(true), button(false)]
}

/// Whether a widget labeled `label` shows among `widgets`.
#[cfg(not(target_os = "android"))]
pub(crate) fn shows(widgets: &[(String, egui::Rect)], label: &str) -> bool {
    widgets.iter().any(|(l, _)| l == label)
}
