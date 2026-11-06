//! The kit's part of the tool grid (see `tool_palette`): which instrument
//! the tools write and change, which voice they edit, and the head of that
//! instrument on the cursor's beat. Shown only on a staff of several
//! instruments; on the one line every note is its rhythm, and the tools edit
//! it as before.
//!
//! An instrument's cell steps its head on the cursor's beat through none,
//! the usual stroke and its techniques (`Grooph::tap_instrument`; a long
//! press takes it away) and makes it the picked one, the head the note
//! values write on a rest (`Grooph::edit_instrument`). Accent, ghost and the
//! technique tools go to the head they mean (`Grooph::aimed` in `tool_palette`).
//! The cursor goes to the voice the instrument's head would be entered in:
//! where it is on the beat already, else where it is elsewhere in the measure,
//! else voice 0 of a measure of one voice that holds notes, else the one the
//! score's `Voicing` writes it in (the feet below, the hands above), onto the
//! beat sounding then. A tap on a head on the staff picks that
//! instrument. The library's "New drum groove" starts a score
//! on the kit's staff, its "New rhythm" one on the one line.

use crate::Grooph;
use crate::Mode;
use crate::documents::Document;
use crate::score_change::Origin;
use crate::shell::Tab;
use crate::state::PendingLoad;
use crate::tools::ToolId;
use grooph_measure::{
    Cursor, Instrument, Measure, Score, StaffKind, TimeSignature, VoiceIdx, Voicing,
};

/// A new drum groove: one bar of `ts`, a voice of rests for each of the
/// `voicing`'s, on the kit's staff.
pub(crate) fn empty_groove(ts: TimeSignature, voicing: Voicing) -> Score {
    let mut bar = Measure::new(ts);
    bar.fill_voices(voicing.voices());
    Score::single(bar).with_staff(StaffKind::Kit).with_voicing(voicing)
}

impl Grooph {
    /// Make `instrument` the one the tools write and change, and edit its
    /// voice (see [`Grooph::go_to_voice`]).
    pub(crate) fn pick_instrument(&mut self, instrument: Instrument) {
        self.editor.instrument = instrument;
        let voice = self.voice_for(instrument);
        self.go_to_voice(voice);
    }

    /// The voice a new head of `instrument` goes to in the cursor's
    /// measure: where it already is, else the measure's own style, else the
    /// score's rule (see [`Measure::voice_for`]).
    fn voice_for(&self, instrument: Instrument) -> VoiceIdx {
        self.current_measure().voice_for(instrument, &self.editor.doc().score().voicing)
    }

    /// A tap on an instrument's cell: it becomes the picked one and its head
    /// on the cursor's beat steps on: none there is added, played the usual
    /// way; one there is played with the instrument's next technique, after
    /// the last it is taken away (see [`Instrument::techniques`]). It stays
    /// in the voice where it is, a new one goes to its usual one. The cursor
    /// stays at its time.
    pub(crate) fn tap_instrument(&mut self, instrument: Instrument) {
        let next = match self.technique_at_cursor(instrument) {
            Some(t) => instrument.techniques().skip_while(|&o| o != t).nth(1).is_some(),
            None => false,
        };
        let tool = if next { ToolId::ModifyCycleTechnique } else { ToolId::ModifyToggleHead };
        self.strike(instrument, tool);
    }

    /// A long press (a right click) on an instrument's cell: its head on the
    /// cursor's beat is taken away however it is played, or added.
    pub(crate) fn hold_instrument(&mut self, instrument: Instrument) {
        self.strike(instrument, ToolId::ModifyToggleHead);
    }

    /// Pick `instrument`, go to the voice of its head on the cursor's beat
    /// (or of a new one) and apply the tool `id` there.
    fn strike(&mut self, instrument: Instrument, id: ToolId) {
        let there = self
            .heads_at_cursor()
            .into_iter()
            .find(|&(i, _)| i == instrument)
            .map(|(_, at)| at.voice);
        self.editor.instrument = instrument;
        let steps = self.editor.doc().history().pushed();
        let voice = there.unwrap_or_else(|| self.voice_for(instrument));
        self.go_to_voice(voice);
        self.apply_tool(id.tool());
        // A voice the bar had to get first is part of the same step.
        if self.editor.doc().history().pushed() == steps + 2 {
            self.editor.doc_mut().history_mut().fold_last();
        }
    }

    /// Move the cursor to `voice` of its measure, onto the beat sounding
    /// then; a measure of the kit without that voice gets it, of rests (an
    /// undoable change). On the one line a measure keeps its voices.
    pub(crate) fn go_to_voice(&mut self, voice: VoiceIdx) {
        let cursor = self.editor.doc().score().clamp_cursor(self.editor.doc().cursor());
        let kit = !self.staff_model().one_line();
        if kit
            && self.current_measure().voices().len() <= voice
            && voice < self.editor.doc().score().voicing.voices()
        {
            self.with_undo_snapshot(|app| {
                app.editor
                    .doc_mut()
                    .edit_score(|score| {
                        score.current_mut(cursor.measure_idx).fill_voices(voice + 1)
                    })
                    .is_some()
            });
        }
        let doc = self.editor.doc_mut();
        *doc.cursor_mut() = doc.score().cursor_in_voice(cursor, voice);
    }

    /// Move the picked instrument's heads in the cursor's measure to the
    /// other voice ("Switch voice"): every one, or with `only_one` only the
    /// one at the cursor (the picked instrument's, else the first head
    /// there). A lower voice left without notes goes in a score of one
    /// voice. The cursor stays at its time, in the voice the instrument
    /// went to. Whether the measure changed; call it inside an undo step.
    pub(crate) fn switch_voice(&mut self, only_one: bool) -> bool {
        let at_cursor = self.heads_at_cursor();
        let picked = self.editor.instrument;
        let instrument = match only_one {
            true if !at_cursor.iter().any(|&(i, _)| i == picked) => {
                let Some(&(first, _)) = at_cursor.first() else {
                    return false;
                };
                first
            }
            _ => picked,
        };
        let score = self.editor.doc().score();
        let cursor = score.clamp_cursor(self.editor.doc().cursor());
        let tick = score.tick_of(cursor);
        let voices = score.voicing.voices();
        let Some(measure) =
            self.current_measure().with_voice_switched(instrument, tick, only_one, voices)
        else {
            return false;
        };
        self.editor.instrument = instrument;
        self.put_revoiced(cursor, tick, measure)
    }

    /// Revoice the cursor's measure by `voicing` ("Revoice measure…"): every
    /// head in the voice it writes the instrument in, as one undo step. The
    /// cursor stays at its time, in the voice of the picked instrument. A
    /// measure whose tuplets cannot meet stays as it is.
    pub(crate) fn revoice_current_measure(&mut self, voicing: Voicing) {
        self.with_undo_snapshot(|app| {
            let score = app.editor.doc().score();
            let cursor = score.clamp_cursor(app.editor.doc().cursor());
            let tick = score.tick_of(cursor);
            match app.current_measure().revoiced(&voicing) {
                Some(measure) => app.put_revoiced(cursor, tick, measure),
                None => false,
            }
        });
    }

    /// Put `measure` in place of the one of `cursor` (at `tick` in it) and
    /// move the cursor to that time in the voice of the picked instrument's
    /// head there, else of its heads in the measure. Whether it changed.
    fn put_revoiced(&mut self, cursor: Cursor, tick: u32, measure: Measure) -> bool {
        if *self.current_measure() == measure {
            return false;
        }
        let instrument = self.editor.instrument;
        let voicing = self.editor.doc().score().voicing;
        let there = measure
            .head_onsets()
            .into_iter()
            .find(|o| o.tick == tick && o.head.instrument() == instrument);
        let voice = there.map_or_else(|| measure.voice_for(instrument, &voicing), |o| o.voice);
        let doc = self.editor.doc_mut();
        doc.edit_score(|score| score.measures_mut()[cursor.measure_idx] = measure);
        *doc.cursor_mut() = doc.score().cursor_at_tick(Cursor { voice, ..cursor }, tick);
        true
    }

    /// Start a new score in the score tab, on `staff`, asking first if its
    /// score has unsaved changes (as a pattern loaded from the library does).
    pub(crate) fn request_new_score(&mut self, staff: StaffKind) {
        match self.editor.docs.score.dirty {
            true => self.ui.dialogs.pending_load = Some(PendingLoad::NewScore(staff)),
            false => self.new_score(staff),
        }
    }

    /// Replace the score tab's score with a new one, one bar of the current
    /// meter (an undoable change), and edit it: a drum groove with its two
    /// voices of rests on the kit's staff, or a bar of rests on the one line.
    pub(crate) fn new_score(&mut self, staff: StaffKind) {
        self.stop_transport();
        self.show_document(Document::Score);
        let ts = self.current_measure().time_signature();
        let score = match staff {
            StaffKind::Kit => empty_groove(ts, Voicing::default()),
            StaffKind::Line => Score::single(Measure::new(ts)),
        };
        self.replace_score(score, Origin::New);
        self.ui.dialogs.picker = None;
        self.select_tab(Tab::Score);
        self.ui.mode = Mode::Edit;
        self.pick_instrument(self.editor.instrument);
    }
}
