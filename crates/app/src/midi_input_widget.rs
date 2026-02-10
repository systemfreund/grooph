use eframe::egui;
use eframe::emath::Align;
use egui::{Direction, Layout};
use grooph_midi::MidiInput;
use log::warn;

pub(crate) fn midi_input_widget(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash,
    refresh_label: &str,
    midi_input: &mut Option<MidiInput>,
    midi_input_ports: &mut Vec<String>,
    midi_selected_port_id: &mut Option<String>,
) {
    let mut do_refresh = false;

    let layout = Layout::from_main_dir_and_cross_align(Direction::LeftToRight, Align::Center)
        .with_cross_justify(true);

    ui.with_layout(layout, |ui| {
        if ui.button(refresh_label).on_hover_text("Refresh MIDI input ports").clicked() {
            do_refresh = true;
        }

        let Some(input) = midi_input.as_mut() else {
            ui.label("MIDI input unavailable.");
            return;
        };

        if do_refresh {
            refresh_midi_input_ports(input, midi_input_ports, midi_selected_port_id.as_ref());
        }

        let connected = input.is_connected();
        let selected_idx =
            midi_selected_port_id.as_ref().and_then(|id| input.find_port_index_by_id(id));
        let selected_text = if connected {
            selected_idx
                .and_then(|idx| midi_input_ports.get(idx))
                .cloned()
                .unwrap_or_else(|| "Connected".to_string())
        } else {
            "Disconnected".to_string()
        };

        let mut should_disconnect = false;
        let mut connect_port = None;

        egui::ComboBox::from_id_salt((id_source, "midi_input_port"))
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                if connected {
                    if ui.selectable_label(false, "Disconnect").clicked() {
                        should_disconnect = true;
                    }
                } else {
                    let _ = ui.selectable_label(false, "Disconnected");
                }

                for (idx, name) in midi_input_ports.iter().enumerate() {
                    if ui.selectable_label(selected_idx == Some(idx), name).clicked() {
                        connect_port = Some(idx);
                    }
                }
            });

        if should_disconnect {
            let _ = input.disconnect();
            *midi_selected_port_id = None;
        } else if let Some(port_index) = connect_port
            && input.connect(port_index).is_ok()
        {
            *midi_selected_port_id = input.port_id(port_index);
        }
    });
}

fn refresh_midi_input_ports(
    input: &mut MidiInput,
    midi_input_ports: &mut Vec<String>,
    selected_port_id: Option<&String>,
) {
    if let Err(e) = input.refresh_ports() {
        warn!("Failed to refresh MIDI input ports: {}", e);
    }
    match input.available_ports() {
        Ok(ports) => {
            *midi_input_ports = ports;
        }
        Err(e) => {
            warn!("Failed to get MIDI input ports: {}", e);
        }
    }

    if let Some(port_id) = selected_port_id
        && let Some(idx) = input.find_port_index_by_id(port_id)
        && !input.is_connected()
    {
        let _ = input.connect(idx);
    }
}
