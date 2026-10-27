//! cloofy's own Spotify Connect device: librespot's player, connected in the background and
//! connected again whenever the connection drops.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Result;
use cloofy_core::{paths::Paths, service::Service};
use cloofy_player::{Engine, events};
use cloofy_spotify::session;
use futures::{FutureExt, StreamExt, future::BoxFuture};
use librespot::{
    connect::{LoadContextOptions, LoadRequest, LoadRequestOptions, Options, PlayingTrack, Spirc},
    core::{SpotifyUri, dealer::protocol::Message, session::Session},
    protocol::connect::ClusterUpdate,
};
use tokio::sync::{
    Notify,
    mpsc::{UnboundedReceiver, UnboundedSender},
    watch,
};

use super::queue;
use crate::{
    app::AppEvent,
    backend::Player,
    playback::{ConnectPush, Repeat},
};

const DEVICE_NAME: &str = "cloofy";
/// A connection that lasted this long was fine: the next one is tried right away.
const STABLE: Duration = Duration::from_secs(30);
/// Waits between attempts while Spotify can't be reached or keeps dropping the connection,
/// doubling up to the longest.
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_LONGEST: Duration = Duration::from_secs(60);

/// The librespot session as the catalog sees it.
#[derive(Clone)]
pub enum SessionState {
    Connecting,
    Ready(Session),
    /// Connecting failed; requests that need the session fail too instead of waiting forever.
    Failed(String),
}

/// cloofy's own Spotify Connect device. Commands only take effect while it is the active device.
pub struct Controller {
    spirc: Spirc,
    session: Session,
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

    /// Empties the queue; the rest of the context stays.
    pub fn clear_queue(&self) -> Result<()> {
        Ok(self.spirc.clear_queue()?)
    }

    /// Adds a track after the ones queued before.
    pub fn add_to_queue(&self, uri: &str) -> Result<()> {
        Ok(self.spirc.add_to_queue(SpotifyUri::from_uri(uri)?)?)
    }

    /// Jumps DJ to its next segment of picks, as the DJ button does in Spotify's apps.
    pub fn next_segment(&self) -> Result<()> {
        Ok(self.spirc.next_segment()?)
    }

    /// Asks DJ for something. The future resolves once DJ took the request; its answer
    /// plays seconds later.
    pub fn ask_dj(&self, prompt: String, suggested: bool) -> Result<BoxFuture<'static, Result<()>>> {
        let answer = self.spirc.ask_dj(prompt, suggested)?;
        Ok(async move { Ok(answer.await?) }.boxed())
    }

    pub fn set_crossfade(&self, crossfade: Duration) {
        self.engine.player.set_crossfade(crossfade);
    }

    /// cloofy's Spotify Connect device ID.
    pub fn device_id(&self) -> String {
        self.session.device_id().to_owned()
    }

    /// The username the session is logged in with.
    pub fn user(&self) -> String {
        self.session.username()
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
/// Playback starts at `volume`, and keeps its volume when connecting again. `retry_now` cuts a
/// wait between attempts short, for when the computer wakes up. `state` follows the session,
/// for the catalog's requests that go through it.
pub fn spawn(
    paths: Paths,
    crossfade: Duration,
    volume: u16,
    retry_now: Arc<Notify>,
    tx: UnboundedSender<AppEvent>,
    state: Arc<watch::Sender<SessionState>>,
) {
    let failed = {
        let (tx, state) = (tx.clone(), state.clone());
        move |err: String| {
            state.send_replace(SessionState::Failed(err.clone()));
            tx.send(AppEvent::PlayerFailed(Service::Spotify, err)).is_ok()
        }
    };
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
                failed(format!("{err:#}"));
                return;
            }
        };
        let mut retry = RETRY_FIRST;
        let mut volume = volume;
        loop {
            let connection = pending.session.clone();
            let wait = match engine.connect_device(pending, DEVICE_NAME.into(), volume).await {
                Ok((spirc, task)) => {
                    if let Err(err) = session::remember(&paths, &connection) {
                        tracing::warn!("saving streaming credentials: {err:#}");
                    }
                    // Before the task starts the dealer: while it starts, it takes no subscriptions.
                    forward_pushes(&connection, tx.clone());
                    let stopping = Arc::new(AtomicBool::new(false));
                    let forced = Arc::new(AtomicBool::new(false));
                    let controller = Controller {
                        spirc,
                        session: connection.clone(),
                        engine: engine.clone(),
                        stopping: stopping.clone(),
                        forced: forced.clone(),
                    };
                    state.send_replace(SessionState::Ready(connection.clone()));
                    if tx.send(AppEvent::PlayerReady(Box::new(Player::Spotify(controller)))).is_err() {
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
                    if !connection.is_invalid() {
                        connection.shutdown();
                    }
                    tracing::warn!("connection to Spotify lost, connecting again");
                    state.send_replace(SessionState::Connecting);
                    if tx.send(AppEvent::PlayerLost(Service::Spotify)).is_err() {
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
                    if !failed(format!("{err:#}")) {
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
                    failed(format!("{err:#}"));
                    return;
                }
            };
            engine.player.set_session(pending.session.clone());
            volume = engine.volume();
        }
    });
}

/// Passes the player's events, as cloofy's own, and lost outputs on to the app.
fn forward_events(engine: &Engine, mut lost_outputs: UnboundedReceiver<String>, tx: &UnboundedSender<AppEvent>) {
    let mut librespot_events = engine.player.get_player_event_channel();
    let player_tx = tx.clone();
    tokio::spawn(async move {
        while let Some(event) = librespot_events.recv().await {
            let Some(event) = events::neutral(event) else { continue };
            if player_tx.send(AppEvent::Player(Service::Spotify, event)).is_err() {
                break;
            }
        }
    });
    let output_tx = tx.clone();
    tokio::spawn(async move {
        while let Some(name) = lost_outputs.recv().await {
            if output_tx.send(AppEvent::OutputLost(Service::Spotify, name)).is_err() {
                break;
            }
        }
    });
}

/// Spotify pushes every change of what plays where to all of the user's devices, cloofy
/// included. The details still come from a poll; the push says when one is worth it. What
/// plays next comes straight from the push.
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
                queue: queue::from_player_state(state),
            };
            if tx.send(AppEvent::Pushed(Service::Spotify, push)).is_err() {
                break;
            }
        }
    });
}
