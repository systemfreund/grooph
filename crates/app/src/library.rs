use grooph_measure::Score;
use serde::{Deserialize, Serialize};

/// A user-saved metronome pattern: a complete score plus its tempo, addressable
/// by a stable id and a human-readable name.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SavedPattern {
    pub id: u64,
    pub name: String,
    pub score: Score,
    pub bpm: u32,
}

/// Persistent collection of [`SavedPattern`]s. `next_id` is a monotonically
/// increasing counter so ids stay stable across renames, deletes and loads.
#[derive(Default, Clone, Serialize, Deserialize)]
pub(crate) struct PatternLibrary {
    pub patterns: Vec<SavedPattern>,
    pub next_id: u64,
}

impl PatternLibrary {
    /// Append a new pattern, returning its freshly allocated id.
    pub fn add(&mut self, name: String, score: Score, bpm: u32) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.patterns.push(SavedPattern { id, name, score, bpm });
        id
    }

    /// Remove the pattern with the given id, if present.
    pub fn remove(&mut self, id: u64) { self.patterns.retain(|p| p.id != id); }

    pub fn get(&self, id: u64) -> Option<&SavedPattern> {
        self.patterns.iter().find(|p| p.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::duration::q;
    use grooph_measure::{Beat, Measure, TimeSignature};

    fn sample_score() -> Score {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.set_beat(0, Beat::note(q())).unwrap();
        Score::single(m)
    }

    #[test]
    fn add_allocates_increasing_ids() {
        let mut lib = PatternLibrary::default();
        let a = lib.add("a".into(), sample_score(), 120);
        let b = lib.add("b".into(), sample_score(), 90);
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(lib.patterns.len(), 2);
    }

    #[test]
    fn remove_drops_only_target() {
        let mut lib = PatternLibrary::default();
        let a = lib.add("a".into(), sample_score(), 120);
        let b = lib.add("b".into(), sample_score(), 90);
        lib.remove(a);
        assert!(lib.get(a).is_none());
        assert!(lib.get(b).is_some());
    }
}
