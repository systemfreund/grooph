use grooph_measure::Score;
use serde::{Deserialize, Serialize};

/// `"{prefix} N"` with N one more than the highest such name in `names`, so
/// a default name does not repeat after a deletion.
pub(crate) fn next_default_name<'a>(prefix: &str, names: impl Iterator<Item = &'a str>) -> String {
    let highest = names
        .filter_map(|name| name.strip_prefix(prefix)?.strip_prefix(' ')?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    format!("{prefix} {}", highest + 1)
}

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

    /// Overwrite the score + tempo of an existing pattern, keeping its id and
    /// name. Returns `true` if a matching pattern was found and updated.
    pub fn update(&mut self, id: u64, score: Score, bpm: u32) -> bool {
        if let Some(p) = self.patterns.iter_mut().find(|p| p.id == id) {
            p.score = score;
            p.bpm = bpm;
            true
        } else {
            false
        }
    }

    /// Remove the pattern with the given id, if present.
    pub fn remove(&mut self, id: u64) {
        self.patterns.retain(|p| p.id != id);
    }

    pub fn get(&self, id: u64) -> Option<&SavedPattern> {
        self.patterns.iter().find(|p| p.id == id)
    }

    pub fn contains(&self, id: u64) -> bool {
        self.patterns.iter().any(|p| p.id == id)
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

    #[test]
    fn default_names_do_not_repeat_after_a_deletion() {
        use super::next_default_name;
        let names = ["Pattern 1", "Pattern 3", "Groove", "Pattern x"];
        assert_eq!(next_default_name("Pattern", names.into_iter()), "Pattern 4");
        assert_eq!(next_default_name("Preset", names.into_iter()), "Preset 1");
    }
}
