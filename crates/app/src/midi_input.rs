//! The MIDI input row of the More menu and its picker: which connected
//! device (an e-drum kit, a pad, a keyboard) the play-along listens to.

use crate::Grooph;
use crate::pickers::{ROW_HEIGHT, choice_grid, wide_button};
use eframe::egui;
use eframe::egui::{Id, Ui};
use log::warn;

impl Grooph {
    /// The connected port's name for the More menu's row, or "Off".
    pub(crate) fn midi_input_label(&self) -> String {
        let midi = &self.playback_ctl.midi;
        let Some(input) = midi.input.as_ref() else {
            return "Unavailable".to_owned();
        };
        if !input.is_connected() {
            return "Off".to_owned();
        }
        midi.selected_port_id
            .as_ref()
            .and_then(|id| input.find_port_index_by_id(id))
            .and_then(|idx| midi.available_ports.get(idx))
            .cloned()
            .unwrap_or_else(|| "Connected".to_owned())
    }

    /// Lists the ports again, e.g. after a device was plugged in, and
    /// reconnects the chosen one if it is back.
    pub(crate) fn refresh_midi_ports(&mut self) {
        let midi = &mut self.playback_ctl.midi;
        let Some(input) = midi.input.as_mut() else {
            return;
        };
        if let Err(e) = input.refresh_ports() {
            warn!("Failed to refresh MIDI input ports: {}", e);
        }
        match input.available_ports() {
            Ok(ports) => midi.available_ports = ports,
            Err(e) => warn!("Failed to get MIDI input ports: {}", e),
        }
        if let Some(port_id) = midi.selected_port_id.as_ref()
            && let Some(idx) = input.find_port_index_by_id(port_id)
            && !input.is_connected()
            && let Err(e) = input.connect(idx)
        {
            warn!("Failed to reconnect MIDI input: {}", e);
        }
    }

    /// Off or one of the ports, one per row, and a button to look for new
    /// devices. Returns whether a port was picked and connected (which closes the
    /// picker); a port that fails to connect keeps it open with a note.
    pub(crate) fn midi_input_choices(&mut self, ui: &mut Ui) -> bool {
        ui.weak(
            "Play along on an e-drum kit, a pad or a keyboard to see how early or late you are.",
        );
        ui.add_space(8.0);
        let midi = &mut self.playback_ctl.midi;
        let Some(input) = midi.input.as_mut() else {
            ui.label("MIDI input is not available here.");
            return false;
        };
        let current = input
            .is_connected()
            .then(|| midi.selected_port_id.as_ref().and_then(|id| input.find_port_index_by_id(id)))
            .flatten();
        let options: Vec<_> = std::iter::once((None, "Off".to_owned()))
            .chain(midi.available_ports.iter().enumerate().map(|(i, name)| (Some(i), name.clone())))
            .collect();
        // Shown until the next pick or search: why the last pick did not connect.
        let failed_id = Id::unique("midi_connect_failed");
        let mut close = false;
        // Many ports (virtual ones, a DAW's) scroll, so the buttons below
        // stay on screen.
        let picked = egui::ScrollArea::vertical()
            .id_salt("midi_ports")
            .max_height(320.0)
            .show(ui, |ui| choice_grid(ui, &options, current, 1))
            .inner;
        match picked {
            Some(None) => {
                let _ = input.disconnect();
                midi.selected_port_id = None;
                ui.data_mut(|d| d.remove::<String>(failed_id));
                close = true;
            }
            Some(Some(idx)) => match input.connect(idx) {
                Ok(()) => {
                    midi.selected_port_id = input.port_id(idx);
                    ui.data_mut(|d| d.remove::<String>(failed_id));
                    close = true;
                }
                Err(e) => {
                    // `connect` closed the old connection first, so nothing
                    // is connected now; forget the old port too.
                    warn!("Failed to connect MIDI input: {}", e);
                    midi.selected_port_id = None;
                    let name = midi.available_ports[idx].clone();
                    ui.data_mut(|d| d.insert_temp(failed_id, name));
                }
            },
            None => {}
        }
        if let Some(name) = ui.data(|d| d.get_temp::<String>(failed_id)) {
            ui.add_space(4.0);
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("Could not connect to {name}. Is it still plugged in?"),
            );
        }
        if midi.available_ports.is_empty() {
            ui.add_space(4.0);
            ui.weak("No device found. Connect one and look again.");
        }
        ui.add_space(8.0);
        if wide_button(ui, "🔄  Look for devices", 17.0, ROW_HEIGHT).clicked() {
            ui.data_mut(|d| d.remove::<String>(failed_id));
            self.refresh_midi_ports();
        }
        close
    }
}
