//! Playback: mpv plays, cloofy keeps the queue. Commands go over a channel to one task that
//! owns mpv and the queue, so none of them waits.
//!
//! Each song's stream URL comes from yt-dlp, which takes a couple of seconds; the next song's is
//! resolved while the current one plays and appended to mpv's playlist, so mpv moves on to it
//! without a gap. The player reports what happens as [`PlayerEvent`]s, the same ones
//! librespot's player sends for Spotify.

mod mpv;
mod queue;
pub(crate) mod stream;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, bail};
use cloofy_core::{
    catalog::Track,
    paths::Paths,
    player::{CurrentTrack, PlayerEvent, QueueTrack},
};
use futures::StreamExt;
use serde_json::json;
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
};

use self::{
    mpv::{Mpv, MpvEvent},
    queue::Queue,
    stream::Stream,
};
use crate::{
    Source, YtMusic, pages,
    parse::{self, ALBUM, ARTIST, PLAYLIST, TRACK},
};

/// "Previous" restarts the song once it has played this long, as Spotify's apps do.
const RESTART_AFTER_MS: u32 = 3_000;
/// How often the position is reported while playing.
const POSITION_INTERVAL: Duration = Duration::from_secs(1);
/// The next song goes into mpv's playlist this long before the current one ends.
const APPEND_BEFORE_END_MS: u32 = 20_000;
/// Tries to play a song: with a fresh URL, then through mpv's own yt-dlp hook.
const ATTEMPTS: u8 = 3;
/// Playback stops after this many songs in a row couldn't play.
const MAX_SKIPS_IN_A_ROW: u32 = 5;

/// The YouTube Music player.
#[derive(Clone)]
pub struct Player {
    commands: UnboundedSender<Command>,
}

#[derive(Debug)]
enum Command {
    Play { context: String, track: Option<String>, position_ms: u32, shuffle: Option<bool> },
    Toggle { has_track: bool },
    Pause,
    Next,
    Previous,
    Seek(u32),
    Volume(u16),
    Shuffle(bool),
    Repeat { context: bool, track: bool },
    ClearQueue,
    AddToQueue(String),
    SkipTo(String),
    Shutdown,
}

impl Player {
    /// Starts mpv at `volume` (0 to 65535). Fails when mpv isn't installed.
    pub async fn spawn(
        client: YtMusic,
        paths: &Paths,
        volume: u16,
        events: UnboundedSender<PlayerEvent>,
    ) -> Result<Player> {
        Self::spawn_with_args(client, paths, volume, events, Vec::new()).await
    }

    /// [`Player::spawn`] with more arguments for mpv, such as `--ao=null` for tests.
    pub async fn spawn_with_args(
        client: YtMusic,
        paths: &Paths,
        volume: u16,
        events: UnboundedSender<PlayerEvent>,
        mpv_args: Vec<String>,
    ) -> Result<Player> {
        let (mpv_tx, mpv_rx) = mpsc::unbounded_channel();
        let mpv = Mpv::spawn(&with_volume(&mpv_args, volume), mpv_tx.clone()).await?;
        let (commands, commands_rx) = mpsc::unbounded_channel();
        let (jobs, jobs_rx) = mpsc::unbounded_channel();
        let engine = Engine {
            client,
            events,
            mpv: Some(mpv),
            mpv_args,
            mpv_events: mpv_tx,
            jobs,
            cache_dir: Some(paths.cache.join("ytmusic").join("yt-dlp")),
            queue: Queue::default(),
            streams: HashMap::new(),
            resolving: HashSet::new(),
            unresolvable: HashSet::new(),
            attempts: HashMap::new(),
            skipped_in_a_row: 0,
            generation: 0,
            loader: None,
            context_loaded: false,
            starting: None,
            current: None,
            next: None,
            waiting: None,
            started: false,
            paused: false,
            position_ms: 0,
            duration_ms: None,
            seeking: None,
            volume,
            active: false,
            autoplay: Autoplay::default(),
            ended_at: None,
            reported: None,
        };
        let _ = engine.events.send(PlayerEvent::VolumeChanged { volume });
        tokio::spawn(engine.run(commands_rx, mpv_rx, jobs_rx));
        Ok(Player { commands })
    }

    /// Plays a playlist, album or artist (its songs), or a song and its radio, from
    /// `track_uri` if given.
    pub fn play(&self, context_uri: String, track_uri: Option<String>) -> Result<()> {
        self.send(Command::Play { context: context_uri, track: track_uri, position_ms: 0, shuffle: None })
    }

    /// Continues a song from the last session at `position_ms`, in its context if known.
    pub fn resume(&self, context: Option<String>, track: String, position_ms: u32) -> Result<()> {
        let context = context.unwrap_or_else(|| track.clone());
        self.send(Command::Play { context, track: Some(track), position_ms, shuffle: None })
    }

    pub fn play_shuffled(&self, context_uri: String) -> Result<()> {
        self.send(Command::Play { context: context_uri, track: None, position_ms: 0, shuffle: Some(true) })
    }

    /// Plays or pauses; with nothing loaded, there is nothing to do.
    pub fn toggle(&self, has_track: bool) -> Result<()> {
        self.send(Command::Toggle { has_track })
    }

    pub fn pause(&self) -> Result<()> {
        self.send(Command::Pause)
    }

    pub fn next(&self) -> Result<()> {
        self.send(Command::Next)
    }

    /// Goes to the song before, or restarts this one once it has played for 3 seconds.
    pub fn previous(&self) -> Result<()> {
        self.send(Command::Previous)
    }

    pub fn seek(&self, position_ms: u32) -> Result<()> {
        self.send(Command::Seek(position_ms))
    }

    /// From 0 to 65535, as for librespot.
    pub fn set_volume(&self, volume: u16) -> Result<()> {
        self.send(Command::Volume(volume))
    }

    pub fn shuffle(&self, on: bool) -> Result<()> {
        self.send(Command::Shuffle(on))
    }

    pub fn repeat(&self, context: bool, track: bool) -> Result<()> {
        self.send(Command::Repeat { context, track })
    }

    /// Empties the queue; the rest of the context stays.
    pub fn clear_queue(&self) -> Result<()> {
        self.send(Command::ClearQueue)
    }

    /// Adds a song after the ones queued before.
    pub fn add_to_queue(&self, uri: &str) -> Result<()> {
        self.send(Command::AddToQueue(uri.to_owned()))
    }

    /// Jumps to an upcoming song.
    pub fn skip_to(&self, uri: &str) -> Result<()> {
        self.send(Command::SkipTo(uri.to_owned()))
    }

    /// Stops playback and mpv.
    pub fn shutdown(&self) {
        let _ = self.commands.send(Command::Shutdown);
    }

    fn send(&self, command: Command) -> Result<()> {
        self.commands.send(command).map_err(|_| anyhow!("The YouTube Music player has stopped"))
    }
}

fn with_volume(args: &[String], volume: u16) -> Vec<String> {
    let mut args = args.to_vec();
    args.push(format!("--volume={}", mpv_volume(volume)));
    args
}

/// mpv's volume goes from 0 to 100, on a cubic curve that sounds even.
fn mpv_volume(volume: u16) -> f64 {
    (f64::from(volume) * 100.0 / f64::from(u16::MAX) * 10.0).round() / 10.0
}

/// Work done off the player's task, and its result.
enum Job {
    Resolved { video: String, result: Result<Stream> },
    Context { generation: u64, page: Result<Vec<Track>>, last: bool },
    Autoplay { generation: u64, tracks: Result<Vec<Track>> },
    Queued { uri: String, track: Result<Option<Track>> },
}

/// A context being loaded, waiting for the page with the song to start at.
struct Starting {
    context: String,
    track: Option<String>,
    position_ms: u32,
    shuffle: bool,
    tracks: Vec<Track>,
}

/// A song in mpv's playlist.
struct Loaded {
    entry: u64,
    uri: String,
}

#[derive(Default)]
struct Autoplay {
    fetching: bool,
    /// The context ended while the radio was being fetched; play it once it's there.
    waiting: bool,
}

struct Engine {
    client: YtMusic,
    events: UnboundedSender<PlayerEvent>,
    mpv: Option<Mpv>,
    mpv_args: Vec<String>,
    mpv_events: UnboundedSender<MpvEvent>,
    jobs: UnboundedSender<Job>,
    cache_dir: Option<PathBuf>,
    queue: Queue,
    /// Resolved stream URLs by video ID.
    streams: HashMap<String, Stream>,
    resolving: HashSet<String>,
    /// Songs yt-dlp couldn't resolve, which mpv's hook gets to try.
    unresolvable: HashSet<String>,
    /// Failed tries to play each song, by URI.
    attempts: HashMap<String, u8>,
    skipped_in_a_row: u32,
    /// Counts loaded contexts, so results for one before are dropped.
    generation: u64,
    loader: Option<JoinHandle<()>>,
    /// Every page of the context arrived.
    context_loaded: bool,
    starting: Option<Starting>,
    /// The song that plays, once mpv has it.
    current: Option<Loaded>,
    /// The song after it, loaded ahead.
    next: Option<Loaded>,
    /// The current song waits for its URL, to start at this position.
    waiting: Option<u32>,
    /// mpv began playing the current song.
    started: bool,
    paused: bool,
    position_ms: u32,
    /// The current song's length as mpv has it, which beats the catalog's.
    duration_ms: Option<u32>,
    /// A seek on its way, to this position.
    seeking: Option<u32>,
    volume: u16,
    active: bool,
    autoplay: Autoplay,
    /// When the song before ended, to log the gap to the next one.
    ended_at: Option<Instant>,
    /// The last queue reported, to report changes only.
    reported: Option<(String, Option<String>, Vec<QueueTrack>)>,
}

impl Engine {
    async fn run(
        mut self,
        mut commands: UnboundedReceiver<Command>,
        mut mpv_events: UnboundedReceiver<MpvEvent>,
        mut jobs: UnboundedReceiver<Job>,
    ) {
        let mut tick = tokio::time::interval(POSITION_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let result = tokio::select! {
                command = commands.recv() => match command {
                    Some(Command::Shutdown) | None => break,
                    Some(command) => self.command(command).await,
                },
                Some(event) = mpv_events.recv() => self.mpv_event(event).await,
                Some(job) = jobs.recv() => self.job(job).await,
                _ = tick.tick() => self.tick().await,
            };
            if let Err(err) = result {
                tracing::warn!("YouTube Music player: {err:#}");
            }
        }
        if let Some(loader) = self.loader.take() {
            loader.abort();
        }
        if let Some(mpv) = self.mpv.take() {
            mpv.quit().await;
        }
        if self.active {
            self.emit(PlayerEvent::Stopped);
            self.emit(PlayerEvent::Deactivated);
        }
    }

    fn emit(&self, event: PlayerEvent) {
        let _ = self.events.send(event);
    }

    async fn command(&mut self, command: Command) -> Result<()> {
        tracing::debug!("player: {command:?}");
        match command {
            Command::Play { context, track, position_ms, shuffle } => {
                self.play(context, track, position_ms, shuffle).await
            }
            Command::Toggle { has_track } => self.toggle(has_track).await,
            Command::Pause => self.set_paused(true).await,
            Command::Next => self.skip().await,
            Command::Previous => self.previous().await,
            Command::Seek(position_ms) => self.seek(position_ms).await,
            Command::Volume(volume) => {
                self.volume = volume;
                self.mpv().await?.set("volume", json!(mpv_volume(volume))).await?;
                self.emit(PlayerEvent::VolumeChanged { volume });
                Ok(())
            }
            Command::Shuffle(on) => {
                self.queue.set_shuffle(on);
                if let Some(starting) = self.starting.as_mut() {
                    starting.shuffle = on;
                }
                self.emit(PlayerEvent::ShuffleChanged { shuffle: on });
                self.queue_changed().await
            }
            Command::Repeat { context, track } => {
                self.queue.repeat_context = context;
                self.queue.repeat_track = track;
                self.emit(PlayerEvent::RepeatChanged { context, track });
                self.queue_changed().await
            }
            Command::ClearQueue => {
                self.queue.clear_queue();
                self.queue_changed().await
            }
            Command::AddToQueue(uri) => {
                let (client, jobs) = (self.client.clone(), self.jobs.clone());
                tokio::spawn(async move {
                    let track = client.tracks_by_uri(std::slice::from_ref(&uri)).await.map(|t| t.into_iter().next());
                    let _ = jobs.send(Job::Queued { uri, track });
                });
                Ok(())
            }
            Command::SkipTo(uri) => {
                if self.queue.skip_to(&uri).is_none() {
                    bail!("{uri} isn't coming up");
                }
                self.begin(0).await
            }
            Command::Shutdown => Ok(()),
        }
    }

    /// mpv, started again should it have gone away.
    async fn mpv(&mut self) -> Result<&mut Mpv> {
        if self.mpv.is_none() {
            let mpv = Mpv::spawn(&with_volume(&self.mpv_args, self.volume), self.mpv_events.clone()).await?;
            self.mpv = Some(mpv);
        }
        Ok(self.mpv.as_mut().expect("just started"))
    }

    async fn play(
        &mut self,
        context: String,
        track: Option<String>,
        position_ms: u32,
        shuffle: Option<bool>,
    ) -> Result<()> {
        if !self.active {
            self.active = true;
            self.emit(PlayerEvent::Activated);
        }
        self.generation += 1;
        self.context_loaded = false;
        self.autoplay = Autoplay::default();
        if let Some(loader) = self.loader.take() {
            loader.abort();
        }
        self.stop_audio().await?;
        if let Some(track) = &track {
            self.emit(PlayerEvent::Loading { uri: track.clone(), position_ms });
            // The URL takes longer than the context; both are on their way at once.
            if track.starts_with(TRACK) && !self.streams.get(video_id(track)).is_some_and(Stream::fresh) {
                self.resolve(video_id(track));
            }
        } else if context.starts_with(TRACK) {
            self.resolve(video_id(&context));
        }
        if let Some(shuffle) = shuffle.filter(|on| *on != self.queue.shuffle) {
            self.emit(PlayerEvent::ShuffleChanged { shuffle });
        }
        let shuffle = shuffle.unwrap_or(self.queue.shuffle);
        self.starting = Some(Starting { context: context.clone(), track, position_ms, shuffle, tracks: Vec::new() });
        self.loader = Some(load_context(self.client.clone(), self.jobs.clone(), self.generation, context));
        Ok(())
    }

    /// Silences mpv; what was loaded is gone.
    async fn stop_audio(&mut self) -> Result<()> {
        self.current = None;
        self.next = None;
        self.waiting = None;
        self.started = false;
        self.seeking = None;
        if let Some(mpv) = self.mpv.as_mut() {
            mpv.command(json!(["stop"])).await?;
        }
        Ok(())
    }

    /// Starts the queue's current song at `start_ms`.
    async fn begin(&mut self, start_ms: u32) -> Result<()> {
        let Some(track) = self.queue.current().cloned() else { return self.stopped().await };
        self.stop_audio().await?;
        // A song that starts plays, even if the one before was paused.
        if self.paused {
            self.mpv().await?.set("pause", json!(false)).await?;
            self.paused = false;
        }
        self.position_ms = start_ms;
        self.emit(PlayerEvent::Loading { uri: track.uri.clone(), position_ms: start_ms });
        self.emit(PlayerEvent::TrackChanged { track: current_track(&track) });
        self.queue_changed().await?;
        self.load_current(start_ms).await?;
        self.want_autoplay();
        Ok(())
    }

    /// Loads the current song into mpv once its URL is there; asks for the URL otherwise.
    async fn load_current(&mut self, start_ms: u32) -> Result<()> {
        let Some(track) = self.queue.current().cloned() else { return Ok(()) };
        let video = video_id(&track.uri).to_owned();
        let url = match self.streams.get(&video).filter(|s| s.fresh()) {
            Some(stream) => stream.url.clone(),
            None if self.unresolvable.contains(&video) => format!("ytdl://{video}"),
            None => {
                self.waiting = Some(start_ms);
                self.resolve(&video);
                return Ok(());
            }
        };
        self.waiting = None;
        let start = (start_ms > 0).then(|| f64::from(start_ms) / 1000.0);
        let entry = self.mpv().await?.load(&url, false, start).await?;
        self.current = Some(Loaded { entry, uri: track.uri });
        self.started = false;
        Ok(())
    }

    fn resolve(&mut self, video: &str) {
        if !self.resolving.insert(video.to_owned()) {
            return;
        }
        let (jobs, video, cache_dir) = (self.jobs.clone(), video.to_owned(), self.cache_dir.clone());
        tokio::spawn(async move {
            let started = Instant::now();
            let result = stream::resolve(&video, cache_dir).await;
            tracing::debug!("resolved {video} in {:?}", started.elapsed());
            let _ = jobs.send(Job::Resolved { video, result });
        });
    }

    /// Gets the song after the current one ready: its URL right away, and into mpv's playlist
    /// near the end of the current one, so mpv moves on to it without a gap. Drops the one
    /// there when the queue changed.
    async fn sync_next(&mut self) -> Result<()> {
        if !self.started || self.current.is_none() {
            return Ok(());
        }
        let wanted = self.queue.peek().map(|t| t.uri.clone());
        if self.next.as_ref().map(|n| &n.uri) == wanted.as_ref() {
            return Ok(());
        }
        if self.next.take().is_some() {
            // Removes every entry but the one that plays.
            self.mpv().await?.command(json!(["playlist-clear"])).await?;
        }
        let Some(uri) = wanted else { return Ok(()) };
        let video = video_id(&uri).to_owned();
        let Some(stream) = self.streams.get(&video).filter(|s| s.fresh()) else {
            if !self.unresolvable.contains(&video) {
                self.resolve(&video);
            }
            return Ok(());
        };
        // mpv opens the next entry ahead of time only once: an entry appended early and then
        // replaced, as when a song is queued, would leave the next one to start with a gap.
        let duration_ms = self.duration_ms.unwrap_or_else(|| self.queue.current().map_or(0, |t| t.duration_ms));
        if duration_ms > 0 && self.position_ms + APPEND_BEFORE_END_MS < duration_ms {
            return Ok(());
        }
        let url = stream.url.clone();
        let entry = self.mpv().await?.load(&url, true, None).await?;
        self.next = Some(Loaded { entry, uri });
        Ok(())
    }

    async fn queue_changed(&mut self) -> Result<()> {
        let report =
            (self.queue.context_uri.clone(), self.queue.current().map(|t| t.uri.clone()), self.queue.upcoming());
        if self.reported.as_ref() != Some(&report) {
            let (context_uri, current, next) = report.clone();
            self.emit(PlayerEvent::QueueChanged { context_uri, current, next });
            self.reported = Some(report);
        }
        self.sync_next().await
    }

    /// Asks for the radio when the context is about to run out.
    fn want_autoplay(&mut self) {
        if !self.context_loaded || self.autoplay.fetching || !self.queue.wants_autoplay() {
            return;
        }
        let Some(seed) = self.queue.seed().map(|t| t.uri.clone()) else { return };
        self.autoplay.fetching = true;
        let (client, jobs, generation) = (self.client.clone(), self.jobs.clone(), self.generation);
        tokio::spawn(async move {
            let tracks = client.radio(&seed).await;
            let _ = jobs.send(Job::Autoplay { generation, tracks });
        });
    }

    async fn toggle(&mut self, has_track: bool) -> Result<()> {
        if self.current.is_some() || self.waiting.is_some() {
            return self.set_paused(!self.paused).await;
        }
        if has_track && self.queue.current().is_some() {
            // Stopped at the end: play the last song again.
            return self.begin(0).await;
        }
        Ok(())
    }

    async fn set_paused(&mut self, paused: bool) -> Result<()> {
        if self.current.is_none() && self.waiting.is_none() {
            return Ok(());
        }
        self.mpv().await?.set("pause", json!(paused)).await
    }

    /// The user skipped.
    async fn skip(&mut self) -> Result<()> {
        if self.queue.advance(false).is_some() {
            return self.begin(0).await;
        }
        self.end_of_queue().await
    }

    async fn previous(&mut self) -> Result<()> {
        if self.position_ms < RESTART_AFTER_MS && self.queue.back().is_some() {
            return self.begin(0).await;
        }
        self.seek(0).await
    }

    async fn seek(&mut self, position_ms: u32) -> Result<()> {
        if self.waiting.is_some() {
            self.waiting = Some(position_ms);
            self.position_ms = position_ms;
            self.emit(PlayerEvent::Seeked { position_ms });
            return Ok(());
        }
        if self.current.is_none() {
            return Ok(());
        }
        self.seeking = Some(position_ms);
        let seconds = f64::from(position_ms) / 1000.0;
        self.mpv().await?.command(json!(["seek", seconds, "absolute"])).await.map(drop)
    }

    /// Nothing is left to play, unless the radio is on its way.
    async fn end_of_queue(&mut self) -> Result<()> {
        if self.autoplay.fetching {
            self.autoplay.waiting = true;
            self.stop_audio().await?;
            return Ok(());
        }
        self.stopped().await
    }

    async fn stopped(&mut self) -> Result<()> {
        self.stop_audio().await?;
        self.emit(PlayerEvent::Stopped);
        Ok(())
    }

    /// Reports the position, and loads the next song when the end draws near.
    async fn tick(&mut self) -> Result<()> {
        if !self.started || self.paused || self.seeking.is_some() || self.current.is_none() {
            return Ok(());
        }
        self.emit(PlayerEvent::PositionChanged { position_ms: self.position_ms });
        if self.next.is_none() {
            self.sync_next().await?;
        }
        Ok(())
    }

    async fn mpv_event(&mut self, event: MpvEvent) -> Result<()> {
        if !matches!(event, MpvEvent::Property { id: mpv::TIME_POS, .. }) {
            tracing::debug!("mpv: {event:?}");
        }
        match event {
            MpvEvent::Property { id: mpv::TIME_POS, data } => {
                if let Some(seconds) = data.as_f64().filter(|_| self.started && self.seeking.is_none()) {
                    self.position_ms = (seconds * 1000.0) as u32;
                }
            }
            MpvEvent::Property { id: mpv::PAUSE, data } => {
                let paused = data.as_bool().unwrap_or(false);
                if paused != self.paused {
                    self.paused = paused;
                    if self.current.is_some() || self.waiting.is_some() {
                        let position_ms = self.position_ms;
                        self.emit(if paused {
                            PlayerEvent::Paused { position_ms }
                        } else {
                            PlayerEvent::Playing { position_ms }
                        });
                    }
                }
            }
            MpvEvent::Property { id: mpv::DURATION, data } => {
                self.duration_ms = data.as_f64().map(|seconds| (seconds * 1000.0) as u32);
            }
            MpvEvent::Property { .. } => {}
            MpvEvent::StartFile { entry } => {
                if self.next.as_ref().is_some_and(|next| next.entry == entry) {
                    self.moved_on().await?;
                }
            }
            MpvEvent::PlaybackRestart => {
                if self.current.is_none() {
                    return Ok(());
                }
                if !self.started {
                    self.started = true;
                    if let Some(ended) = self.ended_at.take() {
                        tracing::debug!("gap between songs: {:?}", ended.elapsed());
                    }
                    if let Some(uri) = self.current.as_ref().map(|c| c.uri.clone()) {
                        self.attempts.remove(&uri);
                    }
                    self.skipped_in_a_row = 0;
                    let position_ms = self.position_ms;
                    self.emit(if self.paused {
                        PlayerEvent::Paused { position_ms }
                    } else {
                        PlayerEvent::Playing { position_ms }
                    });
                    self.sync_next().await?;
                } else if let Some(position_ms) = self.seeking.take() {
                    self.position_ms = position_ms;
                    self.emit(PlayerEvent::Seeked { position_ms });
                    // Maybe close to the end now.
                    self.sync_next().await?;
                }
            }
            MpvEvent::EndFile { entry, reason, error } => {
                if self.current.as_ref().is_none_or(|current| current.entry != entry) {
                    return Ok(());
                }
                match reason.as_str() {
                    // With a song loaded ahead, mpv moves on by itself.
                    "eof" if self.next.is_some() => self.ended_at = Some(Instant::now()),
                    "eof" => self.ended().await?,
                    "error" => self.failed(error.unwrap_or_default()).await?,
                    _ => {}
                }
            }
            MpvEvent::Exited => {
                tracing::warn!("mpv exited");
                self.mpv = None;
                if self.current.take().is_some() || self.waiting.take().is_some() {
                    self.next = None;
                    self.started = false;
                    self.emit(PlayerEvent::Stopped);
                }
            }
        }
        Ok(())
    }

    /// mpv went on to the song loaded ahead.
    async fn moved_on(&mut self) -> Result<()> {
        let next = self.next.take().expect("checked by the caller");
        let track = self.queue.advance(true).cloned();
        if track.as_ref().map(|t| &t.uri) != Some(&next.uri) {
            tracing::warn!("mpv went on to {}, which isn't next any more", next.uri);
            return self.begin(0).await;
        }
        let track = track.expect("checked above");
        self.current = Some(next);
        self.started = false;
        self.position_ms = 0;
        self.emit(PlayerEvent::TrackChanged { track: current_track(&track) });
        // The song that ended is still first in mpv's playlist.
        self.mpv().await?.command(json!(["playlist-clear"])).await?;
        self.queue_changed().await?;
        self.want_autoplay();
        Ok(())
    }

    /// The song ended with nothing loaded ahead.
    async fn ended(&mut self) -> Result<()> {
        if self.queue.advance(true).is_some() {
            return self.begin(0).await;
        }
        self.end_of_queue().await
    }

    /// mpv couldn't play the song: try a fresh URL, then mpv's own yt-dlp hook, then skip it.
    async fn failed(&mut self, error: String) -> Result<()> {
        let Some(track) = self.queue.current().cloned() else { return Ok(()) };
        let video = video_id(&track.uri).to_owned();
        let attempts = self.attempts.entry(track.uri.clone()).or_default();
        *attempts += 1;
        let attempts = *attempts;
        tracing::warn!("can't play {} ({error}), try {attempts} of {ATTEMPTS}", track.uri);
        self.streams.remove(&video);
        if attempts >= ATTEMPTS || self.unresolvable.contains(&video) {
            self.unresolvable.remove(&video);
            self.attempts.remove(&track.uri);
            self.skipped_in_a_row += 1;
            // Something bigger is wrong (no network, yt-dlp missing or outdated); skipping on
            // would run through the whole radio.
            if self.skipped_in_a_row >= MAX_SKIPS_IN_A_ROW {
                tracing::warn!("{} songs in a row couldn't play; stopping", self.skipped_in_a_row);
                self.skipped_in_a_row = 0;
                return self.stopped().await;
            }
            tracing::info!("skipping {} – {}: not available", track.uri, track.name);
            return self.skip().await;
        }
        // The URL may have gone stale; after that, mpv's hook gets to try.
        if attempts > 1 {
            self.unresolvable.insert(video);
        }
        let position_ms = self.position_ms;
        self.current = None;
        self.load_current(position_ms).await
    }

    async fn job(&mut self, job: Job) -> Result<()> {
        match job {
            Job::Resolved { video, result } => {
                self.resolving.remove(&video);
                match result {
                    Ok(stream) => {
                        self.streams.insert(video.clone(), stream);
                    }
                    Err(err) => {
                        tracing::warn!("yt-dlp can't resolve {video}: {err:#}");
                        self.unresolvable.insert(video.clone());
                    }
                }
                let current = self.queue.current().map(|t| video_id(&t.uri) == video).unwrap_or(false);
                if let Some(start_ms) = self.waiting.filter(|_| current) {
                    return self.load_current(start_ms).await;
                }
                self.sync_next().await
            }
            Job::Context { generation, page, last } if generation == self.generation => {
                self.context_page(page, last).await
            }
            Job::Autoplay { generation, tracks } if generation == self.generation => {
                self.autoplay.fetching = false;
                match tracks {
                    Ok(tracks) => self.queue.add_autoplay(tracks),
                    Err(err) => tracing::warn!("radio: {err:#}"),
                }
                if std::mem::take(&mut self.autoplay.waiting) {
                    return self.ended().await;
                }
                self.queue_changed().await
            }
            Job::Queued { uri, track } => {
                match track {
                    Ok(Some(track)) => self.queue.add(track),
                    Ok(None) => tracing::warn!("{uri} can't be queued: not found"),
                    Err(err) => tracing::warn!("{uri} can't be queued: {err:#}"),
                }
                self.queue_changed().await
            }
            Job::Context { .. } | Job::Autoplay { .. } => Ok(()),
        }
    }

    async fn context_page(&mut self, page: Result<Vec<Track>>, last: bool) -> Result<()> {
        let tracks = match page {
            Ok(tracks) => tracks,
            Err(err) => {
                self.context_loaded = true;
                if let Some(starting) = self.starting.take() {
                    self.emit(PlayerEvent::Stopped);
                    bail!("can't play {}: {err:#}", starting.context);
                }
                tracing::warn!("rest of the context: {err:#}");
                return Ok(());
            }
        };
        self.context_loaded = last;
        let Some(starting) = self.starting.as_mut() else {
            self.queue.extend(tracks);
            self.want_autoplay();
            return self.queue_changed().await;
        };
        starting.tracks.extend(tracks);
        let found = starting.track.as_ref().is_none_or(|uri| starting.tracks.iter().any(|t| &t.uri == uri));
        if !found && !last {
            return Ok(());
        }
        let starting = self.starting.take().expect("checked above");
        if !found {
            tracing::warn!(
                "{} isn't in {}; playing it from the start",
                starting.track.as_deref().unwrap_or_default(),
                starting.context
            );
        }
        let start = starting.track.as_deref().filter(|_| found);
        if !self.queue.load(starting.context.clone(), starting.tracks, start, starting.shuffle) {
            self.emit(PlayerEvent::Stopped);
            bail!("{} has nothing to play", starting.context);
        }
        self.begin(if found { starting.position_ms } else { 0 }).await
    }
}

/// Fetches a context's songs page by page for the player.
fn load_context(client: YtMusic, jobs: UnboundedSender<Job>, generation: u64, context: String) -> JoinHandle<()> {
    tokio::spawn(async move {
        let send = |page: Result<Vec<Track>>, last: bool| {
            let _ = jobs.send(Job::Context { generation, page, last });
        };
        let source = if context.starts_with(TRACK) {
            let track = client.tracks_by_uri(std::slice::from_ref(&context)).await;
            let track =
                track.and_then(|t| if t.is_empty() { Err(anyhow!("This song isn't available")) } else { Ok(t) });
            return send(track, true);
        } else if context.starts_with(ALBUM) {
            Source::Album(context.clone())
        } else if context.starts_with(PLAYLIST) {
            Source::Playlist(context.clone())
        } else if context.starts_with(ARTIST) {
            match pages::artist_songs(client.api(), &context).await {
                Ok((_, Some(playlist))) => Source::Playlist(format!("{PLAYLIST}{playlist}")),
                Ok((artist, None)) => return send(Ok(artist.top_tracks.into_iter().map(|t| t.track).collect()), true),
                Err(err) => return send(Err(err), true),
            }
        } else {
            return send(Err(anyhow!("YouTube Music can't play {context}")), true);
        };
        let mut pages = std::pin::pin!(client.tracks(source));
        while let Some(page) = pages.next().await {
            match page {
                Ok(page) => {
                    let last = page.next_offset.is_none();
                    send(Ok(page.items), last);
                    if last {
                        return;
                    }
                }
                Err(err) => return send(Err(err), true),
            }
        }
        send(Ok(Vec::new()), true);
    })
}

fn video_id(uri: &str) -> &str {
    uri.strip_prefix(TRACK).unwrap_or(uri)
}

/// What the player bar shows of a song.
fn current_track(track: &Track) -> CurrentTrack {
    CurrentTrack {
        uri: track.uri.clone(),
        name: track.name.clone(),
        artists: track.artists.clone(),
        artist_uris: track.artist_uris.clone(),
        album: track.album.clone(),
        album_uri: Some(track.album_uri.clone()).filter(|uri| !uri.is_empty()),
        duration_ms: track.duration_ms,
        cover_url: track.image_url.clone(),
        cover_url_large: track.image_url.as_deref().map(parse::large),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_librespots_volume_to_mpvs() {
        assert_eq!(mpv_volume(0), 0.0);
        assert_eq!(mpv_volume(u16::MAX), 100.0);
        assert_eq!(mpv_volume(u16::MAX / 2), 50.0);
    }

    #[test]
    fn the_player_bar_gets_a_large_cover_too() {
        let track = Track {
            uri: "ytmusic:track:x".into(),
            name: "Song".into(),
            artists: vec!["A".into()],
            artist_uris: Vec::new(),
            album: String::new(),
            album_uri: String::new(),
            duration_ms: 1,
            image_url: Some("https://lh3.googleusercontent.com/a=w300-h300-l90-rj".into()),
        };
        let current = current_track(&track);
        assert_eq!(current.album_uri, None);
        assert_eq!(current.cover_url_large.as_deref(), Some("https://lh3.googleusercontent.com/a=w800-h800-l90-rj"));
    }
}
