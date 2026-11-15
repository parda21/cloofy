//! Windows speaks WinRT for the media controls and the notifications, and the Win32 API for
//! sleep and wake, the settings and the secrets. Some of both want an apartment or a subscription
//! set up first, which `run_main` does.

use std::path::PathBuf;

use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

pub mod media;
pub mod notifications;
pub mod power;
pub mod secrets;
pub mod settings;

// Audio servers move streams to a new default output themselves, and watching them here would
// take WASAPI's device notifications, which cloofy doesn't do yet.
pub use crate::fallback::audio;

/// The AppUserModelID of cloofy's Start Menu shortcut, and of its toast notifier. The two must
/// agree: with a different one, Windows shows a toast once and then drops the rest.
pub const AUMID: &str = "dev.cloofy.cloofy";

/// The Start Menu shortcut, whose AppUserModelID is what Windows shows cloofy's toasts under.
/// Without it, `ToastNotificationManager` accepts the notifier and shows nothing.
pub fn shortcut_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("Microsoft\\Windows\\Start Menu\\Programs\\cloofy.lnk"))
}

pub fn run_main<R>(work: impl FnOnce() -> R) -> R {
    // SAFETY: called before anything of WinRT exists on this thread.
    unsafe {
        // `RPC_E_CHANGED_MODE` says this thread already lives in a single-threaded apartment,
        // which serves the media controls just as well; there is nothing to fix then.
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    work()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn the_shortcut_lives_in_the_start_menu() {
        let path = shortcut_path().expect("APPDATA is set on Windows");
        let appdata = std::env::var_os("APPDATA").expect("APPDATA is set on Windows");
        assert!(path.starts_with(appdata));
        assert!(path.ends_with(Path::new("Microsoft/Windows/Start Menu/Programs/cloofy.lnk")), "{}", path.display());
    }
}
