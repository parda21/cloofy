//! What plays next on a Spotify Connect device, from the player state Spotify pushes about it.
//! The state holds the songs the user queued and the rest of the context in one list of next
//! tracks, and tells them apart by who put each one there.

use librespot::protocol::player::{PlayerState, ProvidedTrack};

use crate::{
    backend,
    queue::{Queue, Upcoming, upcoming},
};

/// The lists as Spotify's player state had them, which changing the queue sends back.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub next: Vec<ProvidedTrack>,
    pub previous: Vec<ProvidedTrack>,
    pub revision: String,
}

impl Listed {
    /// The lists to send back to Spotify without `entry`, if it came from them.
    pub fn without(&self, entry: &Upcoming) -> Option<Listed> {
        let index = self.next.iter().position(|t| t.uid == entry.uid && t.uri == entry.uri)?;
        let mut next = self.next.clone();
        next.remove(index);
        Some(Listed { next, ..self.clone() })
    }
}

/// From the player state Spotify pushes about the device that plays.
pub fn from_player_state(state: &PlayerState) -> Queue {
    Queue {
        current: Some(state.track.uri.clone()).filter(|uri| !uri.is_empty()),
        next: state.next_tracks.iter().filter_map(|t| upcoming(&t.uri, &t.uid, &t.provider)).collect(),
        context: state.context_uri.clone(),
        context_name: state.context_metadata.get("context_description").filter(|name| !name.is_empty()).cloned(),
        listed: Some(Box::new(backend::Listed::Spotify(Listed {
            next: state.next_tracks.clone(),
            previous: state.prev_tracks.clone(),
            revision: state.queue_revision.clone(),
        }))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::Provider;

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
        let queue = from_player_state(&state());
        assert_eq!(queue.current.as_deref(), Some("spotify:track:x"));
        assert_eq!(uris(&queue.next), ["spotify:track:a", "spotify:track:b", "spotify:track:c", "spotify:track:d"]);
        let providers: Vec<_> = queue.next.iter().map(|t| t.provider).collect();
        assert_eq!(providers, [Provider::Queue, Provider::Queue, Provider::Context, Provider::Context]);
        assert_eq!(queue.next[2].uid, "c3");
        assert_eq!(queue.context_name.as_deref(), Some("Late Night Coding"));
    }

    #[test]
    fn the_list_moves_on_with_the_playing_track() {
        let queue = from_player_state(&state());
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
        let queue = from_player_state(&state());
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
        let queue = from_player_state(&state());
        let Some(backend::Listed::Spotify(listed)) = queue.without(&queue.next[1]) else { panic!("not listed") };
        let uids: Vec<_> = listed.next.iter().map(|t| t.uid.as_str()).collect();
        assert_eq!(uids, ["q0", "delimiter0", "c3", "c4", "c5"]);
        assert_eq!(listed.revision, "7");
        // Entries the player reported itself aren't Spotify's to send back.
        assert!(Queue::from_player(String::new(), None, &[]).without(&queue.next[0]).is_none());
    }
}
