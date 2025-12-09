use crate::Grooph;
use crate::app::PlayerState;
use eframe::egui;
use eframe::egui::scroll_area::{ScrollBarVisibility, ScrollSource};
use eframe::egui::{Align, Button, Direction, Image, Layout, Widget, include_image};

impl Grooph {
    pub(super) fn main_menu(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu").show_separator_line(false).show(ctx, |ui| {
            egui::ScrollArea::horizontal()
                .scroll_source(ScrollSource::ALL)
                .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    let layout = Layout::from_main_dir_and_cross_align(
                        Direction::LeftToRight,
                        Align::Center,
                    )
                    .with_cross_justify(true);

                    ui.with_layout(layout, |ui| {
                        ui.toggle_value(&mut self.edit_mode_enabled, "🖊");

                        // Playback controls
                        let button_label =
                            if self.player_state == PlayerState::Playing { "⏹" } else { "⏵" };
                        if Button::new(button_label)
                            .selected(self.player_state == PlayerState::Playing)
                            .ui(ui)
                            .clicked()
                        {
                            self.toggle_playback();
                        }
                        let bpm_editor = egui::DragValue::new(&mut self.bpm)
                            .prefix("BPM: ")
                            .range(20..=300)
                            .speed(0.03);
                        let bpm_editor_resp = bpm_editor.ui(ui);
                        if bpm_editor_resp.clicked() {
                            ui.memory_mut(|mem| mem.surrender_focus(bpm_editor_resp.id))
                        }

                        ui.separator();
                        // ui.selectable_label(
                        //     false,
                        //     Image::new(include_image!("../../assets/metronome_dark.svg"))
                        //         .tint(ui.style().visuals.text_color()),
                        // )
                        // .clicked();
                        ui.toggle_value(&mut self.show_mixer, "🔈");
                        ui.toggle_value(&mut self.show_settings, "⚙");
                        ui.toggle_value(&mut self.show_info, "?");
                    });
                });
        });
    }
}
