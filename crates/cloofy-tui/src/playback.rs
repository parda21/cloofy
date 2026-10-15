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
use futures::StreamExt;
use librespot::{
    connect::{LoadContextOptions, LoadRequest, LoadRequestOptions, Options, PlayingTrack, Spirc},
    core::{dealer::protocol::Message, session::Session},
    metadata::audio::{AudioItem, UniqueFields},
    playback::player::PlayerEvent,
    protocol::connect::ClusterUpdate,
};
use tokio::sync::{
    Notify,
    mpsc::{UnboundedReceiver, UnboundedSender},
};

use crate::app::AppEvent;

const DEVICE_NAME: &str = "cloofy";
/// How long a change made in cloofy wins over reports the player sent before it got there.
const ECHO_WINDOW: Duration = Duration::from_millis(1500);
/// A reported position this close to a seek target confirms the seek.
const SEEK_TOLERANCE_MS: u32 = 1000;
/// Spirc reports nothing for a volume it already has, so a change counts as done after this.
const VOLUME_TIMEOUT: Duration = Duration::from_secs(1);
/// How long after Spotify accepted a remote command its state may still show the old values.
const REMOTE_GRACE: Duration = Duration::from_secs(2);
/// Both ends extrapolate remote positions, so they agree less closely than local ones.
const REMOTE_SEEK_TOLERANCE_MS: u32 = 2000;
/// Devices round volumes to their own steps.
const REMOTE_VOLUME_TOLERANCE: u32 = 3;
/// A track start that hasn't begun loading after this long has failed.
pub const START_TIMEOUT: Duration = Duration::from_secs(8);
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

impl Repeat {
    /// The Web API's "off", "context" or "track".
    pub fn from_remote(mode: &str) -> Self {
        match mode {
            "track" => Repeat::Track,
            "context" => Repeat::Context,
            _ => Repeat::Off,
        }
    }

    pub fn remote_name(self) -> &'static str {
        match self {
            Repeat::Off => "off",
            Repeat::Context => "context",
            Repeat::Track => "track",
        }
    }
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

/// A track start that was asked for, or that the player announced, and hasn't begun playing.
pub struct Pending {
    /// The track that will play, once known.
    pub uri: Option<String>,
    /// The track shown when the start was asked for; a skip is through once another one shows.
    from: Option<String>,
    /// Next or previous rather than a new track or context.
    pub skip: bool,
    pub since: Instant,
    /// The local player is loading it.
    loading: bool,
    /// What was shown before the track was switched early, to put back if the start fails.
    previous: Option<Shown>,
}

struct Shown {
    track: CurrentTrack,
    cover: Option<Cover>,
    playing: bool,
    position_ms: u32,
    position_at: Instant,
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
    /// A track on its way; the play button spins meanwhile.
    pub pending: Option<Pending>,
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
            pending: None,
        }
    }

    /// Notes a track start sent to a player, until it plays. `uri` is the track that will
    /// play, if known.
    pub fn start(&mut self, uri: Option<String>) {
        // A start right after another one still waits to replace what played before both.
        let (from, previous) = match self.pending.take() {
            Some(pending) if !pending.loading => (pending.from, pending.previous),
            _ => (self.track.as_ref().map(|t| t.uri.clone()), None),
        };
        self.pending = Some(Pending { uri, from, skip: false, since: Instant::now(), loading: false, previous });
    }

    /// Notes a skip to the next or previous track sent to a player, until it plays.
    pub fn skip(&mut self) {
        self.start(None);
        if let Some(pending) = &mut self.pending {
            pending.skip = true;
        }
    }

    /// Shows the track just asked for at once, before its audio has loaded. The one shown
    /// before comes back if the start fails.
    pub fn show_early(&mut self, track: CurrentTrack) {
        let Some(pending) = &mut self.pending else { return };
        if pending.uri.as_deref() != Some(track.uri.as_str()) || self.track.as_ref().is_some_and(|t| t.uri == track.uri)
        {
            return;
        }
        if pending.previous.is_none()
            && let Some(old) = self.track.take()
        {
            pending.previous = Some(Shown {
                track: old,
                cover: self.cover.take(),
                playing: self.playing,
                position_ms: self.position_ms,
                position_at: self.position_at,
            });
        }
        // The old cover would sit next to the new title; the placeholder shows until the new one loads.
        self.cover = None;
        self.track = Some(track);
        self.seeking = None;
        self.playing = false;
        self.set_position(0);
    }

    /// Whether a start went nowhere: the local player never began loading it.
    pub fn start_failed(&self) -> bool {
        self.pending.as_ref().is_some_and(|p| !p.loading && p.since.elapsed() >= START_TIMEOUT)
    }

    /// Gives up on a start, putting back what was shown before it.
    pub fn cancel_start(&mut self) {
        let Some(shown) = self.pending.take().and_then(|p| p.previous) else { return };
        self.track = Some(shown.track);
        self.cover = shown.cover;
        self.playing = shown.playing;
        self.position_ms = shown.position_ms;
        self.position_at = shown.position_at;
    }

    /// Whether a poll of another device predates the pending start, still showing the track
    /// from before it. `waiting` says Spotify may still lag behind the commands sent. Ends the
    /// start once a poll shows it through.
    pub fn awaits_remote(&mut self, playback: &Playback, waiting: bool) -> bool {
        let Some(pending) = &self.pending else { return false };
        let shown = playback.track.as_ref().map(|t| t.uri.as_str());
        let through = shown != pending.from.as_deref() || (pending.uri.is_some() && shown == pending.uri.as_deref());
        // A device may take a while to load a picked track. Skips and contexts may not change
        // the track at all (the end of a playlist, a context starting with the track already
        // playing), so they wait only as long as Spotify may lag behind.
        let gave_up = if pending.uri.is_some() { pending.since.elapsed() >= START_TIMEOUT } else { !waiting };
        if through || gave_up {
            self.pending = None;
            return false;
        }
        true
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
        // Until the player loads a track shown early, its reports are about the old one.
        if self.pending.as_ref().is_some_and(|p| p.previous.is_some() && !p.loading) {
            return;
        }
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

    /// Mirrors playback on another device, keeping the changes in `intent` that the poll
    /// doesn't show yet. Returns a cover URL when the track changed, and whether some change
    /// is still unconfirmed.
    pub fn apply_remote(&mut self, playback: &Playback, intent: &mut RemoteIntent) -> (Option<String>, bool) {
        let same_track = self.track.as_ref().map(|t| t.uri.as_str()) == playback.track.as_ref().map(|t| t.uri.as_str());
        let shown_ms = self.position_ms();
        let cover = self.mirror(playback);
        let unconfirmed = intent.reconcile(self, playback, same_track.then_some(shown_ms));
        (cover, unconfirmed)
    }

    fn mirror(&mut self, playback: &Playback) -> Option<String> {
        self.device = Some(playback.device.name.clone());
        self.playing = playback.playing;
        self.set_position(playback.progress_ms);
        self.shuffle = playback.shuffle;
        self.repeat = Repeat::from_remote(&playback.repeat);
        if let Some(percent) = playback.device.volume_percent {
            self.volume = (percent.min(100) * u16::MAX as u32 / 100) as u16;
        }
        let Some(remote) = &playback.track else {
            self.track = None;
            return None;
        };
        // A track shown early lacks the details only the device's state has, like its cover.
        if self.track.as_ref().is_some_and(|t| t.uri == remote.uri && t.cover_url == remote.cover_url) {
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
            PlayerEvent::Loading { track_id, position_ms, .. } => {
                let uri = track_id.to_uri();
                // Also covers skips from the phone and next tracks that weren't preloaded.
                if self.pending.is_none() {
                    self.start(None);
                }
                if let Some(pending) = &mut self.pending {
                    pending.uri = Some(uri.clone());
                    pending.loading = true;
                }
                // The old track's audio has stopped, so its clock does too.
                self.set_playing(false);
                if self.track.as_ref().is_some_and(|t| t.uri == uri) {
                    self.report_position(position_ms);
                }
            }
            PlayerEvent::TrackChanged { audio_item } => {
                let track = current_track(&audio_item);
                let cover_url = track.cover_url.clone();
                self.track = Some(track);
                self.pending = None;
                self.seeking = None;
                self.set_position(0);
                if self.cover.as_ref().map(|c| &c.url) != cover_url.as_ref() {
                    return cover_url;
                }
            }
            PlayerEvent::Playing { position_ms, .. } => {
                self.pending = None;
                // Freeze first, so a report from before a seek leaves the shown position put.
                self.set_playing(false);
                self.report_position(position_ms);
                self.playing = true;
            }
            PlayerEvent::Paused { position_ms, .. } => {
                self.pending = None;
                self.set_playing(false);
                self.report_position(position_ms);
            }
            PlayerEvent::Stopped { .. } => {
                self.pending = None;
                self.playing = false;
            }
            PlayerEvent::Seeked { position_ms, .. } => {
                // "Previous" restarts the track instead when it has played for a while.
                if self.pending.as_ref().is_some_and(|p| p.skip && !p.loading) {
                    self.pending = None;
                }
                self.report_position(position_ms);
            }
            PlayerEvent::PositionCorrection { position_ms, .. } | PlayerEvent::PositionChanged { position_ms, .. } => {
                self.report_position(position_ms)
            }
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

/// Volume changes for cloofy's own device. Spirc follows every command with a network round
/// trip, so a drag or a held key would queue dozens of changes and report each one back
/// afterwards, dragging the slider through old values. Only one change is on its way at a
/// time, newer ones replace the waiting one, and reports of values in between are hidden.
#[derive(Default)]
pub struct VolumeSync {
    /// The volume asked for last, until the player reports it.
    target: Option<u16>,
    /// The volume sent to the player and not reported back yet, and when it was sent.
    sent: Option<(u16, Instant)>,
}

impl VolumeSync {
    /// Asks for `volume`. Returns it when it should be sent to the player now.
    pub fn request(&mut self, volume: u16) -> Option<u16> {
        self.target = Some(volume);
        self.next()
    }

    /// Keeps `volume` for when cloofy's device becomes active; Spirc ignores it until then.
    pub fn defer(&mut self, volume: u16) {
        self.target = Some(volume);
    }

    /// The volume to send now: the wanted one, unless another one is still on its way.
    pub fn next(&mut self) -> Option<u16> {
        if self.sent.is_some() {
            return None;
        }
        let target = self.target?;
        self.sent = Some((target, Instant::now()));
        Some(target)
    }

    /// Handles the player reporting `volume`. Returns whether the slider should show it, and
    /// a volume to send next.
    pub fn reported(&mut self, volume: u16) -> (bool, Option<u16>) {
        if self.sent.is_some_and(|(sent, at)| sent == volume || at.elapsed() >= VOLUME_TIMEOUT) {
            self.sent = None;
        }
        match self.target {
            // Changed elsewhere, e.g. from the phone.
            None => (true, None),
            Some(target) if target == volume && self.sent.is_none() => {
                self.target = None;
                (true, None)
            }
            Some(_) => (false, self.next()),
        }
    }

    /// Gives up waiting for a report that doesn't come. Returns a volume to send next.
    pub fn expire(&mut self) -> Option<u16> {
        let (sent, at) = self.sent?;
        if at.elapsed() < VOLUME_TIMEOUT {
            return None;
        }
        self.sent = None;
        if self.target == Some(sent) {
            self.target = None;
        }
        self.next()
    }
}

/// Controls changed in cloofy for another device. Spotify's state lags behind the commands it
/// accepts, so until a poll shows a new value, polls that still show the old one don't undo
/// it on screen.
#[derive(Default)]
pub struct RemoteIntent {
    pub playing: Option<bool>,
    pub shuffle: Option<bool>,
    pub repeat: Option<Repeat>,
    pub volume: Option<u16>,
    /// Seek target and when it was asked for.
    pub seek: Option<(u32, Instant)>,
    /// Commands sent and not answered yet.
    in_flight: usize,
    answered_at: Option<Instant>,
}

impl RemoteIntent {
    pub fn sent(&mut self) {
        self.in_flight += 1;
    }

    /// Spotify answered `commands` commands. After a failure the next poll shows the truth.
    pub fn answered(&mut self, commands: usize, ok: bool) {
        self.in_flight = self.in_flight.saturating_sub(commands);
        self.answered_at = Some(Instant::now());
        if !ok {
            self.forget();
        }
    }

    /// Whether Spotify's state may still lag behind the commands sent.
    pub fn is_waiting(&self) -> bool {
        self.in_flight > 0 || self.answered_at.is_some_and(|at| at.elapsed() < REMOTE_GRACE)
    }

    fn forget(&mut self) {
        *self = Self { in_flight: self.in_flight, answered_at: self.answered_at, ..Self::default() };
    }

    /// Puts back what was asked for where a poll, already applied to `now`, doesn't show it
    /// yet, and forgets what the poll confirms. `shown_ms` is the position shown before the
    /// poll, if it is still the same track. Returns whether something is still unconfirmed.
    fn reconcile(&mut self, now: &mut NowPlaying, playback: &Playback, shown_ms: Option<u32>) -> bool {
        if !self.is_waiting() {
            self.forget();
            return false;
        }
        let mut keep_position = false;
        if let Some(playing) = self.playing {
            if playback.playing == playing {
                self.playing = None;
            } else {
                now.playing = playing;
                keep_position = true;
            }
        }
        if let Some((target, at)) = self.seek {
            let moved = if now.playing { at.elapsed().as_millis() as u32 } else { 0 };
            if shown_ms.is_none() || playback.progress_ms.abs_diff(target + moved) <= REMOTE_SEEK_TOLERANCE_MS {
                self.seek = None;
            } else {
                keep_position = true;
            }
        }
        // A new track brings its own position.
        if let Some(position_ms) = shown_ms.filter(|_| keep_position) {
            now.set_position(position_ms);
        }
        if let Some(shuffle) = self.shuffle {
            if playback.shuffle == shuffle {
                self.shuffle = None;
            } else {
                now.shuffle = shuffle;
            }
        }
        if let Some(repeat) = self.repeat {
            if Repeat::from_remote(&playback.repeat) == repeat {
                self.repeat = None;
            } else {
                now.repeat = repeat;
            }
        }
        if let Some(volume) = self.volume {
            let wanted = volume as u32 * 100 / u16::MAX as u32;
            match playback.device.volume_percent {
                Some(percent) if percent.abs_diff(wanted) > REMOTE_VOLUME_TOLERANCE => now.volume = volume,
                // Confirmed, or a device without volume control.
                _ => self.volume = None,
            }
        }
        self.playing.is_some()
            || self.seek.is_some()
            || self.shuffle.is_some()
            || self.repeat.is_some()
            || self.volume.is_some()
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
    /// Set when cloofy drops the connection to make a new one, as after sleep, so connecting
    /// again doesn't wait.
    forced: Arc<AtomicBool>,
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
        self.forced.store(true, Ordering::Relaxed);
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
pub fn spawn(paths: Paths, crossfade: Duration, volume: u16, retry_now: Arc<Notify>, tx: UnboundedSender<AppEvent>) {
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
                    // Before the task starts the dealer: while it starts, it takes no subscriptions.
                    forward_pushes(&session, tx.clone());
                    let stopping = Arc::new(AtomicBool::new(false));
                    let forced = Arc::new(AtomicBool::new(false));
                    let controller = Controller {
                        spirc,
                        session: session.clone(),
                        engine: engine.clone(),
                        stopping: stopping.clone(),
                        forced: forced.clone(),
                    };
                    if tx.send(AppEvent::PlayerReady(Box::new(controller))).is_err() {
                        return;
                    }
                    let connected = Instant::now();
                    task.await;
                    if stopping.load(Ordering::Relaxed) {
                        return;
                    }
                    // Nothing controls the old track any more, and the app offers to resume it.
                    // A track that was already downloaded would otherwise play on.
                    engine.player.stop();
                    if !session.is_invalid() {
                        session.shutdown();
                    }
                    tracing::warn!("connection to Spotify lost, connecting again");
                    if tx.send(AppEvent::PlayerLost).is_err() {
                        return;
                    }
                    // One cloofy dropped itself, after sleep, goes ahead at once; one that dropped
                    // soon after it started waits like a failed attempt.
                    if forced.load(Ordering::Relaxed) || connected.elapsed() >= STABLE {
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
            volume = engine.volume();
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

/// What Spotify pushed about the device that plays.
#[derive(Debug, Clone)]
pub struct ConnectPush {
    /// Empty when no device is active.
    pub active_device: String,
    pub track: String,
    /// The playlist, album or artist it plays from, or empty.
    pub context: String,
    pub playing: bool,
}

impl ConnectPush {
    /// Whether a poll would learn something new. cloofy's own player reports its changes
    /// itself, except for what it plays from when another app started it; with nothing
    /// playing anywhere, there is only news if another device was playing.
    pub fn worth_polling(&self, own_device: Option<&str>, remote_active: bool, context: Option<&str>) -> bool {
        let here = own_device == Some(self.active_device.as_str());
        let elsewhere = !here && !self.active_device.is_empty();
        let new_context = here && !self.context.is_empty() && context != Some(self.context.as_str());
        elsewhere || new_context || remote_active
    }

    /// Whether a poll of the same device still shows the state from before this push.
    pub fn is_ahead_of(&self, playback: &Playback) -> bool {
        let other_track = !self.track.is_empty() && playback.track.as_ref().is_some_and(|t| t.uri != self.track);
        self.active_device == playback.device.id && (self.playing != playback.playing || other_track)
    }
}

/// Spotify pushes every change of what plays where to all of the user's devices, cloofy
/// included. The details still come from a poll; the push says when one is worth it.
fn forward_pushes(session: &Session, tx: UnboundedSender<AppEvent>) {
    let pushes = session.dealer().listen_for("hm://connect-state/v1/cluster", Message::from_raw::<ClusterUpdate>);
    let mut pushes = match pushes {
        Ok(pushes) => pushes,
        Err(err) => return tracing::warn!("connect state pushes: {err}"),
    };
    tokio::spawn(async move {
        while let Some(update) = pushes.next().await {
            let update = match update {
                Ok(update) if update.cluster.is_some() => update,
                Ok(_) => continue,
                Err(err) => {
                    tracing::warn!("connect state push: {err}");
                    continue;
                }
            };
            let state = &update.cluster.player_state;
            let push = ConnectPush {
                active_device: update.cluster.active_device_id.clone(),
                track: state.track.uri.clone(),
                context: state.context_uri.clone(),
                playing: state.is_playing && !state.is_paused,
            };
            if tx.send(AppEvent::Pushed(push)).is_err() {
                break;
            }
        }
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

    /// A poll of another device playing `uri`.
    fn poll(uri: &str, playing: bool, progress_ms: u32) -> Playback {
        use cloofy_spotify::connect::{Device, PlayingTrack};
        Playback {
            device: Device {
                id: "phone".into(),
                name: "Phone".into(),
                kind: "Smartphone".into(),
                active: true,
                volume_percent: Some(50),
            },
            playing,
            progress_ms,
            shuffle: false,
            repeat: "off".into(),
            track: Some(PlayingTrack {
                uri: uri.into(),
                name: "Song".into(),
                artists: Vec::new(),
                artist_uris: Vec::new(),
                album: String::new(),
                album_uri: None,
                duration_ms: 200_000,
                cover_url: None,
                cover_url_large: None,
            }),
            context: None,
        }
    }

    /// Shows another device paused at `progress_ms`.
    fn remote_track(progress_ms: u32) -> (NowPlaying, RemoteIntent) {
        let (mut now, mut intent) = (NowPlaying::new(), RemoteIntent::default());
        now.apply_remote(&poll(URI, false, progress_ms), &mut intent);
        (now, intent)
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
    fn volume_changes_wait_for_each_other_and_collapse() {
        let mut sync = VolumeSync::default();
        assert_eq!(sync.request(100), Some(100));
        assert_eq!(sync.request(200), None);
        assert_eq!(sync.request(300), None);
        // The first one arrives: hide it and send the newest wish.
        assert_eq!(sync.reported(100), (false, Some(300)));
        assert_eq!(sync.reported(300), (true, None));
        // Nothing pending: changes from elsewhere show.
        assert_eq!(sync.reported(5_000), (true, None));
    }

    #[test]
    fn stale_volume_reports_stay_hidden() {
        let mut sync = VolumeSync::default();
        assert_eq!(sync.request(1_000), Some(1_000));
        assert_eq!(sync.reported(400), (false, None));
        assert_eq!(sync.reported(1_000), (true, None));
    }

    #[test]
    fn a_volume_without_report_gives_way_to_the_next() {
        let mut sync = VolumeSync::default();
        sync.request(100);
        sync.request(200);
        assert_eq!(sync.expire(), None);
        sync.sent = Some((100, Instant::now() - VOLUME_TIMEOUT));
        assert_eq!(sync.expire(), Some(200));
        sync.sent = Some((200, Instant::now() - VOLUME_TIMEOUT));
        assert_eq!(sync.expire(), None);
        assert_eq!(sync.reported(700), (true, None));
    }

    #[test]
    fn a_deferred_volume_goes_out_on_activation() {
        let mut sync = VolumeSync::default();
        sync.defer(900);
        assert_eq!(sync.next(), Some(900));
        // Activation reports the old volume first.
        assert_eq!(sync.reported(500), (false, None));
        assert_eq!(sync.reported(900), (true, None));
    }

    #[test]
    fn remote_play_survives_polls_that_lag_behind() {
        let (mut now, mut intent) = remote_track(10_000);
        now.set_playing(true);
        intent.playing = Some(true);
        intent.sent();
        // Spotify still shows the device paused, a little further along.
        assert!(now.apply_remote(&poll(URI, false, 12_000), &mut intent).1);
        assert!(now.playing);
        assert!((10_000..10_500).contains(&now.position_ms()));
        intent.answered(1, true);
        assert!(!now.apply_remote(&poll(URI, true, 10_100), &mut intent).1);
        assert!(now.playing);
        assert!(intent.playing.is_none());
    }

    #[test]
    fn polls_win_once_spotify_had_time_to_catch_up() {
        let (mut now, mut intent) = remote_track(10_000);
        now.shuffle = true;
        intent.shuffle = Some(true);
        intent.sent();
        intent.answered(1, true);
        intent.answered_at = Some(Instant::now() - REMOTE_GRACE);
        assert!(!now.apply_remote(&poll(URI, false, 10_000), &mut intent).1);
        assert!(!now.shuffle);
    }

    #[test]
    fn polls_win_after_a_failed_command() {
        let (mut now, mut intent) = remote_track(10_000);
        now.repeat = Repeat::Track;
        intent.repeat = Some(Repeat::Track);
        intent.sent();
        intent.answered(1, false);
        now.apply_remote(&poll(URI, false, 10_000), &mut intent);
        assert_eq!(now.repeat, Repeat::Off);
    }

    #[test]
    fn remote_seek_holds_until_a_poll_shows_it() {
        let (mut now, mut intent) = remote_track(10_000);
        now.set_position(60_000);
        intent.seek = Some((60_000, Instant::now()));
        intent.sent();
        assert!(now.apply_remote(&poll(URI, false, 10_000), &mut intent).1);
        assert_eq!(now.position_ms(), 60_000);
        assert!(!now.apply_remote(&poll(URI, false, 60_300), &mut intent).1);
        assert_eq!(now.position_ms(), 60_300);
    }

    #[test]
    fn a_new_remote_track_brings_its_own_position() {
        let (mut now, mut intent) = remote_track(10_000);
        now.set_position(60_000);
        intent.seek = Some((60_000, Instant::now()));
        intent.sent();
        now.apply_remote(&poll("spotify:track:0000000000000000000000", false, 1_000), &mut intent);
        assert_eq!(now.position_ms(), 1_000);
        assert!(intent.seek.is_none());
    }

    #[test]
    fn remote_volume_accepts_the_device_rounding_it() {
        let (mut now, mut intent) = remote_track(0);
        now.volume = u16::MAX;
        intent.volume = Some(u16::MAX);
        intent.sent();
        now.apply_remote(&poll(URI, false, 0), &mut intent);
        assert_eq!(now.volume, u16::MAX);
        let mut rounded = poll(URI, false, 0);
        rounded.device.volume_percent = Some(98);
        assert!(!now.apply_remote(&rounded, &mut intent).1);
        assert_eq!(now.volume, (98 * u16::MAX as u32 / 100) as u16);
    }

    const OTHER: &str = "spotify:track:0000000000000000000000";

    fn playing(position_ms: u32) -> PlayerEvent {
        PlayerEvent::Playing { play_request_id: 1, track_id: id(), position_ms }
    }

    fn loading(uri: &str) -> PlayerEvent {
        PlayerEvent::Loading { play_request_id: 1, track_id: SpotifyUri::from_uri(uri).unwrap(), position_ms: 0 }
    }

    #[test]
    fn a_picked_track_shows_at_once_and_comes_back_if_it_fails() {
        let mut now = paused_track();
        now.playing = true;
        now.position_ms = 42_000;
        now.start(Some(OTHER.into()));
        now.show_early(track(OTHER));
        assert_eq!(now.track.as_ref().unwrap().uri, OTHER);
        assert!(!now.playing);
        assert_eq!(now.position_ms(), 0);
        assert!(!now.start_failed());
        now.pending.as_mut().unwrap().since = Instant::now() - START_TIMEOUT;
        assert!(now.start_failed());
        now.cancel_start();
        assert_eq!(now.track.as_ref().unwrap().uri, URI);
        assert!(now.playing);
        assert!(now.position_ms() >= 42_000);
        assert!(now.pending.is_none());
    }

    #[test]
    fn the_old_track_doesnt_move_one_shown_early() {
        let mut now = paused_track();
        now.start(Some(OTHER.into()));
        now.show_early(track(OTHER));
        now.apply(position_changed(50_000));
        assert_eq!(now.position_ms(), 0);
    }

    #[test]
    fn a_loading_track_waits_as_long_as_it_takes() {
        let mut now = paused_track();
        now.playing = true;
        now.start(Some(OTHER.into()));
        now.show_early(track(OTHER));
        now.apply(loading(OTHER));
        now.pending.as_mut().unwrap().since = Instant::now() - START_TIMEOUT;
        assert!(!now.start_failed());
        now.apply(playing(0));
        assert!(now.pending.is_none());
        assert!(now.playing);
    }

    #[test]
    fn loading_announced_by_the_player_spins_too() {
        let mut now = paused_track();
        now.playing = true;
        now.apply(loading(OTHER));
        assert_eq!(now.pending.as_ref().and_then(|p| p.uri.as_deref()), Some(OTHER));
        assert!(!now.playing);
    }

    #[test]
    fn previous_that_restarts_the_track_ends_the_skip() {
        let mut now = paused_track();
        now.skip();
        now.apply(seeked(0));
        assert!(now.pending.is_none());
    }

    #[test]
    fn a_remote_start_ignores_polls_until_the_device_switches() {
        let (mut now, _) = remote_track(10_000);
        now.start(Some(OTHER.into()));
        now.show_early(track(OTHER));
        assert!(now.awaits_remote(&poll(URI, true, 11_000), true));
        assert!(now.awaits_remote(&poll(URI, true, 12_000), false));
        assert!(!now.awaits_remote(&poll(OTHER, true, 300), false));
        assert!(now.pending.is_none());
        // The device's state fills in what the early track lacked.
        let mut full = poll(OTHER, true, 300);
        full.track.as_mut().unwrap().cover_url = Some("https://i.scdn.co/image/cover".into());
        let (cover, _) = now.apply_remote(&full, &mut RemoteIntent::default());
        assert_eq!(cover.as_deref(), Some("https://i.scdn.co/image/cover"));
    }

    #[test]
    fn quick_picks_still_wait_for_the_track_from_before() {
        let (mut now, _) = remote_track(10_000);
        now.start(Some(OTHER.into()));
        now.show_early(track(OTHER));
        let third = "spotify:track:1111111111111111111111";
        now.start(Some(third.into()));
        now.show_early(track(third));
        // The device still plays the first track: not through yet.
        assert!(now.awaits_remote(&poll(URI, true, 11_000), true));
        // If it fails, the track from before both comes back.
        now.cancel_start();
        assert_eq!(now.track.as_ref().unwrap().uri, URI);
    }

    #[test]
    fn a_remote_skip_waits_only_while_spotify_may_lag() {
        let (mut now, _) = remote_track(10_000);
        now.skip();
        assert!(now.awaits_remote(&poll(URI, true, 11_000), true));
        assert!(!now.awaits_remote(&poll(URI, true, 12_000), false));
        assert!(now.pending.is_none());
    }

    fn push(active_device: &str, track: &str, playing: bool) -> ConnectPush {
        ConnectPush {
            active_device: active_device.into(),
            track: track.into(),
            context: "spotify:playlist:abc".into(),
            playing,
        }
    }

    #[test]
    fn pushes_about_other_devices_are_worth_a_poll() {
        let own = Some("cloofy");
        let context = Some("spotify:playlist:abc");
        assert!(push("phone", URI, true).worth_polling(own, false, context));
        // cloofy's own player reports its own changes…
        assert!(!push("cloofy", URI, true).worth_polling(own, false, context));
        // …but not what another app started it on.
        assert!(push("cloofy", URI, true).worth_polling(own, false, Some("spotify:album:xyz")));
        // Nothing plays anywhere: news only if another device was playing.
        assert!(!push("", "", false).worth_polling(own, false, context));
        assert!(push("", "", false).worth_polling(own, true, context));
    }

    #[test]
    fn a_poll_older_than_a_push_is_noticed() {
        let mut remote = poll(URI, true, 1_000);
        remote.device.id = "phone".into();
        assert!(!push("phone", URI, true).is_ahead_of(&remote));
        assert!(push("phone", URI, false).is_ahead_of(&remote));
        assert!(push("phone", OTHER, true).is_ahead_of(&remote));
        // About another device, or without a track: nothing to compare.
        assert!(!push("speaker", OTHER, false).is_ahead_of(&remote));
        assert!(!push("phone", "", true).is_ahead_of(&remote));
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
