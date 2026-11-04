//! `cloofy play` for YouTube Music: mpv plays, cloofy's queue goes on, and the system's media
//! controls follow.

use std::time::Duration;

use anyhow::{Result, anyhow};
use cloofy_core::{paths::Paths, player::PlayerEvent};
use cloofy_os::media::Command;
use cloofy_ytmusic::{Player, YtMusic, playable};
use tokio::sync::mpsc;

use crate::{
    headless::{Clock, describe, draw_progress},
    media::Media,
};

/// Half, as Spotify's `cloofy play` starts.
const VOLUME: u16 = u16::MAX / 2;

/// Plays a song with its radio, or a playlist, album or artist, until Ctrl+C or the end.
pub async fn play(paths: &Paths, input: &str, shuffle: bool) -> Result<()> {
    let playable = playable(input)?;
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let player = Player::spawn(YtMusic::new(paths)?, paths, VOLUME, events_tx).await?;
    if shuffle {
        player.play_shuffled(playable.context)?;
    } else {
        player.play(playable.context, playable.track)?;
    }

    let (mut media, mut media_events) = Media::new();
    println!();
    let mut clock = Clock::default();
    let mut duration = Duration::ZERO;
    let (mut paused, mut played) = (false, false);
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let result = loop {
        tokio::select! {
            Some(event) = media_events.recv() => media.on_event(event, |command| control(&player, command, paused)),
            Some(event) = events.recv() => {
                media.on_player(&event);
                match event {
                    PlayerEvent::TrackChanged { track } => {
                        played = true;
                        duration = Duration::from_millis(track.duration_ms.into());
                        clock = Clock::default();
                        println!("\r\x1b[2K  ▶  {}", describe(&track));
                    }
                    PlayerEvent::Playing { position_ms } => {
                        paused = false;
                        clock.resume(position_ms);
                    }
                    PlayerEvent::Paused { position_ms } => {
                        paused = true;
                        clock.pause(position_ms);
                    }
                    PlayerEvent::Seeked { position_ms } | PlayerEvent::PositionChanged { position_ms } if paused => {
                        clock.pause(position_ms);
                    }
                    PlayerEvent::Seeked { position_ms } | PlayerEvent::PositionChanged { position_ms } => {
                        clock.resume(position_ms);
                    }
                    // Nothing left to play, or nothing would.
                    PlayerEvent::Stopped if played => break Ok(()),
                    PlayerEvent::Stopped => break Err(anyhow!("YouTube Music can't play {input}")),
                    _ => {}
                }
            }
            _ = tick.tick(), if !duration.is_zero() => draw_progress(clock.position().min(duration), duration),
            _ = tokio::signal::ctrl_c() => break Ok(()),
        }
    };
    player.shutdown();
    println!("\n");
    result
}

/// Hands a request from the system's media controls to the player.
fn control(player: &Player, command: Command, paused: bool) -> Result<()> {
    match command {
        Command::Play if paused => player.toggle(true),
        Command::Pause if !paused => player.pause(),
        Command::Play | Command::Pause => Ok(()),
        Command::Toggle => player.toggle(true),
        Command::Next => player.next(),
        Command::Previous => player.previous(),
        Command::Seek(position) => player.seek(position.as_millis().try_into().unwrap_or(u32::MAX)),
        Command::Volume(volume) => player.set_volume(volume),
    }
}
