//! The MIDI input row of the More menu and its picker: which connected
//! device (an e-drum kit, a pad, a keyboard) the play-along listens to, and
//! the way to its drum map (see `crate::drum_map_list`).

use crate::Grooph;
use crate::drum_map::device_key;
use crate::pickers::{ROW_HEIGHT, choice_grid, dialog_scroll_height, wide_button};
use crate::shell::value_row;
use eframe::egui;
use eframe::egui::{Id, Ui};
use log::warn;

impl Grooph {
    /// The connected port's name for the More menu's row, or "Off".
    pub(crate) fn midi_input_label(&self) -> String {
        let Some(input) = self.playback_ctl.midi.input.as_ref() else {
            return "Unavailable".to_owned();
        };
        if !input.is_connected() {
            return "Off".to_owned();
        }
        self.connected_midi_port().unwrap_or("Connected").to_owned()
    }

    /// Lists the ports again, e.g. after a device was plugged in, and
    /// reconnects the chosen one if it is back.
    pub(crate) fn refresh_midi_ports(&mut self) {
        let midi = &mut self.playback_ctl.midi;
        let port_id = &self.playback_ctl.devices.midi_port_id;
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
        if let Some(port_id) = port_id.as_ref()
            && let Some(idx) = input.find_port_index_by_id(port_id)
            && !input.is_connected()
            && let Err(e) = input.connect(idx)
        {
            warn!("Failed to reconnect MIDI input: {}", e);
        }
    }

    /// The connected port's name; `None` while nothing is connected.
    pub(crate) fn connected_midi_port(&self) -> Option<&str> {
        let midi = &self.playback_ctl.midi;
        let input = midi.input.as_ref().filter(|input| input.is_connected())?;
        let idx = input.find_port_index_by_id(self.playback_ctl.devices.midi_port_id.as_ref()?)?;
        midi.available_ports.get(idx).map(String::as_str)
    }

    /// Off or one of the ports, one per row, the connected device's drum map
    /// under it, the saved drum maps of devices not plugged in, and a
    /// button to look for new devices. Returns whether the picker closes:
    /// after Off only. A picked port that connects stays picked, with its
    /// drum map row; one that fails to connect shows a note.
    pub(crate) fn midi_input_choices(&mut self, ui: &mut Ui) -> bool {
        ui.weak(
            "Play along on an e-drum kit, a pad or a keyboard to see how early or late you are.",
        );
        ui.add_space(8.0);
        let midi = &self.playback_ctl.midi;
        let Some(input) = midi.input.as_ref() else {
            ui.label("MIDI input is not available here.");
            return false;
        };
        let port_id = &self.playback_ctl.devices.midi_port_id;
        let current = input
            .is_connected()
            .then(|| port_id.as_ref().and_then(|id| input.find_port_index_by_id(id)))
            .flatten();
        // Shown until the next pick or search: why the last pick did not connect.
        let failed_id = Id::unique("midi_connect_failed");
        let mut close = false;
        // Many ports (virtual ones, a DAW's) scroll, so the buttons below
        // stay on screen.
        let picked = egui::ScrollArea::vertical()
            .id_salt("midi_ports")
            .max_height(dialog_scroll_height(ui))
            .show(ui, |ui| self.midi_device_rows(ui, current))
            .inner;
        let midi = &mut self.playback_ctl.midi;
        let port_id = &mut self.playback_ctl.devices.midi_port_id;
        let Some(input) = midi.input.as_mut() else {
            return false;
        };
        match picked {
            Some(None) => {
                let _ = input.disconnect();
                *port_id = None;
                ui.data_mut(|d| d.remove::<String>(failed_id));
                close = true;
            }
            Some(Some(idx)) => match input.connect(idx) {
                Ok(()) => {
                    *port_id = input.port_id(idx);
                    ui.data_mut(|d| d.remove::<String>(failed_id));
                }
                Err(e) => {
                    // `connect` closed the old connection first, so nothing
                    // is connected now; forget the old port too.
                    warn!("Failed to connect MIDI input: {}", e);
                    *port_id = None;
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

    /// The rows of the MIDI input picker, with port `connected` connected:
    /// Off and the ports, the *Drum map* row under the connected one, and
    /// the *Saved drum maps* of devices not plugged in. Returns the row
    /// picked: `Some(None)` for Off, `Some(Some(port))` for a port.
    pub(crate) fn midi_device_rows(
        &mut self,
        ui: &mut Ui,
        connected: Option<usize>,
    ) -> Option<Option<usize>> {
        let ports = self.playback_ctl.midi.available_ports.clone();
        let mut picked = None;
        let mut open_map = None;
        if let Some(p) = choice_grid(ui, &[(None, "Off".to_owned())], connected, 1) {
            picked = Some(p);
        }
        for (i, name) in ports.iter().enumerate() {
            if let Some(p) = choice_grid(ui, &[(Some(i), name.clone())], connected, 1) {
                picked = Some(p);
            }
            if connected == Some(i) {
                let key = device_key(name);
                let kind = if self.playback_ctl.devices.drum_maps.contains_key(key) {
                    "Custom"
                } else {
                    "Built-in"
                };
                if value_row(ui, "     Drum map", kind).clicked() {
                    open_map = Some(key.to_owned());
                }
            }
        }
        self.saved_drum_maps(ui, &ports);
        if let Some(key) = open_map {
            self.open_drum_map(key);
        }
        picked
    }

    /// The drum maps of devices not plugged in, each with Delete, which
    /// asks first. Hidden when there are none.
    fn saved_drum_maps(&mut self, ui: &mut Ui, ports: &[String]) {
        let saved: Vec<String> = self
            .playback_ctl
            .devices
            .drum_maps
            .keys()
            .filter(|key| ports.iter().all(|name| device_key(name) != key.as_str()))
            .cloned()
            .collect();
        if saved.is_empty() {
            return;
        }
        ui.add_space(12.0);
        ui.strong("Saved drum maps");
        ui.weak("Of devices not plugged in.");
        let dialogs = &mut self.ui.dialogs;
        for key in saved {
            ui.horizontal(|ui| {
                if dialogs.delete_drum_map.as_ref() == Some(&key) {
                    ui.label(format!("Delete the drum map of {key}?"));
                    if ui.button("Delete").clicked() {
                        self.playback_ctl.devices.drum_maps.remove(&key);
                        dialogs.delete_drum_map = None;
                    }
                    if ui.button("Cancel").clicked() {
                        dialogs.delete_drum_map = None;
                    }
                } else {
                    ui.label(&key);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Delete").clicked() {
                            dialogs.delete_drum_map = Some(key.clone());
                        }
                    });
                }
            });
        }
    }
}
