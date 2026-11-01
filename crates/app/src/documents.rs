//! The documents of the three pages: the metronome's click bar, Practice's
//! generated rhythm and the score tab's own score (written, loaded from the
//! library or opened from a link).
//!
//! Each keeps its score, cursor, undo history, dirty flag and pattern
//! ([`RhythmDoc`]), and each page edits its own in place: the metronome's
//! "Edit clicks", Practice's ✏ and the score tab's ✏. The page shown, or the
//! one the More screen was opened from, is [`EditorState::document`]; the
//! staff, the tools and undo work on its document ([`EditorState::doc`]).
//! The generator only ever writes Practice's document (see
//! [`Grooph::generate_new_score`]), so a written score is never replaced by a
//! generated rhythm.
//!
//! [`EditorState::document`]: crate::state::EditorState::document
//! [`EditorState::doc`]: crate::state::EditorState::doc

use crate::shell::Tab;
use crate::state::EditorState;
use crate::undo::{DEFAULT_UNDO_LIMIT, UndoHistory};
use crate::{Grooph, Mode};
use grooph_measure::{Cursor, Score};
use serde::{Deserialize, Serialize};

/// One of the three documents, by the page that shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Document {
    /// The metronome's click bar (with its counter rhythm).
    Metronome,
    /// The generator's rhythm on the practice page: replaced freely.
    Practice,
    /// The score tab's score.
    #[default]
    Score,
}

impl Document {
    /// The page that shows the document.
    pub(crate) const fn tab(self) -> Tab {
        match self {
            Document::Metronome => Tab::Metronome,
            Document::Practice => Tab::Practice,
            Document::Score => Tab::Score,
        }
    }
}

/// A page's document: its score and how far it was edited.
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

/// The three documents.
pub(crate) struct Documents {
    pub(crate) metronome: RhythmDoc,
    pub(crate) practice: RhythmDoc,
    pub(crate) score: RhythmDoc,
}

impl Documents {
    pub(crate) fn get(&self, doc: Document) -> &RhythmDoc {
        match doc {
            Document::Metronome => &self.metronome,
            Document::Practice => &self.practice,
            Document::Score => &self.score,
        }
    }

    pub(crate) fn get_mut(&mut self, doc: Document) -> &mut RhythmDoc {
        match doc {
            Document::Metronome => &mut self.metronome,
            Document::Practice => &mut self.practice,
            Document::Score => &mut self.score,
        }
    }
}

impl EditorState {
    /// The document of the page shown (or behind the More screen).
    pub(crate) fn doc(&self) -> &RhythmDoc {
        self.docs.get(self.document)
    }

    pub(crate) fn doc_mut(&mut self) -> &mut RhythmDoc {
        self.docs.get_mut(self.document)
    }
}

impl Grooph {
    /// The document of the page shown, or of the page the More screen was
    /// opened from.
    pub(crate) fn document(&self) -> Document {
        self.editor.document
    }

    /// Make `doc`'s page the one whose document is edited and played. A
    /// different one stops playback, as it changes what plays, and the run's
    /// marks are gone. The mode is left to the caller.
    pub(crate) fn show_document(&mut self, doc: Document) {
        if self.editor.document == doc {
            return;
        }
        // Also cancels a calibration run.
        self.stop_transport();
        if self.metronome_mode() {
            self.close_metronome();
        }
        if doc == Document::Metronome {
            self.open_metronome();
        }
        self.editor.document = doc;
        self.playback_ctl.playback.reset();
        self.playback_ctl.accuracy.tracker.clear_for_edit();
    }

    /// The score tab's name for its score: the saved pattern's, "Untitled"
    /// without one, and "•" after it while it has unsaved changes.
    pub(crate) fn score_title(&self) -> String {
        let doc = &self.editor.docs.score;
        let name = doc
            .active_pattern_id
            .and_then(|id| self.editor.library.get(id))
            .map_or_else(|| "Untitled".to_owned(), |p| p.name.clone());
        if doc.dirty { format!("{name} •") } else { name }
    }

    /// The tempo changed: a saved score keeps its tempo, so that is an
    /// unsaved change of the score tab's score when it is shown or is a
    /// saved pattern, whichever page the tempo was changed on.
    pub(crate) fn mark_tempo_unsaved(&mut self) {
        let doc = &mut self.editor.docs.score;
        if self.editor.document == Document::Score || doc.active_pattern_id.is_some() {
            doc.dirty = true;
        }
    }

    /// Pattern `id` was deleted: no document is it any more.
    pub(crate) fn forget_pattern(&mut self, id: u64) {
        let docs = &mut self.editor.docs;
        for doc in [&mut docs.metronome, &mut docs.practice, &mut docs.score] {
            if doc.active_pattern_id == Some(id) {
                doc.active_pattern_id = None;
            }
        }
    }

    /// A page's editor (✏, "Edit clicks"): its tool palette on its document.
    pub(crate) fn edit_page(&mut self) {
        if self.tab() == Tab::More {
            self.select_tab(self.document().tab());
        }
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

    fn score_text(app: &Grooph) -> String {
        text(&app.editor.docs.score.score)
    }

    fn practice_text(app: &Grooph) -> String {
        text(&app.editor.docs.practice.score)
    }

    /// An app with a written kit groove in the score tab.
    fn app_with_groove() -> (Grooph, String) {
        let mut app = app();
        app.select_tab(Tab::Score);
        app.edit_page();
        let groove =
            parse_score("4/4 H:e S-H:e H:e H:e H:e S-H:e H:e H:e ; K:q r:q K:q r:q").unwrap();
        app.with_undo_snapshot(|app| {
            app.editor.doc_mut().score = groove;
            true
        });
        let groove = score_text(&app);
        (app, groove)
    }

    #[test]
    fn each_page_edits_its_own_document() {
        let (mut app, groove) = app_with_groove();
        assert_eq!(app.document(), Document::Score);
        // Practice's ✏ stays on the practice page, on its rhythm.
        app.select_tab(Tab::Practice);
        assert_eq!(app.ui.mode, Mode::Playback);
        app.edit_page();
        assert_eq!(app.tab(), Tab::Practice);
        assert_eq!(app.ui.mode, Mode::Edit);
        assert_eq!(app.document(), Document::Practice);
        app.with_undo_snapshot(|app| {
            app.editor.doc_mut().score = parse_score("4/4 q q q q").unwrap();
            true
        });
        assert_eq!(practice_text(&app), text(&parse_score("4/4 q q q q").unwrap()));
        assert_eq!(score_text(&app), groove);
        // Done closes the editor, still on the practice page.
        assert!(app.back());
        assert_eq!(app.ui.mode, Mode::Playback);
        assert_eq!(app.tab(), Tab::Practice);
        // ✏ from the More screen opens the page it was opened from.
        app.select_tab(Tab::More);
        app.edit_page();
        assert_eq!(app.tab(), Tab::Practice);
        assert_eq!(app.ui.mode, Mode::Edit);
    }

    #[test]
    fn no_generator_action_changes_the_score() {
        let (mut app, groove) = app_with_groove();
        app.select_tab(Tab::Practice);
        // Before each action Practice holds a bar no generator writes, so
        // the action shows by replacing it, also with a rhythm that comes
        // out the same as the one before by chance.
        let unset = parse_score("5/4 q q q q q").unwrap();
        let set_aside = |app: &mut Grooph| app.editor.docs.practice.score = unset.clone();
        let changed = |app: &Grooph| practice_text(app) != text(&unset);
        // 🎲, a setting, a preset, a generator link, also from the score tab.
        set_aside(&mut app);
        app.finish_generator_change(app.editor.generator.settings, false, true);
        assert!(changed(&app));
        set_aside(&mut app);
        let before = app.editor.generator.settings;
        app.editor.generator.settings.complexity = 3;
        app.finish_generator_change(before, false, false);
        assert!(changed(&app));
        set_aside(&mut app);
        app.apply_preset(PresetId::BuiltIn(1));
        assert!(changed(&app));
        set_aside(&mut app);
        app.select_tab(Tab::Score);
        let link = grooph_link::parse_query("v=2&sub=16&lvl=2&bars=1&seed=7").unwrap().unwrap();
        app.open_link(link);
        assert_eq!(app.tab(), Tab::Practice);
        assert!(app.ui.pending_load.is_none(), "Practice's rhythm is not asked for");
        assert!(changed(&app));
        assert_eq!(score_text(&app), groove);
        assert!(app.editor.docs.score.dirty);
    }

    #[test]
    fn a_tempo_link_keeps_the_scores_unsaved_mark() {
        let (mut app, _) = app_with_groove();
        assert!(app.editor.docs.score.dirty);
        app.open_link(grooph_link::parse_query("v=2&bpm=120").unwrap().unwrap());
        assert_eq!(app.playback_ctl.bpm, 120);
        assert!(app.editor.docs.score.dirty);
    }

    #[test]
    fn a_tempo_link_marks_a_saved_score_unsaved() {
        let (mut app, _) = app_with_groove();
        app.save_active_pattern();
        app.select_tab(Tab::Practice);
        app.playback_ctl.bpm = 100;
        app.open_link(grooph_link::parse_query("v=2&bpm=120").unwrap().unwrap());
        assert!(app.editor.docs.score.dirty);
    }

    #[test]
    fn each_document_undoes_its_own_changes() {
        let (mut app, groove) = app_with_groove();
        app.select_tab(Tab::Practice);
        // A bar no generator writes, so the new rhythm always differs.
        app.editor.docs.practice.score = parse_score("5/4 q q q q q").unwrap();
        let first = practice_text(&app);
        app.generate_new_score();
        assert_ne!(practice_text(&app), first);
        // Undo on the practice page brings back its last rhythm, not the
        // score tab's groove.
        app.undo();
        assert_eq!(practice_text(&app), first);
        assert_eq!(score_text(&app), groove);
        // In the score tab: the groove's undo, back to the empty bar.
        app.select_tab(Tab::Score);
        app.undo();
        assert_ne!(score_text(&app), groove);
        app.redo();
        assert_eq!(score_text(&app), groove);
        assert_eq!(practice_text(&app), first);
    }

    #[test]
    fn the_library_saves_the_score_from_any_page() {
        let (mut app, groove) = app_with_groove();
        app.select_tab(Tab::Practice);
        app.select_tab(Tab::More);
        let id = app.save_active_pattern();
        assert_eq!(text(&app.editor.library.get(id).unwrap().score), groove);
        assert!(!app.editor.docs.score.dirty);
        assert_eq!(app.editor.docs.score.active_pattern_id, Some(id));
        // From the metronome too, which keeps running.
        app.select_tab(Tab::Metronome);
        app.start_playback();
        app.save_active_pattern();
        assert!(app.metronome_mode());
        assert_eq!(app.editor.docs.score.active_pattern_id, Some(id));
        app.delete_pattern(id);
        assert_eq!(app.editor.docs.score.active_pattern_id, None);
        assert_eq!(score_text(&app), groove);
        assert_eq!(app.score_title(), "Untitled");
    }

    #[test]
    fn a_tempo_change_on_another_page_marks_the_saved_score_unsaved() {
        let mut app = app();
        // An untitled score stays as it is.
        app.select_tab(Tab::Practice);
        app.playback_ctl.bpm += 5;
        app.mark_tempo_unsaved();
        assert!(!app.editor.docs.score.dirty);
        // A saved pattern keeps its tempo: changed anywhere, it can be saved.
        let (mut app, _) = app_with_groove();
        let id = app.save_active_pattern();
        for tab in [Tab::Practice, Tab::Metronome] {
            app.select_tab(tab);
            app.editor.docs.score.dirty = false;
            app.playback_ctl.bpm += 5;
            app.mark_tempo_unsaved();
            assert!(app.editor.docs.score.dirty, "{tab:?}");
            assert_eq!(app.editor.docs.score.active_pattern_id, Some(id));
        }
    }

    #[test]
    fn a_preset_with_another_tempo_marks_the_saved_score_unsaved() {
        let (mut app, _) = app_with_groove();
        app.save_active_pattern();
        app.select_tab(Tab::Practice);
        app.playback_ctl.bpm = 1;
        app.apply_preset(PresetId::BuiltIn(1));
        assert_ne!(app.playback_ctl.bpm, 1);
        assert!(app.editor.docs.score.dirty);
    }

    #[test]
    fn leaving_the_metronome_shows_practice() {
        let mut app = app();
        app.select_tab(Tab::Score);
        app.select_tab(Tab::Metronome);
        app.leave_metronome();
        assert_eq!(app.document(), Document::Practice);
    }

    #[test]
    fn endless_mode_runs_on_practices_rhythm_only() {
        let mut app = app();
        app.editor.generator.endless = true;
        app.select_tab(Tab::Practice);
        assert!(app.endless_runs());
        app.select_tab(Tab::More);
        assert!(app.endless_runs());
        // Practice's editor plays its rhythm as it is, also behind its
        // meter dialog.
        app.edit_page();
        assert!(!app.endless_runs());
        app.select_tab(Tab::Score);
        assert!(!app.endless_runs());
        app.start_playback();
        assert_eq!(app.document(), Document::Score);
    }

    #[test]
    fn esc_and_back_open_and_close_each_pages_editor() {
        let mut app = app();
        for tab in [Tab::Score, Tab::Practice] {
            app.select_tab(tab);
            assert_eq!(app.ui.mode, Mode::Playback, "{tab:?}");
            app.toggle_edit();
            assert_eq!(app.ui.mode, Mode::Edit, "{tab:?}");
            app.toggle_edit();
            assert_eq!(app.ui.mode, Mode::Playback, "{tab:?}");
            app.edit_page();
            assert!(app.back());
            assert_eq!(app.ui.mode, Mode::Playback, "{tab:?}");
            assert_eq!(app.tab(), tab);
        }
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
    fn both_rhythm_documents_are_saved_from_any_page() {
        let (mut app, groove) = app_with_groove();
        app.ui.show_score_tab = true;
        app.select_tab(Tab::Practice);
        let practice = practice_text(&app);
        for tab in [Tab::Practice, Tab::Metronome, Tab::More, Tab::Score] {
            app.select_tab(tab);
            let state = PersistedState::from_app(&app);
            assert_eq!(text(&state.score_doc.score), groove, "{tab:?}");
            let practice_doc = state.practice_doc.as_ref().unwrap();
            assert_eq!(text(&practice_doc.score), practice, "{tab:?}");
            assert!(state.score_doc.dirty, "{tab:?}");
            let back = restart(&state);
            assert_eq!(back.tab(), tab);
            // More reopens on the document it was opened from.
            assert_eq!(back.document(), app.document(), "{tab:?}");
            assert_eq!(score_text(&back), groove);
            assert!(back.editor.docs.score.dirty);
            assert_eq!(practice_text(&back), practice);
        }
    }

    #[test]
    fn the_score_tab_is_hidden_until_the_developer_setting_shows_it() {
        let (mut app, groove) = app_with_groove();
        assert!(!app.ui.show_score_tab);
        // Saved on the score tab, it reopens on Practice, with its score.
        app.select_tab(Tab::Score);
        let state = PersistedState::from_app(&app);
        let back = restart(&state);
        assert_eq!(back.tab(), Tab::Practice);
        assert_eq!(score_text(&back), groove);
        // Shown, the setting is kept and the score tab reopens.
        app.ui.show_score_tab = true;
        let back = restart(&PersistedState::from_app(&app));
        assert!(back.ui.show_score_tab);
        assert_eq!(back.tab(), Tab::Score);
    }

    #[test]
    fn a_state_saved_before_keeps_its_score_in_the_score_tab() {
        let old = parse_score("3/4 q e e q").unwrap();
        // As saved before (version 2): the score tab's document flat, no
        // Practice rhythm, the tab still called Edit.
        let ron = format!(
            "(version:2,score:{},cursor:{},dirty:true,tab:Edit)",
            ron::to_string(&old).unwrap(),
            ron::to_string(&Cursor::at(0, 2)).unwrap()
        );
        let (mut state, backup) = crate::persist::restore(Some(&ron));
        assert!(backup.is_none());
        assert_eq!(state.tab, Tab::Score);
        state.show_score_tab = true;
        let app = restart(&state);
        assert_eq!(app.tab(), Tab::Score);
        // The score tab is for drum grooves: a rhythm saved there starts a
        // new groove of its meter.
        assert!(app.editor.docs.score.score.staff == grooph_measure::StaffKind::Kit);
        assert_eq!(
            app.editor.docs.score.score.measures()[0].time_signature(),
            old.measures()[0].time_signature()
        );
        assert!(!app.editor.docs.score.dirty);
        // Practice starts with a new rhythm of its settings.
        assert_ne!(practice_text(&app), text(&old));
        let bars = app.editor.generator.effective_bars();
        assert_eq!(app.editor.docs.practice.score.len(), bars);
    }
}
