//! Windows speaks WinRT for the media controls, and the Win32 API for sleep and wake, the
//! settings and the secrets. Some of both want an apartment or a subscription set up first,
//! which `run_main` does.

pub mod media;
pub mod power;
pub mod secrets;
pub mod settings;

// Audio servers move streams to a new default output themselves, and watching them here would
// take WASAPI's device notifications, which cloofy doesn't do yet.
pub use crate::fallback::audio;

use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

pub fn run_main<R>(work: impl FnOnce() -> R) -> R {
    // SAFETY: called before anything of WinRT exists on this thread.
    unsafe {
        // `RPC_E_CHANGED_MODE` says this thread already lives in a single-threaded apartment,
        // which serves the media controls just as well; there is nothing to fix then.
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    work()
}
