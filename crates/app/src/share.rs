//! Sharing the working score or the metronome as a link: the share icon, the
//! system share sheet or clipboard, and the short "Link copied" notice.

use crate::{Grooph, platform};
use eframe::egui::{Color32, Painter, Rect, Shape, Stroke, Ui, pos2, vec2};
use grooph_link::{LinkContent, SharedLink};
use log::warn;

/// How long "Link copied" stays visible.
const LINK_COPIED_NOTICE_S: f64 = 2.0;

/// The iOS share symbol: an open box with an arrow pointing up out of it.
pub(crate) fn paint_share_icon(painter: &Painter, rect: Rect, color: Color32) {
    let c = rect.center() + vec2(0.0, rect.height() * 0.03);
    let s = rect.width() * 0.14;
    let stroke = Stroke::new(2.5_f32, color);
    let box_top = c.y - 0.3 * s;
    let box_bottom = c.y + 1.2 * s;
    painter.add(Shape::line(
        vec![
            pos2(c.x - 0.45 * s, box_top),
            pos2(c.x - s, box_top),
            pos2(c.x - s, box_bottom),
            pos2(c.x + s, box_bottom),
            pos2(c.x + s, box_top),
            pos2(c.x + 0.45 * s, box_top),
        ],
        stroke,
    ));
    let tip = pos2(c.x, c.y - 1.6 * s);
    painter.line_segment([pos2(c.x, c.y + 0.4 * s), tip], stroke);
    painter.line_segment([tip, tip + vec2(-0.5 * s, 0.5 * s)], stroke);
    painter.line_segment([tip, tip + vec2(0.5 * s, 0.5 * s)], stroke);
}

impl Grooph {
    /// Link to what is on screen: in the metronome its meter, tempo, click
    /// bar and counter rhythm; else the working score at the current tempo
    /// and swing, with the click bar of its first meter if that is not the
    /// default.
    pub(crate) fn share_link_url(&self) -> Option<String> {
        let url = self.shared_link().to_url(&platform::share_base_url());
        if url.is_none() {
            warn!("The score cannot be written as a link");
        }
        url
    }

    /// What [`Self::share_link_url`] links to.
    pub(crate) fn shared_link(&self) -> SharedLink {
        let swing = self.playback_ctl.audio_cfg.swing;
        let click = &self.playback_ctl.audio_cfg.click;
        let (content, meter) = if self.metronome_mode() {
            (LinkContent::Metronome(self.metronome_meter()), Some(self.metronome_meter()))
        } else {
            let score = &self.editor.score;
            let meter = score.measures().first().map(|m| m.time_signature());
            (LinkContent::Score(score.clone()), meter)
        };
        SharedLink {
            bpm: Some(self.playback_ctl.bpm),
            swing: (!swing.is_straight()).then_some(swing),
            content: Some(content),
            click: meter.and_then(|ts| click.custom_for(ts)).cloned(),
            counter: self.metronome_mode().then(|| self.counter_bar().cloned()).flatten(),
            // Written as `poly` while its clicks are the ratio's.
            poly: self.metronome_mode().then(|| self.counter_ratio().map(|(r, _)| r)).flatten(),
        }
    }

    /// Open the share sheet with a link to the score, or copy the link where
    /// there is no share sheet.
    pub(crate) fn share_link(&mut self, ui: &Ui) {
        let Some(url) = self.share_link_url() else {
            return;
        };
        let shared = match &self.ui.host {
            Some(host) => host.share(&url),
            None => platform::open_share_sheet(&url),
        };
        if !shared {
            ui.ctx().copy_text(url);
            self.ui.link_copied_at = Some(ui.input(|i| i.time));
        }
    }

    /// Whether a link was copied recently enough to say so. Keeps repainting
    /// while the notice shows, so it goes away on time.
    pub(crate) fn link_copied_recently(&self, ui: &Ui) -> bool {
        let now = ui.input(|i| i.time);
        let recent = self.ui.link_copied_at.is_some_and(|at| now - at < LINK_COPIED_NOTICE_S);
        if recent {
            ui.ctx().request_repaint_after_secs(0.5);
        }
        recent
    }
}
