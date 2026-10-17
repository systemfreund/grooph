//! Microphone input: detects hits (sticks, claps, a practice pad) live and
//! timestamps them where they start in the signal.
//!
//! Natively the input comes from cpal; on the web from `getUserMedia` through
//! an AudioWorklet that forwards sample blocks to the page. Both run the same
//! [`OnsetDetector`].

mod detector;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

pub use detector::{DEFAULT_THRESHOLD_DB, OnsetDetector, REFRACTORY_S, db_to_linear, linear_to_db};

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Called when hits arrive, e.g. to request a repaint.
pub type Notifier = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug, PartialEq)]
pub enum MicStatus {
    /// Waiting for permission or the device.
    Starting,
    /// Listening on the named device.
    Running(String),
    Failed(String),
}

#[derive(Debug)]
struct State {
    hits: Vec<f64>,
    /// Highest level since the last [`MicInput::take_peak`] (linear).
    peak: f32,
    status: MicStatus,
}

/// State shared between the input callback and the app.
pub(crate) struct Shared {
    state: Mutex<State>,
    threshold_db: AtomicU32,
    notifier: Option<Notifier>,
}

impl Shared {
    fn threshold_db(&self) -> f32 {
        f32::from_bits(self.threshold_db.load(Ordering::Relaxed))
    }

    fn with_state<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut state)
    }

    /// Hand over a block's hits and peak level.
    pub(crate) fn push(&self, hits: &[f64], peak: f32) {
        self.with_state(|s| {
            s.hits.extend_from_slice(hits);
            s.peak = s.peak.max(peak);
        });
        if !hits.is_empty()
            && let Some(notify) = &self.notifier
        {
            notify();
        }
    }

    pub(crate) fn set_status(&self, status: MicStatus) {
        self.with_state(|s| s.status = status);
        if let Some(notify) = &self.notifier {
            notify();
        }
    }
}

/// An open microphone. Listening stops when it is dropped.
pub struct MicInput {
    shared: Arc<Shared>,
    #[cfg(not(target_arch = "wasm32"))]
    _backend: Option<native::Backend>,
    #[cfg(target_arch = "wasm32")]
    _backend: web::Backend,
}

impl MicInput {
    /// Start listening on the system's default input, except on an iPhone or
    /// iPad, where the built-in microphone wins over a headset's (headset
    /// microphones filter out percussive sounds). On the web this asks for
    /// permission and finishes in the background; [`MicInput::status`] tells
    /// when it runs.
    pub fn open(threshold_db: f32, notifier: Option<Notifier>) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State { hits: Vec::new(), peak: 0.0, status: MicStatus::Starting }),
            threshold_db: AtomicU32::new(threshold_db.to_bits()),
            notifier,
        });
        #[cfg(not(target_arch = "wasm32"))]
        let backend = match native::Backend::open(shared.clone()) {
            Ok(backend) => Some(backend),
            Err(err) => {
                log::warn!("Microphone unavailable: {err}");
                shared.set_status(MicStatus::Failed(err));
                None
            }
        };
        #[cfg(target_arch = "wasm32")]
        let backend = web::Backend::open(shared.clone());
        Self { shared, _backend: backend }
    }

    pub fn status(&self) -> MicStatus {
        self.shared.with_state(|s| s.status.clone())
    }

    /// Hits since the last call, on the [`MicInput::now_seconds`] timeline.
    pub fn drain_hits(&self) -> Vec<f64> {
        self.shared.with_state(|s| std::mem::take(&mut s.hits))
    }

    /// Peak level (dBFS) since the last call.
    pub fn take_peak_db(&self) -> f32 {
        linear_to_db(self.shared.with_state(|s| std::mem::take(&mut s.peak)))
    }

    pub fn set_threshold_db(&self, db: f32) {
        self.shared.threshold_db.store(db.to_bits(), Ordering::Relaxed);
    }

    /// Current time on the timeline of the hits. On the web this is
    /// `performance.now()` in seconds.
    pub fn now_seconds(&self) -> f64 {
        now_seconds()
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now_seconds() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64()
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn now_seconds() -> f64 {
    web_sys::window().and_then(|w| w.performance()).map(|p| p.now() / 1000.0).unwrap_or(0.0)
}

/// Whether `label` names an iPhone's or iPad's built-in microphone.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn is_builtin_ios_mic(label: &str) -> bool {
    label.contains("iPhone") || label.contains("iPad")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_and_peak_drain() {
        let mic = MicInput {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    hits: Vec::new(),
                    peak: 0.0,
                    status: MicStatus::Starting,
                }),
                threshold_db: AtomicU32::new(DEFAULT_THRESHOLD_DB.to_bits()),
                notifier: None,
            }),
            _backend: None,
        };
        mic.shared.push(&[1.0, 2.0], 0.5);
        mic.shared.push(&[3.0], 0.25);
        assert_eq!(mic.drain_hits(), vec![1.0, 2.0, 3.0]);
        assert!(mic.drain_hits().is_empty());
        assert!((mic.take_peak_db() + 6.02).abs() < 0.01);
        assert_eq!(mic.take_peak_db(), f32::NEG_INFINITY);
        mic.set_threshold_db(-20.0);
        assert_eq!(mic.shared.threshold_db(), -20.0);
    }

    #[test]
    fn recognises_ios_builtin_mic() {
        assert!(is_builtin_ios_mic("iPhone-Mikrofon"));
        assert!(is_builtin_ios_mic("iPad Microphone"));
        assert!(!is_builtin_ios_mic("Dell WL7024 Headset"));
    }
}
