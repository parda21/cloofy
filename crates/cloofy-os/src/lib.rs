//! What cloofy needs from the operating system, behind one interface for every platform.
//!
//! Each OS implements it in its own module; on systems without an implementation everything
//! here does nothing, so callers never need `cfg` attributes of their own.

pub mod audio;
pub mod media;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as os;

#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(not(target_os = "macos"))]
use fallback as os;

/// Runs `work` while the main thread serves the OS, and returns its result.
///
/// macOS delivers media keys and system notifications through the main thread's run loop, so
/// there `work` moves to a thread of its own. Elsewhere it simply runs here. Call this once,
/// first thing in `main`.
pub fn run_main<R: Send + 'static>(work: impl FnOnce() -> R + Send + 'static) -> R {
    os::run_main(work)
}
