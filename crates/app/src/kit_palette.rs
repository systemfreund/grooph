//! The kit's row of the tool palette: which instrument the tools write and
//! change, which voice they edit, and the head of that instrument on the
//! cursor's beat. Shown only on a staff of several instruments; on the one
//! line every note is its rhythm, and the tools edit it as before.
//!
//! The picked instrument is the head the tools mean (`Grooph::edit_instrument`):
//! the note values write it on a rest, accent and ghost change its head, and
//! "Toggle Instrument" adds it to the beat or takes it away. Picking one, in
//! the row or by a tap on its head, moves the cursor to its voice: the feet
//! below, the hands above (`Instrument::default_voice`); "Upper" and "Lower"
//! move it to a voice for a head off its usual side. The library's "New
//! drum groove" starts a score on the kit's staff.

use crate::Grooph;
use crate::Mode;
use crate::documents::Document;
use crate::shell::Tab;
use crate::state::PendingLoad;
use crate::tools::{Modifier, ToolKind, all_tools};
use eframe::egui::scroll_area::{ScrollBarVisibility, ScrollSource};
use eframe::egui::{Button, RichText, ScrollArea, Ui, Vec2};
use grooph_measure::{Cursor, Instrument, Measure, Score, StaffKind, VoiceIdx};

/// The name of `instrument` on its button.
pub(crate) fn instrument_label(instrument: Instrument) -> &'static str {
    match instrument {
        Instrument::Rhythm => "Rhythm",
        Instrument::HiHatFoot => "Hi-hat foot",
        Instrument::Kick => "Kick",
        Instrument::FloorTom => "Floor tom",
        Instrument::Snare => "Snare",
        Instrument::MidTom => "Tom 2",
        Instrument::HighTom => "Tom 1",
        Instrument::Ride => "Ride",
        Instrument::HiHat => "Hi-hat",
        Instrument::Crash => "Crash",
    }
}

/// Height of the row's buttons.
const ROW_HEIGHT: f32 = 36.0;

/// A kit score's two voices, hands above and feet below.
const KIT_VOICES: usize = 2;

impl Grooph {
    /// The kit's row above the tools, on a staff of several instruments:
    /// one line that scrolls sideways, like the tools', the head's tools
    /// first, then the instruments and the voices.
    pub(crate) fn kit_palette(&mut self, ui: &mut Ui) {
        let staff = self.staff_model();
        if staff.one_line() {
            return;
        }
        let chosen = self.edit_instrument();
        let voice = self.editor.doc().cursor.voice;
        let button = |label: &str, selected: bool| {
            Button::selectable(selected, RichText::new(label).size(16.0))
                .min_size(Vec2::new(0.0, ROW_HEIGHT))
        };
        ScrollArea::horizontal()
            .id_salt("kit_palette")
            .scroll_source(ScrollSource::ALL)
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (modifier, label, hover) in [
                        (
                            Modifier::ToggleHead,
                            "+/−",
                            "Add the instrument here, or take it away (H)",
                        ),
                        (Modifier::CycleTechnique, "Technique", "Open, closed, on the rim, … (O)"),
                    ] {
                        let tool =
                            all_tools().iter().find(|t| t.kind == ToolKind::Modify(modifier));
                        let framed = Button::new(RichText::new(label).size(16.0))
                            .min_size(Vec2::new(0.0, ROW_HEIGHT));
                        if ui.add(framed).on_hover_text(hover).clicked()
                            && let Some(tool) = tool
                        {
                            self.apply_tool(tool);
                        }
                    }
                    ui.separator();
                    for &instrument in staff.instruments {
                        let label = instrument_label(instrument);
                        if ui.add(button(label, instrument == chosen)).clicked() {
                            self.pick_instrument(instrument);
                        }
                    }
                    ui.separator();
                    for (v, label, hover) in [
                        (0, "Upper", "Edit the upper voice: the hands"),
                        (1, "Lower", "Edit the lower voice: the feet"),
                    ] {
                        if ui.add(button(label, voice == v)).on_hover_text(hover).clicked() {
                            self.go_to_voice(v);
                        }
                    }
                });
            });
        ui.add_space(4.0);
    }

    /// Make `instrument` the one the tools write and change, and edit its
    /// voice (see [`Grooph::go_to_voice`]).
    pub(crate) fn pick_instrument(&mut self, instrument: Instrument) {
        self.editor.instrument = instrument;
        self.go_to_voice(instrument.default_voice());
    }

    /// Move the cursor to `voice` of its measure, onto the beat sounding
    /// then; a measure of the kit without that voice gets it, of rests (an
    /// undoable change). On the one line a measure keeps its voices.
    pub(crate) fn go_to_voice(&mut self, voice: VoiceIdx) {
        let cursor = self.editor.doc().score.clamp_cursor(self.editor.doc().cursor);
        let kit = !self.staff_model().one_line();
        if kit && self.current_measure().voices().len() <= voice && voice < KIT_VOICES {
            self.with_undo_snapshot(|app| {
                app.current_measure_mut().fill_voices(voice + 1);
                true
            });
        }
        let doc = self.editor.doc_mut();
        doc.cursor = doc.score.cursor_in_voice(cursor, voice);
    }

    /// Start a new drum groove in the score tab, asking first if its score
    /// has unsaved changes (as a pattern loaded from the library does).
    pub(crate) fn request_new_kit_score(&mut self) {
        match self.editor.docs.score.dirty {
            true => self.ui.pending_load = Some(PendingLoad::NewKitScore),
            false => self.new_kit_score(),
        }
    }

    /// Replace the score tab's score with a new drum groove, one bar of the
    /// current meter with its two voices of rests on the kit's staff (an
    /// undoable change), and edit it.
    pub(crate) fn new_kit_score(&mut self) {
        self.stop_transport();
        self.show_document(Document::Score);
        let mut bar = Measure::new(self.current_measure().time_signature());
        bar.fill_voices(KIT_VOICES);
        self.with_undo_snapshot(|app| {
            let doc = app.editor.doc_mut();
            doc.score = Score::single(bar).with_staff(StaffKind::Kit);
            doc.cursor = Cursor::start();
            doc.active_pattern_id = None;
            true
        });
        self.ui.picker = None;
        self.select_tab(Tab::Score);
        self.ui.mode = Mode::Edit;
        self.pick_instrument(self.editor.instrument);
    }
}
