//! A short notice at the bottom of the screen, gone after a few seconds:
//! what an action did not do ("This bar stayed as it was").

use crate::Grooph;
use eframe::egui::{self, Align2, Frame, Id, Margin, Order, RichText, Ui, vec2};

/// How long a notice stays visible.
const NOTICE_S: f64 = 3.0;

/// A notice to show, and the UI time it came up (set when it is first shown,
/// so one given outside a frame shows for as long).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Notice {
    pub(crate) text: String,
    shown_at: Option<f64>,
}

impl Grooph {
    /// Show `text` as a short notice, in place of the one shown.
    pub(crate) fn notify(&mut self, text: impl Into<String>) {
        self.ui.dialogs.notice = Some(Notice { text: text.into(), shown_at: None });
    }

    /// The notice, above everything else, until its time is up. Keeps
    /// repainting while it shows, so it goes away on time.
    pub(crate) fn notice_toast(&mut self, ui: &Ui) {
        let Some(notice) = &mut self.ui.dialogs.notice else {
            return;
        };
        let now = ui.input(|i| i.time);
        let since = *notice.shown_at.get_or_insert(now);
        if now - since >= NOTICE_S {
            self.ui.dialogs.notice = None;
            return;
        }
        ui.ctx().request_repaint_after_secs((NOTICE_S - (now - since)) as f32);
        // Above the tab bar and the editor's grid, where the thumb is not.
        let bottom = ui.ctx().content_rect().height() * 0.25;
        egui::Area::new(Id::unique("notice"))
            .order(Order::Tooltip)
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -bottom))
            .interactable(false)
            .show(ui.ctx(), |ui| {
                Frame::popup(ui.style()).inner_margin(Margin::symmetric(16, 10)).show(ui, |ui| {
                    ui.label(RichText::new(&notice.text).size(17.0));
                });
            });
    }
}
