//! What a host app around [`Grooph`](crate::Grooph) can add that eframe does
//! not offer, such as the Android wrapper with its share sheet and runtime
//! permissions.

/// Platform services a host app supplies through
/// [`Grooph::set_host`](crate::Grooph::set_host).
pub trait Host {
    /// Open the system share sheet for `url`. `false` if there is none, so
    /// the link is copied instead.
    fn share(&self, url: &str) -> bool;

    /// Whether the app may record from the microphone.
    fn mic_permitted(&self) -> bool;

    /// Ask the user for the microphone. The answer shows up in
    /// [`Host::mic_permitted`].
    fn request_mic(&self);
}
