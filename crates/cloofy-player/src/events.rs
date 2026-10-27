//! librespot's player events as cloofy's own, which the interface follows whatever the service.

use cloofy_core::player::{CurrentTrack, PlayerEvent, QueueTrack};
use librespot::{
    metadata::audio::{AudioItem, UniqueFields},
    playback::player::PlayerEvent as LibrespotEvent,
};

/// The event as cloofy's own; `None` for those only librespot and Spirc have a use for.
pub fn neutral(event: LibrespotEvent) -> Option<PlayerEvent> {
    Some(match event {
        LibrespotEvent::Loading { track_id, position_ms, .. } => {
            PlayerEvent::Loading { uri: track_id.to_uri(), position_ms }
        }
        LibrespotEvent::TrackChanged { audio_item } => PlayerEvent::TrackChanged { track: current_track(&audio_item) },
        LibrespotEvent::Playing { position_ms, .. } => PlayerEvent::Playing { position_ms },
        LibrespotEvent::Paused { position_ms, .. } => PlayerEvent::Paused { position_ms },
        LibrespotEvent::Stopped { .. } => PlayerEvent::Stopped,
        LibrespotEvent::Seeked { position_ms, .. } => PlayerEvent::Seeked { position_ms },
        LibrespotEvent::PositionCorrection { position_ms, .. } => PlayerEvent::PositionCorrection { position_ms },
        LibrespotEvent::PositionChanged { position_ms, .. } => PlayerEvent::PositionChanged { position_ms },
        LibrespotEvent::ShuffleChanged { shuffle } => PlayerEvent::ShuffleChanged { shuffle },
        LibrespotEvent::RepeatChanged { context, track } => PlayerEvent::RepeatChanged { context, track },
        LibrespotEvent::VolumeChanged { volume } => PlayerEvent::VolumeChanged { volume },
        LibrespotEvent::SetQueue { context_uri, current_track, next_tracks, .. } => PlayerEvent::QueueChanged {
            context_uri,
            current: current_track.map(|track| track.uri),
            next: next_tracks.into_iter().map(|t| QueueTrack { uri: t.uri, provider: t.provider }).collect(),
        },
        // Spirc made cloofy's device the active one, or another device took over.
        LibrespotEvent::SessionConnected { .. } => PlayerEvent::Activated,
        LibrespotEvent::SessionDisconnected { .. } => PlayerEvent::Deactivated,
        _ => return None,
    })
}

/// What the player bar shows of a track librespot loaded.
pub fn current_track(item: &AudioItem) -> CurrentTrack {
    let (artists, artist_uris, album) = match &item.unique_fields {
        UniqueFields::Track { artists, album, .. } => (
            artists.iter().map(|a| a.name.clone()).collect(),
            artists.iter().map(|a| a.id.to_uri()).collect(),
            album.clone(),
        ),
        UniqueFields::Episode { show_name, .. } => (vec![show_name.clone()], Vec::new(), String::new()),
        UniqueFields::Local { artists, album, .. } => {
            (artists.clone().into_iter().collect(), Vec::new(), album.clone().unwrap_or_default())
        }
    };
    // The smallest cover that is still sharp at a few cells, else the largest one.
    let cover_url = item
        .covers
        .iter()
        .filter(|c| c.width >= 200)
        .min_by_key(|c| c.width)
        .or_else(|| item.covers.iter().max_by_key(|c| c.width))
        .map(|c| c.url.clone());
    let cover_url_large = item.covers.iter().max_by_key(|c| c.width).map(|c| c.url.clone());
    CurrentTrack {
        uri: item.uri.clone(),
        name: item.name.clone(),
        artists,
        artist_uris,
        album,
        // Not in the player's metadata; filled in from the track list when it is open.
        album_uri: None,
        duration_ms: item.duration_ms,
        cover_url,
        cover_url_large,
    }
}

#[cfg(test)]
mod tests {
    use librespot::{core::SpotifyUri, playback::player::QueueTrack as Queued};

    use super::*;

    const URI: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";

    #[test]
    fn keeps_what_cloofy_follows_and_drops_the_rest() {
        let track_id = SpotifyUri::from_uri(URI).unwrap();
        let loading = LibrespotEvent::Loading { play_request_id: 1, track_id: track_id.clone(), position_ms: 500 };
        assert_eq!(neutral(loading), Some(PlayerEvent::Loading { uri: URI.into(), position_ms: 500 }));
        let connected = LibrespotEvent::SessionConnected { connection_id: "c".into(), user_name: "sam".into() };
        assert_eq!(neutral(connected), Some(PlayerEvent::Activated));
        assert_eq!(neutral(LibrespotEvent::Preloading { track_id }), None);
    }

    #[test]
    fn the_queue_keeps_who_put_each_track_there() {
        let queued = |uri: &str, provider: &str| Queued { uri: uri.into(), provider: provider.into() };
        let event = LibrespotEvent::SetQueue {
            context_uri: "spotify:album:1".into(),
            current_track: Some(queued(URI, "context")),
            next_tracks: vec![queued("spotify:track:a", "queue")],
            prev_tracks: vec![queued("spotify:track:z", "context")],
        };
        assert_eq!(
            neutral(event),
            Some(PlayerEvent::QueueChanged {
                context_uri: "spotify:album:1".into(),
                current: Some(URI.into()),
                next: vec![QueueTrack { uri: "spotify:track:a".into(), provider: "queue".into() }],
            })
        );
    }
}
