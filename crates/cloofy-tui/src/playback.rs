use std::time::{Duration, Instant};

use anyhow::Result;
use cloofy_player::Engine;
use cloofy_spotify::connect::Playback;
use cloofy_spotify::{paths::Paths, session};
use librespot::{
    connect::{LoadContextOptions, LoadRequest, LoadRequestOptions, Options, PlayingTrack, Spirc},
    core::session::Session,
    metadata::audio::{AudioItem, UniqueFields},
    playback::player::PlayerEvent,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppEvent;

const DEVICE_NAME: &str = "cloofy";
/// How long a change made in cloofy wins over reports the player sent before it got there.
const ECHO_WINDOW: Duration = Duration::from_millis(1500);
/// A reported position this close to a seek target confirms the seek.
const SEEK_TOLERANCE_MS: u32 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    Context,
    Track,
}

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

pub struct Cover {
    pub url: String,
    pub picture: crate::loader::Picture,
}

/// What the local player is doing, as reported by librespot's player events.
pub struct NowPlaying {
    pub track: Option<CurrentTrack>,
    pub playing: bool,
    position_ms: u32,
    position_at: Instant,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub volume: u16,
    pub cover: Option<Cover>,
    /// The other Spotify Connect device that is playing, if not cloofy itself.
    pub device: Option<String>,
    /// A seek sent to the local player that it hasn't confirmed yet: target and when.
    seeking: Option<(u32, Instant)>,
}

impl NowPlaying {
    pub fn new() -> Self {
        Self {
            track: None,
            playing: false,
            position_ms: 0,
            position_at: Instant::now(),
            shuffle: false,
            repeat: Repeat::Off,
            volume: u16::MAX / 2,
            cover: None,
            device: None,
            seeking: None,
        }
    }

    pub fn position_ms(&self) -> u32 {
        let elapsed = if self.playing { self.position_at.elapsed().as_millis() as u32 } else { 0 };
        let duration = self.track.as_ref().map_or(u32::MAX, |t| t.duration_ms);
        (self.position_ms + elapsed).min(duration)
    }

    pub(crate) fn set_position(&mut self, position_ms: u32) {
        self.position_ms = position_ms;
        self.position_at = Instant::now();
    }

    /// Starts or stops the clock where it stands, so pausing doesn't jump.
    pub fn set_playing(&mut self, playing: bool) {
        self.set_position(self.position_ms());
        self.playing = playing;
    }

    /// Moves to `position_ms` at once for a seek sent to the local player. The player confirms
    /// only after decoding there, which can wait for a download.
    pub fn seek(&mut self, position_ms: u32) {
        self.set_position(position_ms);
        self.seeking = Some((position_ms, Instant::now()));
    }

    /// Takes a position the local player reported, unless it predates the last seek: the
    /// player reports positions it had before the seek, which would make the bar jump back.
    fn report_position(&mut self, position_ms: u32) {
        if let Some((target, at)) = self.seeking {
            if position_ms.abs_diff(target) > SEEK_TOLERANCE_MS && at.elapsed() < ECHO_WINDOW {
                return;
            }
            self.seeking = None;
        }
        self.set_position(position_ms);
    }

    /// Shows the track from the last session, paused where it was left.
    pub fn restore(&mut self, last: &crate::state::LastTrack) {
        self.track = Some(CurrentTrack {
            uri: last.uri.clone(),
            name: last.name.clone(),
            artists: last.artists.clone(),
            artist_uris: last.artist_uris.clone(),
            album: last.album.clone(),
            album_uri: last.album_uri.clone(),
            duration_ms: last.duration_ms,
            cover_url: last.cover_url.clone(),
            cover_url_large: last.cover_url_large.clone(),
        });
        self.playing = false;
        self.set_position(last.position_ms);
    }

    /// Mirrors playback on another device. Returns a cover URL when the track changed.
    pub fn apply_remote(&mut self, playback: &Playback) -> Option<String> {
        self.device = Some(playback.device.name.clone());
        self.playing = playback.playing;
        self.set_position(playback.progress_ms);
        self.shuffle = playback.shuffle;
        self.repeat = match playback.repeat.as_str() {
            "track" => Repeat::Track,
            "context" => Repeat::Context,
            _ => Repeat::Off,
        };
        if let Some(percent) = playback.device.volume_percent {
            self.volume = (percent.min(100) * u16::MAX as u32 / 100) as u16;
        }
        let Some(remote) = &playback.track else {
            self.track = None;
            return None;
        };
        if self.track.as_ref().is_some_and(|t| t.uri == remote.uri) {
            return None;
        }
        self.track = Some(CurrentTrack {
            uri: remote.uri.clone(),
            name: remote.name.clone(),
            artists: remote.artists.clone(),
            artist_uris: remote.artist_uris.clone(),
            album: remote.album.clone(),
            album_uri: remote.album_uri.clone(),
            duration_ms: remote.duration_ms,
            cover_url: remote.cover_url.clone(),
            cover_url_large: remote.cover_url_large.clone(),
        });
        let changed = self.cover.as_ref().map(|c| &c.url) != remote.cover_url.as_ref();
        changed.then(|| remote.cover_url.clone()).flatten()
    }

    /// Applies a player event. Returns a cover URL when the new track needs its art loaded.
    pub fn apply(&mut self, event: PlayerEvent) -> Option<String> {
        match event {
            PlayerEvent::TrackChanged { audio_item } => {
                let track = current_track(&audio_item);
                let cover_url = track.cover_url.clone();
                self.track = Some(track);
                self.seeking = None;
                self.set_position(0);
                if self.cover.as_ref().map(|c| &c.url) != cover_url.as_ref() {
                    return cover_url;
                }
            }
            PlayerEvent::Playing { position_ms, .. } => {
                // Freeze first, so a report from before a seek leaves the shown position put.
                self.set_playing(false);
                self.report_position(position_ms);
                self.playing = true;
            }
            PlayerEvent::Paused { position_ms, .. } => {
                self.set_playing(false);
                self.report_position(position_ms);
            }
            PlayerEvent::Stopped { .. } => self.playing = false,
            PlayerEvent::PositionCorrection { position_ms, .. }
            | PlayerEvent::PositionChanged { position_ms, .. }
            | PlayerEvent::Seeked { position_ms, .. } => self.report_position(position_ms),
            PlayerEvent::ShuffleChanged { shuffle } => self.shuffle = shuffle,
            PlayerEvent::RepeatChanged { context, track } => {
                self.repeat = match (context, track) {
                    (_, true) => Repeat::Track,
                    (true, false) => Repeat::Context,
                    (false, false) => Repeat::Off,
                };
            }
            PlayerEvent::VolumeChanged { volume } => self.volume = volume,
            _ => {}
        }
        None
    }
}

fn current_track(item: &AudioItem) -> CurrentTrack {
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

/// cloofy's own Spotify Connect device. Commands only take effect while it is the active device.
pub struct Controller {
    spirc: Spirc,
    pub session: Session,
    engine: Engine,
}

impl Controller {
    pub fn play(&self, context_uri: String, track_uri: Option<String>) -> Result<()> {
        let options = LoadRequestOptions {
            start_playing: true,
            playing_track: track_uri.map(PlayingTrack::Uri),
            ..LoadRequestOptions::default()
        };
        self.spirc.activate()?;
        self.spirc.load(LoadRequest::from_context_uri(context_uri, options))?;
        Ok(())
    }

    /// Continues a track from the last session at `position_ms`, in its context if known.
    pub fn resume(&self, context: Option<String>, track: String, position_ms: u32) -> Result<()> {
        let options = LoadRequestOptions { start_playing: true, seek_to: position_ms, ..LoadRequestOptions::default() };
        let request = match context {
            Some(context) => LoadRequest::from_context_uri(
                context,
                LoadRequestOptions { playing_track: Some(PlayingTrack::Uri(track)), ..options },
            ),
            None => LoadRequest::from_tracks(vec![track], options),
        };
        self.spirc.activate()?;
        self.spirc.load(request)?;
        Ok(())
    }

    pub fn play_shuffled(&self, context_uri: String) -> Result<()> {
        let options = LoadRequestOptions {
            start_playing: true,
            context_options: Some(LoadContextOptions::Options(Options { shuffle: true, ..Options::default() })),
            ..LoadRequestOptions::default()
        };
        self.spirc.activate()?;
        self.spirc.load(LoadRequest::from_context_uri(context_uri, options))?;
        Ok(())
    }

    /// Plays or pauses; with nothing loaded, takes over playback from the active device.
    pub fn toggle(&self, has_track: bool) -> Result<()> {
        if has_track {
            self.spirc.play_pause()?;
        } else {
            self.spirc.transfer(None)?;
        }
        Ok(())
    }

    pub fn next(&self) -> Result<()> {
        Ok(self.spirc.next()?)
    }

    pub fn previous(&self) -> Result<()> {
        Ok(self.spirc.prev()?)
    }

    pub fn seek(&self, position_ms: u32) -> Result<()> {
        Ok(self.spirc.set_position_ms(position_ms)?)
    }

    pub fn set_volume(&self, volume: u16) -> Result<()> {
        Ok(self.spirc.set_volume(volume)?)
    }

    pub fn shuffle(&self, on: bool) -> Result<()> {
        Ok(self.spirc.shuffle(on)?)
    }

    pub fn repeat(&self, mode: Repeat) -> Result<()> {
        self.spirc.repeat(mode != Repeat::Off)?;
        self.spirc.repeat_track(mode == Repeat::Track)?;
        Ok(())
    }

    pub fn set_crossfade(&self, crossfade: Duration) {
        self.engine.player.set_crossfade(crossfade);
    }

    pub fn shutdown(&self) {
        let _ = self.spirc.shutdown();
    }
}

/// Connects the player in the background so the UI shows up instantly.
pub fn spawn(paths: Paths, crossfade: Duration, tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let result = async {
            let pending = session::prepare(&paths)?;
            let session = pending.session.clone();
            let engine = Engine::new(&session, crossfade)?;
            let mut events = engine.player.get_player_event_channel();
            let (spirc, task) = engine.connect_device(pending, DEVICE_NAME.into()).await?;
            tokio::spawn(task);

            let player_tx = tx.clone();
            tokio::spawn(async move {
                while let Some(event) = events.recv().await {
                    if player_tx.send(AppEvent::Player(event)).is_err() {
                        break;
                    }
                }
            });
            anyhow::Ok(Controller { spirc, session, engine })
        }
        .await;
        let _ = tx.send(match result {
            Ok(controller) => AppEvent::PlayerReady(Box::new(controller)),
            Err(err) => AppEvent::PlayerFailed(format!("{err:#}")),
        });
    });
}

#[cfg(test)]
mod tests {
    use librespot::core::SpotifyUri;

    use super::*;

    const URI: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";

    fn track(uri: &str) -> CurrentTrack {
        CurrentTrack {
            uri: uri.into(),
            name: "Song".into(),
            artists: vec!["Artist".into()],
            artist_uris: Vec::new(),
            album: "Album".into(),
            album_uri: None,
            duration_ms: 200_000,
            cover_url: None,
            cover_url_large: None,
        }
    }

    /// A paused track, so positions don't move while a test runs.
    fn paused_track() -> NowPlaying {
        let mut now = NowPlaying::new();
        now.track = Some(track(URI));
        now
    }

    fn id() -> SpotifyUri {
        SpotifyUri::from_uri(URI).unwrap()
    }

    fn seeked(position_ms: u32) -> PlayerEvent {
        PlayerEvent::Seeked { play_request_id: 1, track_id: id(), position_ms }
    }

    fn position_changed(position_ms: u32) -> PlayerEvent {
        PlayerEvent::PositionChanged { play_request_id: 1, track_id: id(), position_ms }
    }

    #[test]
    fn seek_moves_at_once_and_ignores_reports_from_before() {
        let mut now = paused_track();
        now.seek(60_000);
        assert_eq!(now.position_ms(), 60_000);
        now.apply(position_changed(10_000));
        assert_eq!(now.position_ms(), 60_000);
        now.apply(seeked(60_200));
        assert_eq!(now.position_ms(), 60_200);
        // Confirmed: the player's reports count again.
        now.apply(position_changed(10_000));
        assert_eq!(now.position_ms(), 10_000);
    }

    #[test]
    fn quick_seeks_wait_for_the_last_one() {
        let mut now = paused_track();
        now.seek(5_000);
        now.seek(10_000);
        now.apply(seeked(5_000));
        assert_eq!(now.position_ms(), 10_000);
        now.apply(seeked(10_000));
        assert_eq!(now.position_ms(), 10_000);
    }

    #[test]
    fn a_lost_seek_stops_holding_the_position() {
        let mut now = paused_track();
        now.seek(60_000);
        now.seeking = Some((60_000, Instant::now() - ECHO_WINDOW));
        now.apply(position_changed(10_000));
        assert_eq!(now.position_ms(), 10_000);
    }

    #[test]
    fn pausing_after_a_seek_keeps_the_new_position() {
        let mut now = paused_track();
        now.playing = true;
        now.seek(90_000);
        now.apply(PlayerEvent::Paused { play_request_id: 1, track_id: id(), position_ms: 30_000 });
        assert!(!now.playing);
        assert!((90_000..90_500).contains(&now.position_ms()));
    }

    #[test]
    fn pausing_freezes_the_clock_where_it_stands() {
        let mut now = paused_track();
        now.playing = true;
        now.position_ms = 1_000;
        now.position_at = Instant::now() - Duration::from_secs(2);
        now.set_playing(false);
        let frozen = now.position_ms();
        assert!((3_000..3_500).contains(&frozen));
        assert_eq!(now.position_ms(), frozen);
    }
}
