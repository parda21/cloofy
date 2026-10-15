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
/// Spirc reports nothing for a volume it already has, so a change counts as done after this.
const VOLUME_TIMEOUT: Duration = Duration::from_secs(1);
/// How long after Spotify accepted a remote command its state may still show the old values.
const REMOTE_GRACE: Duration = Duration::from_secs(2);
/// Both ends extrapolate remote positions, so they agree less closely than local ones.
const REMOTE_SEEK_TOLERANCE_MS: u32 = 2000;
/// Devices round volumes to their own steps.
const REMOTE_VOLUME_TOLERANCE: u32 = 3;

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
pub fn spawn(paths: Paths, crossfade: Duration, volume: u16, tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let result = async {
            let pending = session::prepare(&paths)?;
            let session = pending.session.clone();
            let engine = Engine::new(&session, crossfade)?;
            let mut events = engine.player.get_player_event_channel();
            let (spirc, task) = engine.connect_device(pending, DEVICE_NAME.into(), volume).await?;
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
