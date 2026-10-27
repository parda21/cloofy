//! The system's media controls: media keys, headphone buttons, and the Now Playing widget.

use std::{sync::Arc, time::Duration};

use crate::os;

/// A track as the system's media controls show it.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Duration,
    /// Encoded cover art (JPEG or PNG), once it has loaded.
    pub artwork: Option<Arc<[u8]>>,
}

/// Everything the media controls show.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    /// Nothing is shown without a track.
    pub track: Option<Track>,
    pub playing: bool,
    /// Where playback was at the moment of the update; the system advances it while playing.
    pub position: Duration,
}

/// A request from the system: a media key, a headphone button, Control Center, Siri, or an MPRIS
/// client such as playerctl.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Play,
    /// Also sent when headphones are taken off.
    Pause,
    Toggle,
    Next,
    Previous,
    Seek(Duration),
}

/// cloofy's entry in the system's media controls. Requests need [`crate::run_main`].
pub struct Controls(os::media::Controls);

impl Controls {
    /// Starts taking requests; `on_command` may get called on any thread. `None` where cloofy
    /// can't use the system's media controls yet.
    pub fn new(on_command: impl Fn(Command) + Send + Sync + 'static) -> Option<Self> {
        os::media::Controls::new(Arc::new(on_command)).map(Self)
    }

    /// Replaces what the controls show. Send it on changes only: the system keeps the position
    /// moving by itself while playing.
    pub fn update(&self, state: State) {
        self.0.update(state);
    }
}
