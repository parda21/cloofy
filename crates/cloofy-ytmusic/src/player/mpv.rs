//! mpv as a child process, driven over its JSON IPC.
//!
//! The IPC runs over one end of a socket pair that mpv inherits (`--input-ipc-client=fd://N`)
//! instead of a socket file: mpv quits by itself once that connection closes, which happens
//! when cloofy exits in any way, a crash or `kill -9` included. No mpv is left behind playing.

use std::{
    collections::HashMap,
    os::{
        fd::{AsRawFd, RawFd},
        unix::net::UnixStream as StdStream,
    },
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixStream, unix::OwnedWriteHalf},
    process::{Child, Command},
    sync::{mpsc::UnboundedSender, oneshot},
};

/// Waits this long for mpv to answer a command.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// Properties observed, by the ID mpv reports them with.
pub(crate) const TIME_POS: u64 = 1;
pub(crate) const PAUSE: u64 = 2;
pub(crate) const DURATION: u64 = 3;

/// Counts the mpvs started, so the events of one that is gone can be told apart.
static INSTANCES: AtomicU64 = AtomicU64::new(0);

/// Events come with the instance of the mpv that sent them.
pub(crate) type Events = UnboundedSender<(u64, MpvEvent)>;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MpvEvent {
    /// An observed property changed.
    Property { id: u64, data: Value },
    /// A playlist entry starts loading.
    StartFile { entry: u64 },
    /// An entry stopped: `eof`, `stop`, `quit`, `error` or `redirect`.
    EndFile { entry: u64, reason: String, error: Option<String> },
    /// Playback (re)started after loading or seeking.
    PlaybackRestart,
    /// mpv is gone.
    Exited,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>;

pub(crate) struct Mpv {
    /// Which mpv this is, as its events say.
    pub instance: u64,
    child: Child,
    writer: OwnedWriteHalf,
    pending: Pending,
    next_id: u64,
    /// `loadfile` takes options as its fourth argument since mpv 0.38, as its third before.
    pub legacy_loadfile: bool,
}

impl Mpv {
    pub async fn spawn(args: &[String], events: Events) -> Result<Mpv> {
        let instance = INSTANCES.fetch_add(1, Ordering::Relaxed) + 1;
        let (ours, theirs) = StdStream::pair().context("Can't create a socket for mpv")?;
        let fd = theirs.as_raw_fd();
        let mut command = Command::new("mpv");
        command
            .args([
                "--idle=yes",
                "--no-video",
                // Neither the user's mpv.conf nor scripts like mpv-mpris, which would add a second
                // media player to the system's controls.
                "--no-config",
                "--load-scripts=no",
                // Streams come resolved by yt-dlp. mpv's own yt-dlp hook only plays `ytdl://`
                // URLs, as a fallback; it can't be turned on later, so it is on but kept away
                // from the resolved URLs.
                "--ytdl=yes",
                "--ytdl-format=bestaudio/best",
                // A Lua pattern; mpv's option parser would take `%` for its own quoting.
                "--script-opts=ytdl_hook-exclude=googlevideo",
                "--no-input-terminal",
                "--audio-display=no",
                "--audio-client-name=cloofy",
                "--prefetch-playlist=yes",
                "--gapless-audio=weak",
                "--keep-open=no",
                "--msg-level=all=error",
            ])
            .arg(format!("--input-ipc-client=fd://{fd}"))
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            command.pre_exec(move || inherit(fd));
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                bail!("mpv is not installed; YouTube Music plays through it")
            }
            Err(err) => return Err(err).context("Can't start mpv"),
        };
        drop(theirs);
        // mpv only prints errors; the last one explains a failed start.
        let last_error: Arc<Mutex<Option<String>>> = Arc::default();
        if let Some(stderr) = child.stderr.take() {
            let last_error = last_error.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::warn!("mpv: {line}");
                    *last_error.lock().unwrap() = Some(line);
                }
            });
        }
        ours.set_nonblocking(true)?;
        let (reader, writer) = UnixStream::from_std(ours)?.into_split();
        let pending: Pending = Arc::default();
        tokio::spawn(read(reader, pending.clone(), instance, events));
        let mut mpv = Mpv { instance, child, writer, pending, next_id: 0, legacy_loadfile: false };
        let version = match mpv.get("mpv-version").await {
            Ok(version) => version,
            Err(err) => {
                let _ = tokio::time::timeout(Duration::from_secs(1), mpv.child.wait()).await;
                let reason = last_error.lock().unwrap().take();
                return Err(err.context(reason.unwrap_or_else(|| "mpv didn't start".into())));
            }
        };
        mpv.legacy_loadfile = version.as_str().is_some_and(older_than_0_38);
        for (id, name) in [(TIME_POS, "time-pos"), (PAUSE, "pause"), (DURATION, "duration")] {
            mpv.command(json!(["observe_property", id, name])).await?;
        }
        Ok(mpv)
    }

    /// Sends a command and waits for its answer.
    pub async fn command(&mut self, command: Value) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let mut line = serde_json::to_vec(&json!({ "command": command, "request_id": id }))?;
        line.push(b'\n');
        if let Err(err) = self.writer.write_all(&line).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(err).context("mpv is gone");
        }
        match tokio::time::timeout(REPLY_TIMEOUT, rx).await {
            Ok(Ok(result)) => result.with_context(|| format!("mpv: {command}")),
            Ok(Err(_)) => bail!("mpv is gone"),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                bail!("mpv didn't answer {command}")
            }
        }
    }

    pub async fn get(&mut self, name: &str) -> Result<Value> {
        self.command(json!(["get_property", name])).await
    }

    pub async fn set(&mut self, name: &str, value: Value) -> Result<()> {
        self.command(json!(["set_property", name, value])).await.map(drop)
    }

    /// Loads a URL, replacing what plays or appended to the playlist, maybe starting at
    /// `start` seconds. Returns the playlist entry's ID.
    pub async fn load(&mut self, url: &str, append: bool, start: Option<f64>) -> Result<u64> {
        let mode = if append { "append" } else { "replace" };
        let options = start.filter(|s| *s > 0.0).map(|s| format!("start={s:.3}")).unwrap_or_default();
        let command = match (self.legacy_loadfile, options.is_empty()) {
            (_, true) => json!(["loadfile", url, mode]),
            (true, false) => json!(["loadfile", url, mode, options]),
            (false, false) => json!(["loadfile", url, mode, -1, options]),
        };
        let reply = self.command(command).await?;
        reply["playlist_entry_id"].as_u64().context("mpv didn't say which entry it loaded")
    }

    /// Asks mpv to quit, and makes sure it does.
    pub async fn quit(mut self) {
        let _ = tokio::time::timeout(Duration::from_millis(500), self.command(json!(["quit"]))).await;
        if tokio::time::timeout(Duration::from_secs(1), self.child.wait()).await.is_err() {
            let _ = self.child.kill().await;
        }
    }
}

/// Runs in the child before mpv starts: keeps mpv's end of the socket open across exec.
fn inherit(fd: RawFd) -> std::io::Result<()> {
    // SAFETY: fcntl on a descriptor this process owns.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, 0) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

async fn read(reader: tokio::net::unix::OwnedReadHalf, pending: Pending, instance: u64, events: Events) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(id) = message["request_id"].as_u64().filter(|_| message.get("event").is_none()) {
            if let Some(reply) = pending.lock().unwrap().remove(&id) {
                let result = match message["error"].as_str() {
                    Some("success") => Ok(message.get("data").cloned().unwrap_or(Value::Null)),
                    error => Err(anyhow::anyhow!("{}", error.unwrap_or("no answer"))),
                };
                let _ = reply.send(result);
            }
            continue;
        }
        if let Some(event) = parse_event(&message) {
            let _ = events.send((instance, event));
        }
    }
    pending.lock().unwrap().clear();
    let _ = events.send((instance, MpvEvent::Exited));
}

fn parse_event(message: &Value) -> Option<MpvEvent> {
    let entry = message["playlist_entry_id"].as_u64().unwrap_or(0);
    Some(match message["event"].as_str()? {
        "property-change" => MpvEvent::Property {
            id: message["id"].as_u64()?,
            data: message.get("data").cloned().unwrap_or(Value::Null),
        },
        "start-file" => MpvEvent::StartFile { entry },
        "end-file" => MpvEvent::EndFile {
            entry,
            reason: message["reason"].as_str().unwrap_or_default().to_owned(),
            error: message["file_error"].as_str().map(str::to_owned),
        },
        "playback-restart" => MpvEvent::PlaybackRestart,
        _ => return None,
    })
}

/// Whether an `mpv-version` such as `mpv v0.37.0` or `mpv 0.35.1` is older than 0.38.
fn older_than_0_38(version: &str) -> bool {
    let number = version.split_whitespace().nth(1).unwrap_or_default().trim_start_matches('v');
    let mut parts = number.split(['.', '-', '+']).map(|p| p.parse::<u32>().ok());
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(major), Some(minor)) => (major, minor) < (0, 38),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_which_mpv_takes_options_where() {
        assert!(older_than_0_38("mpv 0.35.1"));
        assert!(older_than_0_38("mpv v0.37.0-dirty"));
        assert!(!older_than_0_38("mpv v0.41.0"));
        assert!(!older_than_0_38("mpv v0.38.0-123-gabcdef"));
        assert!(!older_than_0_38("mpv git-2026"), "builds from git are new");
    }

    #[test]
    fn parses_mpv_events() {
        let event =
            json!({ "event": "end-file", "reason": "error", "playlist_entry_id": 3, "file_error": "loading failed" });
        assert_eq!(
            parse_event(&event),
            Some(MpvEvent::EndFile { entry: 3, reason: "error".into(), error: Some("loading failed".into()) })
        );
        let event = json!({ "event": "property-change", "id": 1, "name": "time-pos", "data": 1.5 });
        assert_eq!(parse_event(&event), Some(MpvEvent::Property { id: 1, data: json!(1.5) }));
        assert_eq!(parse_event(&json!({ "event": "audio-reconfig" })), None);
    }
}
