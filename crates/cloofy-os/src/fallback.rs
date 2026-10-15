//! Systems without an integration yet: nothing to set up, nothing shown.

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
