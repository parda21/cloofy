//! System settings that cloofy follows.

use crate::os;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    /// The user asked for less motion on screen.
    pub reduce_motion: bool,
    /// The system is saving energy, so cloofy should too.
    pub low_power: bool,
}

/// Reads the current settings. Cheap enough to check every few seconds; all off where the
/// system has no such settings cloofy can read.
pub fn current() -> Settings {
    os::settings::current()
}
