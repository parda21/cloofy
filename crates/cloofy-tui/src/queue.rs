//! What plays next, the way Spotify's apps show it: the songs the user queued, then the rest of
//! the playlist or album. Spotify's player state holds both in one list of next tracks and tells
//! them apart by who put each one there.

use librespot::{
    playback::player::QueueTrack,
    protocol::player::{PlayerState, ProvidedTrack},
};

/// Where an upcoming track comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// Queued by the user: "Next in queue".
    Queue,
    /// The rest of the playlist or album, or what autoplay adds after it: "Next from".
    Context,
    /// From the Web API's list, which doesn't say.
    Unknown,
}

/// A track that plays later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upcoming {
    pub uri: String,
    /// Spotify's ID for the entry, since a track can be listed twice; empty where unknown.
    pub uid: String,
    pub provider: Provider,
}

impl Upcoming {
    /// Spotify's name for the provider, as commands about the entry name it.
    pub fn provider_name(&self) -> &'static str {
        match self.provider {
            Provider::Queue => "queue",
            Provider::Context | Provider::Unknown => "context",
        }
    }
}

/// What plays next on the device that plays, as last reported.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Queue {
    /// The track that played when the list was reported.
    pub current: Option<String>,
    pub next: Vec<Upcoming>,
    /// The playlist, album or artist the tracks come from.
    pub context: String,
    /// The context's name, where Spotify sends one along.
    pub context_name: Option<String>,
    /// The lists as Spotify's player state had them, which changing the queue sends back.
    /// Only there when the queue came from Spotify's player state.
    pub listed: Option<Box<Listed>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub next: Vec<ProvidedTrack>,
    pub previous: Vec<ProvidedTrack>,
    pub revision: String,
}

impl Queue {
    /// From the player state Spotify pushes about the device that plays.
    pub fn from_player_state(state: &PlayerState) -> Self {
        Self {
            current: Some(state.track.uri.clone()).filter(|uri| !uri.is_empty()),
            next: state.next_tracks.iter().filter_map(|t| upcoming(&t.uri, &t.uid, &t.provider)).collect(),
            context: state.context_uri.clone(),
            context_name: state.context_metadata.get("context_description").filter(|name| !name.is_empty()).cloned(),
            listed: Some(Box::new(Listed {
                next: state.next_tracks.clone(),
                previous: state.prev_tracks.clone(),
                revision: state.queue_revision.clone(),
            })),
        }
    }

    /// From cloofy's own player, which reports it whenever a context loads or the queue
    /// changes, without entry IDs.
    pub fn from_player(context: String, current: Option<QueueTrack>, next: &[QueueTrack]) -> Self {
        Self {
            current: current.map(|t| t.uri).filter(|uri| !uri.is_empty()),
            next: next.iter().filter_map(|t| upcoming(&t.uri, "", &t.provider)).collect(),
            context,
            ..Self::default()
        }
    }

    /// From the Web API's list of what plays after `playing`, which doesn't tell queued songs
    /// from the rest. Queued songs always come first, so those this queue knew of stay queued
    /// and the rest is the context's; if it knew nothing about `playing`, the list stays unsorted.
    pub fn with_listing(&self, playing: &str, context: String, uris: Vec<String>) -> Self {
        let known = self.knows(playing);
        let queued = if known { self.after(Some(playing)) } else { &self.next[..] };
        let queued: Vec<&str> =
            queued.iter().filter(|t| t.provider == Provider::Queue).map(|t| t.uri.as_str()).collect();
        let lead = uris.iter().zip(&queued).take_while(|(uri, queued)| uri == *queued).count();
        let rest = if known || lead > 0 { Provider::Context } else { Provider::Unknown };
        let next = uris
            .into_iter()
            .enumerate()
            .map(|(index, uri)| Upcoming {
                uri,
                uid: String::new(),
                provider: if index < lead { Provider::Queue } else { rest },
            })
            .collect();
        let context_name = self.context_name.clone().filter(|_| self.context == context);
        Self { current: Some(playing.to_owned()), next, context, context_name, listed: None }
    }

    /// Whether the list says what comes after `playing`.
    pub fn knows(&self, playing: &str) -> bool {
        self.current.as_deref() == Some(playing) || self.next.iter().any(|t| t.uri == playing)
    }

    /// What plays after `playing`. Tracks move on before Spotify reports the list again, so it
    /// starts after `playing` where that is in it; a track it doesn't know leaves nothing to say.
    pub fn after(&self, playing: Option<&str>) -> &[Upcoming] {
        let Some(playing) = playing.filter(|uri| self.current.as_deref() != Some(uri)) else { return &self.next };
        match self.next.iter().position(|t| t.uri == playing) {
            Some(index) => &self.next[index + 1..],
            None => &[],
        }
    }

    /// The lists to send back to Spotify without `entry`, if it came from them.
    pub fn without(&self, entry: &Upcoming) -> Option<Listed> {
        let listed = self.listed.as_deref()?;
        let index = listed.next.iter().position(|t| t.uid == entry.uid && t.uri == entry.uri)?;
        let mut next = listed.next.clone();
        next.remove(index);
        Some(Listed { next, ..listed.clone() })
    }
}

/// An entry worth showing: a track or episode, not one of the markers Spotify keeps between
/// pages of a context, nor a track it can't play.
fn upcoming(uri: &str, uid: &str, provider: &str) -> Option<Upcoming> {
    let provider = match provider {
        "queue" => Provider::Queue,
        "context" | "autoplay" | "" => Provider::Context,
        _ => return None,
    };
    let playable = uri.starts_with("spotify:track:") || uri.starts_with("spotify:episode:");
    playable.then(|| Upcoming { uri: uri.to_owned(), uid: uid.to_owned(), provider })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provided(uri: &str, uid: &str, provider: &str) -> ProvidedTrack {
        ProvidedTrack { uri: uri.into(), uid: uid.into(), provider: provider.into(), ..ProvidedTrack::default() }
    }

    fn uris(entries: &[Upcoming]) -> Vec<&str> {
        entries.iter().map(|t| t.uri.as_str()).collect()
    }

    fn state() -> PlayerState {
        let mut state = PlayerState {
            context_uri: "spotify:playlist:late".into(),
            queue_revision: "7".into(),
            next_tracks: vec![
                provided("spotify:track:a", "q0", "queue"),
                provided("spotify:track:b", "q1", "queue"),
                provided("spotify:delimiter", "delimiter0", "context"),
                provided("spotify:track:c", "c3", "context"),
                provided("spotify:track:d", "c4", "autoplay"),
                provided("spotify:track:e", "c5", "unavailable"),
            ],
            ..PlayerState::default()
        };
        state.track.mut_or_insert_default().uri = "spotify:track:x".into();
        state.context_metadata.insert("context_description".into(), "Late Night Coding".into());
        state
    }

    #[test]
    fn the_player_state_splits_queued_songs_from_the_context() {
        let queue = Queue::from_player_state(&state());
        assert_eq!(queue.current.as_deref(), Some("spotify:track:x"));
        assert_eq!(uris(&queue.next), ["spotify:track:a", "spotify:track:b", "spotify:track:c", "spotify:track:d"]);
        let providers: Vec<_> = queue.next.iter().map(|t| t.provider).collect();
        assert_eq!(providers, [Provider::Queue, Provider::Queue, Provider::Context, Provider::Context]);
        assert_eq!(queue.next[2].uid, "c3");
        assert_eq!(queue.context_name.as_deref(), Some("Late Night Coding"));
    }

    #[test]
    fn cloofys_own_player_reports_without_ids() {
        let track = |uri: &str, provider: &str| QueueTrack { uri: uri.into(), provider: provider.into() };
        let next = [
            track("spotify:track:a", "queue"),
            track("spotify:delimiter", "context"),
            track("spotify:track:c", "context"),
        ];
        let queue = Queue::from_player("spotify:album:1".into(), Some(track("spotify:track:x", "context")), &next);
        assert_eq!(queue.current.as_deref(), Some("spotify:track:x"));
        assert_eq!(uris(&queue.next), ["spotify:track:a", "spotify:track:c"]);
        assert_eq!(queue.next[0].provider, Provider::Queue);
        assert!(queue.next[0].uid.is_empty());
    }

    #[test]
    fn the_list_moves_on_with_the_playing_track() {
        let queue = Queue::from_player_state(&state());
        assert_eq!(queue.after(Some("spotify:track:x")).len(), 4);
        assert_eq!(
            uris(queue.after(Some("spotify:track:a"))),
            ["spotify:track:b", "spotify:track:c", "spotify:track:d"]
        );
        assert_eq!(uris(queue.after(Some("spotify:track:c"))), ["spotify:track:d"]);
        // A track from elsewhere: the list says nothing about what follows it.
        assert!(queue.after(Some("spotify:track:z")).is_empty());
        assert!(!queue.knows("spotify:track:z"));
    }

    #[test]
    fn a_listing_keeps_known_queued_songs_queued() {
        // After shuffling, the queued songs lead as before and the rest comes in a new order.
        let queue = Queue::from_player_state(&state());
        let listing = ["spotify:track:a", "spotify:track:b", "spotify:track:f", "spotify:track:c"];
        let listed =
            queue.with_listing("spotify:track:x", "spotify:playlist:late".into(), listing.map(String::from).to_vec());
        let providers: Vec<_> = listed.next.iter().map(|t| t.provider).collect();
        assert_eq!(providers, [Provider::Queue, Provider::Queue, Provider::Context, Provider::Context]);
        assert_eq!(listed.context_name.as_deref(), Some("Late Night Coding"));
        // Nothing known: nothing to sort by.
        let fresh = Queue::default().with_listing("spotify:track:x", String::new(), vec!["spotify:track:a".into()]);
        assert_eq!(fresh.next[0].provider, Provider::Unknown);
    }

    #[test]
    fn removing_a_queued_song_sends_back_the_rest() {
        let queue = Queue::from_player_state(&state());
        let listed = queue.without(&queue.next[1]).unwrap();
        let uids: Vec<_> = listed.next.iter().map(|t| t.uid.as_str()).collect();
        assert_eq!(uids, ["q0", "delimiter0", "c3", "c4", "c5"]);
        assert_eq!(listed.revision, "7");
        // Entries the player reported itself aren't Spotify's to send back.
        assert!(Queue::from_player(String::new(), None, &[]).without(&queue.next[0]).is_none());
    }
}
