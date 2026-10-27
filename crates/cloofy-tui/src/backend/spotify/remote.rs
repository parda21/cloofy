//! Commands for another Spotify Connect device, through the Web API.

use std::sync::Arc;

use cloofy_spotify::{connect, webapi::WebApi};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::{app::AppEvent, backend::Remote, loader::DataEvent, playback::Repeat};

/// Spotify's form of a command.
pub fn command(remote: Remote) -> connect::Command {
    match remote {
        Remote::Play => connect::Command::Play,
        Remote::Pause => connect::Command::Pause,
        Remote::Next => connect::Command::Next,
        Remote::Previous => connect::Command::Previous,
        Remote::Seek(position_ms) => connect::Command::Seek(position_ms),
        Remote::Volume(percent) => connect::Command::Volume(percent),
        Remote::Shuffle(on) => connect::Command::Shuffle(on),
        Remote::Repeat(mode) => connect::Command::Repeat(repeat_name(mode)),
        Remote::PlayContext { context, track, shuffle } => connect::Command::PlayContext { context, track, shuffle },
    }
}

/// The Web API's "off", "context" or "track".
fn repeat_name(mode: Repeat) -> &'static str {
    match mode {
        Repeat::Off => "off",
        Repeat::Context => "context",
        Repeat::Track => "track",
    }
}

/// Sends remote commands one at a time, so they reach Spotify in the order they were given.
/// Volume and seek commands that pile up meanwhile collapse into the newest one.
pub async fn send_remote(
    api: Arc<WebApi>,
    tx: UnboundedSender<AppEvent>,
    mut commands: mpsc::UnboundedReceiver<connect::Command>,
) {
    let mut waiting = None;
    while let Some((command, count)) = next_remote(&mut commands, &mut waiting).await {
        let result = connect::send(&api, command).await.map_err(|e| format!("{e:#}"));
        if tx.send(AppEvent::Data(DataEvent::RemoteDone { commands: count, result })).is_err() {
            return;
        }
    }
}

/// Waits for the next command to send and collapses the ones queued behind it that it makes
/// pointless. Returns it with the number of commands it stands for.
async fn next_remote(
    commands: &mut mpsc::UnboundedReceiver<connect::Command>,
    waiting: &mut Option<connect::Command>,
) -> Option<(connect::Command, usize)> {
    let mut command = match waiting.take() {
        Some(command) => command,
        None => commands.recv().await?,
    };
    let mut count = 1;
    while let Ok(newer) = commands.try_recv() {
        if !supersedes(&newer, &command) {
            *waiting = Some(newer);
            break;
        }
        command = newer;
        count += 1;
    }
    Some((command, count))
}

/// Whether `newer` makes `older` pointless: only the last volume or position counts.
fn supersedes(newer: &connect::Command, older: &connect::Command) -> bool {
    use connect::Command::{Seek, Volume};
    matches!((newer, older), (Volume(_), Volume(_)) | (Seek(_), Seek(_)))
}

#[cfg(test)]
mod tests {
    use connect::Command::{Next, Play, Seek, Volume};

    use super::*;

    #[tokio::test]
    async fn queued_volume_and_seek_commands_collapse_in_order() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        for command in [Volume(10), Volume(20), Seek(5), Seek(6), Play, Volume(30), Next] {
            tx.send(command).unwrap();
        }
        drop(tx);
        let mut waiting = None;
        let mut sent = Vec::new();
        while let Some((command, count)) = next_remote(&mut rx, &mut waiting).await {
            sent.push(format!("{command:?}×{count}"));
        }
        assert_eq!(sent, ["Volume(20)×2", "Seek(6)×2", "Play×1", "Volume(30)×1", "Next×1"]);
    }
}
