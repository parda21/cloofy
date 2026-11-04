//! What plays next, the way Spotify's apps show it: the songs the user queued, then the rest of
//! the playlist or album. Players report both in one list of next tracks and tell them apart by
//! who put each one there.

use cloofy_core::player::QueueTrack;

use crate::backend::{self, Listed};

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
    /// The lists as the service had them, which changing another device's queue sends back.
    /// Only there when the queue came from the service rather than from cloofy's player.
    pub listed: Option<Box<Listed>>,
}

impl Queue {
    /// From cloofy's own player, which reports it whenever a context loads or the queue
    /// changes, without entry IDs.
    pub fn from_player(context: String, current: Option<String>, next: &[QueueTrack]) -> Self {
        Self {
            current: current.filter(|uri| !uri.is_empty()),
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

    /// The lists to send back to the service without `entry`, if it came from them.
    pub fn without(&self, entry: &Upcoming) -> Option<Listed> {
        self.listed.as_deref()?.without(entry)
    }
}

/// An entry worth showing: a track or episode, not one of the markers Spotify keeps between
/// pages of a context, nor a track it can't play.
pub(crate) fn upcoming(uri: &str, uid: &str, provider: &str) -> Option<Upcoming> {
    let provider = match provider {
        "queue" => Provider::Queue,
        "context" | "autoplay" | "" => Provider::Context,
        _ => return None,
    };
    backend::is_playable(uri).then(|| Upcoming { uri: uri.to_owned(), uid: uid.to_owned(), provider })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uris(entries: &[Upcoming]) -> Vec<&str> {
        entries.iter().map(|t| t.uri.as_str()).collect()
    }

    #[test]
    fn cloofys_own_player_reports_without_ids() {
        let track = |uri: &str, provider: &str| QueueTrack { uri: uri.into(), provider: provider.into() };
        let next = [
            track("spotify:track:a", "queue"),
            track("spotify:delimiter", "context"),
            track("spotify:track:c", "context"),
        ];
        let queue = Queue::from_player("spotify:album:1".into(), Some("spotify:track:x".into()), &next);
        assert_eq!(queue.current.as_deref(), Some("spotify:track:x"));
        assert_eq!(uris(&queue.next), ["spotify:track:a", "spotify:track:c"]);
        assert_eq!(queue.next[0].provider, Provider::Queue);
        assert!(queue.next[0].uid.is_empty());
    }
}
