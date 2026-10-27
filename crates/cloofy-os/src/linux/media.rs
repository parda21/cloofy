//! The media controls as MPRIS, the D-Bus interface through which media keys, the desktop's
//! media widgets and playerctl find and control players.

use std::{
    borrow::Cow,
    collections::HashMap,
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    process,
    sync::{Arc, Mutex, PoisonError},
    thread,
    time::{Duration, Instant},
};

use tokio::sync::watch;
use zbus::{
    Connection, connection,
    fdo::{self, RequestNameFlags},
    interface,
    names::InterfaceName,
    object_server::SignalEmitter,
    zvariant::{ObjectPath, OwnedObjectPath, Value},
};

use super::bus;
use crate::media::{Command, State, Track};

/// The name clients look for. A second cloofy adds its process ID, as MPRIS asks.
const NAME: &str = "org.mpris.MediaPlayer2.cloofy";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
/// How far the position may move other than by playing before clients hear of a seek.
const SEEK_TOLERANCE: Duration = Duration::from_secs(1);

type Callback = Arc<dyn Fn(Command) + Send + Sync>;

pub struct Controls {
    updates: watch::Sender<State>,
    /// The cover file clients were pointed to last.
    cover: Arc<Mutex<Option<PathBuf>>>,
}

impl Controls {
    pub fn new(on_command: Callback) -> Option<Self> {
        let connection = bus::run(connect(on_command))?;
        let (updates, states) = watch::channel(State::default());
        let cover = Arc::new(Mutex::new(None));
        bus::spawn(serve(connection, states, CoverFile::new(cover.clone(), cover_dir())));
        Some(Self { updates, cover })
    }

    pub fn update(&self, state: State) {
        // Only the latest state matters, so this never waits for the D-Bus thread.
        self.updates.send_replace(state);
    }
}

impl Drop for Controls {
    /// Removes the cover file right away: the D-Bus thread may not get to it before cloofy exits.
    fn drop(&mut self) {
        if let Some(path) = self.cover.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = fs::remove_file(path);
        }
    }
}

/// Puts cloofy on the session bus with a connection of its own, which takes it off again
/// when dropped.
async fn connect(on_command: Callback) -> Option<Connection> {
    let connected = async {
        let connection = connection::Builder::session()?
            .serve_at(PATH, Root { can_raise: env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() })?
            .serve_at(PATH, Player { on_command, shown: Shown::new(Instant::now()) })?
            .build()
            .await?;
        let flags = RequestNameFlags::DoNotQueue.into();
        match connection.request_name_with_flags(NAME, flags).await {
            Ok(_) => {}
            Err(zbus::Error::NameTaken) => {
                let instance = format!("{NAME}.instance{}", process::id());
                connection.request_name_with_flags(instance.as_str(), flags).await?;
            }
            Err(err) => return Err(err),
        }
        zbus::Result::Ok(connection)
    };
    connected.await.inspect_err(|err| tracing::info!("no media controls: {err}")).ok()
}

/// Shows each new state until the controls are dropped.
async fn serve(connection: Connection, mut states: watch::Receiver<State>, mut cover: CoverFile) {
    let player = match connection.object_server().interface::<_, Player>(PATH).await {
        Ok(player) => player,
        Err(err) => return tracing::warn!("media controls: {err}"),
    };
    while states.changed().await.is_ok() {
        let state = states.borrow_and_update().clone();
        let art_url = cover.url(state.track.as_ref().and_then(|track| track.artwork.as_ref()));
        let changes = player.get_mut().await.shown.show(state, art_url, Instant::now());
        if let Err(err) = changes.announce(player.signal_emitter()).await {
            tracing::warn!("media controls: {err}");
        }
    }
}

/// `org.mpris.MediaPlayer2`: cloofy as an application. It lives in a terminal, which only
/// Hyprland lets it bring to the front.
struct Root {
    can_raise: bool,
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {
        if self.can_raise {
            // hyprctl takes a moment, which the bus shouldn't wait for.
            let _ = thread::Builder::new().name("cloofy-raise".into()).spawn(raise_window);
        }
    }

    fn quit(&self) {}

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_raise(&self) -> bool {
        self.can_raise
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn identity(&self) -> &str {
        "cloofy"
    }

    /// Lets clients find cloofy's name and icon in `cloofy.desktop`.
    #[zbus(property(emits_changed_signal = "const"))]
    fn desktop_entry(&self) -> &str {
        "cloofy"
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

/// `org.mpris.MediaPlayer2.Player`: what plays, and the requests for it. Without a track every
/// request is ignored, as MPRIS has it for players that can't play.
struct Player {
    on_command: Callback,
    shown: Shown,
}

impl Player {
    fn request(&self, command: Option<Command>) {
        if let Some(command) = command.filter(|_| self.shown.track.is_some()) {
            (self.on_command)(command);
        }
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn next(&self) {
        self.request(Some(Command::Next));
    }

    fn previous(&self) {
        self.request(Some(Command::Previous));
    }

    fn pause(&self) {
        self.request(Some(Command::Pause));
    }

    fn play_pause(&self) {
        self.request(Some(Command::Toggle));
    }

    /// cloofy can't stop, only pause, which keeps the track.
    fn stop(&self) {
        self.request(Some(Command::Pause));
    }

    fn play(&self) {
        self.request(Some(Command::Play));
    }

    fn seek(&self, offset: i64) {
        self.request(self.shown.seek(offset, Instant::now()));
    }

    fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
        self.request(self.shown.set_position(&track_id, position));
    }

    fn open_uri(&self, _uri: &str) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported("cloofy opens no URIs".into()))
    }

    /// Tells clients that the position jumped, since it changes without PropertiesChanged.
    #[zbus(signal)]
    async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> &str {
        self.shown.status()
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<&str, Value<'_>> {
        self.shown.metadata()
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        micros(self.shown.position(Instant::now()))
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        fraction(self.shown.volume)
    }

    #[zbus(property)]
    fn set_volume(&mut self, volume: f64) {
        // MPRIS allows more than 1 to amplify, which cloofy doesn't.
        let volume = (volume.clamp(0.0, 1.0) * f64::from(u16::MAX)).round() as u16;
        self.request(Some(Command::Volume(volume)));
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.shown.track.is_some()
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.shown.track.is_some()
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.shown.track.is_some()
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.shown.track.is_some()
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.shown.track.is_some()
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_control(&self) -> bool {
        true
    }
}

/// The player properties that follow whether there is a track.
const CAPABILITIES: [&str; 5] = ["CanGoNext", "CanGoPrevious", "CanPlay", "CanPause", "CanSeek"];

/// What the controls show, the way MPRIS describes it.
#[derive(Debug)]
struct Shown {
    track: Option<Current>,
    playing: bool,
    /// Where playback was at `at`.
    position: Duration,
    at: Instant,
    volume: u16,
    /// Tracks shown so far, which numbers their IDs.
    tracks: u64,
}

/// The track shown, with what MPRIS adds to it.
#[derive(Debug)]
struct Current {
    track: Track,
    id: OwnedObjectPath,
    art_url: Option<String>,
}

/// What an update changed, for the signals that tell clients.
#[derive(Debug, Default, PartialEq)]
struct Changes {
    properties: HashMap<&'static str, Value<'static>>,
    /// The new position, when it jumped.
    seeked: Option<Duration>,
}

impl Shown {
    fn new(now: Instant) -> Self {
        Self { track: None, playing: false, position: Duration::ZERO, at: now, volume: 0, tracks: 0 }
    }

    fn status(&self) -> &'static str {
        match &self.track {
            None => "Stopped",
            Some(_) if self.playing => "Playing",
            Some(_) => "Paused",
        }
    }

    fn metadata(&self) -> HashMap<&'static str, Value<'static>> {
        let Some(Current { track, id, art_url }) = &self.track else { return HashMap::new() };
        let mut metadata = HashMap::from([
            ("mpris:trackid", Value::from(id.clone())),
            ("mpris:length", Value::from(micros(track.duration))),
            ("xesam:title", Value::from(track.title.clone())),
            // One entry: the names come joined, and some contain commas themselves.
            ("xesam:artist", Value::from(vec![track.artist.clone()])),
        ]);
        if !track.album.is_empty() {
            metadata.insert("xesam:album", Value::from(track.album.clone()));
        }
        if let Some(url) = art_url {
            metadata.insert("mpris:artUrl", Value::from(url.clone()));
        }
        if let Some(link) = &track.link {
            metadata.insert("xesam:url", Value::from(link.clone()));
        }
        metadata
    }

    /// Where playback is by now; it moves on while playing, up to the end of the track.
    fn position(&self, now: Instant) -> Duration {
        let Some(current) = &self.track else { return Duration::ZERO };
        let moved = if self.playing { now.saturating_duration_since(self.at) } else { Duration::ZERO };
        (self.position + moved).min(current.track.duration)
    }

    /// Shows `state`, and returns what clients need to hear about it.
    fn show(&mut self, state: State, art_url: Option<String>, now: Instant) -> Changes {
        let (status, metadata, volume) = (self.status(), self.metadata(), self.volume);
        let had_track = self.track.is_some();
        let expected = self.position(now);
        let same = match (&self.track, &state.track) {
            (Some(current), Some(track)) => same_track(&current.track, track).then(|| current.id.clone()),
            _ => None,
        };
        let seeked = same.is_some() && state.position.abs_diff(expected) >= SEEK_TOLERANCE;
        self.track = state.track.map(|track| {
            let id = same.unwrap_or_else(|| {
                self.tracks += 1;
                track_id(self.tracks)
            });
            Current { track, id, art_url }
        });
        self.playing = state.playing;
        self.position = state.position;
        self.at = now;
        self.volume = state.volume;

        let mut changes = Changes { seeked: seeked.then_some(state.position), ..Changes::default() };
        if self.status() != status {
            changes.properties.insert("PlaybackStatus", Value::from(self.status()));
        }
        let new_metadata = self.metadata();
        if new_metadata != metadata {
            changes.properties.insert("Metadata", Value::from(new_metadata));
        }
        if self.volume != volume {
            changes.properties.insert("Volume", Value::from(fraction(self.volume)));
        }
        if self.track.is_some() != had_track {
            let can = self.track.is_some();
            changes.properties.extend(CAPABILITIES.map(|name| (name, Value::from(can))));
        }
        changes
    }

    /// The request for `Seek`, which moves by `offset` microseconds. Past the end it skips, as
    /// MPRIS asks.
    fn seek(&self, offset: i64, now: Instant) -> Option<Command> {
        let current = self.track.as_ref()?;
        let target = micros(self.position(now)).saturating_add(offset).max(0);
        if target > micros(current.track.duration) {
            return Some(Command::Next);
        }
        Some(Command::Seek(Duration::from_micros(target.unsigned_abs())))
    }

    /// The request for `SetPosition`, which only applies to the track it names.
    fn set_position(&self, track_id: &ObjectPath<'_>, position: i64) -> Option<Command> {
        let current = self.track.as_ref().filter(|current| current.id.as_str() == track_id.as_str())?;
        let position = u64::try_from(position).ok().map(Duration::from_micros)?;
        (position <= current.track.duration).then_some(Command::Seek(position))
    }
}

impl Changes {
    async fn announce(self, emitter: &SignalEmitter<'_>) -> zbus::Result<()> {
        if !self.properties.is_empty() {
            let interface = InterfaceName::from_static_str_unchecked(PLAYER);
            fdo::Properties::properties_changed(emitter, interface, self.properties, Cow::Borrowed(&[])).await?;
        }
        if let Some(position) = self.seeked {
            Player::seeked(emitter, micros(position)).await?;
        }
        Ok(())
    }
}

/// Whether two updates show the same track; the cover may arrive later.
fn same_track(a: &Track, b: &Track) -> bool {
    a.title == b.title && a.artist == b.artist && a.album == b.album && a.duration == b.duration
}

fn track_id(number: u64) -> OwnedObjectPath {
    OwnedObjectPath::from(ObjectPath::from_string_unchecked(format!("/cloofy/track/{number}")))
}

/// Brings cloofy's window to the front on Hyprland: the one Omarchy started it in, or else
/// the terminal it runs in, found by walking up from cloofy's process.
fn raise_window() {
    // A pattern without backslashes, which would need escaping twice on the way to Hyprland.
    let omarchy = String::from("class:^(org.omarchy.cloofy)$");
    let terminal = ancestors().map(|pid| format!("pid:{pid}"));
    for window in std::iter::once(omarchy).chain(terminal) {
        if focus(&window) {
            return;
        }
    }
}

/// Asks Hyprland to focus `window`, the way its Lua configuration takes dispatches and then the
/// way older versions do. It answers "ok" only when it found the window.
fn focus(window: &str) -> bool {
    let lua = format!("hl.dsp.focus({{ window = \"{window}\" }})");
    [vec![lua.as_str()], vec!["focuswindow", window]].into_iter().any(|args| {
        process::Command::new("hyprctl")
            .arg("dispatch")
            .args(args)
            .output()
            .is_ok_and(|output| output.status.success() && output.stdout.trim_ascii() == b"ok")
    })
}

/// The processes cloofy runs under, nearest first: its shell, then the terminal.
fn ancestors() -> impl Iterator<Item = u32> {
    std::iter::successors(parent(process::id()), |&pid| parent(pid)).take(8)
}

fn parent(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The process name before it sits in parentheses and may hold spaces; the parent is the
    // second field after them.
    let ppid = stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()?;
    (ppid > 1).then_some(ppid)
}

/// A volume as MPRIS has it, from 0 to 1.
fn fraction(volume: u16) -> f64 {
    f64::from(volume) / f64::from(u16::MAX)
}

/// Durations as MPRIS has them.
fn micros(duration: Duration) -> i64 {
    duration.as_micros().try_into().unwrap_or(i64::MAX)
}

/// The cover as a file: MPRIS clients take a URL rather than the image.
struct CoverFile {
    /// Shared with [`Controls`], which removes the last file on its way out.
    path: Arc<Mutex<Option<PathBuf>>>,
    dir: PathBuf,
    shown: Option<(Arc<[u8]>, String)>,
    /// Covers written so far. Each gets a name of its own, since clients keep what they loaded
    /// from a URL.
    written: u64,
}

impl CoverFile {
    fn new(path: Arc<Mutex<Option<PathBuf>>>, dir: PathBuf) -> Self {
        Self { path, dir, shown: None, written: 0 }
    }

    /// The URL of `cover`, written to a file first when it's new. The previous one goes.
    fn url(&mut self, cover: Option<&Arc<[u8]>>) -> Option<String> {
        match (cover, &self.shown) {
            (Some(cover), Some((shown, url))) if Arc::ptr_eq(cover, shown) || cover == shown => {
                return Some(url.clone());
            }
            (None, None) => return None,
            _ => {}
        }
        self.shown = None;
        let old = self.path.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(old) = old {
            let _ = fs::remove_file(old);
        }
        let cover = cover?;
        self.written += 1;
        let name = format!("cloofy-{}-cover-{}.{}", process::id(), self.written, extension(cover));
        let path = self.dir.join(name);
        // A new file only: the temporary directory is shared, and a link planted under this name
        // must not get followed.
        let written = OpenOptions::new().write(true).create_new(true).open(&path).and_then(|mut f| f.write_all(cover));
        if let Err(err) = written {
            tracing::warn!("cover for the media controls: {}: {err}", path.display());
            let _ = fs::remove_file(&path);
            return None;
        }
        let url = url::Url::from_file_path(&path).ok()?.to_string();
        *self.path.lock().unwrap_or_else(PoisonError::into_inner) = Some(path);
        self.shown = Some((cover.clone(), url.clone()));
        Some(url)
    }
}

/// The user's runtime directory, which lives in memory and goes at logout.
pub(super) fn cover_dir() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|dir| dir.is_absolute()).unwrap_or_else(env::temp_dir)
}

pub(super) fn extension(image: &[u8]) -> &'static str {
    if image.starts_with(b"\x89PNG") {
        "png"
    } else if image.starts_with(b"RIFF") && image.get(8..12) == Some(b"WEBP") {
        "webp"
    } else {
        "jpg"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str) -> Track {
        Track {
            title: title.into(),
            artist: "Daft Punk, Romanthony".into(),
            album: "Discovery".into(),
            duration: Duration::from_secs(320),
            artwork: None,
            link: Some("https://open.spotify.com/track/0DiWol3AO6WpXZgp0goxAV".into()),
        }
    }

    fn playing(title: &str, position: u64) -> State {
        State { track: Some(track(title)), playing: true, position: Duration::from_secs(position), volume: 32768 }
    }

    /// A player that showed `state` at `start`.
    fn showing(state: State, start: Instant) -> Shown {
        let mut shown = Shown::new(start);
        shown.show(state, None, start);
        shown
    }

    #[test]
    fn describes_the_track() {
        let start = Instant::now();
        let mut shown = showing(playing("One More Time", 0), start);
        let changes = shown.show(playing("One More Time", 0), Some("file:///run/cover.jpg".into()), start);
        let metadata = shown.metadata();
        assert_eq!(metadata["mpris:trackid"], Value::from(ObjectPath::from_static_str_unchecked("/cloofy/track/1")));
        assert_eq!(metadata["mpris:length"], Value::I64(320_000_000));
        assert_eq!(metadata["xesam:title"], Value::from("One More Time"));
        assert_eq!(metadata["xesam:artist"], Value::from(vec!["Daft Punk, Romanthony"]));
        assert_eq!(metadata["xesam:album"], Value::from("Discovery"));
        assert_eq!(metadata["mpris:artUrl"], Value::from("file:///run/cover.jpg"));
        assert_eq!(metadata["xesam:url"], Value::from("https://open.spotify.com/track/0DiWol3AO6WpXZgp0goxAV"));
        assert_eq!(shown.status(), "Playing");
        // The cover came later: the same track, with new metadata.
        assert_eq!(changes.properties.keys().collect::<Vec<_>>(), [&"Metadata"]);
        assert_eq!(changes.seeked, None);
    }

    #[test]
    fn leaves_out_what_is_missing() {
        let mut episode = track("Episode");
        episode.album.clear();
        let shown = showing(State { track: Some(episode), ..State::default() }, Instant::now());
        let metadata = shown.metadata();
        assert!(!metadata.contains_key("xesam:album"));
        assert!(!metadata.contains_key("mpris:artUrl"));
        assert_eq!(shown.status(), "Paused");
    }

    #[test]
    fn tells_clients_when_the_volume_changes() {
        let start = Instant::now();
        let mut shown = showing(playing("One More Time", 0), start);
        assert!(shown.show(playing("One More Time", 0), None, start).properties.is_empty());
        let quieter = State { volume: 16384, ..playing("One More Time", 0) };
        let changes = shown.show(quieter, None, start);
        assert_eq!(changes.properties.keys().collect::<Vec<_>>(), [&"Volume"]);
        assert_eq!(changes.properties["Volume"], Value::F64(16384.0 / 65535.0));
    }

    #[test]
    fn finds_the_parent_process() {
        let expected = Some(std::os::unix::process::parent_id()).filter(|&pid| pid > 1);
        assert_eq!(parent(process::id()), expected);
        assert_eq!(ancestors().next(), expected);
        assert_eq!(parent(1), None, "init has no parent worth focusing");
    }

    #[test]
    fn shows_nothing_without_a_track() {
        let start = Instant::now();
        let mut shown = Shown::new(start);
        assert_eq!(shown.status(), "Stopped");
        assert!(shown.metadata().is_empty());
        assert_eq!(shown.seek(1_000_000, start), None);

        let changes = shown.show(playing("One More Time", 0), None, start);
        assert_eq!(changes.properties["PlaybackStatus"], Value::from("Playing"));
        assert!(changes.properties.contains_key("Metadata"));
        assert!(CAPABILITIES.iter().all(|name| changes.properties[name] == Value::Bool(true)));

        let changes = shown.show(State::default(), None, start);
        assert_eq!(changes.properties["PlaybackStatus"], Value::from("Stopped"));
        assert_eq!(changes.properties["Metadata"], Value::from(HashMap::<&str, Value>::new()));
        assert!(CAPABILITIES.iter().all(|name| changes.properties[name] == Value::Bool(false)));
    }

    #[test]
    fn keeps_time_while_playing() {
        let start = Instant::now();
        let mut shown = showing(playing("One More Time", 10), start);
        assert_eq!(shown.position(start + Duration::from_secs(5)), Duration::from_secs(15));
        assert_eq!(shown.position(start + Duration::from_secs(900)), Duration::from_secs(320), "not past the end");

        let paused = State { playing: false, ..playing("One More Time", 15) };
        let changes = shown.show(paused, None, start + Duration::from_secs(5));
        assert_eq!(changes.properties.keys().collect::<Vec<_>>(), [&"PlaybackStatus"]);
        assert_eq!(changes.seeked, None, "pausing is no seek");
        assert_eq!(shown.position(start + Duration::from_secs(60)), Duration::from_secs(15));
    }

    #[test]
    fn reports_jumps_as_seeks() {
        let start = Instant::now();
        let mut shown = showing(playing("One More Time", 0), start);
        let later = start + Duration::from_secs(10);
        assert_eq!(shown.show(playing("One More Time", 10), None, later).seeked, None, "as expected");
        let changes = shown.show(playing("One More Time", 60), None, later);
        assert_eq!(changes.seeked, Some(Duration::from_secs(60)));
        assert!(changes.properties.is_empty());
        // Played again from the start.
        assert_eq!(shown.show(playing("One More Time", 0), None, later).seeked, Some(Duration::ZERO));
    }

    #[test]
    fn gives_each_new_track_an_id() {
        let start = Instant::now();
        let mut shown = showing(playing("One More Time", 100), start);
        let changes = shown.show(playing("Aerodynamic", 0), None, start);
        assert_eq!(changes.seeked, None, "a new track starts where it starts");
        assert_eq!(
            shown.metadata()["mpris:trackid"],
            Value::from(ObjectPath::from_static_str_unchecked("/cloofy/track/2"))
        );
        assert!(!changes.properties.contains_key("CanPlay"), "unchanged");
    }

    #[test]
    fn seeks_from_where_playback_is() {
        let start = Instant::now();
        let shown = showing(playing("One More Time", 10), start);
        let now = start + Duration::from_secs(5);
        assert_eq!(shown.seek(10_000_000, now), Some(Command::Seek(Duration::from_secs(25))));
        assert_eq!(shown.seek(-60_000_000, now), Some(Command::Seek(Duration::ZERO)));
        assert_eq!(shown.seek(400_000_000, now), Some(Command::Next), "past the end skips");
    }

    #[test]
    fn sets_the_position_of_the_current_track_only() {
        let shown = showing(playing("One More Time", 0), Instant::now());
        let current = ObjectPath::from_static_str_unchecked("/cloofy/track/1");
        let stale = ObjectPath::from_static_str_unchecked("/cloofy/track/0");
        assert_eq!(shown.set_position(&current, 30_000_000), Some(Command::Seek(Duration::from_secs(30))));
        assert_eq!(shown.set_position(&stale, 30_000_000), None);
        assert_eq!(shown.set_position(&current, -1), None);
        assert_eq!(shown.set_position(&current, 321_000_000), None, "past the end");
    }

    #[test]
    fn writes_each_cover_once() {
        let dir = env::temp_dir().join(format!("cloofy-os-covers-{}", process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = Arc::new(Mutex::new(None));
        let mut covers = CoverFile::new(path.clone(), dir.clone());
        let jpeg: Arc<[u8]> = Arc::from(&b"\xff\xd8\xffjpeg"[..]);
        let png: Arc<[u8]> = Arc::from(&b"\x89PNG\r\n\x1a\npng"[..]);

        let first = covers.url(Some(&jpeg)).expect("the cover was written");
        let file = path.lock().unwrap().clone().expect("the file is known");
        assert!(first.starts_with("file:///") && first.ends_with(".jpg"));
        assert_eq!(fs::read(&file).unwrap(), &*jpeg);
        assert_eq!(covers.url(Some(&jpeg.clone())).as_ref(), Some(&first));

        let second = covers.url(Some(&png)).expect("the new cover was written");
        assert!(second != first && second.ends_with(".png"));
        assert!(!file.exists(), "the previous cover is gone");

        assert_eq!(covers.url(None), None);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        fs::remove_dir(dir).unwrap();
    }
}
