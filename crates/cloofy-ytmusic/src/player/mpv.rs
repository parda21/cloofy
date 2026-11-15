//! mpv as a child process, driven over its JSON IPC.
//!
//! The IPC runs over one end of a socket pair that mpv inherits (`--input-ipc-client=fd://N`)
//! instead of a socket file: mpv quits by itself once that connection closes, which happens
//! when cloofy exits in any way, a crash or `kill -9` included. No mpv is left behind playing.
//!
//! Windows has no socket pair to hand over, so there mpv serves its IPC on a named pipe of its
//! own (`--input-ipc-server`). mpv doesn't quit when that pipe's client goes away, so stopping it
//! relies on `kill_on_drop` and [`Mpv::quit`]; the pipe disappears with its handle.

use std::{
    collections::HashMap,
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
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{mpsc::UnboundedSender, oneshot},
};

#[cfg(unix)]
use std::os::{
    fd::{AsRawFd, RawFd},
    unix::net::UnixStream as StdStream,
};
#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(windows)]
use tokio::net::windows::named_pipe::ClientOptions;

/// Waits this long for mpv to answer a command.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// How long mpv may take to serve the pipe it was told to create. Starting is a few
/// milliseconds even with a cold disk; waiting longer only delays the complaint about an mpv
/// that never got there.
#[cfg(windows)]
const PIPE_DEADLINE: Duration = Duration::from_secs(2);
/// Between attempts to open that pipe, for the moment mpv takes to create it.
#[cfg(windows)]
const PIPE_RETRY: Duration = Duration::from_millis(10);
/// Keeps mpv, a console program, from flashing a window of its own.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The read end of mpv's IPC, which [`read`] takes over.
type ReadEnd = Box<dyn AsyncRead + Send + Unpin>;
/// The write end, which [`Mpv`] keeps to send commands.
type WriteEnd = Box<dyn AsyncWrite + Send + Unpin>;

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
    writer: WriteEnd,
    pending: Pending,
    next_id: u64,
    /// `loadfile` takes options as its fourth argument since mpv 0.38, as its third before.
    pub legacy_loadfile: bool,
}

impl Mpv {
    pub async fn spawn(args: &[String], events: Events) -> Result<Mpv> {
        let instance = INSTANCES.fetch_add(1, Ordering::Relaxed) + 1;
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
                // Mixers name the stream by the window title: the song, as `load` sets it,
                // instead of the stream's URL with " - mpv".
                "--title=${media-title}",
                "--prefetch-playlist=yes",
                "--gapless-audio=weak",
                "--keep-open=no",
                "--msg-level=all=error",
            ])
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let ipc = Ipc::new(&mut command, instance)?;
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                bail!("mpv is not installed; YouTube Music plays through it")
            }
            Err(err) => return Err(err).context("Can't start mpv"),
        };
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
        let (reader, writer) = match ipc.connect().await {
            Ok(pair) => pair,
            Err(err) => return Err(start_error(&mut child, &last_error, err).await),
        };
        let pending: Pending = Arc::default();
        tokio::spawn(read(reader, pending.clone(), instance, events));
        let mut mpv = Mpv { instance, child, writer, pending, next_id: 0, legacy_loadfile: false };
        let version = match mpv.get("mpv-version").await {
            Ok(version) => version,
            Err(err) => return Err(start_error(&mut mpv.child, &last_error, err).await),
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
    /// `start` seconds, with `title` for the song. Returns the playlist entry's ID.
    pub async fn load(&mut self, url: &str, append: bool, start: Option<f64>, title: &str) -> Result<u64> {
        let mode = if append { "append" } else { "replace" };
        let options = load_options(start, title);
        let command = if self.legacy_loadfile {
            json!(["loadfile", url, mode, options])
        } else {
            json!(["loadfile", url, mode, -1, options])
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

/// Why mpv failed to start, with the last line it printed before it did.
async fn start_error(child: &mut Child, last_error: &Mutex<Option<String>>, err: anyhow::Error) -> anyhow::Error {
    let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
    let reason = last_error.lock().unwrap().take();
    err.context(reason.unwrap_or_else(|| "mpv didn't start".into()))
}

/// mpv's end of the IPC, which differs by platform: a socket pair cloofy passes as a descriptor,
/// or a named pipe mpv creates once it runs.
#[cfg(unix)]
struct Ipc {
    ours: StdStream,
    /// Never read here, just held open until mpv has inherited it; then dropped, so cloofy keeps
    /// no copy of mpv's end and a gone mpv reads as end of file.
    theirs: StdStream,
}

#[cfg(unix)]
impl Ipc {
    fn new(command: &mut Command, _instance: u64) -> Result<Self> {
        let (ours, theirs) = StdStream::pair().context("Can't create a socket for mpv")?;
        let fd = theirs.as_raw_fd();
        command.arg(format!("--input-ipc-client=fd://{fd}"));
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            command.pre_exec(move || inherit(fd));
        }
        Ok(Self { ours, theirs })
    }

    async fn connect(self) -> Result<(ReadEnd, WriteEnd)> {
        drop(self.theirs);
        self.ours.set_nonblocking(true)?;
        let (reader, writer) = UnixStream::from_std(self.ours)?.into_split();
        Ok((Box::new(reader), Box::new(writer)))
    }
}

/// The pipe mpv serves its IPC on. The process ID keeps cloofy's own mpvs apart, the instance
/// every mpv of this process, so two cloofys or two restarts never meet on one pipe.
#[cfg(windows)]
fn pipe_name(instance: u64) -> String {
    format!(r"\\.\pipe\cloofy-{}-{instance}", std::process::id())
}

#[cfg(windows)]
struct Ipc(String);

#[cfg(windows)]
impl Ipc {
    fn new(command: &mut Command, instance: u64) -> Result<Self> {
        let name = pipe_name(instance);
        command.arg(format!("--input-ipc-server={name}")).creation_flags(CREATE_NO_WINDOW);
        Ok(Self(name))
    }

    /// Opens the pipe mpv creates. It exists only a few milliseconds after mpv starts, so the
    /// first attempt is expected to find nothing.
    async fn connect(self) -> Result<(ReadEnd, WriteEnd)> {
        let deadline = tokio::time::Instant::now() + PIPE_DEADLINE;
        loop {
            match ClientOptions::new().open(&self.0) {
                Ok(stream) => {
                    let (reader, writer) = tokio::io::split(stream);
                    return Ok((Box::new(reader), Box::new(writer)));
                }
                Err(err) if tokio::time::Instant::now() >= deadline => {
                    return Err(err).context("Can't connect to mpv's pipe");
                }
                Err(_) => tokio::time::sleep(PIPE_RETRY).await,
            }
        }
    }
}

/// The options of a `loadfile`: where to start, and the title mixers and the window show. mpv
/// takes `%<bytes>%` before a value as its quoting, so a title may have commas and `=`.
fn load_options(start: Option<f64>, title: &str) -> String {
    let mut options = format!("force-media-title=%{}%{title}", title.len());
    if let Some(start) = start.filter(|s| *s > 0.0) {
        options.push_str(&format!(",start={start:.3}"));
    }
    options
}

/// Runs in the child before mpv starts: keeps mpv's end of the socket open across exec.
#[cfg(unix)]
fn inherit(fd: RawFd) -> std::io::Result<()> {
    // SAFETY: fcntl on a descriptor this process owns.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, 0) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

async fn read(reader: impl AsyncRead + Unpin + Send + 'static, pending: Pending, instance: u64, events: Events) {
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
    fn quotes_the_title_for_mpv() {
        assert_eq!(load_options(None, "Song – Artist"), "force-media-title=%15%Song – Artist");
        assert_eq!(
            load_options(Some(12.5), "A, B=C – D"),
            "force-media-title=%12%A, B=C – D,start=12.500",
            "commas and = stay in the title"
        );
        assert_eq!(load_options(Some(0.0), "x"), "force-media-title=%1%x");
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

    /// Two mpvs must not meet on one pipe, nor must two cloofys.
    #[test]
    #[cfg(windows)]
    fn names_a_pipe_of_its_own() {
        let first = pipe_name(1);
        assert!(first.starts_with(r"\\.\pipe\cloofy-"), "{first}");
        assert!(first.contains(&std::process::id().to_string()), "{first}");
        assert_ne!(pipe_name(1), pipe_name(2));
    }
}
