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
    pub accented: bool,
    /// Identifier of a tuplet group this beat belongs to
    pub tuplet_group_id: Option<u32>,
}

impl Beat {
    /// Creates a new note with the given duration
    pub fn note(duration: Duration) -> Self {
        Self { duration, kind: Note, accented: false, tuplet_group_id: None }
    }

    /// Creates a new rest with the given duration
    pub fn rest(duration: Duration) -> Self {
        Self { duration, kind: Rest, accented: false, tuplet_group_id: None }
    }

    pub fn new(duration: Duration, kind: BeatKind) -> Self {
        Self { duration, kind, accented: false, tuplet_group_id: None }
    }

    pub fn with_kind(&self, kind: BeatKind) -> Self {
        Self {
            duration: self.duration,
            kind,
            accented: self.accented && kind == Note,
            tuplet_group_id: self.tuplet_group_id,
        }
    }
}
