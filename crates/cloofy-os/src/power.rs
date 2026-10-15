//! Sleep and wake of the whole system.

use std::sync::Arc;

use crate::os;

/// A change of the system's power state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The system is about to sleep. It waits a moment before it does, so playback can pause.
    Sleep,
    /// The system woke up again.
    Wake,
}

/// Calls `on_event` for the rest of the process, on any thread. Needs [`crate::run_main`].
/// Returns `false` where the system doesn't report sleep and wake.
pub fn watch(on_event: impl Fn(Event) + Send + Sync + 'static) -> bool {
    os::power::watch(Arc::new(on_event))
}
