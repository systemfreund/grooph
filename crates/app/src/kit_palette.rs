//! The kit's row of the tool palette: which instrument the tools write and
//! change, which voice they edit, and the head of that instrument on the
//! cursor's beat. Shown only on a staff of several instruments; on the one
//! line every note is its rhythm, and the tools edit it as before.
//!
//! The picked instrument is the head the tools mean (`Grooph::edit_instrument`):
//! the note values write it on a rest, accent and ghost change its head, and
//! "Toggle Instrument" adds it to the beat or takes it away. Picking one, in
//! the row or by a tap on its head, moves the cursor to its voice: the feet
//! below, the hands above (`Instrument::default_voice`); "Voice" moves it to
//! the other voice for a head off its usual side.

use crate::Grooph;
use crate::tools::{Modifier, ToolKind, all_tools};
use eframe::egui::{Button, RichText, Ui, Vec2};
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

/// A kit score's two voices, hands above and feet below.
const KIT_VOICES: usize = 2;

impl Grooph {
    /// The kit's row above the tools, on a staff of several instruments.
    pub(crate) fn kit_palette(&mut self, ui: &mut Ui) {
        let staff = self.staff_model();
        if staff.one_line() {
            return;
        }
        let chosen = self.edit_instrument();
        let height = 36.0;
        ui.horizontal_wrapped(|ui| {
            for &instrument in staff.instruments {
                let button = Button::selectable(
                    instrument == chosen,
                    RichText::new(instrument_label(instrument)).size(16.0),
                )
                .min_size(Vec2::new(0.0, height));
                if ui.add(button).clicked() {
                    self.pick_instrument(instrument);
                }
            }
            ui.separator();
            let voice = self.editor.doc().cursor.voice;
            let other = if voice == 0 { "Voice ↓" } else { "Voice ↑" };
            if ui
                .add(Button::new(RichText::new(other).size(16.0)).min_size(Vec2::new(0.0, height)))
                .on_hover_text("Edit the other voice: hands above, feet below")
                .clicked()
            {
                self.go_to_voice(if voice == 0 { 1 } else { 0 });
            }
            for (modifier, label, hover) in [
                (Modifier::ToggleHead, "+/−", "Add the instrument here, or take it away (H)"),
                (Modifier::CycleTechnique, "Technique", "Play it open, closed, on the rim … (O)"),
            ] {
                let tool = all_tools().iter().find(|t| t.kind == ToolKind::Modify(modifier));
                if ui
                    .add(
                        Button::new(RichText::new(label).size(16.0))
                            .min_size(Vec2::new(0.0, height)),
                    )
                    .on_hover_text(hover)
                    .clicked()
                    && let Some(tool) = tool
                {
                    self.apply_tool(tool);
                }
            }
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

    /// Start a new score on the kit's staff, one bar of the current meter
    /// with its two voices of rests (an undoable change).
    pub(crate) fn new_kit_score(&mut self) {
        let mut bar = Measure::new(self.current_measure().time_signature());
        bar.fill_voices(KIT_VOICES);
        self.with_undo_snapshot(|app| {
            let doc = app.editor.doc_mut();
            doc.score = Score::single(bar).with_staff(StaffKind::Kit);
            doc.cursor = Cursor::start();
            doc.active_pattern_id = None;
            true
        });
        self.pick_instrument(self.editor.instrument);
    }
}
