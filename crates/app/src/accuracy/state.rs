use crate::TransportState;

use super::tracker::AccuracyTracker;

/// UI-level wrapper around `AccuracyTracker` that adds an `enabled` toggle and
/// reacts to the toggle in the context of the current transport state.
pub(crate) struct AccuracyState {
    pub(crate) tracker: AccuracyTracker,
    pub(crate) enabled: bool,
}

impl AccuracyState {
    pub(crate) fn new(enabled: bool) -> Self { Self { tracker: AccuracyTracker::new(), enabled } }

    pub(crate) fn set_enabled(&mut self, enabled: bool, transport: TransportState) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        if enabled {
            if transport == TransportState::Playing {
                self.tracker.on_playback_stop();
            } else {
                self.tracker.clear_for_edit();
            }
            return;
        }
        self.tracker.on_playback_stop();
    }
}
