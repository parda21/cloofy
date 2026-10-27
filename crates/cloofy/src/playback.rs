use std::{
    io::Write,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use cloofy_core::{
    paths::Paths,
    player::{CurrentTrack, PlayerEvent},
};
use cloofy_player::{DEFAULT_VOLUME, Engine, events};
use cloofy_spotify::session;
use librespot::{
    connect::{LoadContextOptions, LoadRequest, LoadRequestOptions, Options, PlayingTrack},
    core::SpotifyUri,
    metadata::{Metadata, Track},
};

use crate::{media::Media, style};

const BAR_WIDTH: usize = 40;
const DEVICE_NAME: &str = "cloofy";

/// Plays standalone: cloofy activates itself as the playing device and loads the context.
pub async fn play(paths: &Paths, input: &str, shuffle: bool) -> Result<()> {
    let uri = parse_uri(input)?;
    let pending = session::prepare(paths)?;
    let session = pending.session.clone();
    let (engine, mut lost_outputs) = Engine::new(&session, Duration::ZERO)?;
    let mut player_events = engine.player.get_player_event_channel();
    let (spirc, task) = engine.connect_device(pending, DEVICE_NAME.into(), DEFAULT_VOLUME).await?;
    session::remember(paths, &session)?;
    tokio::pin!(task);

    let mut options = LoadRequestOptions {
        start_playing: true,
        context_options: Some(LoadContextOptions::Options(Options { shuffle, ..Options::default() })),
        ..LoadRequestOptions::default()
    };
    // Like Spotify itself: a single track plays within its album, so next/previous work.
    let context = match &uri {
        SpotifyUri::Track { .. } => {
            let track = Track::get(&session, &uri).await?;
            options.playing_track = Some(PlayingTrack::Uri(uri.to_uri()));
            track.album.id.to_uri()
        }
        _ => uri.to_uri(),
    };
    spirc.activate()?;
    spirc.load(LoadRequest::from_context_uri(context, options))?;

    let (mut media, mut media_events) = Media::new();
    println!();
    let mut clock = Clock::default();
    let mut duration = Duration::ZERO;
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            () = &mut task => break,
            Some(event) = media_events.recv() => media.on_event(event, &spirc),
            Some(name) = lost_outputs.recv() => {
                spirc.pause()?;
                println!("\r\x1b[2K  {}", style::dim(&format!("❚❚ paused: {name} disconnected")));
            }
            Some(event) = player_events.recv() => {
                let Some(event) = events::neutral(event) else { continue };
                media.on_player(&event);
                match event {
                    PlayerEvent::TrackChanged { track } => {
                        duration = Duration::from_millis(track.duration_ms.into());
                        clock = Clock::default();
                        println!("\r\x1b[2K  ▶  {}", describe(&track));
                    }
                    PlayerEvent::Playing { position_ms } => clock.resume(position_ms),
                    PlayerEvent::Paused { position_ms } => clock.pause(position_ms),
                    _ => {}
                }
            }
            _ = tick.tick(), if !duration.is_zero() => draw_progress(clock.position().min(duration), duration),
            _ = tokio::signal::ctrl_c() => {
                spirc.shutdown()?;
                (&mut task).await;
                break;
            }
        }
    }
    println!("\n");
    Ok(())
}

/// Remote mode: waits until a Spotify app picks cloofy as its playback device.
pub async fn connect(paths: &Paths, name: String) -> Result<()> {
    let pending = session::prepare(paths)?;
    let session = pending.session.clone();
    let (engine, mut lost_outputs) = Engine::new(&pending.session, Duration::ZERO)?;
    let mut player_events = engine.player.get_player_event_channel();
    let (spirc, task) = engine.connect_device(pending, name.clone(), DEFAULT_VOLUME).await?;
    session::remember(paths, &session)?;
    tokio::pin!(task);

    println!(
        "\n  {} {} is online. Pick it as a device in the Spotify app.  {}\n",
        style::green("◉"),
        style::bold(&name),
        style::dim("Ctrl+C quits")
    );
    let (mut media, mut media_events) = Media::new();
    let mut paused = false;
    loop {
        tokio::select! {
            () = &mut task => break,
            Some(event) = media_events.recv() => media.on_event(event, &spirc),
            Some(name) = lost_outputs.recv() => {
                spirc.pause()?;
                paused = true;
                println!("  {}", style::dim(&format!("❚❚ paused: {name} disconnected")));
            }
            Some(event) = player_events.recv() => {
                let Some(event) = events::neutral(event) else { continue };
                media.on_player(&event);
                match event {
                    PlayerEvent::TrackChanged { track } => println!("  ▶  {}", describe(&track)),
                    PlayerEvent::Paused { .. } if !paused => {
                        paused = true;
                        println!("  {}", style::dim("❚❚ paused"));
                    }
                    PlayerEvent::Playing { .. } if paused => {
                        paused = false;
                        println!("  {}", style::dim("▶  resumed"));
                    }
                    _ => {}
                }
            }
            _ = tokio::signal::ctrl_c() => {
                spirc.shutdown()?;
                (&mut task).await;
                break;
            }
        }
    }
    Ok(())
}

/// Accepts `spotify:…` URIs as well as `https://open.spotify.com/…` links.
fn parse_uri(input: &str) -> Result<SpotifyUri> {
    let uri = match input.strip_prefix("https://open.spotify.com/") {
        Some(path) => {
            let mut parts = path.split(['/', '?']).filter(|p| !p.is_empty() && !p.starts_with("intl-"));
            match (parts.next(), parts.next()) {
                (Some(kind), Some(id)) => format!("spotify:{kind}:{id}"),
                _ => bail!("Unknown Spotify link: {input}"),
            }
        }
        None => input.to_owned(),
    };
    SpotifyUri::from_uri(&uri).with_context(|| format!("“{input}” is not a valid Spotify URI"))
}

fn describe(track: &CurrentTrack) -> String {
    format!("{}  {}", style::bold(&track.name), style::dim(&track.artists.join(", ")))
}

fn draw_progress(position: Duration, duration: Duration) {
    let ratio = if duration.is_zero() { 0.0 } else { position.as_secs_f64() / duration.as_secs_f64() };
    let filled = ((ratio * BAR_WIDTH as f64).round() as usize).min(BAR_WIDTH);
    print!(
        "\r\x1b[2K  {}  {}{}{}  {}",
        style::dim(&clock(position)),
        style::green(&"━".repeat(filled)),
        style::green("●"),
        style::dim(&"─".repeat(BAR_WIDTH - filled)),
        style::dim(&clock(duration)),
    );
    let _ = std::io::stdout().flush();
}

fn clock(duration: Duration) -> String {
    let secs = duration.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Playback position, extrapolated between player events.
#[derive(Default)]
struct Clock {
    base_ms: u32,
    since: Option<Instant>,
}

impl Clock {
    fn resume(&mut self, position_ms: u32) {
        self.base_ms = position_ms;
        self.since = Some(Instant::now());
    }

    fn pause(&mut self, position_ms: u32) {
        self.base_ms = position_ms;
        self.since = None;
    }

    fn position(&self) -> Duration {
        Duration::from_millis(self.base_ms.into()) + self.since.map_or(Duration::ZERO, |s| s.elapsed())
    }
}
