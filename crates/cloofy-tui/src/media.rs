//! Keeps the system's media controls (media keys, Control Center) in step with playback.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use cloofy_os::media::{Controls, State, Track};

use crate::playback::{CurrentTrack, NowPlaying};

/// How far the system's own clock may drift from the player before it gets corrected, which
/// is how seeks reach it.
const DRIFT: Duration = Duration::from_millis(1500);
/// How long a cover that failed to load waits before it is tried again.
const ARTWORK_RETRY: Duration = Duration::from_secs(30);

/// Sends the controls an update whenever what they show would change.
pub struct MediaSync {
    send: Box<dyn Fn(State)>,
    shown: Shown,
    /// Cover art for the controls, by URL.
    artwork: Option<(String, Arc<[u8]>)>,
    /// The cover last asked for, so it gets loaded once.
    requested: Option<String>,
    /// A cover that failed to load, and when.
    failed: Option<(String, Instant)>,
}

/// What the controls were told last.
#[derive(Default)]
struct Shown {
    uri: Option<String>,
    playing: bool,
    position: Duration,
    at: Option<Instant>,
    artwork: bool,
}

impl Shown {
    /// Where the system thinks playback is now.
    fn position(&self) -> Duration {
        match self.at {
            Some(at) if self.playing => self.position + at.elapsed(),
            _ => self.position,
        }
    }
}

impl MediaSync {
    pub fn new(controls: Controls) -> Self {
        Self::with_sender(Box::new(move |state| controls.update(state)))
    }

    fn with_sender(send: Box<dyn Fn(State)>) -> Self {
        Self { send, shown: Shown::default(), artwork: None, requested: None, failed: None }
    }

    /// The cover the controls should show but don't have yet. Returned once per cover.
    pub fn missing_artwork(&mut self, track: Option<&CurrentTrack>) -> Option<String> {
        let url = track.and_then(artwork_url)?;
        let failed_lately = self.failed.as_ref().is_some_and(|(f, at)| f == url && at.elapsed() < ARTWORK_RETRY);
        if self.artwork.as_ref().is_some_and(|(have, _)| have == url)
            || self.requested.as_deref() == Some(url)
            || failed_lately
        {
            return None;
        }
        self.requested = Some(url.to_owned());
        Some(url.to_owned())
    }

    pub fn set_artwork(&mut self, url: String, bytes: Arc<[u8]>) {
        if self.requested.as_deref() == Some(url.as_str()) {
            self.artwork = Some((url, bytes));
        }
    }

    /// Lets a cover that failed to load be tried again, after a while.
    pub fn artwork_failed(&mut self, url: String) {
        if self.requested.as_deref() == Some(url.as_str()) {
            self.requested = None;
            self.failed = Some((url, Instant::now()));
        }
    }

    /// Shows cloofy's playback, or nothing while another device plays: that one has controls
    /// of its own, and showing it here would take them over.
    pub fn sync(&mut self, now: &NowPlaying, local: bool) {
        let track = now.track.as_ref().filter(|_| local);
        let artwork = track.and_then(|t| self.artwork_for(t)).cloned();
        let playing = track.is_some() && now.playing;
        let position = Duration::from_millis(now.position_ms().into());
        let shown = &self.shown;
        let unchanged = track.map(|t| t.uri.as_str()) == shown.uri.as_deref()
            && playing == shown.playing
            && artwork.is_some() == shown.artwork
            && shown.position().abs_diff(position) < DRIFT;
        if unchanged {
            return;
        }

        self.shown = Shown {
            uri: track.map(|t| t.uri.clone()),
            playing,
            position,
            at: Some(Instant::now()),
            artwork: artwork.is_some(),
        };
        let track = track.map(|t| Track {
            title: t.name.clone(),
            artist: t.artists.join(", "),
            album: t.album.clone(),
            duration: Duration::from_millis(t.duration_ms.into()),
            artwork,
        });
        (self.send)(State { track, playing, position });
    }

    fn artwork_for(&self, track: &CurrentTrack) -> Option<&Arc<[u8]>> {
        let (url, bytes) = self.artwork.as_ref()?;
        (artwork_url(track) == Some(url.as_str())).then_some(bytes)
    }
}

/// The largest cover: the controls show it bigger than the player bar does.
fn artwork_url(track: &CurrentTrack) -> Option<&str> {
    track.cover_url_large.as_deref().or(track.cover_url.as_deref())
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::*;
    use crate::state::LastTrack;

    fn recorder() -> (MediaSync, Rc<RefCell<Vec<State>>>) {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let log = sent.clone();
        (MediaSync::with_sender(Box::new(move |state| log.borrow_mut().push(state))), sent)
    }

    fn paused_at(position_ms: u32) -> NowPlaying {
        let mut now = NowPlaying::new();
        now.restore(&LastTrack {
            uri: "spotify:track:1".into(),
            name: "One More Time".into(),
            artists: vec!["Daft Punk".into(), "Romanthony".into()],
            artist_uris: Vec::new(),
            album: "Discovery".into(),
            album_uri: None,
            duration_ms: 320_000,
            cover_url: Some("https://i.scdn.co/small".into()),
            cover_url_large: Some("https://i.scdn.co/large".into()),
            context: None,
            position_ms,
        });
        now
    }

    #[test]
    fn shows_a_track_once() {
        let (mut media, sent) = recorder();
        let now = paused_at(10_000);
        media.sync(&now, true);
        media.sync(&now, true);
        let sent = sent.borrow();
        assert_eq!(sent.len(), 1);
        let track = sent[0].track.as_ref().expect("a track is shown");
        assert_eq!(track.artist, "Daft Punk, Romanthony");
        assert_eq!(track.duration, Duration::from_secs(320));
        assert_eq!(sent[0].position, Duration::from_secs(10));
        assert!(!sent[0].playing);
    }

    #[test]
    fn shows_nothing_while_another_device_plays() {
        let (mut media, sent) = recorder();
        let now = paused_at(0);
        media.sync(&now, false);
        assert!(sent.borrow().is_empty(), "nothing was shown, so there is nothing to clear");
        media.sync(&now, true);
        media.sync(&now, false);
        let sent = sent.borrow();
        assert_eq!(sent.len(), 2);
        assert!(sent[1].track.is_none());
    }

    #[test]
    fn follows_play_and_seeks() {
        let (mut media, sent) = recorder();
        let mut now = paused_at(0);
        media.sync(&now, true);
        now.playing = true;
        media.sync(&now, true);
        // A jump a running clock can't explain.
        now = paused_at(60_000);
        now.playing = true;
        media.sync(&now, true);
        let sent = sent.borrow();
        assert_eq!(sent.len(), 3);
        assert!(sent[1].playing);
        assert_eq!(sent[2].position, Duration::from_secs(60));
    }

    #[test]
    fn loads_the_large_cover_once_and_shows_it() {
        let (mut media, sent) = recorder();
        let now = paused_at(0);
        let url = media.missing_artwork(now.track.as_ref());
        assert_eq!(url.as_deref(), Some("https://i.scdn.co/large"));
        assert_eq!(media.missing_artwork(now.track.as_ref()), None);
        media.sync(&now, true);
        media.set_artwork("https://i.scdn.co/large".into(), Arc::from(&b"jpeg"[..]));
        media.sync(&now, true);
        let sent = sent.borrow();
        assert_eq!(sent.len(), 2);
        let artwork = sent[1].track.as_ref().and_then(|t| t.artwork.as_deref());
        assert_eq!(artwork, Some(&b"jpeg"[..]));
    }

    #[test]
    fn tries_a_failed_cover_again_later() {
        let (mut media, _) = recorder();
        let now = paused_at(0);
        let url = media.missing_artwork(now.track.as_ref()).expect("the cover gets loaded");
        media.artwork_failed(url.clone());
        assert_eq!(media.missing_artwork(now.track.as_ref()), None, "not right away");
        media.failed = Some((url.clone(), Instant::now() - ARTWORK_RETRY));
        assert_eq!(media.missing_artwork(now.track.as_ref()), Some(url));
    }

    #[test]
    fn ignores_covers_nobody_asked_for() {
        let (mut media, sent) = recorder();
        let now = paused_at(0);
        media.set_artwork("https://i.scdn.co/other".into(), Arc::from(&b"jpeg"[..]));
        media.sync(&now, true);
        assert!(sent.borrow()[0].track.as_ref().is_some_and(|t| t.artwork.is_none()));
    }
}
