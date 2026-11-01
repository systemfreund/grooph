//! The kit's part of the tool grid (see `tool_palette`): which instrument
//! the tools write and change, which voice they edit, and the head of that
//! instrument on the cursor's beat. Shown only on a staff of several
//! instruments; on the one line every note is its rhythm, and the tools edit
//! it as before.
//!
//! An instrument's cell toggles its head on the cursor's beat
//! (`Grooph::tap_instrument`) and makes it the picked one, the head the note
//! values write on a rest (`Grooph::edit_instrument`). Accent, ghost and the
//! techniques go to the head they mean (`Grooph::aimed` in `tool_palette`).
//! The cursor goes to the instrument's voice, the feet below, the hands above
//! (`Instrument::default_voice`), unless it is there already in the other,
//! onto the beat sounding then. A tap on a head on the staff picks that
//! instrument. The library's "New drum groove" starts a score
//! on the kit's staff.

use crate::Grooph;
use crate::Mode;
use crate::documents::Document;
use crate::shell::Tab;
use crate::state::PendingLoad;
use grooph_measure::{Cursor, Instrument, Measure, Score, StaffKind, TimeSignature, VoiceIdx};

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

/// The short name of `instrument` on its cell of the tool grid.
pub(crate) fn instrument_short(instrument: Instrument) -> &'static str {
    match instrument {
        Instrument::Rhythm => "R",
        Instrument::HiHatFoot => "HF",
        Instrument::Kick => "BD",
        Instrument::FloorTom => "FT",
        Instrument::Snare => "SN",
        Instrument::MidTom => "T2",
        Instrument::HighTom => "T1",
        Instrument::Ride => "RD",
        Instrument::HiHat => "HH",
        Instrument::Crash => "CR",
    }
}

/// A kit score's two voices, hands above and feet below.
const KIT_VOICES: usize = 2;

/// A new drum groove: one bar of `ts`, its two voices of rests, on the kit's staff.
pub(crate) fn empty_groove(ts: TimeSignature) -> Score {
    let mut bar = Measure::new(ts);
    bar.fill_voices(KIT_VOICES);
    Score::single(bar).with_staff(StaffKind::Kit)
}

impl Grooph {
    /// Make `instrument` the one the tools write and change, and edit its
    /// voice (see [`Grooph::go_to_voice`]).
    pub(crate) fn pick_instrument(&mut self, instrument: Instrument) {
        self.editor.instrument = instrument;
        self.go_to_voice(instrument.default_voice());
    }

    /// A tap on an instrument's cell: it becomes the picked one and its head
    /// on the cursor's beat is added or taken away, in the voice where it
    /// already is there, else its usual one. The cursor stays at its time.
    pub(crate) fn tap_instrument(&mut self, instrument: Instrument) {
        let there = self
            .heads_at_cursor()
            .into_iter()
            .find(|&(i, _)| i == instrument)
            .map(|(_, at)| at.voice);
        self.editor.instrument = instrument;
        self.go_to_voice(there.unwrap_or(instrument.default_voice()));
        self.apply_tool(crate::tool_palette::tool("modify.toggle.head"));
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
        let groove = empty_groove(self.current_measure().time_signature());
        self.with_undo_snapshot(|app| {
            let doc = app.editor.doc_mut();
            doc.score = groove;
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
