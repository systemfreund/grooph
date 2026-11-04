use crate::TransportState;

use super::tracker::AccuracyTracker;

/// UI-level wrapper around `AccuracyTracker` that reacts to the timing
/// feedback switch (`FeedbackSettings::accuracy_enabled`) in the context of
/// the current transport state.
pub(crate) struct AccuracyState {
    pub(crate) tracker: AccuracyTracker,
}

impl Default for AccuracyState {
    fn default() -> Self {
        Self { tracker: AccuracyTracker::new() }
    }
}

impl AccuracyState {
    /// Switch timing feedback (`enabled`, the setting) on or off.
    pub(crate) fn set_enabled(&mut self, enabled: &mut bool, on: bool, transport: TransportState) {
        if *enabled == on {
            return;
        }
        *enabled = on;
        if on {
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
