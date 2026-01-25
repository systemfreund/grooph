use std::fmt::{Debug, Formatter};
use crate::measure::beat::BeatKind::{Note, Rest};
use crate::measure::duration::{duration_to_debug_str, Duration};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BeatKind {
    Note,
    Rest,
}

#[derive(Copy, Clone, Eq, Serialize, Deserialize)]
pub struct Beat {
    pub duration: Duration,
    pub kind: BeatKind,
    pub accented: bool,
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

impl PartialEq for Beat {
    fn eq(&self, other: &Self) -> bool {
        // Excludes `tuplet_group_id` from the comparison
        self.duration == other.duration
            && self.kind == other.kind
            && self.accented == other.accented
    }
}

impl Debug for Beat {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.kind == Rest {
            write!(f, "(")?
        }
        self.duration.fmt(f)?;
        if self.kind == Rest {
            write!(f, ")")?
        }
        Ok(())
    }
}
