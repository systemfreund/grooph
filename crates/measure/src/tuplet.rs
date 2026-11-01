//! Tuplet group identity and registry.
//!
//! Beats that belong to the same tuplet group share a stable
//! [`TupletGroupId`]; the [`TupletRegistry`] owns the per-group [`TupletAnchor`]
//! metadata (n:m ratio, frozen tick span, base hint) and hands out fresh ids.

use crate::duration::{Duration, NoteValue};
use crate::grid::DEFAULT_GRID;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Stable identifier for a tuplet group within a [`crate::Measure`].
///
/// Wraps a `u32` for type-safety so that "tuplet group id" can't be confused
/// with a beat index or anchor count. The wire format matches a plain `u32`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TupletGroupId(pub u32);

impl TupletGroupId {
    pub const fn new(id: u32) -> Self {
        Self(id)
    }
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

impl From<u32> for TupletGroupId {
    fn from(id: u32) -> Self {
        Self(id)
    }
}

/// Stable anchor describing a tuplet group span and semantics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "StoredAnchor")]
pub struct TupletAnchor {
    pub id: TupletGroupId,
    pub n: u8,
    pub m: u8,
    /// Frozen logical span = ticks(Simple(base_hint)) * m
    pub target_ticks: u32,
    /// Intended base for UI/export; not authoritative for grid validity
    pub base_hint: NoteValue,
}

/// A [`TupletAnchor`] as saved. Its span is recomputed on load: it was saved
/// in the ticks of the grid of its time, which grows with the durations.
#[derive(Deserialize)]
struct StoredAnchor {
    id: TupletGroupId,
    n: u8,
    m: u8,
    target_ticks: u32,
    base_hint: NoteValue,
}

impl From<StoredAnchor> for TupletAnchor {
    fn from(a: StoredAnchor) -> Self {
        let StoredAnchor { id, n, m, target_ticks, base_hint } = a;
        let target_ticks = DEFAULT_GRID
            .ticks_of(&Duration::Simple(base_hint))
            .map_or(target_ticks, |base| u32::from(m) * base);
        Self { id, n, m, target_ticks, base_hint }
    }
}

/// Registry of tuplet anchors for a measure.
///
/// Encapsulates the anchor table plus id allocation so that callers cannot
/// accidentally register two anchors under the same id or leak an unregistered
/// id back onto a beat.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TupletRegistry {
    /// Stored map keyed by raw `u32`. Keeping the raw key keeps the JSON wire
    /// format identical to the previous `HashMap<u32, TupletAnchor>` and lets
    /// HashMap hash on a primitive.
    #[serde(rename = "tuplet_anchors")]
    anchors: HashMap<u32, TupletAnchor>,
    /// Next id to hand out from [`Self::register`].
    #[serde(rename = "next_tuplet_id")]
    next: u32,
}

impl Default for TupletRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TupletRegistry {
    pub fn new() -> Self {
        Self { anchors: HashMap::new(), next: 1 }
    }

    /// Allocate a fresh id and store an anchor for it. Returns the new id.
    pub fn register(
        &mut self,
        n: u8,
        m: u8,
        target_ticks: u32,
        base_hint: NoteValue,
    ) -> TupletGroupId {
        let id = TupletGroupId(self.next);
        self.next = self.next.saturating_add(1);
        self.anchors.insert(id.0, TupletAnchor { id, n, m, target_ticks, base_hint });
        id
    }

    /// Drop the anchor for `id` and return it if it existed.
    pub fn unregister(&mut self, id: TupletGroupId) -> Option<TupletAnchor> {
        self.anchors.remove(&id.0)
    }

    /// Look up the anchor for `id`.
    pub fn get(&self, id: TupletGroupId) -> Option<&TupletAnchor> {
        self.anchors.get(&id.0)
    }

    /// Make sure [`Self::register`] hands out no id of an anchor again.
    pub(crate) fn reserve_anchor_ids(&mut self) {
        if let Some(max) = self.anchors.keys().max() {
            self.next = self.next.max(max.saturating_add(1));
        }
    }

    /// Keep the anchors of the groups `keep` names, drop the others.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(TupletGroupId) -> bool) {
        self.anchors.retain(|_, anchor| keep(anchor.id));
    }

    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }

    pub fn len(&self) -> usize {
        self.anchors.len()
    }
}

#[cfg(test)]
mod tests {
    use crate::duration::{q, t8};
    use crate::{Beat, Measure, TimeSignature};

    #[test]
    fn saved_spans_are_recomputed_on_the_current_grid() {
        let mut m = Measure::new(TimeSignature::FOUR_FOUR);
        m.first_voice_mut().set_beat(0, Beat::note(t8())).unwrap();
        let saved = ron::to_string(&m).unwrap();
        let span = m
            .first_voice()
            .tuplets()
            .get(m.first_voice().beats()[0].tuplet_group_id.unwrap())
            .unwrap()
            .target_ticks;
        // Saved when the grid had half as many ticks.
        let old =
            saved.replace(&format!("target_ticks:{span}"), &format!("target_ticks:{}", span / 2));
        assert_ne!(old, saved);
        let mut back: Measure = ron::from_str(&old).unwrap();
        assert!(back.first_voice_mut().dissolve_tuplet_group(0).is_some());
        assert_eq!(back.first_voice().beats()[0].duration, q());
        assert_eq!(back.first_voice().beats().len(), 4);
    }
}
