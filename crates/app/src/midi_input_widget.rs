use eframe::egui;
use eframe::emath::Align;
use egui::{Direction, Layout};

#[derive(Debug, Default)]
pub(crate) struct MidiInputUiResponse {
    pub refresh: bool,
    pub disconnect: bool,
    pub connect_port: Option<usize>,
}

pub(crate) struct MidiInputWidgetState<'a> {
    pub available: bool,
    pub connected: bool,
    pub ports: &'a [String],
    pub selected_port_index: Option<usize>,
}

pub(crate) fn midi_input_widget(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash,
    refresh_label: &str,
    state: MidiInputWidgetState<'_>,
) -> MidiInputUiResponse {
    let mut response = MidiInputUiResponse::default();

    // ui.with_layout(layout, |ui| {
        if ui
            .button(refresh_label)
            .on_hover_text("Refresh MIDI input ports")
            .clicked()
        {
            response.refresh = true;
        }

        if !state.available {
            ui.label("MIDI input unavailable.");
            return response;
        }

        let selected_text = if state.connected {
            state
                .selected_port_index
                .and_then(|idx| state.ports.get(idx))
                .cloned()
                .unwrap_or_else(|| "Connected".to_string())
        } else {
            "Disconnected".to_string()
        };

        egui::ComboBox::from_id_salt((id_source, "midi_input_port"))
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                if state.connected {
                    if ui.selectable_label(false, "Disconnect").clicked() {
                        response.disconnect = true;
                    }
                } else {
                    let _ = ui.selectable_label(false, "Disconnected");
                }

                for (idx, name) in state.ports.iter().enumerate() {
                    if ui
                        .selectable_label(state.selected_port_index == Some(idx), name)
                        .clicked()
                    {
                        response.connect_port = Some(idx);
                    }
                }
            });

    // });

    response
}
