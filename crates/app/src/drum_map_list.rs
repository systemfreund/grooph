//! The drum map list (Play along → MIDI input → Drum map): which note of
//! the connected device plays which instrument and technique, learned by
//! picking a row and hitting the pad.
//!
//! The list shows the built-in table until the first change, which makes
//! the device's custom map a copy of it with the change applied (see
//! [`crate::drum_map::CustomDrumMap`]). While the list is open, the
//! device's NoteOns come here only: nothing is judged, even with playback
//! stopped, and no sound plays.

use crate::Grooph;
use crate::drum_map::{
    Combo, CustomDrumMap, DrumMapEntry, GM_DRUM_MAP, combo_label, lookup, notes_of, offered_combos,
};
use crate::pickers::{Picker, ROW_HEIGHT, dialog_scroll_height};
use eframe::egui::{self, Button, RichText, Ui, vec2};

/// How long a hit shows in the list (s).
const HIT_SHOWN_S: f64 = 1.2;

/// What a NoteOn showed in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListHit {
    /// The row the note plays.
    Row(Combo),
    /// The note plays nothing.
    Unmapped(u8),
}

/// The drum map list while it is open.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DrumMapList {
    /// The device key ([`crate::drum_map::device_key`]) of the map shown.
    pub(crate) device: String,
    /// The row the next NoteOn is learned for.
    pub(crate) listening: Option<Combo>,
    /// The last NoteOn and the UI time it came, shown for a moment.
    pub(crate) hit: Option<(ListHit, f64)>,
    /// *Reset to built-in* awaits its confirmation.
    pub(crate) confirm_reset: bool,
}

impl DrumMapList {
    pub(crate) fn new(device: String) -> Self {
        Self { device, listening: None, hit: None, confirm_reset: false }
    }
}

impl Grooph {
    /// Opens the drum map list of the device `key` from the MIDI input
    /// picker, which comes back after it. A run playing stops and is
    /// discarded: no "Your run" window, and no earlier one shows again.
    pub(crate) fn open_drum_map(&mut self, key: String) {
        self.playback_ctl.accuracy.tracker.discard_session();
        self.stop_transport();
        self.playback_ctl.accuracy.tracker.dismiss_summary();
        let dialogs = &mut self.ui.dialogs;
        if let Some(open) = dialogs.picker.replace(Picker::DrumMap) {
            dialogs.picker_return.push(open);
        }
        dialogs.delete_drum_map = None;
        dialogs.drum_map = Some(DrumMapList::new(key));
    }

    /// Whether the drum map list is open: MIDI NoteOns go to it alone.
    pub(crate) fn drum_map_open(&self) -> bool {
        self.ui.dialogs.picker == Some(Picker::DrumMap) && self.ui.dialogs.drum_map.is_some()
    }

    /// A NoteOn of `note` while the list is open, at UI time `time`: learned
    /// for the listening row, which stops listening; with none listening,
    /// shown on the row it plays or as not mapped.
    pub(crate) fn drum_map_hit(&mut self, note: u8, time: f64) {
        let Some(list) = self.ui.dialogs.drum_map.as_mut() else {
            return;
        };
        let maps = &mut self.playback_ctl.devices.drum_maps;
        let shown = match list.listening.take() {
            Some(combo) => {
                maps.entry(list.device.clone()).or_default().assign(note, combo);
                ListHit::Row(combo)
            }
            None => {
                let map = maps.get(&list.device).map_or(GM_DRUM_MAP, CustomDrumMap::entries);
                lookup(map, note).map_or(ListHit::Unmapped(note), ListHit::Row)
            }
        };
        list.hit = Some((shown, time));
    }

    /// The list: the device, one row per combination with its notes, and
    /// *Reset to built-in* while the device has a custom map.
    pub(crate) fn drum_map_list(&mut self, ui: &mut Ui) {
        let Some(mut list) = self.ui.dialogs.drum_map.take() else {
            return;
        };
        let now = ui.input(|i| i.time);
        let maps = &mut self.playback_ctl.devices.drum_maps;
        let custom = maps.contains_key(&list.device);
        ui.label(RichText::new(&list.device).strong());
        ui.weak(if custom {
            "Custom drum map. Tap a row and hit a pad to add its note there."
        } else {
            "Built-in drum map. Tap a row and hit a pad to add its note there; \
             the first change makes it this device's own."
        });
        ui.add_space(4.0);
        let hit = list.hit.filter(|&(_, at)| now - at < HIT_SHOWN_S).map(|(hit, at)| {
            ui.ctx().request_repaint_after_secs((HIT_SHOWN_S - (now - at)) as f32);
            hit
        });
        // Always one line, so the rows do not jump.
        let status = match (list.listening, hit) {
            (Some(combo), _) => format!("Hit a pad for {}…", combo_label(combo)),
            (None, Some(ListHit::Unmapped(note))) => format!("Note {note}: not mapped"),
            _ => String::new(),
        };
        ui.label(RichText::new(status).strong());
        ui.add_space(4.0);

        let entries: Vec<DrumMapEntry> =
            maps.get(&list.device).map_or(GM_DRUM_MAP, CustomDrumMap::entries).to_vec();
        let mut removed = None;
        egui::ScrollArea::vertical()
            .id_salt("drum_map_rows")
            .max_height(dialog_scroll_height(ui))
            .show(ui, |ui| {
                let label_width = (ui.available_width() * 0.5).min(190.0);
                for combo in offered_combos() {
                    ui.horizontal_wrapped(|ui| {
                        let listening = list.listening == Some(combo);
                        let flashed = hit == Some(ListHit::Row(combo));
                        let mut text = RichText::new(combo_label(combo)).size(16.0);
                        if flashed {
                            text = text.strong();
                        }
                        let mut button = Button::new(text)
                            .selected(listening)
                            .min_size(vec2(label_width, ROW_HEIGHT * 0.8));
                        if flashed {
                            button =
                                button.fill(ui.visuals().selection.bg_fill.gamma_multiply(0.6));
                        }
                        if ui.add(button).clicked() {
                            // Tapped again it stops; another row takes over.
                            list.listening = (!listening).then_some(combo);
                        }
                        for note in notes_of(&entries, combo) {
                            let chip = Button::new(RichText::new(format!("{note} ✕")).size(14.0));
                            if ui.add(chip).on_hover_text("Remove this note").clicked() {
                                removed = Some(note);
                            }
                        }
                    });
                }
            });
        if let Some(note) = removed {
            maps.entry(list.device.clone()).or_default().remove(note);
        }

        if maps.contains_key(&list.device) {
            ui.add_space(8.0);
            if list.confirm_reset {
                ui.label("Reset to the built-in drum map? Your changes are lost.");
                ui.horizontal(|ui| {
                    if ui.button("Reset").clicked() {
                        maps.remove(&list.device);
                        list.confirm_reset = false;
                    }
                    if ui.button("Cancel").clicked() {
                        list.confirm_reset = false;
                    }
                });
            } else if ui.button("Reset to built-in").clicked() {
                list.confirm_reset = true;
            }
        } else {
            list.confirm_reset = false;
        }
        self.ui.dialogs.drum_map = Some(list);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drum_map::device_key;
    use crate::test_support::{creation_context, frame, shows, tap};
    use crate::{TransportState, clock};
    use grooph_measure::{Instrument, Technique};

    const TD17: &str = "TD-17:TD-17 MIDI 1 24:0";

    fn app(ctx: &egui::Context) -> Grooph {
        Grooph::new(&creation_context(ctx.clone()))
    }

    #[test]
    fn opening_the_list_discards_the_run_playing() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.select_tab(crate::Tab::Score);
        app.playback_ctl.feedback.accuracy_enabled = true;
        app.editor.docs.score.score =
            grooph_measure::notation::parse_score("4/4 H-K:e H:e H-S:e H:e H-K:e H:e H-S:e H:e")
                .unwrap();
        // An earlier run's summary, then a run with a hit.
        app.toggle_playback();
        app.playback_ctl.accuracy.tracker.on_playback_start_at(0.0, 0.0);
        let (timing, score) = (app.score_timing(), app.played_score().clone());
        let tracker = &mut app.playback_ctl.accuracy.tracker;
        tracker.record_instrument_hit(0.0, Instrument::Kick, &timing, &score);
        app.toggle_playback();
        assert!(app.playback_ctl.accuracy.tracker.last_summary().is_some());
        app.toggle_playback();
        app.playback_ctl.accuracy.tracker.on_playback_start_at(0.0, 0.0);
        let tracker = &mut app.playback_ctl.accuracy.tracker;
        tracker.record_instrument_hit(0.0, Instrument::Kick, &timing, &score);

        app.ui.dialogs.picker = Some(Picker::MidiInput);
        app.ui.dialogs.picker_return = vec![Picker::PlayAlong];
        app.open_drum_map(device_key(TD17).to_owned());
        assert_eq!(app.playback_ctl.transport_state, TransportState::Stopped);
        assert!(app.playback_ctl.accuracy.tracker.last_summary().is_none());
        assert!(app.drum_map_open());
        // Hits while it is open: none is recorded, no run starts.
        app.playback_ctl.play_along.pending_taps.push(clock::now_seconds());
        frame(&ctx, 0.0, vec![], |ui| app.handle_play_along_input(ui));
        app.drum_map_hit(36, 0.0);
        app.toggle_playback();
        assert_eq!(app.playback_ctl.transport_state, TransportState::Stopped);
        assert!(!app.playback_ctl.accuracy.tracker.has_start_time());
        assert!(app.playback_ctl.accuracy.tracker.last_summary().is_none());
        assert!(app.playback_ctl.play_along.pending_taps.is_empty());

        // Back: the device picker, then Play along.
        assert!(app.back());
        assert_eq!(app.ui.dialogs.picker, Some(Picker::MidiInput));
        assert!(app.ui.dialogs.drum_map.is_none());
        assert!(app.back());
        assert_eq!(app.ui.dialogs.picker, Some(Picker::PlayAlong));
        assert!(app.back());
        assert_eq!(app.ui.dialogs.picker, None);
    }

    #[test]
    fn a_hit_is_learned_for_the_listening_row_and_moves_its_note() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let key = device_key(TD17).to_owned();
        app.open_drum_map(key.clone());
        let snare = (Instrument::Snare, Technique::Normal);
        // Shown, not learned: the built-in table still applies.
        app.drum_map_hit(36, 1.0);
        let list = app.ui.dialogs.drum_map.as_ref().unwrap();
        assert_eq!(list.hit, Some((ListHit::Row((Instrument::Kick, Technique::Normal)), 1.0)));
        app.drum_map_hit(60, 2.0);
        let list = app.ui.dialogs.drum_map.as_ref().unwrap();
        assert_eq!(list.hit, Some((ListHit::Unmapped(60), 2.0)));
        assert!(app.playback_ctl.devices.drum_maps.is_empty());

        app.ui.dialogs.drum_map.as_mut().unwrap().listening = Some(snare);
        app.drum_map_hit(36, 3.0);
        // One hit, one note.
        app.drum_map_hit(60, 4.0);
        let map = &app.playback_ctl.devices.drum_maps[&key];
        assert_eq!(lookup(map.entries(), 36), Some(snare));
        assert_eq!(lookup(map.entries(), 60), None);
        assert_eq!(map.entries().iter().filter(|e| e.note == 36).count(), 1);
        assert_eq!(app.ui.dialogs.drum_map.as_ref().unwrap().listening, None);
    }

    #[test]
    fn the_list_learns_removes_and_resets_by_tapping() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let key = device_key(TD17).to_owned();
        app.open_drum_map(key.clone());
        let list = |app: &mut Grooph, time: f64, events| {
            frame(&ctx, time, events, |ui| app.drum_map_list(ui))
        };
        let shown = list(&mut app, 0.0, vec![]);
        assert!(shows(&shown, "Hi-hat · Half-open"));
        assert!(shows(&shown, "36 ✕"));
        assert!(!shows(&shown, "Reset to built-in"));
        // Tap a row, tap it again: it listens, then stops.
        let shown = list(&mut app, 0.1, tap(&shown, "Snare · Rimshot"));
        let rimshot = (Instrument::Snare, Technique::Rimshot);
        assert_eq!(app.ui.dialogs.drum_map.as_ref().unwrap().listening, Some(rimshot));
        let shown = list(&mut app, 0.2, tap(&shown, "Snare · Rimshot"));
        assert_eq!(app.ui.dialogs.drum_map.as_ref().unwrap().listening, None);
        // Another row takes the listening.
        let shown = list(&mut app, 0.3, tap(&shown, "Snare · Rimshot"));
        list(&mut app, 0.4, tap(&shown, "Ride · Bell"));
        let bell = (Instrument::Ride, Technique::Bell);
        assert_eq!(app.ui.dialogs.drum_map.as_ref().unwrap().listening, Some(bell));
        let shown = list(&mut app, 0.45, vec![]);
        assert!(shows(&shown, "Hit a pad for Ride · Bell…"));
        app.drum_map_hit(61, 0.5);
        let shown = list(&mut app, 0.6, vec![]);
        assert!(shows(&shown, "61 ✕"));
        // A note that plays nothing, shown for a moment.
        app.drum_map_hit(99, 0.6);
        assert!(shows(&list(&mut app, 0.7, vec![]), "Note 99: not mapped"));
        assert!(!shows(&list(&mut app, 3.0, vec![]), "Note 99: not mapped"));
        // Removing a note: at once.
        list(&mut app, 3.1, tap(&shown, "36 ✕"));
        let shown = list(&mut app, 3.2, vec![]);
        assert!(!shows(&shown, "36 ✕"));
        assert_eq!(lookup(app.playback_ctl.devices.drum_maps[&key].entries(), 36), None);
        // Reset asks first.
        list(&mut app, 3.3, tap(&shown, "Reset to built-in"));
        let shown = list(&mut app, 3.4, vec![]);
        list(&mut app, 3.5, tap(&shown, "Cancel"));
        let shown = list(&mut app, 3.6, vec![]);
        assert!(app.playback_ctl.devices.drum_maps.contains_key(&key));
        list(&mut app, 3.7, tap(&shown, "Reset to built-in"));
        let shown = list(&mut app, 3.8, vec![]);
        assert!(app.playback_ctl.devices.drum_maps.contains_key(&key));
        list(&mut app, 3.9, tap(&shown, "Reset"));
        assert!(app.playback_ctl.devices.drum_maps.is_empty());
    }

    #[test]
    fn the_device_picker_shows_the_drum_map_row_and_the_saved_maps() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.playback_ctl.midi.available_ports = vec![TD17.to_owned(), "Keys 20:0".to_owned()];
        let devices = &mut app.playback_ctl.devices;
        devices.drum_maps.insert("Old pad".to_owned(), CustomDrumMap::default());
        app.ui.dialogs.picker = Some(Picker::MidiInput);
        app.ui.dialogs.picker_return = vec![Picker::PlayAlong];
        let rows = |app: &mut Grooph, time: f64, connected, events| {
            let mut picked = None;
            let shown =
                frame(&ctx, time, events, |ui| picked = app.midi_device_rows(ui, connected));
            (shown, picked)
        };
        // Nothing connected: no drum map row.
        let (shown, _) = rows(&mut app, 0.0, None, vec![]);
        assert!(!shows(&shown, "Drum map: Built-in"));
        assert!(shows(&shown, "Old pad"));
        let (_, picked) = rows(&mut app, 0.1, None, tap(&shown, TD17));
        assert_eq!(picked, Some(Some(0)));
        // Connected: its drum map row, built-in until it has one.
        let (shown, _) = rows(&mut app, 0.2, Some(0), vec![]);
        assert!(shows(&shown, "Drum map: Built-in"));
        app.playback_ctl
            .devices
            .drum_maps
            .insert(device_key(TD17).to_owned(), CustomDrumMap::default());
        let (shown, _) = rows(&mut app, 0.3, Some(0), vec![]);
        assert!(shows(&shown, "Drum map: Custom"));
        // Only the device not plugged in is under Saved drum maps.
        assert!(shows(&shown, "Saved drum maps"));
        assert!(!shows(&shown, device_key(TD17)));
        // Delete asks first.
        rows(&mut app, 0.4, Some(0), tap(&shown, "Delete"));
        let (shown, _) = rows(&mut app, 0.5, Some(0), vec![]);
        assert!(shows(&shown, "Delete the drum map of Old pad?"));
        assert!(app.playback_ctl.devices.drum_maps.contains_key("Old pad"));
        rows(&mut app, 0.6, Some(0), tap(&shown, "Delete"));
        let (shown, _) = rows(&mut app, 0.7, Some(0), vec![]);
        assert!(!app.playback_ctl.devices.drum_maps.contains_key("Old pad"));
        assert!(!shows(&shown, "Saved drum maps"));
        // The drum map row opens the list over the picker.
        rows(&mut app, 0.8, Some(0), tap(&shown, "Drum map: Custom"));
        assert_eq!(app.ui.dialogs.picker, Some(Picker::DrumMap));
        assert_eq!(app.ui.dialogs.picker_return, [Picker::PlayAlong, Picker::MidiInput]);
        assert_eq!(app.ui.dialogs.drum_map.as_ref().unwrap().device, device_key(TD17));
    }
}
