use crate::measure::beat::BeatKind::{Note, Rest};
use crate::measure::duration::Duration;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BeatKind {
    Note,
    Rest,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Beat {
    pub duration: Duration,
    pub kind: BeatKind,
    pub tremolo: Option<Tremolo>,
    pub accented: bool,
    /// Identifier of a tuplet group this beat belongs to
    pub tuplet_group_id: Option<u32>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Tremolo {
    /// Number of slashes (1..=3 typical)
    pub slashes: u8,
    /// If true, indicates measured tremolo; otherwise unmeasured (for future use)
    pub measured: bool,
}

impl Beat {
    /// Creates a new note with the given duration
    pub fn note(duration: Duration) -> Self {
        Self { duration, kind: Note, tremolo: None, accented: false, tuplet_group_id: None }
    }

    /// Creates a new rest with the given duration
    pub fn rest(duration: Duration) -> Self {
        Self { duration, kind: Rest, tremolo: None, accented: false, tuplet_group_id: None }
    }

    pub fn new(duration: Duration, kind: BeatKind) -> Self {
        Self { duration, kind, tremolo: None, accented: false, tuplet_group_id: None }
    }

    pub fn with_kind(&self, kind: BeatKind) -> Self {
        Self {
            duration: self.duration,
            kind,
            tremolo: self.tremolo,
            accented: self.accented && kind == Note,
            tuplet_group_id: self.tuplet_group_id,
        }
    }
}
