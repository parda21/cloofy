//! Systems without an integration yet: nothing to set up, nothing shown. Linux uses the parts it
//! has nothing better for.

pub fn run_main<R>(work: impl FnOnce() -> R) -> R {
    work()
}

pub mod audio {
    use std::sync::Arc;

    /// Never exists: the output can't be watched here, or follows the default by itself.
    pub enum OutputWatch {}

    impl OutputWatch {
        pub fn new(_on_change: Arc<dyn Fn() + Send + Sync>) -> Option<Self> {
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub mod power {
    use std::sync::Arc;

    use crate::power::Event;

    pub fn watch(_on_event: Arc<dyn Fn(Event) + Send + Sync>) -> bool {
        false
    }
}

#[cfg(not(target_os = "linux"))]
pub mod settings {
    use crate::settings::Settings;

    pub fn current() -> Settings {
        Settings::default()
    }
}

pub mod secrets {
    use std::io;

    pub const AVAILABLE: bool = false;

    fn unsupported() -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, "no secret store on this system")
    }

    pub fn get(_name: &str) -> io::Result<Option<Vec<u8>>> {
        Err(unsupported())
    }

    pub fn set(_name: &str, _secret: &[u8]) -> io::Result<()> {
        Err(unsupported())
    }

    pub fn delete(_name: &str) -> io::Result<()> {
        Err(unsupported())
    }
}

#[cfg(not(target_os = "linux"))]
pub mod media {
    use std::sync::Arc;

    use crate::media::{Command, State};

    /// Never exists: there are no media controls to talk to.
    pub enum Controls {}

    impl Controls {
        pub fn new(_on_command: Arc<dyn Fn(Command) + Send + Sync>) -> Option<Self> {
            None
        }

        pub fn update(&self, _state: State) {
            match *self {}
        }
    }
}
