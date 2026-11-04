//! Natively without the `device` feature: no microphone. The detector and
//! the rest of the crate work as with one, for tests on machines without the
//! audio system's headers (cpal needs them).

use crate::Shared;
use std::sync::Arc;

pub(crate) struct Backend;

impl Backend {
    pub(crate) fn open(_shared: Arc<Shared>) -> Result<Self, String> {
        Err("built without microphone input (feature `device`)".into())
    }
}
