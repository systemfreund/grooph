use grooph_measure::{Cursor, Score};
use std::collections::VecDeque;

pub(crate) const DEFAULT_UNDO_LIMIT: usize = 200;

pub(crate) struct EditorSnapshot {
    pub score: Score,
    pub cursor: Cursor,
}

pub(crate) struct UndoHistory {
    past: VecDeque<EditorSnapshot>,
    future: Vec<EditorSnapshot>,
    limit: usize,
}

impl UndoHistory {
    pub fn new(limit: usize) -> Self { Self { past: VecDeque::new(), future: Vec::new(), limit } }

    pub fn push(&mut self, snap: EditorSnapshot) {
        if self.past.len() >= self.limit {
            self.past.pop_front();
        }
        self.past.push_back(snap);
        self.future.clear();
    }

    pub fn pop_undo(&mut self, current: EditorSnapshot) -> Option<EditorSnapshot> {
        let prev = self.past.pop_back()?;
        self.future.push(current);
        Some(prev)
    }

    pub fn pop_redo(&mut self, current: EditorSnapshot) -> Option<EditorSnapshot> {
        let next = self.future.pop()?;
        self.past.push_back(current);
        Some(next)
    }

    pub fn can_undo(&self) -> bool { !self.past.is_empty() }

    pub fn can_redo(&self) -> bool { !self.future.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::duration::q;
    use grooph_measure::{Beat, Measure, TimeSignature};

    fn snap(beat_idx: usize) -> EditorSnapshot {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.set_beat(0, Beat::note(q())).unwrap();
        EditorSnapshot { score: Score::single(m), cursor: Cursor::at(0, beat_idx) }
    }

    fn beat_at(snap: &EditorSnapshot) -> usize { snap.cursor.beat_idx }

    #[test]
    fn push_then_undo_restores_previous_snapshot() {
        let mut h = UndoHistory::new(10);
        h.push(snap(1));
        let restored = h.pop_undo(snap(2)).expect("had snapshot");
        assert_eq!(beat_at(&restored), 1);
    }

    #[test]
    fn redo_after_undo_replays_state() {
        let mut h = UndoHistory::new(10);
        h.push(snap(1));
        let prev = h.pop_undo(snap(2)).unwrap();
        // simulate state restored from undo
        let after_redo = h.pop_redo(prev).unwrap();
        assert_eq!(beat_at(&after_redo), 2);
    }

    #[test]
    fn new_push_clears_future_stack() {
        let mut h = UndoHistory::new(10);
        h.push(snap(1));
        let _ = h.pop_undo(snap(2));
        assert!(h.can_redo());
        h.push(snap(3));
        assert!(!h.can_redo());
    }

    #[test]
    fn undo_empty_is_noop() {
        let mut h = UndoHistory::new(10);
        assert!(!h.can_undo());
        assert!(h.pop_undo(snap(0)).is_none());
        assert!(!h.can_redo());
    }

    #[test]
    fn redo_empty_is_noop() {
        let mut h = UndoHistory::new(10);
        assert!(!h.can_redo());
        assert!(h.pop_redo(snap(0)).is_none());
        assert!(!h.can_undo());
    }

    #[test]
    fn limit_evicts_oldest_when_exceeded() {
        let mut h = UndoHistory::new(3);
        h.push(snap(1));
        h.push(snap(2));
        h.push(snap(3));
        h.push(snap(4)); // evicts snap(1)
        // pop in LIFO order: 4, 3, 2 ... no more
        assert_eq!(beat_at(&h.pop_undo(snap(99)).unwrap()), 4);
        assert_eq!(beat_at(&h.pop_undo(snap(99)).unwrap()), 3);
        assert_eq!(beat_at(&h.pop_undo(snap(99)).unwrap()), 2);
        assert!(h.pop_undo(snap(99)).is_none());
    }

    #[test]
    fn clone_independence_via_push() {
        // Pushed snapshot must not be affected by later mutation of the source score.
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.set_beat(0, Beat::note(q())).unwrap();
        let mut score = Score::single(m);
        let snap = EditorSnapshot { score: score.clone(), cursor: Cursor::start() };

        let mut h = UndoHistory::new(10);
        h.push(snap);

        // mutate source after push
        score.current_mut(0).set_beat(1, Beat::note(q())).unwrap();

        // restored snapshot retains original beat count (before mutation)
        let restored = h.pop_undo(EditorSnapshot { score, cursor: Cursor::start() }).unwrap();
        // the mutation added a beat to the *source* score but the snapshot is independent
        // we just check that pop returns something with a distinct measure state
        assert_eq!(restored.score.current(0).beats().len(), 4); // original 4/4 default beats
    }
}
