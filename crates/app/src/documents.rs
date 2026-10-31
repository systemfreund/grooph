//! The two rhythm documents: Practice's generated rhythm and the score tab's
//! own score (written, loaded from the library or opened from a link).
//!
//! Each keeps its score, cursor, undo history, dirty flag and pattern. The
//! one of the page being shown is the working score in [`EditorState`]
//! (`score`, `cursor`, `history`, ...), so the staff, the tools and undo work
//! on it unchanged; the other waits in [`EditorState::other`]. While the
//! metronome tab is open its click bar is the working score, and the
//! document it put aside is the shown one. The generator only ever writes
//! Practice's document (see [`Grooph::generate_new_score`]): a written score
//! is never replaced by a generated rhythm. The one way from one to the other
//! is "Edit in Score", which copies Practice's rhythm into the score tab.
//!
//! [`EditorState`]: crate::state::EditorState
//! [`EditorState::other`]: crate::state::EditorState::other

use crate::state::{EditorState, PendingLoad};
use crate::undo::{DEFAULT_UNDO_LIMIT, UndoHistory};
use crate::{Grooph, Mode};
use grooph_measure::{Cursor, Score};
use serde::{Deserialize, Serialize};

/// One of the two rhythm documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Document {
    /// The generator's rhythm on the practice page: thrown away freely.
    Practice,
    /// The score tab's score.
    Score,
}

/// A rhythm document while it is not the working score: put aside by the
/// other page, or by the metronome tab.
pub(crate) struct RhythmDoc {
    pub(crate) score: Score,
    pub(crate) cursor: Cursor,
    pub(crate) history: UndoHistory,
    pub(crate) dirty: bool,
    pub(crate) active_pattern_id: Option<u64>,
}

impl RhythmDoc {
    /// A document of `score`, unchanged and with nothing to undo.
    pub(crate) fn new(score: Score, cursor: Cursor) -> Self {
        let cursor = score.clamp_cursor(cursor);
        Self {
            score,
            cursor,
            history: UndoHistory::new(DEFAULT_UNDO_LIMIT),
            dirty: false,
            active_pattern_id: None,
        }
    }
}

impl EditorState {
    /// Swap the working score with the other document.
    fn swap_with_other(&mut self) {
        let Self { score, cursor, history, dirty, active_pattern_id, other, .. } = self;
        std::mem::swap(score, &mut other.score);
        std::mem::swap(cursor, &mut other.cursor);
        std::mem::swap(history, &mut other.history);
        std::mem::swap(dirty, &mut other.dirty);
        std::mem::swap(active_pattern_id, &mut other.active_pattern_id);
    }
}

/// What the library needs of the score tab's document, wherever it is.
pub(crate) struct ScoreDocMut<'a> {
    pub(crate) dirty: &'a mut bool,
    pub(crate) active_pattern_id: &'a mut Option<u64>,
}

impl<'a> From<&'a mut RhythmDoc> for ScoreDocMut<'a> {
    fn from(doc: &'a mut RhythmDoc) -> Self {
        Self { dirty: &mut doc.dirty, active_pattern_id: &mut doc.active_pattern_id }
    }
}

impl Grooph {
    /// The document of the rhythm page being shown, or of the page the More
    /// screen or the metronome tab was opened from.
    pub(crate) fn document(&self) -> Document {
        self.editor.document
    }

    /// The document the mode's page shows; `None` on the More screen and in
    /// the metronome tab, which leave it as it is.
    fn document_of_mode(&self) -> Option<Document> {
        if self.metronome_mode() {
            return None;
        }
        match self.ui.mode {
            Mode::MoreMenu => None,
            Mode::Playback | Mode::Generator => Some(Document::Practice),
            Mode::Score | Mode::Edit | Mode::TimeSignature(_) => Some(Document::Score),
        }
    }

    /// Make `doc` the shown document and the working score. A different one
    /// stops playback, as it changes what plays, closes the metronome, whose
    /// click bar is the working score while it is open, and its run's marks
    /// are gone.
    pub(crate) fn show_document(&mut self, doc: Document) {
        if self.editor.document == doc {
            return;
        }
        // Also puts back a calibration's score before the swap.
        self.stop_transport();
        self.close_metronome();
        self.editor.swap_with_other();
        self.editor.document = doc;
        self.playback_ctl.playback.reset();
        self.playback_ctl.accuracy.tracker.clear_for_edit();
    }

    /// Show the document of the mode's page, whatever set the mode.
    pub(crate) fn sync_document(&mut self) {
        if let Some(doc) = self.document_of_mode() {
            self.show_document(doc);
        }
    }

    /// The score tab's document, wherever it is: the working score, put
    /// aside by the metronome tab, or waiting behind Practice.
    pub(crate) fn score_document_mut(&mut self) -> ScoreDocMut<'_> {
        let editor = &mut self.editor;
        if editor.document != Document::Score {
            return (&mut editor.other).into();
        }
        match editor.metronome.stash_mut() {
            Some(stash) => stash.into(),
            None => ScoreDocMut {
                dirty: &mut editor.dirty,
                active_pattern_id: &mut editor.active_pattern_id,
            },
        }
    }

    /// The score tab's score, see [`Self::score_document_mut`].
    pub(crate) fn score_document(&self) -> &Score {
        let editor = &self.editor;
        if editor.document != Document::Score {
            return &editor.other.score;
        }
        self.metronome_stash().map_or(&editor.score, |stash| &stash.score)
    }

    /// Whether the score tab's score has unsaved changes.
    pub(crate) fn score_dirty(&self) -> bool {
        let editor = &self.editor;
        if editor.document != Document::Score {
            return editor.other.dirty;
        }
        self.metronome_stash().map_or(editor.dirty, |stash| stash.dirty)
    }

    /// The saved pattern the score tab's score was loaded from or saved as.
    pub(crate) fn score_pattern(&self) -> Option<u64> {
        let editor = &self.editor;
        if editor.document != Document::Score {
            return editor.other.active_pattern_id;
        }
        self.metronome_stash().map_or(editor.active_pattern_id, |stash| stash.active_pattern_id)
    }

    /// Pattern `id` was deleted: no document is it any more.
    pub(crate) fn forget_pattern(&mut self, id: u64) {
        let editor = &mut self.editor;
        for active in [&mut editor.active_pattern_id, &mut editor.other.active_pattern_id] {
            if *active == Some(id) {
                *active = None;
            }
        }
        if let Some(stash) = editor.metronome.stash_mut()
            && stash.active_pattern_id == Some(id)
        {
            stash.active_pattern_id = None;
        }
    }

    /// The score tab's name for its score: the saved pattern's, "Untitled"
    /// without one, and "•" after it while it has unsaved changes.
    pub(crate) fn score_title(&self) -> String {
        let name = self
            .score_pattern()
            .and_then(|id| self.editor.library.get(id))
            .map_or_else(|| "Untitled".to_owned(), |p| p.name.clone());
        if self.score_dirty() { format!("{name} •") } else { name }
    }

    /// "Edit in Score" on the practice page: asks first when the score tab's
    /// score has unsaved changes, which Practice's rhythm would replace.
    pub(crate) fn request_edit_in_score(&mut self) {
        if self.score_dirty() {
            self.ui.pending_load = Some(PendingLoad::PracticeRhythm);
        } else {
            self.edit_in_score();
        }
    }

    /// Copy Practice's rhythm into the score tab (undoable there) and edit
    /// it. It is a new, unsaved score.
    pub(crate) fn edit_in_score(&mut self) {
        self.leave_metronome();
        let practice = match self.editor.document {
            Document::Practice => self.editor.score.clone(),
            Document::Score => self.editor.other.score.clone(),
        };
        self.show_document(Document::Score);
        self.with_undo_snapshot(|app| {
            app.editor.score = practice;
            app.editor.cursor = Cursor::start();
            true
        });
        self.editor.active_pattern_id = None;
        self.ui.picker = None;
        self.ui.mode = Mode::Edit;
    }
}

#[cfg(test)]
mod tests {
    use super::Document;
    use crate::presets::PresetId;
    use crate::shell::Tab;
    use crate::{APP_STATE_KEY, Grooph, Mode, PersistedState};
    use grooph_measure::notation::{format_score, parse_score};
    use grooph_measure::{Cursor, Score};
    use std::collections::HashMap;

    fn app() -> Grooph {
        let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        Grooph::new(&cc)
    }

    fn text(score: &Score) -> String {
        format_score(score).unwrap()
    }

    /// An app with a written kit groove in the score tab.
    fn app_with_groove() -> (Grooph, String) {
        let mut app = app();
        app.edit_score();
        let groove =
            parse_score("4/4 H:e S-H:e H:e H:e H:e S-H:e H:e H:e ; K:q r:q K:q r:q").unwrap();
        app.with_undo_snapshot(|app| {
            app.editor.score = groove;
            true
        });
        let groove = text(&app.editor.score);
        (app, groove)
    }

    #[test]
    fn no_generator_action_changes_the_score() {
        let (mut app, groove) = app_with_groove();
        app.select_tab(Tab::Practice);
        let mut practice = text(&app.editor.score);
        let mut changed = |app: &Grooph| {
            let now = text(&app.editor.score);
            let new = now != practice;
            practice = now;
            new
        };
        // 🎲, a setting, a preset, a generator link, also from the score tab.
        app.finish_generator_change(app.editor.generator.settings, false, true);
        assert!(changed(&app));
        let before = app.editor.generator.settings;
        app.editor.generator.settings.complexity = 3;
        app.finish_generator_change(before, false, false);
        assert!(changed(&app));
        app.apply_preset(PresetId::BuiltIn(1));
        assert!(changed(&app));
        app.select_tab(Tab::Score);
        let link = grooph_link::parse_query("v=2&sub=16&lvl=2&bars=1&seed=7").unwrap().unwrap();
        app.open_link(link);
        assert_eq!(app.tab(), Tab::Practice);
        assert!(app.ui.pending_load.is_none(), "Practice's rhythm is not asked for");
        assert!(changed(&app));
        assert_eq!(text(app.score_document()), groove);
        app.select_tab(Tab::Score);
        assert_eq!(text(&app.editor.score), groove);
    }

    #[test]
    fn a_tempo_link_keeps_the_scores_unsaved_mark() {
        let (mut app, _) = app_with_groove();
        assert!(app.score_dirty());
        app.open_link(grooph_link::parse_query("v=2&bpm=120").unwrap().unwrap());
        assert_eq!(app.playback_ctl.bpm, 120);
        assert!(app.score_dirty());
    }

    #[test]
    fn each_document_undoes_its_own_changes() {
        let (mut app, groove) = app_with_groove();
        app.select_tab(Tab::Practice);
        let first = text(&app.editor.score);
        app.generate_new_score();
        assert_ne!(text(&app.editor.score), first);
        // Undo on the practice page brings back its last rhythm, not the
        // score tab's groove.
        app.undo();
        assert_eq!(text(&app.editor.score), first);
        assert_eq!(text(app.score_document()), groove);
        // In the score tab: the groove's undo, back to the empty bar.
        app.select_tab(Tab::Score);
        app.undo();
        assert_ne!(text(&app.editor.score), groove);
        app.redo();
        assert_eq!(text(&app.editor.score), groove);
        app.select_tab(Tab::Practice);
        assert_eq!(text(&app.editor.score), first);
    }

    #[test]
    fn edit_in_score_copies_practices_rhythm_and_asks_first() {
        let (mut app, groove) = app_with_groove();
        app.select_tab(Tab::Practice);
        let practice = text(&app.editor.score);
        // The groove has unsaved changes: asked first, nothing changes yet.
        app.request_edit_in_score();
        assert!(app.ui.pending_load.is_some());
        assert_eq!(text(app.score_document()), groove);
        let pending = app.ui.pending_load.take().unwrap();
        app.finish_pending_load(pending);
        // The score tab's editor, on a new unsaved copy.
        assert_eq!(app.tab(), Tab::Score);
        assert_eq!(app.ui.mode, Mode::Edit);
        assert_eq!(text(&app.editor.score), practice);
        assert_eq!(app.editor.active_pattern_id, None);
        assert!(app.score_dirty());
        // Undo there brings the groove back; Practice keeps its rhythm.
        app.undo();
        assert_eq!(text(&app.editor.score), groove);
        app.select_tab(Tab::Practice);
        assert_eq!(text(&app.editor.score), practice);
        // A saved score is replaced without asking.
        app.save_active_pattern();
        app.request_edit_in_score();
        assert!(app.ui.pending_load.is_none());
        assert_eq!(text(&app.editor.score), practice);
        assert_eq!(app.tab(), Tab::Score);
    }

    #[test]
    fn the_library_saves_the_score_from_any_page() {
        let (mut app, groove) = app_with_groove();
        app.select_tab(Tab::Practice);
        app.select_tab(Tab::More);
        let id = app.save_active_pattern();
        assert_eq!(text(&app.editor.library.get(id).unwrap().score), groove);
        assert!(!app.score_dirty());
        assert_eq!(app.score_pattern(), Some(id));
        // Behind the metronome too.
        app.select_tab(Tab::Score);
        app.select_tab(Tab::Metronome);
        assert_eq!(app.score_pattern(), Some(id));
        app.delete_pattern(id);
        assert_eq!(app.score_pattern(), None);
        app.select_tab(Tab::Score);
        assert_eq!(app.editor.active_pattern_id, None);
        assert_eq!(text(&app.editor.score), groove);
        assert_eq!(app.score_title(), "Untitled");
    }

    #[test]
    fn endless_mode_runs_on_practices_rhythm_only() {
        let mut app = app();
        app.editor.generator.endless = true;
        app.select_tab(Tab::Practice);
        assert!(app.endless_runs());
        app.select_tab(Tab::More);
        assert!(app.endless_runs());
        app.select_tab(Tab::Score);
        assert!(!app.endless_runs());
        app.start_playback();
        assert_eq!(app.document(), Document::Score);
    }

    #[test]
    fn esc_and_back_open_and_close_the_score_editor() {
        let mut app = app();
        app.select_tab(Tab::Score);
        assert_eq!(app.ui.mode, Mode::Score);
        app.toggle_edit();
        assert_eq!(app.ui.mode, Mode::Edit);
        app.toggle_edit();
        assert_eq!(app.ui.mode, Mode::Score);
        app.edit_score();
        assert!(app.back());
        assert_eq!(app.ui.mode, Mode::Score);
        // The practice page has no editor.
        app.select_tab(Tab::Practice);
        app.toggle_edit();
        assert_eq!(app.ui.mode, Mode::Generator);
        // A mode set elsewhere brings its page's document.
        app.ui.mode = Mode::Edit;
        app.sync_document();
        assert_eq!(app.document(), Document::Score);
    }

    #[derive(Default)]
    struct MemoryStorage(HashMap<String, String>);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    fn restart(state: &PersistedState) -> Grooph {
        let mut storage = MemoryStorage::default();
        eframe::set_value(&mut storage, APP_STATE_KEY, state);
        let mut cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
        cc.storage = Some(&storage);
        Grooph::new(&cc)
    }

    #[test]
    fn both_documents_are_saved_from_any_page() {
        let (mut app, groove) = app_with_groove();
        app.select_tab(Tab::Practice);
        let practice = text(&app.editor.score);
        for tab in [Tab::Practice, Tab::Metronome, Tab::More, Tab::Score] {
            app.select_tab(tab);
            let state = PersistedState::from_app(&app);
            assert_eq!(text(&state.score), groove, "{tab:?}");
            assert_eq!(text(state.practice_score.as_ref().unwrap()), practice, "{tab:?}");
            assert!(state.dirty, "{tab:?}");
            let mut back = restart(&state);
            assert_eq!(back.tab(), tab);
            // More reopens on the document it was opened from.
            assert_eq!(back.document(), app.document(), "{tab:?}");
            back.select_tab(Tab::Score);
            assert_eq!(text(&back.editor.score), groove);
            assert!(back.editor.dirty);
            back.select_tab(Tab::Practice);
            assert_eq!(text(&back.editor.score), practice);
        }
    }

    #[test]
    fn a_state_saved_before_keeps_its_score_in_the_score_tab() {
        let old = parse_score("3/4 q e e q").unwrap();
        let state = PersistedState {
            score: old.clone(),
            cursor: Cursor::at(0, 2),
            dirty: true,
            ..Default::default()
        };
        // As saved before: no Practice rhythm, the tab still called Edit.
        let ron = ron::to_string(&state).unwrap().replace("tab:Metronome", "tab:Edit");
        let ron = ron.replace("practice_score:None,", "");
        assert!(!ron.contains("practice_score:"), "{ron}");
        let (state, backup) = crate::persist::restore(Some(&ron));
        assert!(backup.is_none());
        assert_eq!(state.tab, Tab::Score);
        let mut app = restart(&state);
        assert_eq!(app.tab(), Tab::Score);
        assert_eq!(text(&app.editor.score), text(&old));
        assert_eq!(app.editor.cursor, Cursor::at(0, 2));
        assert!(app.editor.dirty);
        // Practice starts with a new rhythm of its settings.
        app.select_tab(Tab::Practice);
        assert_ne!(text(&app.editor.score), text(&old));
        let bars = app.editor.generator.effective_bars();
        assert_eq!(app.editor.score.len(), bars);
    }
}
