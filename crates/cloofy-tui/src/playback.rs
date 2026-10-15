use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

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
use tokio::sync::{
    Notify,
    mpsc::{UnboundedReceiver, UnboundedSender},
};

use crate::app::AppEvent;

const DEVICE_NAME: &str = "cloofy";
/// A connection that lasted this long was fine: the next one is tried right away.
const STABLE: Duration = Duration::from_secs(30);
/// Waits between attempts while Spotify can't be reached or keeps dropping the connection,
/// doubling up to the longest.
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_LONGEST: Duration = Duration::from_secs(60);

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
        }
    }

    pub fn position_ms(&self) -> u32 {
        let elapsed = if self.playing { self.position_at.elapsed().as_millis() as u32 } else { 0 };
        let duration = self.track.as_ref().map_or(u32::MAX, |t| t.duration_ms);
        (self.position_ms + elapsed).min(duration)
    }

    fn set_position(&mut self, position_ms: u32) {
        self.position_ms = position_ms;
        self.position_at = Instant::now();
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
                self.set_position(0);
                if self.cover.as_ref().map(|c| &c.url) != cover_url.as_ref() {
                    return cover_url;
                }
            }
            PlayerEvent::Playing { position_ms, .. } => {
                self.playing = true;
                self.set_position(position_ms);
            }
            PlayerEvent::Paused { position_ms, .. } => {
                self.playing = false;
                self.set_position(position_ms);
            }
            PlayerEvent::Stopped { .. } => self.playing = false,
            PlayerEvent::PositionCorrection { position_ms, .. }
            | PlayerEvent::PositionChanged { position_ms, .. }
            | PlayerEvent::Seeked { position_ms, .. } => self.set_position(position_ms),
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
    engine: Arc<Engine>,
    /// Set when the connection ends on purpose, so it isn't restored.
    stopping: Arc<AtomicBool>,
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

    pub fn pause(&self) -> Result<()> {
        Ok(self.spirc.pause()?)
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

    /// Drops the connection; the background task connects again.
    pub fn reconnect(&self) {
        self.session.shutdown();
        let _ = self.spirc.shutdown();
    }

    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::Relaxed);
        let _ = self.spirc.shutdown();
    }
}

/// Connects the player in the background so the UI shows up instantly, and connects again
/// whenever the connection drops: after the computer slept, or while the network was gone.
/// Playback starts at the saved `volume`, and keeps its volume when connecting again.
/// `retry_now` cuts a wait between attempts short, for when the computer wakes up.
pub fn spawn(
    paths: Paths,
    crossfade: Duration,
    volume: Option<u16>,
    retry_now: Arc<Notify>,
    tx: UnboundedSender<AppEvent>,
) {
    tokio::spawn(async move {
        let started = async {
            let pending = session::prepare(&paths)?;
            let (engine, lost_outputs) = Engine::new(&pending.session, crossfade)?;
            forward_events(&engine, lost_outputs, &tx);
            anyhow::Ok((pending, Arc::new(engine)))
        };
        let (mut pending, engine) = match started.await {
            Ok(started) => started,
            Err(err) => {
                let _ = tx.send(AppEvent::PlayerFailed(format!("{err:#}")));
                return;
            }
        };
        let mut retry = RETRY_FIRST;
        let mut volume = volume;
        loop {
            let session = pending.session.clone();
            let wait = match engine.connect_device(pending, DEVICE_NAME.into(), volume).await {
                Ok((spirc, task)) => {
                    let stopping = Arc::new(AtomicBool::new(false));
                    let controller = Controller {
                        spirc,
                        session: session.clone(),
                        engine: engine.clone(),
                        stopping: stopping.clone(),
                    };
                    if tx.send(AppEvent::PlayerReady(Box::new(controller))).is_err() {
                        return;
                    }
                    let connected = Instant::now();
                    task.await;
                    if stopping.load(Ordering::Relaxed) {
                        return;
                    }
                    if !session.is_invalid() {
                        session.shutdown();
                    }
                    tracing::warn!("connection to Spotify lost, connecting again");
                    if tx.send(AppEvent::PlayerLost).is_err() {
                        return;
                    }
                    // One that dropped soon after it started waits like a failed attempt.
                    if connected.elapsed() >= STABLE {
                        retry = RETRY_FIRST;
                        false
                    } else {
                        true
                    }
                }
                Err(err) => {
                    tracing::warn!("connecting to Spotify: {err:#}");
                    if tx.send(AppEvent::PlayerFailed(format!("{err:#}"))).is_err() {
                        return;
                    }
                    true
                }
            };
            if wait {
                tokio::select! {
                    () = tokio::time::sleep(retry) => {}
                    () = retry_now.notified() => {}
                }
                retry = (retry * 2).min(RETRY_LONGEST);
            }
            pending = match session::prepare(&paths) {
                Ok(pending) => pending,
                Err(err) => {
                    let _ = tx.send(AppEvent::PlayerFailed(format!("{err:#}")));
                    return;
                }
            };
            engine.player.set_session(pending.session.clone());
            volume = Some(engine.volume());
        }
    });
}

/// Passes the player's events and lost outputs on to the app.
fn forward_events(engine: &Engine, mut lost_outputs: UnboundedReceiver<String>, tx: &UnboundedSender<AppEvent>) {
    let mut events = engine.player.get_player_event_channel();
    let player_tx = tx.clone();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if player_tx.send(AppEvent::Player(event)).is_err() {
                break;
            }
        }
    });
    let output_tx = tx.clone();
    tokio::spawn(async move {
        while let Some(name) = lost_outputs.recv().await {
            if output_tx.send(AppEvent::OutputLost(name)).is_err() {
                break;
            }
        }
    });
}
