//! What a player reports as it plays, the same for every service. Each service's player turns
//! its own events into these where it hands them on, so the interface and the system's media
//! controls follow any of them alike.

use crate::catalog::Track;

/// A player's report. Positions are milliseconds into the track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerEvent {
    /// A track is loading; the audio of the one before has stopped. Also comes for skips from
    /// elsewhere and for next tracks that weren't loaded ahead.
    Loading {
        uri: String,
        position_ms: u32,
    },
    /// Another track is the current one.
    TrackChanged {
        track: CurrentTrack,
    },
    /// Playing from `position_ms`: a track started, playback resumed, or the position was off.
    Playing {
        position_ms: u32,
    },
    Paused {
        position_ms: u32,
    },
    /// Nothing plays any more.
    Stopped,
    /// The player got to `position_ms` after a seek, or "previous" restarted the track.
    Seeked {
        position_ms: u32,
    },
    /// The position had drifted from what was reported before.
    PositionCorrection {
        position_ms: u32,
    },
    /// The position, reported now and then while playing.
    PositionChanged {
        position_ms: u32,
    },
    ShuffleChanged {
        shuffle: bool,
    },
    /// Repeating the playlist or album, the track, or neither.
    RepeatChanged {
        context: bool,
        track: bool,
    },
    /// From 0 to `u16::MAX`.
    VolumeChanged {
        volume: u16,
    },
    /// What plays next, whenever a playlist or album loads or the queue changes.
    QueueChanged {
        context_uri: String,
        current: Option<String>,
        next: Vec<QueueTrack>,
    },
    /// cloofy's own player became the one that plays for the account, so it takes commands.
    Activated,
    /// cloofy's own player stopped being the one that plays, usually for another device.
    Deactivated,
}

/// A track that plays later, as the player listed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueTrack {
    pub uri: String,
    /// Who put it there: `queue` for songs the user queued, `context` or `autoplay` for the
    /// rest of the playlist or album and what follows it. Entries under other names, such as
    /// markers between pages of a playlist, aren't songs.
    pub provider: String,
}

/// The track a player has, with what the player bar, notifications and the system's media
/// controls show of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentTrack {
    pub uri: String,
    pub name: String,
    pub artists: Vec<String>,
    /// Parallel to `artists`; may be shorter when unknown.
    pub artist_uris: Vec<String>,
    pub album: String,
    pub album_uri: Option<String>,
    pub duration_ms: u32,
    pub cover_url: Option<String>,
    /// The largest available cover, for the Now Playing screen.
    pub cover_url_large: Option<String>,
}

impl CurrentTrack {
    /// The track as a row of a track list.
    pub fn to_track(&self) -> Track {
        Track {
            uri: self.uri.clone(),
            name: self.name.clone(),
            artists: self.artists.clone(),
            artist_uris: self.artist_uris.clone(),
            album: self.album.clone(),
            album_uri: self.album_uri.clone().unwrap_or_default(),
            duration_ms: self.duration_ms,
            image_url: self.cover_url.clone(),
        }
    }
}
