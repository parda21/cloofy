//! The system's audio output.

use std::sync::Arc;

use crate::os;

/// Calls back whenever the system's default output changes: headphones plugged in, AirPods
/// connected or gone, another output picked in Control Center. Stops when dropped.
pub struct OutputWatch {
    _watch: os::audio::OutputWatch,
}

impl OutputWatch {
    /// `on_change` may get called on any thread. `None` where the system can't tell, or where
    /// its audio follows the default output by itself.
    pub fn new(on_change: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        os::audio::OutputWatch::new(Arc::new(on_change)).map(|watch| Self { _watch: watch })
    }
}
