//! What plays, and in which order: the context (a playlist, album or artist), the songs the
//! user queued, which go first, and the radio that follows once the context ran out, the way
//! YouTube Music goes on after the last song.

use std::{
    collections::{HashSet, VecDeque},
    hash::{BuildHasher, RandomState},
};

use cloofy_core::{catalog::Track, player::QueueTrack};

/// Upcoming songs reported at most; the rest of a long playlist follows as it moves on.
const REPORTED: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    /// From the context, at this index of `context`.
    Context(usize),
    Queue,
    Autoplay,
}

#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub track: Track,
    pub origin: Origin,
}

#[derive(Default)]
pub(crate) struct Queue {
    pub context_uri: String,
    context: Vec<Track>,
    /// Indices of `context` in play order: the identity, or shuffled.
    order: Vec<usize>,
    /// Where in `order` the last song from the context is.
    position: Option<usize>,
    queued: VecDeque<Track>,
    autoplay: VecDeque<Track>,
    current: Option<Entry>,
    history: Vec<Entry>,
    pub shuffle: bool,
    pub repeat_context: bool,
    pub repeat_track: bool,
}

impl Queue {
    /// Starts a context, at `start` if given (a URI in `tracks`), else at its first song.
    /// Songs the user queued stay queued.
    pub fn load(&mut self, context_uri: String, tracks: Vec<Track>, start: Option<&str>, shuffle: bool) -> bool {
        self.context_uri = context_uri;
        self.context = tracks;
        self.autoplay.clear();
        self.history.clear();
        self.current = None;
        self.position = None;
        self.shuffle = shuffle;
        let start = start.and_then(|uri| self.context.iter().position(|t| t.uri == uri));
        self.order = (0..self.context.len()).collect();
        if shuffle {
            shuffle_slice(&mut self.order);
            if let Some(start) = start {
                let at = self.order.iter().position(|&i| i == start).unwrap_or(0);
                self.order.swap(0, at);
            }
        }
        let first = match start {
            Some(start) => self.order.iter().position(|&i| i == start),
            None if self.context.is_empty() => None,
            None => Some(0),
        };
        let Some(first) = first else { return false };
        self.position = Some(first);
        let index = self.order[first];
        self.current = Some(Entry { track: self.context[index].clone(), origin: Origin::Context(index) });
        true
    }

    /// More songs of the context, as later pages of a playlist arrive. Shuffled, they mix in
    /// with the songs that haven't played yet.
    pub fn extend(&mut self, tracks: Vec<Track>) {
        let start = self.context.len();
        self.context.extend(tracks);
        let next = self.position.map_or(0, |p| p + 1);
        for index in start..self.context.len() {
            if self.shuffle && self.order.len() > next {
                let at = next + random_below(self.order.len() - next + 1);
                self.order.insert(at, index);
            } else {
                self.order.push(index);
            }
        }
    }

    pub fn current(&self) -> Option<&Track> {
        self.current.as_ref().map(|e| &e.track)
    }

    /// What follows the current song when it ends.
    pub fn peek(&self) -> Option<&Track> {
        if self.repeat_track {
            return self.current();
        }
        self.peek_next()
    }

    /// What "next" moves to.
    fn peek_next(&self) -> Option<&Track> {
        if let Some(track) = self.queued.front() {
            return Some(track);
        }
        if let Some(index) = self.next_position() {
            return Some(&self.context[self.order[index]]);
        }
        self.autoplay.front()
    }

    fn next_position(&self) -> Option<usize> {
        let next = self.position.map_or(0, |p| p + 1);
        if next < self.order.len() {
            Some(next)
        } else if self.repeat_context && !self.order.is_empty() {
            Some(0)
        } else {
            None
        }
    }

    /// Moves on: because the song ended (`natural`, which repeats a song on repeat), or
    /// because the user skipped. `None` at the end of everything.
    pub fn advance(&mut self, natural: bool) -> Option<&Track> {
        if natural && self.repeat_track && self.current.is_some() {
            return self.current();
        }
        let entry = if let Some(track) = self.queued.pop_front() {
            Entry { track, origin: Origin::Queue }
        } else if let Some(next) = self.next_position() {
            self.position = Some(next);
            let index = self.order[next];
            Entry { track: self.context[index].clone(), origin: Origin::Context(index) }
        } else {
            Entry { track: self.autoplay.pop_front()?, origin: Origin::Autoplay }
        };
        self.history.extend(self.current.replace(entry));
        self.current()
    }

    /// Goes back to the song before; `None` when there is none.
    pub fn back(&mut self) -> Option<&Track> {
        let previous = self.history.pop()?;
        if let Some(current) = self.current.take() {
            match current.origin {
                Origin::Queue => self.queued.push_front(current.track),
                Origin::Autoplay => self.autoplay.push_front(current.track),
                // The context goes on from this song again, also when a queued song came before.
                Origin::Context(index) => {
                    if let Some(at) = self.order.iter().position(|&i| i == index) {
                        self.position = at.checked_sub(1);
                    }
                }
            }
        }
        if let Origin::Context(index) = previous.origin {
            self.position = self.order.iter().position(|&i| i == index);
        }
        self.current = Some(previous);
        self.current()
    }

    /// Jumps to an upcoming song. Queued songs before it are dropped, as in Spotify.
    pub fn skip_to(&mut self, uri: &str) -> Option<&Track> {
        if let Some(at) = self.queued.iter().position(|t| t.uri == uri) {
            self.queued.drain(..at);
            return self.advance(false);
        }
        // Where it comes up next: a playlist can have a song twice. One that played already is
        // found further back.
        let next = self.position.map_or(0, |p| p + 1).min(self.order.len());
        let found = (next..self.order.len()).chain(0..next).find(|&at| self.context[self.order[at]].uri == uri);
        if let Some(at) = found {
            let entry = Entry { track: self.context[self.order[at]].clone(), origin: Origin::Context(self.order[at]) };
            self.position = Some(at);
            self.history.extend(self.current.replace(entry));
            return self.current();
        }
        let at = self.autoplay.iter().position(|t| t.uri == uri)?;
        self.autoplay.drain(..at);
        let track = self.autoplay.pop_front()?;
        self.history.extend(self.current.replace(Entry { track, origin: Origin::Autoplay }));
        self.current()
    }

    pub fn add(&mut self, track: Track) {
        self.queued.push_back(track);
    }

    pub fn clear_queue(&mut self) {
        self.queued.clear();
    }

    /// Shuffles what hasn't played yet, or puts the context back in order, keeping its place.
    pub fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
        let current = self.position.map(|p| self.order[p]);
        self.order = (0..self.context.len()).collect();
        if on {
            shuffle_slice(&mut self.order);
            if let Some(current) = current {
                let at = self.order.iter().position(|&i| i == current).unwrap_or(0);
                self.order.swap(0, at);
            }
            self.position = current.map(|_| 0);
        } else {
            self.position = current;
        }
    }

    /// Whether the radio should be asked for more: nothing is left after the current song.
    pub fn wants_autoplay(&self) -> bool {
        self.queued.is_empty() && self.next_position().is_none() && self.autoplay.len() < 2
    }

    /// The song the radio continues from.
    pub fn seed(&self) -> Option<&Track> {
        self.autoplay.back().or(self.current())
    }

    /// Adds radio songs, leaving out the ones that already played or are listed.
    pub fn add_autoplay(&mut self, tracks: Vec<Track>) {
        let mut known: HashSet<&str> = self.context.iter().map(|t| t.uri.as_str()).collect();
        known.extend(self.history.iter().map(|e| e.track.uri.as_str()));
        known.extend(self.autoplay.iter().map(|t| t.uri.as_str()));
        known.extend(self.current().map(|t| t.uri.as_str()));
        let fresh: Vec<Track> = tracks.into_iter().filter(|t| !known.contains(t.uri.as_str())).collect();
        self.autoplay.extend(fresh);
    }

    /// What plays next, as the player reports it: queued songs, the rest of the context, then
    /// the radio.
    pub fn upcoming(&self) -> Vec<QueueTrack> {
        let entry = |track: &Track, provider: &str| QueueTrack { uri: track.uri.clone(), provider: provider.into() };
        let start = self.position.map_or(0, |p| p + 1).min(self.order.len());
        self.queued
            .iter()
            .map(|t| entry(t, "queue"))
            .chain(self.order[start..].iter().map(|&i| entry(&self.context[i], "context")))
            .chain(self.autoplay.iter().map(|t| entry(t, "autoplay")))
            .take(REPORTED)
            .collect()
    }
}

/// Fisher–Yates with the standard library's random keys, which is plenty for a playlist.
fn shuffle_slice(items: &mut [usize]) {
    for i in (1..items.len()).rev() {
        items.swap(i, random_below(i + 1));
    }
}

fn random_below(n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    (RandomState::new().hash_one(n) % n as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(n: u32) -> Track {
        Track {
            uri: format!("ytmusic:track:{n}"),
            name: format!("Song {n}"),
            artists: Vec::new(),
            artist_uris: Vec::new(),
            album: String::new(),
            album_uri: String::new(),
            duration_ms: 1000,
            image_url: None,
        }
    }

    fn tracks(range: std::ops::Range<u32>) -> Vec<Track> {
        range.map(track).collect()
    }

    fn uri(track: Option<&Track>) -> Option<&str> {
        track.map(|t| t.uri.as_str())
    }

    fn upcoming(queue: &Queue) -> Vec<(String, String)> {
        queue.upcoming().into_iter().map(|t| (t.uri.replace("ytmusic:track:", ""), t.provider)).collect()
    }

    #[test]
    fn plays_a_context_in_order_from_the_chosen_song() {
        let mut queue = Queue::default();
        assert!(queue.load("ytmusic:album:a".into(), tracks(0..4), Some("ytmusic:track:1"), false));
        assert_eq!(uri(queue.current()), Some("ytmusic:track:1"));
        assert_eq!(uri(queue.peek()), Some("ytmusic:track:2"));
        assert_eq!(uri(queue.advance(true)), Some("ytmusic:track:2"));
        assert_eq!(uri(queue.advance(false)), Some("ytmusic:track:3"));
        assert!(queue.advance(true).is_none(), "the end of the album");
        assert!(queue.wants_autoplay());
    }

    #[test]
    fn queued_songs_go_first_and_keep_their_place_on_previous() {
        let mut queue = Queue::default();
        queue.load("c".into(), tracks(0..3), None, false);
        queue.add(track(10));
        assert_eq!(
            upcoming(&queue),
            [("10".into(), "queue".into()), ("1".into(), "context".into()), ("2".into(), "context".into())]
        );
        assert_eq!(uri(queue.advance(true)), Some("ytmusic:track:10"));
        assert_eq!(uri(queue.advance(true)), Some("ytmusic:track:1"), "the context goes on where it was");
        assert_eq!(uri(queue.back()), Some("ytmusic:track:10"));
        assert_eq!(uri(queue.back()), Some("ytmusic:track:0"));
        assert_eq!(uri(queue.advance(false)), Some("ytmusic:track:10"), "going back requeued it");
        assert_eq!(uri(queue.advance(false)), Some("ytmusic:track:1"));
    }

    #[test]
    fn previous_to_a_queued_song_then_next_skips_nothing() {
        let mut queue = Queue::default();
        queue.load("ytmusic:album:a".into(), tracks(0..4), None, false);
        queue.add(track(10));
        assert_eq!(uri(queue.advance(false)), Some("ytmusic:track:10"));
        assert_eq!(uri(queue.advance(false)), Some("ytmusic:track:1"));
        assert_eq!(uri(queue.back()), Some("ytmusic:track:10"));
        assert_eq!(upcoming(&queue)[0], ("1".into(), "context".into()));
        assert_eq!(uri(queue.advance(false)), Some("ytmusic:track:1"), "the song gone back from comes again");
        assert_eq!(uri(queue.advance(false)), Some("ytmusic:track:2"));
    }

    #[test]
    fn repeat_track_repeats_only_when_the_song_ends() {
        let mut queue = Queue::default();
        queue.load("c".into(), tracks(0..3), None, false);
        queue.repeat_track = true;
        assert_eq!(uri(queue.peek()), Some("ytmusic:track:0"));
        assert_eq!(uri(queue.advance(true)), Some("ytmusic:track:0"));
        assert_eq!(uri(queue.advance(false)), Some("ytmusic:track:1"), "skipping moves on");
    }

    #[test]
    fn repeat_context_starts_over() {
        let mut queue = Queue::default();
        queue.load("c".into(), tracks(0..2), Some("ytmusic:track:1"), false);
        assert!(queue.peek().is_none());
        queue.repeat_context = true;
        assert_eq!(uri(queue.peek()), Some("ytmusic:track:0"));
        assert!(!queue.wants_autoplay());
    }

    #[test]
    fn shuffle_keeps_the_current_song_and_plays_every_other_once() {
        let mut queue = Queue::default();
        queue.load("c".into(), tracks(0..20), Some("ytmusic:track:5"), false);
        queue.set_shuffle(true);
        assert_eq!(uri(queue.current()), Some("ytmusic:track:5"));
        let mut played: Vec<String> = std::iter::from_fn(|| queue.advance(false).map(|t| t.uri.clone())).collect();
        played.sort();
        let mut expected: Vec<String> = (0..20).filter(|&n| n != 5).map(|n| format!("ytmusic:track:{n}")).collect();
        expected.sort();
        assert_eq!(played, expected);

        let last: u32 = queue.current().unwrap().uri.trim_start_matches("ytmusic:track:").parse().unwrap();
        queue.set_shuffle(false);
        let after = (last < 19).then(|| format!("ytmusic:track:{}", last + 1));
        assert_eq!(uri(queue.peek()), after.as_deref(), "unshuffled, the album goes on after the song that plays");
    }

    #[test]
    fn play_shuffled_starts_anywhere_but_plays_all() {
        let mut queue = Queue::default();
        queue.load("c".into(), tracks(0..10), None, true);
        let mut played = vec![queue.current().unwrap().uri.clone()];
        played.extend(std::iter::from_fn(|| queue.advance(true).map(|t| t.uri.clone())));
        assert_eq!(played.len(), 10);
        assert_eq!(played.iter().collect::<HashSet<_>>().len(), 10);
    }

    #[test]
    fn later_pages_join_the_context() {
        let mut queue = Queue::default();
        queue.load("c".into(), tracks(0..2), Some("ytmusic:track:1"), false);
        assert!(queue.wants_autoplay());
        queue.extend(tracks(2..4));
        assert!(!queue.wants_autoplay());
        assert_eq!(uri(queue.advance(true)), Some("ytmusic:track:2"));

        let mut shuffled = Queue::default();
        shuffled.load("c".into(), tracks(0..3), None, true);
        shuffled.extend(tracks(3..50));
        let mut played = vec![shuffled.current().unwrap().uri.clone()];
        played.extend(std::iter::from_fn(|| shuffled.advance(true).map(|t| t.uri.clone())));
        assert_eq!(played.iter().collect::<HashSet<_>>().len(), 50);
    }

    #[test]
    fn the_radio_follows_without_repeats() {
        let mut queue = Queue::default();
        queue.load("c".into(), tracks(0..2), Some("ytmusic:track:1"), false);
        queue.add_autoplay(tracks(0..5));
        assert_eq!(
            upcoming(&queue),
            [("2".into(), "autoplay".into()), ("3".into(), "autoplay".into()), ("4".into(), "autoplay".into())]
        );
        assert_eq!(uri(queue.seed()), Some("ytmusic:track:4"));
        assert_eq!(uri(queue.advance(true)), Some("ytmusic:track:2"));
    }

    #[test]
    fn skips_to_an_upcoming_song() {
        let mut queue = Queue::default();
        queue.load("c".into(), tracks(0..5), None, false);
        queue.add(track(10));
        queue.add(track(11));
        assert_eq!(uri(queue.skip_to("ytmusic:track:11")), Some("ytmusic:track:11"));
        assert_eq!(upcoming(&queue)[0], ("1".into(), "context".into()), "the queued song before it is gone");
        assert_eq!(uri(queue.skip_to("ytmusic:track:3")), Some("ytmusic:track:3"));
        assert_eq!(uri(queue.peek()), Some("ytmusic:track:4"));
        assert!(queue.skip_to("ytmusic:track:99").is_none());
    }

    #[test]
    fn skips_to_where_a_song_comes_up_next() {
        let mut queue = Queue::default();
        let mut playlist = tracks(0..4);
        playlist.push(track(1));
        queue.load("c".into(), playlist, Some("ytmusic:track:3"), false);
        assert_eq!(uri(queue.skip_to("ytmusic:track:1")), Some("ytmusic:track:1"));
        assert!(upcoming(&queue).is_empty(), "the playlist's last song played, not its second");
        assert_eq!(uri(queue.skip_to("ytmusic:track:2")), Some("ytmusic:track:2"), "one that played is found too");
        assert_eq!(uri(queue.peek()), Some("ytmusic:track:3"));
    }

    #[test]
    fn clearing_the_queue_keeps_the_context() {
        let mut queue = Queue::default();
        queue.load("c".into(), tracks(0..2), None, false);
        queue.add(track(10));
        queue.clear_queue();
        assert_eq!(uri(queue.peek()), Some("ytmusic:track:1"));
    }
}
