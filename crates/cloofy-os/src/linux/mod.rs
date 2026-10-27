//! Linux speaks D-Bus: MPRIS for the media controls, logind for sleep and wake,
//! power-profiles-daemon and the desktop's settings portal for the settings.

mod bus;
pub mod media;
pub mod power;
pub mod settings;

// Audio servers move streams to a new default output themselves, and there is no secret
// store cloofy uses here yet.
pub use crate::fallback::{audio, run_main, secrets};
