mod library;
mod menu;
mod persist;
mod queue;
mod settings;

use std::{
    collections::{HashMap, HashSet},
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use cloofy_core::{
    catalog::{AlbumInfo, ArtistInfo, Page, PlaylistInfo, ShowInfo, Track, User},
    connect::{Device, Playback},
    home::{Home, HomeItem, ItemKind},
    lyrics::Lyrics,
    paths::Paths,
    player::{CurrentTrack, PlayerEvent},
    update::{PackageManager, Release},
};
use cloofy_os::settings::Settings;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect, Size};
use tachyonfx::{CellFilter, Effect, EffectManager, Interpolation, Motion, fx};

use crate::{
    backend::{self, Player, Remote, Service},
    commands::Command,
    graphics::{INLINE_CELL, ImageProtocol, InlineLayer},
    loader::{DataEvent, Loader, Picture},
    media::MediaSync,
    notify::Notifier,
    overlay::{DjChoice, MenuAction, Overlay, PaletteAction, PickItem, Picker, PickerOutcome, dj_items},
    playback::{ConnectPush, Cover, NowPlaying, RemoteIntent, Repeat, VolumeSync},
    preferences::{AccentChoice, Preferences, Setting},
    queue::Queue,
    snapshot::Placement,
    theme::{self, Accent, Rgb},
    views::{
        ArtistSelection, ArtistView, HomeView, LibraryView, ListCursor, QueueRow, SearchEntry, SearchView, TrackSource,
        TrackView, View,
    },
};

/// Cover art size in the player bar, in cells (roughly square in most fonts).
pub const COVER_SIZE: Size = Size { width: 6, height: 3 };
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
const STATUS_TTL: Duration = Duration::from_secs(4);
const SEEK_STEP_MS: i64 = 5_000;
/// "Previous" restarts the track once it has played this long, as in Spotify's own clients.
const RESTART_AFTER_MS: u32 = 3_000;
const HISTORY_LIMIT: usize = 50;
/// Opening a page this soon after the last one skips the transition, so browsing fast stays
/// instant.
const QUICK_NAV: Duration = Duration::from_millis(300);
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Images unseen for this long are dropped from memory, unless the terminal holds them.
const IMAGE_TTL: Duration = Duration::from_secs(120);
/// Memory kitty may keep for the covers cloofy drew; the least recently drawn ones beyond it are
/// deleted from the terminal.
const TERMINAL_IMAGE_BUDGET: usize = 48 << 20;
/// A cover that failed to load, e.g. while offline, is tried again when drawn after this long.
const IMAGE_RETRY: Duration = Duration::from_secs(30);
const EVICTION_INTERVAL: Duration = Duration::from_secs(30);
/// Freed memory goes back to the system this long after the last of a burst of loads.
const TRIM_DELAY: Duration = Duration::from_secs(1);
/// Now Playing loads its cover for a new size once the window has kept that size this long.
const ZEN_COVER_SETTLE: Duration = Duration::from_millis(150);
/// A Now Playing cover that failed to load is tried again after this long.
const ZEN_COVER_RETRY: Duration = Duration::from_secs(10);
const SCROLL_STEP: isize = 3;
/// The character Kitty's unicode-placeholder images are drawn with.
const KITTY_PLACEHOLDER: char = '\u{10EEEE}';
/// How often to check which device plays, and how soon to look again while a change sent to
/// another device doesn't show yet.
const POLL_INTERVAL: Duration = Duration::from_secs(3);
const QUICK_POLL: Duration = Duration::from_millis(500);
/// Once Spotify pushes changes, polls follow the pushes; this is only a safety net.
const PUSHED_POLL_INTERVAL: Duration = Duration::from_secs(15);
/// Pushes come in bursts; one poll covers a burst.
const PUSH_DELAY: Duration = Duration::from_millis(150);
/// How long a push is newer than what the Web API may still report.
const PUSH_LEAD: Duration = Duration::from_secs(2);
/// A remote track that ends is followed by the next one about this much later.
const TRACK_END_SLACK: Duration = Duration::from_millis(300);
/// How long after Spotify accepted a remote command to check that the device applied it.
const CONFIRM_DELAY: Duration = Duration::from_millis(300);
/// How long the device label shows a transfer that the device hasn't confirmed.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(5);
/// How long after a track change Spotify may take to say what plays next, before the queue
/// page asks the Web API instead.
const QUEUE_GRACE: Duration = Duration::from_secs(2);
/// Session state is also saved periodically, in case cloofy gets killed.
const STATE_SAVE_INTERVAL: Duration = Duration::from_secs(20);
/// Rapid like/unlike toggles collapse into one request after this pause.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(350);
/// Animation frame length (spinners, equalizer, progress bar).
const FRAME: Duration = Duration::from_millis(120);
/// Track lists and artist pages opened again within this time are shown as they were; older
/// ones are shown too, and refreshed behind the scenes.
const PAGE_FRESH: Duration = Duration::from_secs(60);
/// Home fetched within this time is shown again without asking Spotify.
const HOME_FRESH: Duration = Duration::from_secs(5 * 60);
/// While data keeps arriving, the data cache is written at most this often.
const CACHE_SAVE_DELAY: Duration = Duration::from_secs(2);
/// Parts of the library besides playlists, each loaded on its own.
const LIBRARY_PARTS: u8 = 4;
/// How long a clicked target stays lit.
const PRESS_FLASH: Duration = Duration::from_millis(150);
/// In Low Power Mode, a playing track redraws once a second: often enough for the clock.
const LOW_POWER_FRAME: Duration = Duration::from_secs(1);
/// How often to look at the system's motion and power settings.
const SETTINGS_INTERVAL: Duration = Duration::from_secs(3);
/// After sleeping this long, the connection is gone, even if Spotify takes a while to notice.
const LONG_SLEEP: Duration = Duration::from_secs(60);

/// What the app opens with.
pub enum Start<'a> {
    /// The last session: the page, the selection and the track to resume.
    Restore(Box<crate::state::State>),
    /// The demo's playing track and queue, with nothing of an account restored.
    Demo(&'a crate::demo::Demo),
}

/// A newer cloofy, offered in the top bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    Available(Release),
    /// Out, but for the package manager that installed cloofy to install.
    Managed {
        release: Release,
        manager: PackageManager,
    },
    /// Downloading, with the percentage once the size is known.
    Installing {
        release: Release,
        percent: Option<u8>,
    },
    /// In place of the running binary, which carries on until restarted.
    Installed {
        release: Release,
        binary: PathBuf,
    },
}

// Events move through a channel once each; boxing the big ones would only add allocations.
#[allow(clippy::large_enum_variant)]
pub enum AppEvent {
    Data(DataEvent),
    /// What a service's player reports.
    Player(Service, PlayerEvent),
    /// A service pushed a change of what plays on the account's devices.
    Pushed(Service, ConnectPush),
    /// A service's player is up; also after it connected again.
    PlayerReady(Box<Player>),
    PlayerFailed(Service, String),
    /// The player's connection to the service dropped; a new one is on its way.
    PlayerLost(Service),
    /// The computer is about to sleep, or woke up.
    Power(cloofy_os::power::Event),
    /// A media key, headphone button or Control Center.
    Media(cloofy_os::media::Command),
    /// The audio output a service's player used went away, named.
    OutputLost(Service, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Nav,
    Content,
    SearchInput,
}

/// An entry of the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavItem {
    Home,
    Search,
    Library(LibraryEntry),
}

impl NavItem {
    /// Entries above the library; Home and Search live in the top bar now.
    pub const FIXED: usize = 0;
}

/// An item of "Your Library", pointing into the app's library lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryEntry {
    Liked,
    Playlist(usize),
    Album(usize),
    Artist(usize),
    Show(usize),
}

/// The filter chips above the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum LibraryFilter {
    #[default]
    All,
    Playlists,
    Albums,
    Artists,
    Podcasts,
}

impl LibraryFilter {
    pub fn label(self) -> &'static str {
        match self {
            LibraryFilter::All => "All",
            LibraryFilter::Playlists => "Playlists",
            LibraryFilter::Albums => "Albums",
            LibraryFilter::Artists => "Artists",
            LibraryFilter::Podcasts => "Podcasts",
        }
    }

    fn shows(self, kind: LibraryFilter) -> bool {
        self == LibraryFilter::All || self == kind
    }
}

pub enum ImageSlot {
    /// Holds the number of the last frame that drew the image, which the loader checks
    /// before decoding it.
    Loading(Arc<AtomicU64>),
    Ready(Picture),
    Failed(Instant),
}

/// A clickable region, registered while drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    NavArea,
    Nav(usize),
    ContentArea,
    Row(usize),
    Card {
        shelf: usize,
        item: usize,
    },
    /// The play button shown on a hovered card.
    CardPlay {
        shelf: usize,
        item: usize,
    },
    /// The play button on a hovered card of the full-page library.
    RowPlay(usize),
    /// The arrows that page a shelf sideways.
    ShelfScroll {
        shelf: usize,
        forward: bool,
    },
    /// An artist, album or playlist name that leads to its page.
    Link(Link),
    Chip(LibraryFilter),
    /// A filter tab of the full-page library.
    PageChip(LibraryFilter),
    /// A clickable entry of the hint bar.
    Hint(HintAction),
    /// Play, shuffle, save or more on a playlist, album or artist page.
    Header(HeaderButton),
    /// The heart next to the playing track.
    LikePlaying,
    SearchInput,
    Command(Command),
    Seek {
        x: u16,
        width: u16,
    },
    Volume {
        x: u16,
        width: u16,
    },
    Cover,
    /// The arrows, or the switch, that change the `index`th setting on the Settings page.
    SettingStep {
        index: usize,
        forward: bool,
    },
    OverlayArea,
    OverlayItem(usize),
}

impl Hit {
    /// Things that react to a click, as opposed to the areas behind them.
    pub fn is_interactive(&self) -> bool {
        !matches!(self, Hit::NavArea | Hit::ContentArea | Hit::OverlayArea | Hit::Seek { .. } | Hit::Volume { .. })
    }
}

/// Where a clickable name leads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// The `artist`th artist of track `row` in the open track list or search results.
    TrackArtist {
        row: usize,
        artist: usize,
    },
    TrackAlbum(usize),
    /// The `n`th artist of the playing track.
    PlayingArtist(usize),
    PlayingAlbum,
    /// The playlist, album or artist the music plays from.
    PlayingContext,
}

/// What a click on the hint bar does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintAction {
    Run(Command),
    Activate,
    Back,
    ToggleFocus,
    /// Does exactly what the key does where it is pressed, for keys whose meaning depends
    /// on the focus or the page.
    Key(KeyCode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderButton {
    Play,
    Shuffle,
    Save,
    More,
}

/// Slots for running animations; a new animation replaces the one in its slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Fx {
    #[default]
    Content,
    Overlay,
    Screen,
}

pub struct LargeCover {
    pub url: String,
    pub size: Size,
    pub picture: Picture,
}

pub struct LyricsState {
    pub uri: String,
    /// `None` while loading; `Ok(None)` when the song has no lyrics.
    pub result: Option<Result<Option<Lyrics>, String>>,
}

pub struct Status {
    pub text: String,
    pub error: bool,
    since: Instant,
}

/// A transfer of playback to another device, until that device shows up playing.
struct Transfer {
    device_id: String,
    name: String,
    since: Instant,
}

/// What `f`, `q` and `a` act on.
struct Target {
    uri: String,
    name: String,
    is_track: bool,
}

pub struct App {
    loader: Loader,
    state_path: std::path::PathBuf,
    last_save: Instant,
    /// The last session's track, until something else plays; ␣ continues it.
    resume: Option<crate::state::LastTrack>,
    /// What the current track plays in, so a restart can continue there.
    last_context: Option<String>,
    /// cloofy's own volume, saved while the slider shows another device's.
    restored_volume: Option<u16>,
    /// Local volume changes on their way to the player.
    volume_sync: VolumeSync,
    /// cloofy's own players, one per service that is connected.
    players: HashMap<Service, Player>,
    /// The service whose player is the one that plays, or played last. Only one plays at a
    /// time: starting playback on another one pauses this one.
    active: Option<Service>,
    /// cloofy's own device ID among the account's devices.
    device_id: Option<String>,
    /// cloofy's own device is the active one. Until something activates it, Spirc ignores
    /// every command but loading a track.
    local_active: bool,
    /// Playback runs on another device; commands go through Spotify Connect.
    remote_active: bool,
    /// The Spotify Connect device that plays, by ID, as Spotify last said.
    active_device: Option<String>,
    /// Changes sent to the other device that Spotify doesn't show yet.
    remote_intent: RemoteIntent,
    /// When the last command went to another device; polls started before show older state.
    last_command: Instant,
    /// The last device list, so the picker opens at once while a fresh one loads.
    devices_cache: Option<Vec<Device>>,
    /// The picker should open when the device list arrives.
    devices_wanted: bool,
    /// Playback is moving to another device.
    transfer: Option<Transfer>,
    /// What the user chose on the Settings page.
    pub prefs: Preferences,
    next_poll: Instant,
    polling: bool,
    poll_started: Instant,
    /// The last change Spotify pushed, and when.
    last_push: Option<(ConnectPush, Instant)>,
    /// Where the data cache and the state file live.
    paths: Paths,
    pub player_error: Option<String>,
    pub now: NowPlaying,
    /// When the shown track last changed.
    track_changed_at: Instant,
    /// What plays next on the device that plays, as Spotify or cloofy's player last said.
    queue: Queue,
    /// Details of the tracks in the queue by URI; `None` while they load.
    queue_tracks: HashMap<String, Option<Track>>,
    /// Tracks in the queue Spotify has no details for; they are left off the queue page.
    queue_tracks_missing: HashSet<String>,
    /// Counts the reports of what plays next, so a slower answer can tell it is outdated.
    queue_version: u64,
    /// The track the Web API was last asked what plays after, and the report it was asked
    /// about, so it is asked once per report.
    upcoming_asked: Option<(String, u64)>,
    /// The Web API's answer is on its way.
    upcoming_pending: bool,
    /// When to ask the Web API what plays next.
    upcoming_due: Option<Instant>,
    /// cloofy's player shuffled what plays next and hasn't said how.
    queue_reshuffled: bool,
    /// The system's media controls, where there are any.
    media: Option<MediaSync>,
    /// Announces new tracks while cloofy is in the background, in terminals that can.
    notifier: Option<Notifier>,
    /// Whether the terminal window has keyboard focus, as far as it reports.
    focused: bool,
    /// When the computer went to sleep, in wall-clock time: the monotonic clock stops meanwhile.
    slept_at: Option<std::time::SystemTime>,
    /// Reduce Motion and Low Power Mode, as the system reported them at the last check.
    system_settings: Settings,
    /// The system's settings with the user's choice for power saving applied.
    pub settings: Settings,
    settings_checked: Instant,
    pub accent: Accent,
    /// The color of the playing cover, for when the accent follows it.
    cover_accent: Option<Rgb>,
    pub me: Option<User>,
    pub playlists: Vec<PlaylistInfo>,
    pub playlists_loading: bool,
    pub albums: Vec<AlbumInfo>,
    pub artists: Vec<ArtistInfo>,
    pub shows: Vec<ShowInfo>,
    pub liked_total: Option<u32>,
    pub library_loading: bool,
    /// Parts of the library still on their way at startup.
    library_pending: u8,
    pub library_filter: LibraryFilter,
    /// The last Home feed, shown instantly while a fresh one loads.
    home_cache: Option<Home>,
    home_fetched_at: Option<Instant>,
    /// Library, user or Home changed since the data cache was last written.
    data_dirty: bool,
    last_cache_save: Instant,
    /// The track list the last run left open, until the restored view has taken it.
    cached_tracks: Option<crate::datacache::Tracks>,
    /// Which load of which list the track cache holds, to write it only when it changed.
    cached_tracks_version: Option<(u64, Instant)>,
    /// Cover art by cell size and URL.
    pub images: HashMap<String, ImageSlot>,
    /// When each image was last drawn; long-unseen ones are dropped to save memory.
    image_used: HashMap<String, Instant>,
    last_eviction: Instant,
    /// When to hand freed memory back to the system.
    trim_due: Option<Instant>,
    /// Frames drawn so far.
    draws: u64,
    /// When the last frame started; images used since were drawn in it.
    frame_started: Instant,
    /// Escape sequences to write after the next frame, such as deleting images the terminal
    /// holds.
    pub terminal_out: Vec<String>,
    /// Covers the frame being drawn will likely show soon; see [`App::prefetch`].
    prefetch: Vec<(String, Size, bool)>,
    /// Known library state per URI: liked songs, saved albums, followed playlists.
    pub saved: HashMap<String, bool>,
    /// Library changes not sent yet: wanted state, item name, time of the last toggle.
    pending_saves: HashMap<String, (bool, String, Instant)>,
    /// Library changes Spotify hasn't confirmed yet; one request per URI at a time.
    saving: HashSet<String>,
    /// Items taken out of the library before Spotify confirmed it, and where they were, to
    /// put them back should it fail.
    unsaved: HashMap<String, (usize, library::LibraryItem)>,
    /// Library lists to load again after confirmed changes, and when.
    library_reloads: HashMap<library::LibraryPart, Instant>,
    pub focus: Focus,
    pub nav: ListCursor,
    pub view: View,
    history: Vec<View>,
    /// When the page last changed.
    last_nav: Option<Instant>,
    next_view_id: u64,
    pub status: Option<Status>,
    pub overlay: Option<Overlay>,
    /// Set when an overlay opens; the first draw that knows its area starts the animation.
    pub overlay_fx_pending: bool,
    pub zen: bool,
    pub zen_cover: Option<LargeCover>,
    /// Cover size the Now Playing layout asked for at the last draw.
    pub zen_cover_want: Option<Size>,
    /// The size asked for, and since when; a resize passes through many sizes.
    zen_cover_want_since: Option<(Size, Instant)>,
    zen_cover_pending: Option<(String, Size)>,
    /// The last cover that failed to load, and when.
    zen_cover_failed: Option<(String, Size, Instant)>,
    pub lyrics: Option<LyricsState>,
    /// Smoothly animated lyric line position.
    pub lyrics_scroll: f32,
    pub hits: Vec<(Rect, Hit)>,
    /// Last known mouse position, for hover highlights.
    pub mouse: Option<Position>,
    hovered: Option<Hit>,
    /// The seek bar or volume slider while the left button holds it.
    dragging: Option<Hit>,
    /// Where a drag on the seek bar will jump to; shown until the button is released.
    pub seek_preview: Option<u32>,
    /// Images drawn in the current frame, for dev snapshots.
    pub placements: Vec<Placement>,
    /// Writes the frame's Sixel and iTerm2 images.
    pub inline: InlineLayer,
    /// F12 was pressed; the next frame gets written as a PNG.
    pub snapshot_requested: bool,
    last_click: Option<(Instant, Hit)>,
    /// The target clicked last, where it was and when, so it can light up for a moment.
    pressed: Option<(Hit, Rect, Instant)>,
    pub effects: EffectManager<Fx>,
    pub last_draw: Instant,
    pub content_area: Rect,
    pub screen: Rect,
    /// Rows of the content list at the last draw, for paging.
    pub page_height: usize,
    started: Instant,
    last_frame: u64,
    pub quit: bool,
    /// A newer cloofy, and how far installing it got.
    pub update: Option<Update>,
    /// The updated binary to start once the interface has closed.
    pub restart: Option<PathBuf>,
    /// A showcase with sample data (`cloofy demo`): nothing is saved.
    demo: bool,
}

impl App {
    pub fn new(
        loader: Loader,
        prefs: Preferences,
        start: Start,
        paths: Paths,
        media: Option<cloofy_os::media::Controls>,
        notifier: Option<Notifier>,
    ) -> Self {
        loader.me();
        loader.playlists();
        loader.library();
        loader.home(0);
        let system_settings = cloofy_os::settings::current();
        let mut accent = Accent::new();
        if let Some(color) = prefs.accent.fixed() {
            accent.jump(color);
        }
        let mut app = Self {
            loader,
            state_path: paths.state_file(),
            last_save: Instant::now(),
            resume: None,
            last_context: None,
            restored_volume: None,
            volume_sync: VolumeSync::default(),
            players: HashMap::new(),
            active: None,
            device_id: None,
            local_active: false,
            remote_active: false,
            active_device: None,
            remote_intent: RemoteIntent::default(),
            last_command: Instant::now(),
            devices_cache: None,
            devices_wanted: false,
            transfer: None,
            settings: prefs.apply(system_settings),
            prefs,
            next_poll: Instant::now(),
            polling: false,
            poll_started: Instant::now(),
            last_push: None,
            paths,
            player_error: None,
            now: NowPlaying::new(),
            track_changed_at: Instant::now(),
            queue: Queue::default(),
            queue_tracks: HashMap::new(),
            queue_tracks_missing: HashSet::new(),
            queue_version: 0,
            upcoming_asked: None,
            upcoming_pending: false,
            upcoming_due: None,
            queue_reshuffled: false,
            media: media.map(MediaSync::new),
            notifier,
            focused: true,
            slept_at: None,
            system_settings,
            settings_checked: Instant::now(),
            accent,
            cover_accent: None,
            me: None,
            playlists: Vec::new(),
            playlists_loading: true,
            albums: Vec::new(),
            artists: Vec::new(),
            shows: Vec::new(),
            liked_total: None,
            library_loading: true,
            library_pending: LIBRARY_PARTS,
            library_filter: LibraryFilter::All,
            home_cache: None,
            home_fetched_at: None,
            data_dirty: false,
            last_cache_save: Instant::now(),
            cached_tracks: None,
            cached_tracks_version: None,
            images: HashMap::new(),
            image_used: HashMap::new(),
            last_eviction: Instant::now(),
            trim_due: None,
            draws: 0,
            frame_started: Instant::now(),
            terminal_out: Vec::new(),
            prefetch: Vec::new(),
            saved: HashMap::new(),
            pending_saves: HashMap::new(),
            saving: HashSet::new(),
            unsaved: HashMap::new(),
            library_reloads: HashMap::new(),
            focus: Focus::Content,
            nav: ListCursor::default(),
            view: View::Home(HomeView::new(0, None)),
            history: Vec::new(),
            last_nav: None,
            next_view_id: 1,
            status: None,
            overlay: None,
            overlay_fx_pending: false,
            zen: false,
            zen_cover: None,
            zen_cover_want: None,
            zen_cover_want_since: None,
            zen_cover_pending: None,
            zen_cover_failed: None,
            lyrics: None,
            lyrics_scroll: 0.0,
            hits: Vec::new(),
            placements: Vec::new(),
            inline: InlineLayer::default(),
            snapshot_requested: false,
            mouse: None,
            hovered: None,
            dragging: None,
            seek_preview: None,
            last_click: None,
            pressed: None,
            effects: EffectManager::default(),
            last_draw: Instant::now(),
            content_area: Rect::default(),
            screen: Rect::default(),
            page_height: 10,
            started: Instant::now(),
            last_frame: 0,
            quit: false,
            update: None,
            restart: None,
            demo: matches!(start, Start::Demo(_)),
        };
        match start {
            Start::Restore(state) => {
                app.load_cache();
                app.restore(*state);
            }
            // A showcase starts fresh, and never touches the account's saved session.
            Start::Demo(demo) => app.start_demo(demo),
        }
        app.cached_tracks = None;
        app
    }

    /// Shows the demo's track as playing, with its queue. Nothing plays: there is no player.
    fn start_demo(&mut self, demo: &crate::demo::Demo) {
        let playing = &demo.playing;
        let track = &playing.track;
        let last = crate::state::LastTrack {
            uri: track.uri.clone(),
            name: track.name.clone(),
            artists: track.artists.clone(),
            artist_uris: track.artist_uris.clone(),
            album: track.album.clone(),
            album_uri: Some(track.album_uri.clone()),
            duration_ms: track.duration_ms,
            cover_url: track.image_url.clone(),
            cover_url_large: playing.cover_url_large.clone(),
            context: playing.context.clone(),
            position_ms: playing.position_ms,
        };
        self.now.restore(&last);
        self.now.set_playing(true);
        self.now.volume = (u32::from(u16::MAX) * 7 / 10) as u16;
        self.last_context = last.context.clone();
        if let Some(url) = &last.cover_url {
            self.load_cover(url.clone());
        }
        self.after_track_change(None);
        let (queued, rest) = demo.upcoming();
        self.start_demo_queue(queued, rest);
    }

    /// The last session's track is shown and ␣ would continue it.
    pub fn can_resume(&self) -> bool {
        self.resume.is_some()
    }

    pub fn is_connected(&self) -> bool {
        self.player().is_some()
    }

    /// The player that plays, or played last, while it is connected.
    fn player(&self) -> Option<&Player> {
        self.players.get(&self.active?)
    }

    /// The service playing, or else the one shown.
    pub fn service(&self) -> Option<Service> {
        self.active.or(self.loader.service())
    }

    /// The name of the service playing or shown, for messages about it.
    pub fn service_name(&self) -> &'static str {
        self.service().map_or("your music service", Service::name)
    }

    /// Whether Spotify's DJ is there, for the service that plays.
    pub fn supports_dj(&self) -> bool {
        self.service().is_some_and(backend::supports_dj)
    }

    /// Whether the service that plays can play on the account's other devices.
    pub fn supports_devices(&self) -> bool {
        self.service().is_some_and(backend::supports_devices)
    }

    /// Whether the service that plays can fade songs into each other.
    pub fn supports_crossfade(&self) -> bool {
        self.service().is_some_and(backend::supports_crossfade)
    }

    /// Whether `command` does anything here. The palette and the help leave out the ones for
    /// what the service lacks.
    pub fn offers(&self, command: Command) -> bool {
        match command {
            Command::Dj => self.supports_dj(),
            Command::Devices => self.supports_devices(),
            Command::Crossfade => self.supports_crossfade(),
            // A build without a service is never released.
            Command::Update => backend::VARIANT.is_some(),
            _ => true,
        }
    }

    /// Animation frame counter, shared by all animated elements.
    pub fn frame(&self) -> u64 {
        let frame = if self.settings.low_power { LOW_POWER_FRAME } else { FRAME };
        (self.started.elapsed().as_millis() / frame.as_millis()) as u64
    }

    /// Without movement that only decorates: for Reduce Motion, and to save energy.
    pub fn calm(&self) -> bool {
        self.settings.reduce_motion || self.settings.low_power
    }

    pub fn is_saved(&self, uri: &str) -> bool {
        self.saved.get(uri).copied().unwrap_or(false)
    }

    /// The target that was just clicked and its area, while it should still look pressed.
    pub fn pressed(&self) -> Option<(Hit, Rect)> {
        self.pressed.filter(|(_, _, at)| at.elapsed() < PRESS_FLASH).map(|(hit, area, _)| (hit, area))
    }

    pub fn shutdown(&mut self) {
        self.save_state();
        if self.data_dirty {
            self.save_library(false);
        }
        self.save_open_tracks(false);
        for player in self.players.values() {
            player.shutdown();
        }
    }

    // ── Updates ──────────────────────────────────────────────────────────────────────────

    /// Looks for a newer cloofy; one the user `asked` for reports what it finds either way.
    pub fn check_for_update(&mut self, asked: bool) {
        if self.offers(Command::Update) {
            self.loader.check_update(self.paths.clone(), asked);
        }
    }

    fn on_update_available(&mut self, release: Release) {
        // An update on its way, or done, stays as it is.
        if matches!(self.update, Some(Update::Installing { .. } | Update::Installed { .. })) {
            return;
        }
        let update = match PackageManager::of_running() {
            Some(manager) => Update::Managed { release, manager },
            None => Update::Available(release),
        };
        if self.update.as_ref() != Some(&update) {
            self.announce(&update);
        }
        self.update = Some(update);
    }

    /// Says that a newer cloofy is out, and how to get it.
    fn announce(&mut self, update: &Update) {
        match update {
            Update::Available(release) => self.notice(format!("cloofy {} is out · U updates", release.version())),
            Update::Managed { release, manager } => {
                self.notice(format!("cloofy {} is out · {}", release.version(), manager.how_to_update()))
            }
            Update::Installing { .. } | Update::Installed { .. } => {}
        }
    }

    /// The update button: installs the update, or restarts into the installed one, or says how
    /// to update where a package manager does. Without an update, looks for one.
    fn run_update(&mut self) {
        match self.update.clone() {
            None if self.demo => self.error("This is the demo: nothing updates here".into()),
            None => {
                self.notice("Looking for updates…".into());
                self.check_for_update(true);
            }
            Some(update @ Update::Managed { .. }) => self.announce(&update),
            Some(Update::Available(release)) => {
                self.notice(format!("Updating to cloofy {}…", release.version()));
                self.loader.install_update(release.clone());
                self.update = Some(Update::Installing { release, percent: None });
            }
            Some(Update::Installing { .. }) => {}
            Some(Update::Installed { binary, .. }) => {
                self.restart = Some(binary);
                self.quit = true;
            }
        }
    }

    /// What the update command does now, for the palette.
    fn update_label(&self) -> String {
        match &self.update {
            None => Command::Update.label().to_owned(),
            Some(Update::Available(release)) => format!("Update to cloofy {}", release.version()),
            Some(Update::Managed { release, manager }) => {
                format!("cloofy {} is out · {}", release.version(), manager.how_to_update())
            }
            Some(Update::Installing { .. }) => "Updating cloofy…".to_owned(),
            Some(Update::Installed { release, .. }) => format!("Restart into cloofy {}", release.version()),
        }
    }

    // ── Events ───────────────────────────────────────────────────────────────────────────

    /// Returns whether the screen needs a redraw.
    pub fn on_input(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                self.on_key(key);
                true
            }
            Event::Mouse(mouse) => self.on_mouse(mouse),
            Event::Resize(..) => true,
            Event::FocusGained | Event::FocusLost => {
                self.focused = matches!(event, Event::FocusGained);
                false
            }
            _ => false,
        }
    }

    /// Returns whether the screen needs a redraw.
    pub fn on_event(&mut self, event: AppEvent) -> bool {
        match event {
            AppEvent::Player(service, event) => return self.on_player(service, event),
            // A push brings the next poll forward; only the queue page shows it at once.
            AppEvent::Pushed(service, push) if self.is_active(service) => return self.on_push(push),
            AppEvent::Pushed(..) => return false,
            AppEvent::PlayerReady(player) => self.on_player_ready(*player),
            AppEvent::PlayerFailed(service, err) => self.on_player_failed(service, err),
            AppEvent::PlayerLost(service) => self.on_player_lost(service),
            AppEvent::Power(event) => self.on_power(event),
            AppEvent::Media(command) => self.on_media(command),
            AppEvent::OutputLost(service, name) => self.on_output_lost(service, name),
            AppEvent::Data(data) => return self.on_data(data),
        }
        true
    }

    /// Whether `service`'s player is the one that plays, or nothing has played yet.
    fn is_active(&self, service: Service) -> bool {
        self.active.is_none_or(|active| active == service)
    }

    fn on_player_ready(&mut self, player: Player) {
        let service = player.service();
        self.player_error = None;
        self.active.get_or_insert(service);
        if self.is_active(service) {
            self.device_id = player.device_id();
        }
        player.set_crossfade(Duration::from_secs(self.prefs.crossfade));
        self.players.insert(service, player);
        self.reload_failed_view();
    }

    /// The new connection starts out idle; what played gets ready to continue, as after a restart.
    fn on_player_lost(&mut self, service: Service) {
        self.players.remove(&service);
        if !self.is_active(service) {
            return;
        }
        if !self.remote_active
            && self.resume.is_none()
            && let Some(last) = self.snapshot().last_track
        {
            self.now.restore(&last);
            self.resume = Some(last);
        }
        self.notice(format!("Connection to {} lost, reconnecting…", service.name()));
    }

    fn on_power(&mut self, event: cloofy_os::power::Event) {
        use cloofy_os::power::Event;
        match event {
            Event::Sleep => {
                self.slept_at = Some(std::time::SystemTime::now());
                if !self.remote_active && self.now.playing {
                    self.control(|c| c.pause());
                }
                self.save_state();
            }
            Event::Wake => {
                let slept = self.slept_at.take().and_then(|at| at.elapsed().ok());
                if slept.is_none_or(|slept| slept >= LONG_SLEEP) {
                    for player in self.players.values() {
                        player.reconnect();
                    }
                }
                // Whatever plays elsewhere may have changed meanwhile.
                self.next_poll = Instant::now();
            }
        }
    }

    /// Unplugged headphones pause playback rather than move it to the speakers.
    fn on_output_lost(&mut self, service: Service, name: String) {
        if !self.is_active(service) {
            if let Some(player) = self.players.get(&service) {
                let _ = player.pause();
            }
            return;
        }
        // The report itself says cloofy's player is writing audio, whatever the state shows,
        // and pausing a player that isn't playing does nothing.
        self.control(Player::pause);
        // Shown paused at once: a pause from the headphones that arrives before the player
        // confirms must not toggle playback back on, through the speakers.
        self.now.set_playing(false);
        self.notice(format!("Paused: {name} disconnected"));
    }

    fn on_media(&mut self, command: cloofy_os::media::Command) {
        use cloofy_os::media::Command as Media;
        match command {
            Media::Toggle => self.run(Command::PlayPause),
            Media::Play if !self.now.playing => self.run(Command::PlayPause),
            // Pausing the local player twice stays paused, unlike toggling it.
            Media::Pause if !self.remote_active => {
                self.control(|c| c.pause());
            }
            Media::Pause if self.now.playing => self.run(Command::PlayPause),
            Media::Play | Media::Pause => {}
            Media::Next => self.run(Command::Next),
            Media::Previous => self.run(Command::Previous),
            Media::Seek(position) => self.seek_to(position.as_millis().try_into().unwrap_or(u32::MAX)),
        }
    }

    /// Writes a desktop notification once a new track plays while cloofy is in the background.
    pub fn announce_track(&mut self) {
        let Some(notifier) = self.notifier.as_mut().filter(|_| self.prefs.notifications) else { return };
        let track = self.now.track.as_ref().filter(|_| !self.remote_active);
        if let Some(sequence) = notifier.update(track, self.now.playing, self.focused) {
            let mut stdout = std::io::stdout();
            let _ = stdout.write_all(sequence.as_bytes()).and_then(|()| stdout.flush());
        }
    }

    /// Brings the system's media controls up to date; cheap when nothing changed.
    pub fn sync_media(&mut self) {
        let Some(media) = &mut self.media else { return };
        let track = self.now.track.as_ref().filter(|_| !self.remote_active);
        if let Some(url) = media.missing_artwork(track) {
            self.loader.artwork(url);
        }
        media.sync(&self.now, !self.remote_active);
    }

    /// The player keeps trying to connect and reports back with `PlayerReady`.
    fn on_player_failed(&mut self, service: Service, err: String) {
        tracing::warn!("player: {err}");
        if self.is_active(service) {
            self.player_error = Some(err);
        }
    }

    /// Loads the open page again once the session is up, if it failed without one.
    fn reload_failed_view(&mut self) {
        match &mut self.view {
            View::Home(v) if v.error.is_some() => {
                v.error = None;
                v.loading = true;
                self.loader.home(v.id);
            }
            View::Artist(v) if v.error.is_some() => {
                v.error = None;
                self.loader.artist(v.id, v.uri.clone());
            }
            View::Tracks(v) if v.error.is_some() => {
                self.next_view_id += 1;
                v.load(self.next_view_id, |id, source| self.loader.tracks(id, source));
            }
            _ => {}
        }
    }

    fn on_player(&mut self, service: Service, event: PlayerEvent) -> bool {
        if !self.is_active(service) {
            // Playback started on another service's player, say from another app: that one
            // plays now, and this one stops.
            if !matches!(event, PlayerEvent::TrackChanged { .. } | PlayerEvent::Playing { .. }) {
                return false;
            }
            self.take_over(service);
        }
        // What plays next, as cloofy's own player has it; while another device plays, it has
        // nothing to say.
        if let PlayerEvent::QueueChanged { context_uri, current, next } = event {
            if !self.remote_active {
                self.set_queue(Queue::from_player(context_uri, current, &next));
            }
            return matches!(self.view, View::Queue(_));
        }
        if matches!(event, PlayerEvent::ShuffleChanged { shuffle } if shuffle != self.now.shuffle)
            && !self.remote_active
        {
            self.queue_reshuffled();
        }
        match &event {
            PlayerEvent::Activated => {
                self.local_active = true;
                // A volume set while inactive was ignored; Spirc takes it now.
                if let Some(volume) = self.volume_sync.next() {
                    self.control(|c| c.set_volume(volume));
                }
            }
            PlayerEvent::Deactivated => {
                self.local_active = false;
                // Usually another device took over; find out which before the next command.
                self.next_poll = Instant::now();
            }
            _ => {}
        }
        if matches!(event, PlayerEvent::TrackChanged { .. } | PlayerEvent::Playing { .. }) {
            self.resume = None;
            // Playback moved here.
            if self.transfer.as_ref().is_some_and(|t| Some(&t.device_id) == self.device_id.as_ref()) {
                self.transfer = None;
            }
        }
        if self.remote_active {
            match event {
                // Playback just moved to cloofy.
                PlayerEvent::TrackChanged { .. } | PlayerEvent::Playing { .. } => {
                    self.remote_active = false;
                    self.now.device = None;
                }
                // The local player idles while another device plays.
                _ => return false,
            }
        }
        if let PlayerEvent::VolumeChanged { volume } = event {
            let (show, send) = self.volume_sync.reported(volume);
            if let Some(next) = send {
                self.control(|c| c.set_volume(next));
            }
            if !show {
                return false;
            }
            self.restored_volume = Some(volume);
        }
        let before = self.now.track.as_ref().map(|t| t.uri.clone());
        if let Some(url) = self.now.apply(event) {
            self.load_cover(url);
        }
        self.after_track_change(before);
        true
    }

    /// Spotify pushed a change of what plays where; a poll brings the details.
    /// What plays next comes with the push. Returns whether the screen shows it.
    fn on_push(&mut self, push: ConnectPush) -> bool {
        if push.worth_polling(self.device_id.as_deref(), self.remote_active, self.last_context.as_deref()) {
            self.next_poll = self.next_poll.min(Instant::now() + PUSH_DELAY);
        }
        self.active_device = Some(push.active_device.clone()).filter(|id| !id.is_empty());
        self.set_queue(push.queue.clone());
        self.last_push = Some((push, Instant::now()));
        matches!(self.view, View::Queue(_))
    }

    /// Returns whether anything shown changed: while cloofy plays or nothing does, the poll
    /// mostly confirms what is on screen.
    fn on_playback(&mut self, playback: Option<Playback>) -> bool {
        if let (Some(transfer), Some(p)) = (&self.transfer, &playback)
            && transfer.device_id == p.device.id
        {
            self.transfer = None;
        }
        if let Some(p) = &playback
            && Some(p.device.id.as_str()) != self.device_id.as_deref()
        {
            // While cloofy starts a track itself, Spotify may still name the device that
            // played before; another device starting a track may still show the old one.
            // Either would undo the early switch.
            let stale = if self.remote_active {
                self.now.awaits_remote(p, self.remote_intent.is_waiting())
            } else {
                self.now.pending.is_some()
            };
            if stale {
                self.next_poll = self.next_poll.min(Instant::now() + QUICK_POLL);
                return false;
            }
        }
        self.active_device = playback.as_ref().map(|p| p.device.id.clone());
        match playback {
            Some(p) if Some(p.device.id.as_str()) == self.device_id.as_deref() => {
                let changed = self.remote_active || (p.context.is_some() && p.context != self.last_context);
                // Playback started from another app on cloofy's device: learn what it plays.
                if p.context.is_some() {
                    self.last_context = p.context.clone();
                }
                if self.remote_active {
                    self.remote_active = false;
                    self.now.device = None;
                }
                changed
            }
            Some(p) => {
                self.remote_active = true;
                self.resume = None;
                if p.context.is_some() {
                    self.last_context = p.context.clone();
                }
                let before = self.now.track.as_ref().map(|t| t.uri.clone());
                let dragged_volume = matches!(self.dragging, Some(Hit::Volume { .. })).then_some(self.now.volume);
                let (cover, unconfirmed) = self.now.apply_remote(&p, &mut self.remote_intent);
                if let Some(url) = cover {
                    self.load_cover(url);
                }
                // The Web API may lag behind a push; look again until it catches up.
                let behind =
                    self.last_push.as_ref().is_some_and(|(push, at)| at.elapsed() < PUSH_LEAD && push.is_ahead_of(&p));
                if unconfirmed || behind {
                    self.next_poll = self.next_poll.min(Instant::now() + QUICK_POLL);
                }
                // Catch the next track as soon as this one ends.
                if self.now.playing
                    && let Some(track) = &self.now.track
                {
                    let left = track.duration_ms.saturating_sub(self.now.position_ms());
                    if left > 0 {
                        let end = Instant::now() + Duration::from_millis(left.into()) + TRACK_END_SLACK;
                        self.next_poll = self.next_poll.min(end);
                    }
                }
                // The slider follows the mouse while it holds it, not the device.
                if let Some(volume) = dragged_volume {
                    self.now.volume = volume;
                }
                self.after_track_change(before);
                true
            }
            None if self.remote_active => {
                self.remote_active = false;
                self.now.device = None;
                self.now.pending = None;
                self.now.set_playing(false);
                true
            }
            None => false,
        }
    }

    /// Loads lyrics and the like state for a new track.
    fn after_track_change(&mut self, before: Option<String>) {
        // The player does not name the album; the open track list may.
        if let (Some(track), View::Tracks(v)) = (&mut self.now.track, &self.view)
            && track.album_uri.is_none()
            && let Some(listed) = v.tracks.iter().find(|t| t.uri == track.uri)
        {
            track.album_uri = Some(listed.album_uri.clone()).filter(|uri| !uri.is_empty());
            if track.artist_uris.is_empty() {
                track.artist_uris = listed.artist_uris.clone();
            }
        }
        if before.as_deref() != self.now.track.as_ref().map(|t| t.uri.as_str()) {
            self.track_changed_at = Instant::now();
            self.check_queue();
        }
        let Some(track) = &self.now.track else { return };
        if before.as_deref() == Some(track.uri.as_str()) {
            return;
        }
        self.lyrics = Some(LyricsState { uri: track.uri.clone(), result: None });
        self.lyrics_scroll = 0.0;
        self.loader.lyrics(track);
        if !self.saved.contains_key(&track.uri) {
            self.loader.check_saved(vec![track.uri.clone()]);
        }
    }

    /// Returns whether the screen needs a redraw.
    fn on_data(&mut self, event: DataEvent) -> bool {
        // Decoding covers and pages allocates a lot for a moment. Each such load pushes the
        // trim back, so a burst ends in one.
        if matches!(
            event,
            DataEvent::Image { .. }
                | DataEvent::Cover { .. }
                | DataEvent::LargeCover { .. }
                | DataEvent::Home { .. }
                | DataEvent::Artist { .. }
                | DataEvent::Tracks { .. }
        ) {
            self.trim_due = Some(Instant::now() + TRIM_DELAY);
        }
        match event {
            DataEvent::Me(result) => match result {
                Ok(user) => {
                    // The data cache belongs to whoever was logged in last time.
                    if self.me.as_ref().is_some_and(|me| me.id != user.id) {
                        self.forget_library();
                    }
                    self.me = Some(user);
                    self.data_dirty = true;
                }
                Err(err) => self.error(err),
            },
            DataEvent::Playlists(result) => {
                self.playlists_loading = false;
                match result {
                    Ok(playlists) => self.set_playlists(playlists),
                    Err(err) => self.error(err),
                }
            }
            DataEvent::LikedCount(result) => {
                self.library_part_done();
                match result {
                    Ok(total) => {
                        self.liked_total = Some(total);
                        self.data_dirty = true;
                    }
                    Err(err) => tracing::warn!("liked songs: {err}"),
                }
            }
            DataEvent::Albums(result) => {
                self.library_part_done();
                match result {
                    Ok(list) => self.set_albums(list),
                    Err(err) => tracing::warn!("albums: {err}"),
                }
            }
            DataEvent::Artists(result) => {
                self.library_part_done();
                match result {
                    Ok(list) => self.set_artists(list),
                    Err(err) => tracing::warn!("artists: {err}"),
                }
            }
            DataEvent::Shows(result) => {
                self.library_part_done();
                match result {
                    Ok(list) => self.set_shows(list),
                    Err(err) => tracing::warn!("shows: {err}"),
                }
            }
            DataEvent::Artist { view, result } => {
                let Some(View::Artist(v)) = self.view_mut(view) else { return false };
                let page = match result {
                    Ok(page) => page,
                    Err(err) => {
                        v.error = Some(err);
                        return true;
                    }
                };
                let uris = page.top_tracks.iter().map(|t| t.track.uri.clone()).collect();
                let (uri, followed) = (page.uri.clone(), page.saved);
                v.set_page(page);
                self.saved.insert(uri, followed);
                self.loader.check_saved(uris);
            }
            DataEvent::Home { view, result } => {
                if let Ok(home) = &result {
                    self.home_cache = Some(home.clone());
                    self.home_fetched_at = Some(Instant::now());
                    self.data_dirty = true;
                }
                let Some(View::Home(v)) = self.view_mut(view) else { return false };
                match result {
                    Ok(home) => v.set_home(home),
                    Err(err) => {
                        v.loading = false;
                        v.error = Some(err);
                    }
                }
            }
            // Loads still under way when cover art was turned off.
            DataEvent::Image { .. } | DataEvent::Cover { .. } | DataEvent::LargeCover { .. } if !self.prefs.covers => {
                return false;
            }
            // Encoded for the protocol used before a change in Settings: loaded again when drawn.
            DataEvent::Image { key, result: Ok(picture) } if picture.kind() != self.loader.image_kind() => {
                if matches!(self.images.get(&key), Some(ImageSlot::Loading(_))) {
                    self.images.remove(&key);
                    self.image_used.remove(&key);
                }
            }
            DataEvent::Image { key, result } => {
                let slot = match result {
                    Ok(picture) => ImageSlot::Ready(picture),
                    Err(err) => {
                        tracing::warn!("image {key}: {err}");
                        ImageSlot::Failed(Instant::now())
                    }
                };
                self.images.insert(key, slot);
            }
            DataEvent::ImageSkipped { key } => {
                if matches!(self.images.get(&key), Some(ImageSlot::Loading(_))) {
                    self.images.remove(&key);
                    self.image_used.remove(&key);
                }
            }
            DataEvent::Tracks { view, page } => self.on_tracks(view, page),
            DataEvent::Search { view, query, result } => {
                let Some(View::Search(v)) = self.view_mut(view) else { return false };
                if v.query.trim() != query {
                    return false;
                }
                v.loading = false;
                let uris = match result {
                    Ok(results) => {
                        v.error = None;
                        let uris = results.tracks.iter().map(|t| t.uri.clone()).collect();
                        v.set_results(query, results);
                        uris
                    }
                    Err(err) => {
                        v.error = Some(err);
                        Vec::new()
                    }
                };
                self.loader.check_saved(uris);
            }
            DataEvent::Cover { url, result } => {
                let current = self.now.track.as_ref().and_then(|t| t.cover_url.as_deref());
                let stale = result.as_ref().is_ok_and(|(picture, _)| picture.kind() != self.loader.image_kind());
                if current != Some(url.as_str()) || stale {
                    return false;
                }
                match result {
                    Ok((picture, accent)) => {
                        if let Some(old) = self.now.cover.replace(Cover { url, picture }) {
                            self.terminal_out.extend(old.picture.terminal_delete());
                        }
                        self.cover_accent = Some(accent);
                        if self.prefs.accent == AccentChoice::Cover {
                            if self.settings.low_power {
                                self.accent.jump(accent);
                            } else {
                                self.accent.set(accent);
                            }
                        }
                    }
                    Err(err) => tracing::warn!("cover {url}: {err}"),
                }
            }
            DataEvent::Artwork { url, result } => {
                let Some(media) = &mut self.media else { return false };
                match result {
                    Ok(bytes) => media.set_artwork(url, bytes),
                    Err(err) => {
                        tracing::warn!("artwork {url}: {err}");
                        media.artwork_failed(url);
                    }
                }
            }
            DataEvent::LargeCover { url, size, result } => {
                if self.zen_cover_pending.as_ref().is_some_and(|(u, s)| *u == url && *s == size) {
                    self.zen_cover_pending = None;
                }
                let current = self.now.track.as_ref().and_then(|t| t.cover_url_large.as_deref());
                let wanted = self.zen && self.zen_cover_want == Some(size) && current == Some(url.as_str());
                let wanted = wanted && result.as_ref().is_ok_and(|picture| picture.kind() == self.loader.image_kind());
                match result {
                    Ok(picture) if wanted => {
                        if let Some(old) = self.zen_cover.replace(LargeCover { url, size, picture }) {
                            self.terminal_out.extend(old.picture.terminal_delete());
                        }
                    }
                    // For a size or track no longer shown: dropped before it is ever sent to the
                    // terminal, which would cost megabytes for nothing.
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!("large cover {url}: {err}");
                        self.zen_cover_failed = Some((url, size, Instant::now()));
                    }
                }
            }
            DataEvent::Lyrics { uri, result } => {
                if let Some(state) = &mut self.lyrics
                    && state.uri == uri
                {
                    state.result = Some(result);
                }
            }
            DataEvent::Saved(flags) => {
                // A fresher local toggle wins over a check that was already under way.
                let fresh: Vec<_> = flags.into_iter().filter(|(uri, _)| !self.save_in_progress(uri)).collect();
                // Most checks only confirm what is already known.
                let changed = fresh.iter().any(|(uri, saved)| self.saved.get(uri) != Some(saved));
                self.saved.extend(fresh);
                return changed;
            }
            DataEvent::SaveDone { uri, notice } => {
                self.saving.remove(&uri);
                if !self.pending_saves.contains_key(&uri) {
                    self.unsaved.remove(&uri);
                    self.schedule_reload(&uri);
                    self.notice(notice);
                }
            }
            DataEvent::SaveFailed { uri, saved, error } => {
                self.saving.remove(&uri);
                if !self.pending_saves.contains_key(&uri) {
                    self.saved.insert(uri.clone(), saved);
                    self.show_library_change(&uri, saved);
                    self.error(error);
                }
            }
            DataEvent::AddFailed { playlist_id, count, error } => {
                if let Some(playlist) = self.playlists.iter_mut().find(|p| p.id == playlist_id) {
                    playlist.total = playlist.total.saturating_sub(count);
                }
                self.error(error);
            }
            DataEvent::Playback(result) => {
                self.polling = false;
                // Started before the last remote command, it shows the state from before it.
                if self.poll_started < self.last_command {
                    return false;
                }
                match result {
                    Ok(playback) => return self.on_playback(playback),
                    Err(err) => {
                        tracing::warn!("playback state: {err}");
                        return false;
                    }
                }
            }
            DataEvent::RemoteDone { commands, result } => {
                self.remote_intent.answered(commands, result.is_ok());
                match result {
                    // Look again once the device has had a moment to apply it.
                    Ok(()) => self.next_poll = self.next_poll.min(Instant::now() + CONFIRM_DELAY),
                    Err(err) => {
                        self.cancel_start();
                        self.next_poll = Instant::now();
                        self.error(format!("Remote control: {err}"));
                    }
                }
            }
            DataEvent::Devices(result) => {
                let wanted = std::mem::take(&mut self.devices_wanted);
                match result {
                    Ok(devices) => {
                        self.devices_cache = Some(devices.clone());
                        if matches!(self.overlay, Some(Overlay::Devices { .. })) {
                            self.refresh_devices(devices);
                        } else if wanted {
                            self.open_devices(devices, false);
                        }
                    }
                    Err(err) => {
                        if let Some(Overlay::Devices { refreshing, .. }) = &mut self.overlay {
                            *refreshing = false;
                        }
                        self.error(format!("Devices: {err}"));
                    }
                }
            }
            DataEvent::Transferred { name, result } => match result {
                Ok(()) => {
                    self.notice(format!("Playing on {name}"));
                    self.next_poll = self.next_poll.min(Instant::now() + CONFIRM_DELAY);
                }
                Err(err) => {
                    self.transfer = None;
                    self.error(format!("Transfer: {err}"));
                }
            },
            DataEvent::QueueTracks { asked, tracks, reached } => {
                self.on_queue_tracks(asked, tracks, reached);
                return matches!(self.view, View::Queue(_));
            }
            DataEvent::Upcoming { playing, result } => self.on_upcoming(playing, result),
            DataEvent::SkippedTo(result) => self.on_skipped_to(result),
            DataEvent::DjSuggestions(result) => {
                let Some(Overlay::Dj { picker, suggestions, loading }) = &mut self.overlay else { return true };
                *loading = false;
                match result {
                    Ok(fresh) => {
                        let selected = picker.selected().copied();
                        *suggestions = fresh;
                        picker.set_items(dj_items(&picker.query, suggestions), |&choice| Some(choice) == selected);
                    }
                    // Without suggestions, the panel still asks and jumps.
                    Err(err) => tracing::warn!("DJ suggestions: {err}"),
                }
            }
            // DJ's answer speaks for itself, seconds later.
            DataEvent::DjAsked(Ok(())) => {}
            DataEvent::DjAsked(Err(err)) => {
                self.cancel_start();
                self.error(format!("DJ: {err}"));
            }
            DataEvent::UpdateAvailable(release) => self.on_update_available(release),
            DataEvent::UpdateProgress(percent) => {
                if let Some(Update::Installing { percent: shown, .. }) = &mut self.update {
                    *shown = Some(percent);
                }
            }
            DataEvent::UpdateInstalled(binary) => {
                if let Some(Update::Installing { release, .. }) = self.update.take() {
                    self.notice(format!("cloofy {} is installed · U restarts into it", release.version()));
                    self.update = Some(Update::Installed { release, binary });
                }
            }
            DataEvent::UpdateFailed(err) => {
                if let Some(Update::Installing { release, .. }) = self.update.take() {
                    self.update = Some(Update::Available(release));
                }
                // The status line has one row.
                self.error(format!("Update failed: {}", err.split_whitespace().collect::<Vec<_>>().join(" ")));
            }
            DataEvent::Notice(text) => self.notice(text),
            DataEvent::Failed(text) => self.error(text),
        }
        true
    }

    /// Counts down the library parts loading at startup; later reloads don't show a spinner.
    fn library_part_done(&mut self) {
        self.library_pending = self.library_pending.saturating_sub(1);
        self.library_loading = self.library_pending > 0;
    }

    fn on_tracks(&mut self, view: u64, page: Result<Page<Track>, String>) {
        // A page may predate a like or unlike that Spotify has not confirmed yet.
        let unconfirmed: HashMap<String, bool> =
            self.pending_saves.keys().chain(&self.saving).map(|uri| (uri.clone(), self.is_saved(uri))).collect();
        let Some(View::Tracks(v)) = self.view_mut(view) else { return };
        let page = match page {
            Ok(page) => page,
            Err(err) => {
                v.loading = false;
                v.fetch = None;
                // A failed refresh leaves the songs on screen as they were.
                match v.incoming.take() {
                    Some(_) => tracing::warn!("refreshing a track list: {err}"),
                    None => v.error = Some(err),
                }
                return;
            }
        };
        let liked = matches!(v.source, TrackSource::Liked);
        let mut items = page.items;
        if liked {
            items.retain(|t| unconfirmed.get(&t.uri) != Some(&false));
        }
        let uris: Vec<String> = items.iter().map(|t| t.uri.clone()).collect();
        let done = page.next_offset.is_none();
        match &mut v.incoming {
            Some(incoming) => incoming.extend(items),
            None => {
                v.tracks.extend(items);
                v.total = Some(page.total);
                v.loading = !done;
            }
        }
        if done {
            if let Some(mut fresh) = v.incoming.take() {
                if liked {
                    // Songs liked meanwhile may be missing from the fresh copy.
                    let missing: Vec<Track> = v
                        .tracks
                        .iter()
                        .filter(|t| unconfirmed.get(&t.uri) == Some(&true) && !fresh.iter().any(|f| f.uri == t.uri))
                        .cloned()
                        .collect();
                    fresh.splice(0..0, missing);
                }
                v.total = Some(page.total.max(fresh.len() as u32));
                v.replace_tracks(fresh);
            }
            v.fetched_at = Some(Instant::now());
            v.fetch = None;
        }
        if liked {
            self.saved.extend(uris.into_iter().map(|uri| (uri, true)));
        } else {
            self.loader.check_saved(uris);
        }
    }

    /// Advances timers. Returns whether anything visible changed.
    pub fn on_tick(&mut self) -> bool {
        let mut redraw = false;

        if self.settings_checked.elapsed() >= SETTINGS_INTERVAL {
            self.settings_checked = Instant::now();
            self.system_settings = cloofy_os::settings::current();
            let settings = self.prefs.apply(self.system_settings);
            redraw |= settings != self.settings;
            self.settings = settings;
        }

        if let View::Search(search) = &mut self.view
            && search.typed_at.is_some_and(|t| t.elapsed() >= SEARCH_DEBOUNCE)
        {
            search.typed_at = None;
            let query = search.query.trim().to_owned();
            if query.is_empty() {
                search.entries.clear();
                search.searched.clear();
            } else if query != search.searched {
                search.loading = true;
                self.loader.search(search.id, query);
            }
            redraw = true;
        }

        self.flush_saves();
        self.reload_library();
        if self.last_save.elapsed() >= STATE_SAVE_INTERVAL {
            self.save_state();
            self.save_open_tracks(true);
        }
        if self.data_dirty && self.last_cache_save.elapsed() >= CACHE_SAVE_DELAY {
            self.save_library(true);
        }
        if self.last_eviction.elapsed() >= EVICTION_INTERVAL {
            self.evict_images();
        }
        if self.trim_due.is_some_and(|due| Instant::now() >= due) {
            self.trim_due = None;
            crate::release_freed_memory();
        }

        if let Some((_, _, at)) = self.pressed {
            if at.elapsed() >= PRESS_FLASH {
                self.pressed = None;
            }
            // One more frame after the flash ends clears it.
            redraw = true;
        }

        if self.status.as_ref().is_some_and(|s| s.since.elapsed() >= STATUS_TTL) {
            self.status = None;
            redraw = true;
        }

        if let Some(volume) = self.volume_sync.expire() {
            self.control(|c| c.set_volume(volume));
        }
        if self.transfer.as_ref().is_some_and(|t| t.since.elapsed() >= TRANSFER_TIMEOUT) {
            self.transfer = None;
            redraw = true;
        }
        if self.now.start_failed() {
            // Spirc only logs a failed load; another device's state comes with the next poll.
            let failed_here = self.now.pending.as_ref().is_some_and(|p| !p.skip) && !self.remote_active;
            self.cancel_start();
            if failed_here {
                self.error("Playback didn't start.".into());
            }
            redraw = true;
        }

        if self.player().is_some_and(Player::supports_devices) && !self.polling && Instant::now() >= self.next_poll {
            self.polling = true;
            self.poll_started = Instant::now();
            let interval = if self.last_push.is_some() { PUSHED_POLL_INTERVAL } else { POLL_INTERVAL };
            self.next_poll = Instant::now() + interval;
            self.loader.playback(self.active);
        }

        if self.upcoming_due.is_some_and(|due| Instant::now() >= due) {
            self.ask_upcoming();
            redraw = true;
        }

        if self.zen {
            self.request_zen_cover();
            redraw |= self.scroll_lyrics();
        }

        let frame = self.frame();
        if frame != self.last_frame && self.is_animating() {
            self.last_frame = frame;
            redraw = true;
        }
        redraw || self.accent.is_fading() || self.effects.is_running()
    }

    fn is_animating(&self) -> bool {
        let loading = match &self.view {
            View::Home(v) => v.loading && v.home.is_none(),
            View::Library(_) | View::Settings(_) => false,
            View::Artist(v) => v.page.is_none() && v.error.is_none(),
            View::Tracks(v) => v.loading,
            View::Search(v) => v.loading,
            View::Queue(v) => v.loading,
        };
        let lyrics_loading = self.zen && self.lyrics.as_ref().is_some_and(|l| l.result.is_none());
        self.now.playing
            || self.now.pending.is_some()
            || self.transfer.is_some()
            || matches!(self.overlay, Some(Overlay::Devices { refreshing: true, .. }))
            || loading
            || lyrics_loading
            || self.playlists_loading
            || self.library_loading
            || (!self.is_connected() && self.player_error.is_none())
    }

    /// Requests the big cover once the Now Playing layout knows its size.
    fn request_zen_cover(&mut self) {
        if !self.prefs.covers {
            return;
        }
        let (Some(size), Some(track)) = (self.zen_cover_want, &self.now.track) else { return };
        let Some(url) = track.cover_url_large.clone() else { return };
        if self.zen_cover_want_since.is_none_or(|(wanted, _)| wanted != size) {
            self.zen_cover_want_since = Some((size, Instant::now()));
        }
        let shown = self.zen_cover.as_ref().is_some_and(|c| c.url == url);
        let loaded = shown && self.zen_cover.as_ref().is_some_and(|c| c.size == size);
        // One cover per track at a time: while a resize goes on, a request for an older size is
        // let finish (and dropped) rather than joined by one for every size passed through.
        let pending = self.zen_cover_pending.as_ref().is_some_and(|(u, _)| *u == url);
        let failed = self
            .zen_cover_failed
            .as_ref()
            .is_some_and(|(u, s, at)| *u == url && *s == size && at.elapsed() < ZEN_COVER_RETRY);
        // A new track or a first look loads at once; after a resize, the cover at the old size
        // stays until the window settles.
        let settled = self.zen_cover_want_since.is_some_and(|(_, since)| since.elapsed() >= ZEN_COVER_SETTLE);
        if loaded || pending || failed || (shown && !settled) {
            return;
        }
        self.zen_cover_pending = Some((url.clone(), size));
        self.loader.large_cover(url, size);
    }

    /// Eases the lyrics towards the current line. Returns whether they moved.
    fn scroll_lyrics(&mut self) -> bool {
        let target = self
            .lyrics
            .as_ref()
            .and_then(|l| l.result.as_ref())
            .and_then(|r| r.as_ref().ok())
            .and_then(|l| l.as_ref())
            .and_then(|l| l.current_line(self.now.position_ms()))
            .map_or(0.0, |line| line as f32);
        let distance = target - self.lyrics_scroll;
        if distance.abs() < 0.02 || self.calm() {
            self.lyrics_scroll = target;
            // A jump needs drawing; the last tiny step of a glide does not.
            return distance.abs() >= 0.02;
        }
        self.lyrics_scroll += distance * 0.25;
        true
    }

    // ── Keys ─────────────────────────────────────────────────────────────────────────────

    fn on_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::F(12) {
            self.snapshot_requested = true;
            return;
        }
        if self.overlay.is_some() {
            if Command::from_key(&key) == Some(Command::Quit) {
                self.quit = true;
            } else if matches!(self.overlay, Some(Overlay::Help)) {
                self.overlay = None;
            } else if matches!(self.overlay, Some(Overlay::Menu(_))) {
                self.on_menu_key(key);
            } else {
                self.on_picker_key(key);
            }
            return;
        }
        if self.focus == Focus::SearchInput && !self.zen {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match Command::from_key(&key) {
                Some(command) if ctrl => self.run(command),
                _ => self.on_search_input(key),
            }
            return;
        }
        if let Some(command) = Command::from_key(&key) {
            self.run(command);
            return;
        }
        if self.zen {
            if key.code == KeyCode::Esc {
                self.toggle_zen();
            }
            return;
        }
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = if self.focus == Focus::Nav { Focus::Content } else { Focus::Nav };
            }
            _ => match self.focus {
                Focus::Nav => self.on_nav_key(key),
                Focus::Content => self.on_content_key(key),
                Focus::SearchInput => {}
            },
        }
    }

    fn on_nav_key(&mut self, key: KeyEvent) {
        let len = self.nav_len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.nav.move_by(-1, len),
            KeyCode::Down | KeyCode::Char('j') => self.nav.move_by(1, len),
            KeyCode::Home | KeyCode::Char('g') => self.nav.selected = 0,
            KeyCode::End | KeyCode::Char('G') => self.nav.selected = len.saturating_sub(1),
            KeyCode::Enter | KeyCode::Char('l') => self.open_nav_item(),
            KeyCode::Left => self.cycle_filter(-1),
            KeyCode::Right => self.cycle_filter(1),
            _ => {}
        }
    }

    fn on_content_key(&mut self, key: KeyEvent) {
        if matches!(self.view, View::Settings(_)) {
            return self.on_settings_key(key);
        }
        if let View::Artist(artist) = &mut self.view {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => artist.move_row(-1),
                KeyCode::Down | KeyCode::Char('j') => artist.move_row(1),
                KeyCode::Right | KeyCode::Char('l') => {
                    artist.move_item(1);
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if !artist.move_item(-1) {
                        self.focus = Focus::Nav;
                    }
                }
                KeyCode::Home | KeyCode::Char('g') => artist.move_row(isize::MIN / 2),
                KeyCode::End | KeyCode::Char('G') => artist.move_row(isize::MAX / 2),
                KeyCode::Enter => self.activate(),
                KeyCode::Char('p') => {
                    let uri = artist.uri.clone();
                    self.play(uri, None);
                }
                KeyCode::Char('F') => {
                    let (uri, name) = (artist.uri.clone(), artist.name.clone());
                    self.toggle_library(uri, name);
                }
                KeyCode::Esc => self.back(),
                _ => {}
            }
            return;
        }
        if let View::Library(library) = &mut self.view {
            let columns = library.columns.max(1) as isize;
            let delta = match key.code {
                KeyCode::Left | KeyCode::Char('h') if library.cursor % library.columns.max(1) == 0 => {
                    self.focus = Focus::Nav;
                    return;
                }
                KeyCode::Left | KeyCode::Char('h') => -1,
                KeyCode::Right | KeyCode::Char('l') => 1,
                KeyCode::Up | KeyCode::Char('k') => -columns,
                KeyCode::Down | KeyCode::Char('j') => columns,
                KeyCode::Home | KeyCode::Char('g') => isize::MIN / 2,
                KeyCode::End | KeyCode::Char('G') => isize::MAX / 2,
                KeyCode::Char('<') => return self.cycle_page_filter(-1),
                KeyCode::Char('>') => return self.cycle_page_filter(1),
                KeyCode::Enter => return self.activate(),
                KeyCode::Esc => return self.back(),
                _ => return,
            };
            let filter = library.filter;
            let len = self.library_entries_for(filter).len();
            if let View::Library(library) = &mut self.view {
                library.detached = false;
                library.cursor = (library.cursor as isize + delta).clamp(0, len.saturating_sub(1) as isize) as usize;
            }
            return;
        }
        if let View::Home(home) = &mut self.view {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => home.move_shelf(-1),
                KeyCode::Down | KeyCode::Char('j') => home.move_shelf(1),
                KeyCode::Right | KeyCode::Char('l') => {
                    home.move_item(1);
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if !home.move_item(-1) {
                        self.focus = Focus::Nav;
                    }
                }
                KeyCode::Home | KeyCode::Char('g') => home.move_shelf(isize::MIN / 2),
                KeyCode::End | KeyCode::Char('G') => home.move_shelf(isize::MAX / 2),
                KeyCode::Enter => self.activate(),
                KeyCode::Char('p') => self.play_home_item(),
                KeyCode::Esc => self.back(),
                _ => {}
            }
            return;
        }
        let page = self.page_height.max(1) as isize;
        let delta = match key.code {
            KeyCode::Up | KeyCode::Char('k') => Some(-1),
            KeyCode::Down | KeyCode::Char('j') => Some(1),
            KeyCode::PageUp => Some(-page),
            KeyCode::PageDown => Some(page),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(-page / 2),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(page / 2),
            KeyCode::Home | KeyCode::Char('g') => Some(isize::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => Some(isize::MAX / 2),
            _ => None,
        };
        if let Some(delta) = delta {
            self.move_content(delta);
            return;
        }
        match key.code {
            KeyCode::Enter => self.activate(),
            KeyCode::Left | KeyCode::Char('h') => self.focus = Focus::Nav,
            KeyCode::Esc => self.back(),
            KeyCode::Delete | KeyCode::Backspace if matches!(self.view, View::Queue(_)) => {
                self.remove_selected_from_queue();
            }
            _ => {}
        }
    }

    /// Mouse wheel: moves the view, not the selection.
    fn scroll_content(&mut self, delta: isize) {
        match &mut self.view {
            View::Tracks(v) => v.list.scroll(delta),
            View::Search(v) => {
                // Results with thumbnails are three rows tall; scroll by about as many rows.
                let rows = v.entry_rows.max(1) as usize;
                v.list.scroll(delta.signum() * delta.unsigned_abs().div_ceil(rows) as isize);
            }
            View::Queue(v) => {
                let rows = v.entry_rows.max(1) as usize;
                v.list.scroll(delta.signum() * delta.unsigned_abs().div_ceil(rows) as isize);
            }
            View::Home(v) => v.scroll_by(delta),
            View::Artist(v) => v.scroll_by(delta),
            View::Library(v) => {
                v.top = (v.top as isize + delta).max(0) as usize;
                v.detached = true;
            }
            View::Settings(v) => {
                v.top = (v.top as isize + delta).max(0) as usize;
                v.detached = true;
            }
        }
    }

    fn move_content(&mut self, delta: isize) {
        let library_len = match &self.view {
            View::Library(v) => self.library_entries_for(v.filter).len(),
            _ => 0,
        };
        match &mut self.view {
            View::Home(v) => v.move_shelf(delta.signum()),
            View::Library(v) => {
                let step = delta.signum() * v.columns.max(1) as isize;
                v.cursor = (v.cursor as isize + step).clamp(0, library_len.saturating_sub(1) as isize) as usize;
            }
            View::Artist(v) => v.move_row(delta.signum()),
            View::Tracks(v) => v.list.move_by(delta, v.tracks.len()),
            View::Search(v) => v.move_by(delta),
            View::Settings(v) => v.move_by(delta.signum()),
            View::Queue(v) => v.move_by(delta),
        }
    }

    fn on_search_input(&mut self, key: KeyEvent) {
        let View::Search(search) = &mut self.view else {
            self.focus = Focus::Content;
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('u' | 'w') if ctrl => search.query.clear(),
            KeyCode::Char(c) if !ctrl => search.query.push(c),
            KeyCode::Backspace => {
                search.query.pop();
            }
            KeyCode::Enter | KeyCode::Down | KeyCode::Tab => {
                if !search.entries.is_empty() {
                    self.focus = Focus::Content;
                }
                return;
            }
            KeyCode::Esc => {
                if search.entries.is_empty() {
                    self.back();
                } else {
                    self.focus = Focus::Content;
                }
                return;
            }
            _ => return,
        }
        search.typed_at = Some(Instant::now());
    }

    fn on_picker_key(&mut self, key: KeyEvent) {
        let outcome = match &mut self.overlay {
            Some(Overlay::Palette(picker)) => picker.on_key(key).map(Choice::Palette),
            Some(Overlay::AddToPlaylist { picker, .. }) => picker.on_key(key).map(Choice::Playlist),
            Some(Overlay::Devices { picker, .. }) => picker.on_key(key).map(Choice::Device),
            Some(Overlay::Dj { picker, suggestions, .. }) => {
                let query = picker.query.clone();
                let outcome = picker.on_key(key);
                // The first row asks for what was typed.
                if picker.query != query {
                    let query = picker.query.clone();
                    picker.set_items(dj_items(&query, suggestions), |_| false);
                }
                outcome.map(Choice::Dj)
            }
            _ => PickerOutcome::Stay,
        };
        self.apply_choice(outcome);
    }

    fn on_menu_key(&mut self, key: KeyEvent) {
        let Some(Overlay::Menu(menu)) = &mut self.overlay else { return };
        let len = menu.items.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => menu.cursor = menu.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => menu.cursor = (menu.cursor + 1).min(len.saturating_sub(1)),
            KeyCode::Enter => {
                let cursor = menu.cursor;
                self.choose_menu_item(cursor);
            }
            KeyCode::Esc | KeyCode::Char('.') => self.overlay = None,
            _ => {}
        }
    }

    fn choose_menu_item(&mut self, index: usize) {
        let Some(Overlay::Menu(menu)) = self.overlay.take() else { return };
        if let Some(&(action, _)) = menu.items.get(index) {
            self.run_menu(action, menu.target);
        }
    }

    fn apply_choice(&mut self, outcome: PickerOutcome<Choice>) {
        match outcome {
            PickerOutcome::Select(Choice::Palette(action)) => self.run_palette(action),
            PickerOutcome::Select(Choice::Playlist(index)) => self.add_to_playlist(index),
            PickerOutcome::Select(Choice::Device(index)) => self.transfer_to(index),
            PickerOutcome::Select(Choice::Dj(choice)) => self.choose_dj(choice),
            PickerOutcome::Close => self.overlay = None,
            PickerOutcome::Stay => {}
        }
    }

    // ── Mouse ────────────────────────────────────────────────────────────────────────────

    fn on_mouse(&mut self, mouse: MouseEvent) -> bool {
        let position = Position::new(mouse.column, mouse.row);
        self.mouse = Some(position);
        if let Some(target) = self.dragging {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    self.drag(target, mouse.column, false);
                    return true;
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.drag(target, mouse.column, true);
                    self.dragging = None;
                    return true;
                }
                _ => {}
            }
        }
        if matches!(mouse.kind, MouseEventKind::Moved | MouseEventKind::Drag(_)) {
            // Redraw only when the highlight moves to another target.
            let hovered = self.hit_at(position).filter(Hit::is_interactive);
            let changed = hovered != self.hovered;
            self.hovered = hovered;
            return changed;
        }
        let hit = self.hit_at(position);
        let in_overlay = matches!(hit, Some(Hit::OverlayArea | Hit::OverlayItem(_)));
        // Shift turns the wheel sideways, like in browsers.
        let sideways = mouse.modifiers.contains(KeyModifiers::SHIFT);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.overlay.is_some() && !in_overlay {
                    self.overlay = None;
                } else if let Some(hit @ (Hit::Seek { .. } | Hit::Volume { .. })) = hit {
                    self.dragging = Some(hit);
                    self.drag(hit, mouse.column, false);
                } else if let Some(hit) = hit {
                    // Lit at once, even where the result takes a moment (another device, a
                    // track that still loads).
                    if hit.is_interactive()
                        && let Some(&(area, _)) = self.hits.iter().rev().find(|(area, _)| area.contains(position))
                    {
                        self.pressed = Some((hit, area, Instant::now()));
                    }
                    self.click(hit, mouse.column);
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                self.overlay = None;
                // Names and play buttons lie on top of their row or card; the menu is theirs.
                let hit = hit.map(|hit| match hit {
                    Hit::Link(Link::TrackArtist { row, .. } | Link::TrackAlbum(row)) => Hit::Row(row),
                    Hit::CardPlay { shelf, item } => Hit::Card { shelf, item },
                    Hit::RowPlay(index) => Hit::Row(index),
                    hit => hit,
                });
                let anchor = (mouse.column, mouse.row);
                match hit {
                    Some(hit @ (Hit::Row(_) | Hit::Card { .. } | Hit::Nav(_))) => {
                        self.select(hit);
                        self.open_menu(anchor);
                    }
                    Some(
                        Hit::Cover
                        | Hit::LikePlaying
                        | Hit::Link(Link::PlayingArtist(_) | Link::PlayingAlbum | Link::PlayingContext),
                    ) => self.open_playing_menu(anchor),
                    _ => {}
                }
            }
            MouseEventKind::ScrollDown if sideways => self.scroll_sideways(hit, 1),
            MouseEventKind::ScrollUp if sideways => self.scroll_sideways(hit, -1),
            MouseEventKind::ScrollRight => self.scroll_sideways(hit, 1),
            MouseEventKind::ScrollLeft => self.scroll_sideways(hit, -1),
            MouseEventKind::ScrollDown => self.scroll(hit, SCROLL_STEP),
            MouseEventKind::ScrollUp => self.scroll(hit, -SCROLL_STEP),
            _ => return false,
        }
        true
    }

    /// Follows the mouse on the seek bar or volume slider; `release` ends the drag.
    fn drag(&mut self, target: Hit, column: u16, release: bool) {
        match target {
            Hit::Seek { x, width } => {
                let Some(track) = &self.now.track else { return };
                let ratio = column.saturating_sub(x) as f64 / width.max(1) as f64;
                let position = (ratio.clamp(0.0, 1.0) * track.duration_ms as f64) as u32;
                if release {
                    self.seek_preview = None;
                    self.seek_to(position);
                } else {
                    self.seek_preview = Some(position);
                }
            }
            Hit::Volume { x, width } => {
                let ratio = (column.saturating_sub(x) + 1) as f64 / width.max(1) as f64;
                let volume = (ratio.clamp(0.0, 1.0) * u16::MAX as f64) as u16;
                self.now.volume = volume;
                // Another device gets one request at the end, not one per mouse move.
                if release || !self.remote_active {
                    self.set_volume(volume);
                }
            }
            _ => {}
        }
    }

    /// Pages the shelf under the mouse sideways by one card.
    fn scroll_sideways(&mut self, hit: Option<Hit>, delta: isize) {
        let shelf = match hit {
            Some(Hit::Card { shelf, .. } | Hit::CardPlay { shelf, .. } | Hit::ShelfScroll { shelf, .. }) => shelf,
            _ => return,
        };
        match &mut self.view {
            View::Home(v) => v.scroll_shelf(shelf, delta),
            View::Artist(v) => v.scroll_shelf(shelf, delta),
            _ => {}
        }
    }

    /// Plays what the cursor is on, as the play button on a card does.
    fn play_selected(&mut self) {
        if let Some(target) = self.context_target() {
            self.run_menu(MenuAction::Play, target);
        }
    }

    /// The topmost click target under `position` from the last frame.
    pub fn hit_at(&self, position: Position) -> Option<Hit> {
        self.hits.iter().rev().find(|(area, _)| area.contains(position)).map(|(_, hit)| *hit)
    }

    /// Notes what the mouse is over in the frame just drawn. Scrolling, keys and loaded data
    /// move things under a resting mouse; without this, moving it onto the target it was over
    /// before would look like no change and leave the highlight behind.
    pub fn sync_hover(&mut self) {
        self.hovered = self.mouse.and_then(|position| self.hit_at(position)).filter(Hit::is_interactive);
    }

    /// Moves the cursor to a clicked row, card or sidebar entry.
    fn select(&mut self, hit: Hit) {
        match hit {
            Hit::Nav(index) => {
                self.focus = Focus::Nav;
                self.nav.selected = index;
            }
            Hit::Row(index) => {
                self.focus = Focus::Content;
                match &mut self.view {
                    View::Tracks(v) => v.list.selected = index,
                    View::Search(v) => {
                        if !matches!(v.entries.get(index), Some(SearchEntry::Header(_)) | None) {
                            v.list.selected = index;
                        }
                    }
                    View::Queue(v) => {
                        if !matches!(v.rows.get(index), Some(QueueRow::Header(_)) | None) {
                            v.list.selected = index;
                        }
                    }
                    View::Artist(v) => {
                        v.row = index;
                        v.item = 0;
                        v.detached = true;
                    }
                    View::Library(v) => {
                        v.cursor = index;
                        v.detached = true;
                    }
                    View::Settings(v) => {
                        v.selected = index;
                        v.detached = true;
                    }
                    View::Home(_) => {}
                }
            }
            Hit::Card { shelf, item } => {
                self.focus = Focus::Content;
                match &mut self.view {
                    View::Home(v) => {
                        v.shelf = shelf;
                        v.item = item;
                        v.detached = true;
                    }
                    View::Artist(v) => {
                        v.row = shelf;
                        v.item = item;
                        v.detached = true;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn click(&mut self, hit: Hit, column: u16) {
        let double = self.last_click.is_some_and(|(at, last)| last == hit && at.elapsed() < DOUBLE_CLICK);
        self.last_click = Some((Instant::now(), hit));
        match hit {
            Hit::Nav(_) => {
                self.select(hit);
                self.open_nav_item();
            }
            // Cards open on a single click, like in the official client; tracks select first
            // and play on a double click.
            Hit::Card { .. } => {
                self.select(hit);
                self.activate();
            }
            Hit::Row(_) => {
                self.select(hit);
                if double || self.opens_on_click() {
                    self.activate();
                }
            }
            Hit::CardPlay { shelf, item } => {
                self.select(Hit::Card { shelf, item });
                self.play_selected();
            }
            Hit::RowPlay(index) => {
                self.select(Hit::Row(index));
                self.play_selected();
            }
            Hit::ShelfScroll { shelf, forward } => {
                let page = |cards: usize| if forward { cards as isize } else { -(cards as isize) };
                match &mut self.view {
                    View::Home(v) => v.scroll_shelf(shelf, page(v.cards)),
                    View::Artist(v) => v.scroll_shelf(shelf, page(v.cards)),
                    _ => {}
                }
            }
            Hit::Link(link) => self.follow_link(link),
            Hit::Hint(action) => match action {
                HintAction::Run(command) => self.run(command),
                HintAction::Activate if self.focus == Focus::Nav => self.open_nav_item(),
                HintAction::Activate => self.activate(),
                HintAction::Back => {
                    if self.zen {
                        self.toggle_zen();
                    } else {
                        self.back();
                    }
                }
                HintAction::ToggleFocus => {
                    self.focus = if self.focus == Focus::Nav { Focus::Content } else { Focus::Nav };
                }
                HintAction::Key(code) => self.on_key(KeyEvent::new(code, KeyModifiers::NONE)),
            },
            Hit::Header(button) => self.header_button(button),
            Hit::LikePlaying => {
                if let Some(track) = &self.now.track {
                    let (uri, name) = (track.uri.clone(), track.name.clone());
                    self.toggle_library(uri, name);
                }
            }
            Hit::SearchInput => self.run(Command::Search),
            Hit::PageChip(filter) => self.set_page_filter(filter),
            Hit::Chip(filter) => {
                let filter = if self.library_filter == filter { LibraryFilter::All } else { filter };
                self.set_filter(filter);
            }
            Hit::Command(command) => self.run(command),
            Hit::Seek { .. } | Hit::Volume { .. } => self.drag(hit, column, true),
            Hit::Cover => self.toggle_zen(),
            Hit::SettingStep { index, forward } => {
                self.select(Hit::Row(index));
                if let View::Settings(v) = &self.view {
                    let setting = v.setting();
                    self.change_setting(setting, forward);
                }
            }
            Hit::OverlayItem(position) if matches!(self.overlay, Some(Overlay::Menu(_))) => {
                self.choose_menu_item(position);
            }
            Hit::OverlayItem(position) => {
                let outcome = match &self.overlay {
                    Some(Overlay::Palette(picker)) => picker.select(position).map(Choice::Palette),
                    Some(Overlay::AddToPlaylist { picker, .. }) => picker.select(position).map(Choice::Playlist),
                    Some(Overlay::Devices { picker, .. }) => picker.select(position).map(Choice::Device),
                    Some(Overlay::Dj { picker, .. }) => picker.select(position).map(Choice::Dj),
                    _ => PickerOutcome::Stay,
                };
                self.apply_choice(outcome);
            }
            Hit::NavArea | Hit::ContentArea | Hit::OverlayArea => {}
        }
    }

    /// Rows that stand for a page rather than a track: library cards and search results
    /// other than tracks.
    fn opens_on_click(&self) -> bool {
        match &self.view {
            View::Library(_) => true,
            View::Search(v) => !matches!(v.selected(), Some(SearchEntry::Track(_)) | None),
            _ => false,
        }
    }

    fn scroll(&mut self, hit: Option<Hit>, delta: isize) {
        match hit {
            Some(Hit::OverlayArea | Hit::OverlayItem(_)) => match &mut self.overlay {
                Some(Overlay::Menu(menu)) => {
                    menu.cursor =
                        (menu.cursor as isize + delta.signum()).clamp(0, menu.items.len() as isize - 1) as usize;
                }
                Some(Overlay::Palette(picker)) => picker.cursor.move_by(delta.signum(), picker.matches.len()),
                Some(Overlay::AddToPlaylist { picker, .. }) | Some(Overlay::Devices { picker, .. }) => {
                    picker.cursor.move_by(delta.signum(), picker.matches.len())
                }
                Some(Overlay::Dj { picker, .. }) => picker.cursor.move_by(delta.signum(), picker.matches.len()),
                _ => {}
            },
            Some(Hit::NavArea | Hit::Nav(_) | Hit::Chip(_)) => self.nav.scroll(delta.signum()),
            Some(
                Hit::ContentArea
                | Hit::Row(_)
                | Hit::Card { .. }
                | Hit::CardPlay { .. }
                | Hit::RowPlay(_)
                | Hit::ShelfScroll { .. }
                | Hit::PageChip(_)
                | Hit::Header(_)
                | Hit::SettingStep { .. },
            ) => self.scroll_content(delta),
            _ => {}
        }
    }

    // ── Commands ─────────────────────────────────────────────────────────────────────────

    pub fn run(&mut self, command: Command) {
        match command {
            Command::Home => {
                self.zen = false;
                if !matches!(self.view, View::Home(_)) {
                    self.open_home();
                }
            }
            Command::Library => {
                self.zen = false;
                if !matches!(self.view, View::Library(_)) {
                    self.open_library_view();
                }
            }
            Command::Quit => self.quit = true,
            Command::Update => self.run_update(),
            Command::PlayPause if self.remote_active => {
                let playing = !self.now.playing;
                self.now.set_playing(playing);
                self.remote_intent.playing = Some(playing);
                self.remote(if playing { Remote::Play } else { Remote::Pause });
            }
            Command::PlayPause if self.resume.is_some() && self.is_connected() => {
                if let Some(last) = self.resume.take() {
                    let uri = last.uri.clone();
                    if self.start_on(&uri, |p| p.resume(last.context, last.uri, last.position_ms)) {
                        self.now.start(Some(uri));
                    }
                }
            }
            Command::PlayPause if self.idle() && self.is_connected() => self.play_shown(),
            Command::PlayPause => {
                let has_track = self.now.track.is_some();
                self.control(|p| p.toggle(has_track));
            }
            Command::Next | Command::Previous | Command::Shuffle | Command::Repeat
                if self.idle() && self.is_connected() =>
            {
                self.notice("Nothing is playing here yet · press space to start".into());
            }
            Command::Next => self.skip(true),
            Command::Previous => self.skip(false),
            Command::SeekBack => self.seek_by(-SEEK_STEP_MS),
            Command::SeekForward => self.seek_by(SEEK_STEP_MS),
            Command::VolumeUp => self.change_volume(1),
            Command::VolumeDown => self.change_volume(-1),
            Command::Shuffle => {
                let on = !self.now.shuffle;
                if self.remote_active {
                    self.now.shuffle = on;
                    self.remote_intent.shuffle = Some(on);
                    self.remote(Remote::Shuffle(on));
                } else {
                    self.control(|p| p.shuffle(on));
                }
            }
            Command::Repeat => {
                let next = match self.now.repeat {
                    Repeat::Off => Repeat::Context,
                    Repeat::Context => Repeat::Track,
                    Repeat::Track => Repeat::Off,
                };
                if self.remote_active {
                    self.now.repeat = next;
                    self.remote_intent.repeat = Some(next);
                    self.remote(Remote::Repeat(next));
                } else {
                    self.control(|p| p.repeat(next));
                }
            }
            Command::Search => {
                if self.zen {
                    self.toggle_zen();
                }
                self.open_search();
            }
            Command::NowPlaying => self.toggle_zen(),
            Command::ShowQueue => self.toggle_queue(),
            Command::Like => self.toggle_saved(),
            Command::Queue => match self.target() {
                Some(target) if target.is_track => self.queue(target.uri, target.name),
                Some(_) => self.error("Only songs can be added to the queue.".into()),
                None => self.error("No song selected.".into()),
            },
            Command::AddToPlaylist => self.open_playlist_picker(),
            Command::Menu => {
                let area = self.content_area;
                self.open_menu((area.x + area.width / 3, area.y + area.height / 3));
            }
            Command::Dj if self.dj_playing() => self.open_dj(),
            Command::Dj => match self.service().and_then(backend::dj_context) {
                Some(dj) => {
                    self.notice("Starting DJ…".into());
                    self.play(dj.into(), None);
                }
                None => self.error(format!("{} has no DJ", self.service_name())),
            },
            Command::Crossfade if !self.supports_crossfade() => {
                self.error(format!("{} doesn't crossfade", self.service_name()));
            }
            Command::Crossfade => {
                self.change_setting(Setting::Crossfade, true);
                let text = match self.prefs.crossfade {
                    0 => "Crossfade off".to_owned(),
                    seconds => format!("Crossfade: {seconds} s"),
                };
                self.notice(text);
            }
            Command::Settings => self.open_settings(),
            Command::Devices if !self.supports_devices() => {
                self.error(format!("{} doesn't play on other devices", self.service_name()));
            }
            Command::Devices => {
                self.loader.devices(self.service());
                match self.devices_cache.clone().filter(|d| !d.is_empty()) {
                    // The last list opens at once and updates when the fresh one arrives.
                    Some(devices) => self.open_devices(devices, true),
                    None => {
                        self.devices_wanted = true;
                        self.notice("Finding devices…".into());
                    }
                }
            }
            Command::Palette => self.open_palette(),
            Command::Help => self.open_overlay(Overlay::Help),
        }
    }

    fn run_palette(&mut self, action: PaletteAction) {
        self.overlay = None;
        match action {
            PaletteAction::Run(command) => self.run(command),
            PaletteAction::OpenHome => {
                self.zen = false;
                self.select_nav(NavItem::Home);
                self.open_home();
            }
            PaletteAction::OpenLiked => {
                self.zen = false;
                self.open_library_entry(LibraryEntry::Liked);
            }
            PaletteAction::OpenPlaylist(index) => {
                self.zen = false;
                self.open_library_entry(LibraryEntry::Playlist(index));
            }
        }
    }

    fn open_palette(&mut self) {
        let mut items: Vec<_> = Command::PALETTE
            .iter()
            .filter(|&&command| self.offers(command))
            .map(|&command| PickItem {
                label: match command {
                    Command::Dj if self.dj_playing() => "Ask DJ…".to_owned(),
                    Command::Update => self.update_label(),
                    _ => command.label().to_owned(),
                },
                detail: command.key().to_owned(),
                image_url: None,
                value: PaletteAction::Run(command),
            })
            .collect();
        // An update waiting to be installed or started comes first.
        if matches!(self.update, Some(Update::Available(_) | Update::Installed { .. }))
            && let Some(index) = items.iter().position(|item| matches!(item.value, PaletteAction::Run(Command::Update)))
        {
            let item = items.remove(index);
            items.insert(0, item);
        }
        items.push(PickItem {
            label: "Home".into(),
            detail: "open".into(),
            image_url: None,
            value: PaletteAction::OpenHome,
        });
        items.push(PickItem {
            label: "Liked Songs".into(),
            detail: "open".into(),
            image_url: None,
            value: PaletteAction::OpenLiked,
        });
        items.extend(self.playlists.iter().enumerate().map(|(index, playlist)| PickItem {
            label: playlist.name.clone(),
            detail: "Playlist".into(),
            image_url: None,
            value: PaletteAction::OpenPlaylist(index),
        }));
        self.open_overlay(Overlay::Palette(Picker::new(items)));
    }

    fn open_playlist_picker(&mut self) {
        match self.target().filter(|t| t.is_track) {
            Some(target) => self.open_playlist_picker_for(target.uri),
            None => self.error("Select a song first.".into()),
        }
    }

    pub(super) fn open_playlist_picker_for(&mut self, uri: String) {
        let me = self.me.as_ref().map(|u| u.id.as_str());
        let items: Vec<_> = self
            .playlists
            .iter()
            .enumerate()
            .filter(|(_, p)| p.collaborative || Some(p.owner_id.as_str()) == me)
            .map(|(index, p)| PickItem {
                label: p.name.clone(),
                detail: format!("{} songs", p.total),
                image_url: p.image_url.clone(),
                value: index,
            })
            .collect();
        if items.is_empty() {
            return self.error("You don’t own any playlists yet.".into());
        }
        self.open_overlay(Overlay::AddToPlaylist { uris: vec![uri], picker: Picker::new(items) });
    }

    fn add_to_playlist(&mut self, index: usize) {
        let Some(Overlay::AddToPlaylist { uris, .. }) = self.overlay.take() else { return };
        if let Some(playlist) = self.playlists.get_mut(index) {
            playlist.total += uris.len() as u32;
            let text = format!("Added to “{}”", playlist.name);
            self.loader.add_to_playlist(playlist, uris);
            self.notice(text);
        }
    }

    /// Confirms at once; the request takes a moment and only reports back when it fails.
    pub(super) fn queue(&mut self, uri: String, name: String) {
        self.loader.queue(uri);
        self.notice(format!("“{name}” added to queue"));
    }

    fn open_overlay(&mut self, overlay: Overlay) {
        self.overlay = Some(overlay);
        self.overlay_fx_pending = true;
    }

    /// Called by the first draw of an overlay, which knows where it ended up.
    pub fn animate_overlay(&mut self, area: Rect) {
        let p = theme::palette();
        self.overlay_fx_pending = false;
        let effect = if self.settings.reduce_motion {
            fx::fade_from_fg(p.raised, (150, Interpolation::QuadOut))
        } else {
            fx::sweep_in(Motion::UpToDown, 6, 0, p.raised, (180, Interpolation::QuadOut))
        };
        self.animate(Fx::Overlay, effect.with_area(area));
    }

    fn toggle_saved(&mut self) {
        match self.target() {
            Some(target) => self.toggle_library(target.uri, target.name),
            None => self.error("Nothing selected.".into()),
        }
    }

    /// Likes/saves/follows or undoes it. The UI flips at once; the request follows shortly.
    pub(super) fn toggle_library(&mut self, uri: String, name: String) {
        let saved = !self.is_saved(&uri);
        self.saved.insert(uri.clone(), saved);
        self.show_library_change(&uri, saved);
        self.pending_saves.insert(uri, (saved, name, Instant::now()));
    }

    /// Sends settled library changes, one request per item at a time.
    fn flush_saves(&mut self) {
        let ready: Vec<String> = self
            .pending_saves
            .iter()
            .filter(|(uri, (_, _, at))| at.elapsed() >= SAVE_DEBOUNCE && !self.saving.contains(*uri))
            .map(|(uri, _)| uri.clone())
            .collect();
        for uri in ready {
            if let Some((saved, name, _)) = self.pending_saves.remove(&uri) {
                self.saving.insert(uri.clone());
                self.loader.set_saved(uri, name, saved);
            }
        }
    }

    /// Whether a library change for `uri` is still on its way to Spotify.
    fn save_in_progress(&self, uri: &str) -> bool {
        self.pending_saves.contains_key(uri) || self.saving.contains(uri)
    }

    /// The selected item in the content area, or else the playing track.
    fn target(&self) -> Option<Target> {
        let track = |uri: &str, name: &str, is_track| Target { uri: uri.into(), name: name.into(), is_track };
        if !self.zen && self.focus == Focus::Content {
            let selected = match &self.view {
                View::Home(v) => v.selected().map(|i| track(&i.uri, &i.title, false)),
                View::Library(v) => self
                    .library_entries_for(v.filter)
                    .get(v.cursor)
                    .and_then(|entry| self.library_target(*entry))
                    .map(|t| track(&t.uri, &t.name, false)),
                View::Artist(v) => v.selected().map(|selection| match selection {
                    ArtistSelection::Track(t) => track(&t.track.uri, &t.track.name, true),
                    ArtistSelection::Item(i) => track(&i.uri, &i.title, false),
                }),
                View::Tracks(v) => v.tracks.get(v.list.selected).map(|t| track(&t.uri, &t.name, true)),
                View::Search(v) => v.selected().and_then(|entry| match entry {
                    SearchEntry::Track(t) => Some(track(&t.uri, &t.name, true)),
                    SearchEntry::Album(a) => Some(track(&a.uri, &a.name, false)),
                    SearchEntry::Artist(a) => Some(track(&a.uri, &a.name, false)),
                    SearchEntry::Playlist(p) => Some(track(&p.uri, &p.name, false)),
                    SearchEntry::Header(_) => None,
                }),
                View::Settings(_) => None,
                View::Queue(v) => v.selected().and_then(QueueRow::track).map(|t| track(&t.uri, &t.name, true)),
            };
            if selected.is_some() {
                return selected;
            }
        }
        self.now.track.as_ref().map(|t| track(&t.uri, &t.name, true))
    }

    fn toggle_zen(&mut self) {
        let p = theme::palette();
        self.zen = !self.zen;
        let screen = self.screen;
        self.animate(Fx::Screen, fx::fade_from_fg(p.bg, (320, Interpolation::QuadOut)).with_area(screen));
    }

    /// Starts an animation; the next frame measures time from now. None in Low Power Mode.
    fn animate(&mut self, slot: Fx, effect: Effect) {
        if self.settings.low_power {
            return;
        }
        // Kitty images are drawn through placeholder cells whose color encodes the image;
        // recoloring them mid-animation would show the wrong picture. Sixel and iTerm2 images
        // are not recolored either, so that they keep counting as uncovered.
        let effect = effect.with_filter(CellFilter::eval_cell(|cell| {
            !cell.symbol().starts_with(KITTY_PLACEHOLDER) && cell.symbol() != INLINE_CELL
        }));
        self.effects.add_unique_effect(slot, effect);
        self.last_draw = Instant::now();
    }

    // ── Navigation ───────────────────────────────────────────────────────────────────────

    /// Filter chips worth showing: only kinds the library actually contains.
    pub fn library_filters(&self) -> Vec<LibraryFilter> {
        let mut filters = vec![LibraryFilter::Playlists];
        let kinds = [
            (LibraryFilter::Albums, self.albums.is_empty()),
            (LibraryFilter::Artists, self.artists.is_empty()),
            (LibraryFilter::Podcasts, self.shows.is_empty()),
        ];
        filters.extend(kinds.into_iter().filter(|(_, empty)| !empty).map(|(filter, _)| filter));
        filters
    }

    pub fn library_entries(&self) -> Vec<LibraryEntry> {
        self.library_entries_for(self.library_filter)
    }

    pub fn library_entries_for(&self, filter: LibraryFilter) -> Vec<LibraryEntry> {
        let mut entries = Vec::new();
        if filter.shows(LibraryFilter::Playlists) {
            entries.push(LibraryEntry::Liked);
            entries.extend((0..self.playlists.len()).map(LibraryEntry::Playlist));
        }
        if filter.shows(LibraryFilter::Albums) {
            entries.extend((0..self.albums.len()).map(LibraryEntry::Album));
        }
        if filter.shows(LibraryFilter::Artists) {
            entries.extend((0..self.artists.len()).map(LibraryEntry::Artist));
        }
        if filter.shows(LibraryFilter::Podcasts) {
            entries.extend((0..self.shows.len()).map(LibraryEntry::Show));
        }
        entries
    }

    /// The sidebar list; Home and Search are in the top bar.
    pub fn nav_items(&self) -> Vec<NavItem> {
        self.library_entries().into_iter().map(NavItem::Library).collect()
    }

    fn nav_len(&self) -> usize {
        NavItem::FIXED + self.library_entries().len()
    }

    fn select_nav(&mut self, item: NavItem) {
        if let Some(index) = self.nav_items().iter().position(|i| *i == item) {
            self.nav.selected = index;
        }
    }

    fn set_filter(&mut self, filter: LibraryFilter) {
        self.library_filter = filter;
        self.nav = ListCursor { selected: NavItem::FIXED, ..ListCursor::default() };
        self.nav.move_by(0, self.nav_len());
    }

    fn cycle_filter(&mut self, delta: isize) {
        let filters: Vec<_> = std::iter::once(LibraryFilter::All).chain(self.library_filters()).collect();
        let current = filters.iter().position(|f| *f == self.library_filter).unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(filters.len() as isize) as usize;
        self.set_filter(filters[next]);
    }

    /// Whether a sidebar entry is the view currently open.
    /// The playlist, album or artist the current track plays from.
    pub fn playing_context(&self) -> Option<&str> {
        self.now.track.as_ref()?;
        self.last_context.as_deref()
    }

    /// Whether cloofy itself plays DJ, so the DJ button moves it on to different picks.
    pub fn dj_playing(&self) -> bool {
        self.local_active && self.playing_context().is_some_and(backend::is_dj)
    }

    /// Track `row` of the open track list or search results.
    fn listed_track(&self, row: usize) -> Option<&Track> {
        match &self.view {
            View::Tracks(v) => v.tracks.get(row),
            View::Search(v) => match v.entries.get(row) {
                Some(SearchEntry::Track(track)) => Some(track),
                _ => None,
            },
            View::Queue(v) => v.rows.get(row).and_then(QueueRow::track),
            _ => None,
        }
    }

    fn follow_link(&mut self, link: Link) {
        let (uri, name) = match link {
            Link::TrackArtist { row, artist } => {
                let Some(track) = self.listed_track(row) else { return };
                match (track.artist_uris.get(artist), track.artists.get(artist)) {
                    (Some(uri), Some(name)) => (uri.clone(), name.clone()),
                    _ => return,
                }
            }
            Link::TrackAlbum(row) => {
                let Some(track) = self.listed_track(row).filter(|t| !t.album_uri.is_empty()) else { return };
                let (uri, name) = (track.album_uri.clone(), track.album.clone());
                let (artists, cover) = (track.artists.clone(), track.image_url.clone());
                return self.open_album(uri, name, artists, cover);
            }
            Link::PlayingArtist(artist) => {
                let Some(track) = &self.now.track else { return };
                match (track.artist_uris.get(artist), track.artists.get(artist)) {
                    (Some(uri), Some(name)) => (uri.clone(), name.clone()),
                    _ => return,
                }
            }
            Link::PlayingAlbum => {
                let Some(track) = &self.now.track else { return };
                let Some(uri) = track.album_uri.clone() else { return };
                let (name, artists, cover) = (track.album.clone(), track.artists.clone(), track.cover_url.clone());
                return self.open_album(uri, name, artists, cover);
            }
            Link::PlayingContext => {
                if let Some(uri) = self.last_context.clone() {
                    self.open_context(uri);
                }
                return;
            }
        };
        self.open_artist(uri, name);
    }

    /// Whether there is a page to go back to.
    pub fn can_go_back(&self) -> bool {
        !self.history.is_empty()
    }

    pub fn is_open(&self, item: NavItem) -> bool {
        let NavItem::Library(entry) = item else {
            return matches!((item, &self.view), (NavItem::Home, View::Home(_)) | (NavItem::Search, View::Search(_)));
        };
        let View::Tracks(view) = &self.view else { return false };
        match (entry, &view.source) {
            (LibraryEntry::Liked, TrackSource::Liked) => true,
            (LibraryEntry::Playlist(i), TrackSource::Playlist(p)) => {
                self.playlists.get(i).is_some_and(|q| q.id == p.id)
            }
            (LibraryEntry::Album(i), TrackSource::Album(a)) => self.albums.get(i).is_some_and(|b| b.id == a.id),
            _ => false,
        }
    }

    fn open_nav_item(&mut self) {
        let Some(item) = self.nav_items().get(self.nav.selected).copied() else { return };
        if self.is_open(item) {
            self.focus = if item == NavItem::Search { Focus::SearchInput } else { Focus::Content };
            return;
        }
        match item {
            NavItem::Home => self.open_home(),
            NavItem::Search => self.open_search(),
            NavItem::Library(entry) => self.open_library_entry(entry),
        }
    }

    /// Opens playlists and albums; artists and podcasts start playing.
    fn open_library_entry(&mut self, entry: LibraryEntry) {
        if !self.nav_items().contains(&NavItem::Library(entry)) {
            self.library_filter = LibraryFilter::All;
        }
        self.select_nav(NavItem::Library(entry));
        match entry {
            LibraryEntry::Liked => self.open_tracks(TrackSource::Liked),
            LibraryEntry::Playlist(i) => {
                if let Some(playlist) = self.playlists.get(i).cloned() {
                    self.open_tracks(TrackSource::Playlist(playlist));
                }
            }
            LibraryEntry::Album(i) => {
                if let Some(album) = self.albums.get(i).cloned() {
                    self.open_tracks(TrackSource::Album(album));
                }
            }
            LibraryEntry::Artist(i) => {
                if let Some(artist) = self.artists.get(i).cloned() {
                    self.open_artist(artist.uri, artist.name);
                }
            }
            LibraryEntry::Show(i) => {
                if let Some(show) = self.shows.get(i).cloned() {
                    self.play(show.uri, None);
                }
            }
        }
    }

    fn open_home(&mut self) {
        let id = self.next_id();
        let mut view = HomeView::new(id, self.home_cache.clone());
        if self.home_fetched_at.is_some_and(|at| at.elapsed() < HOME_FRESH) {
            view.loading = false;
        } else {
            self.loader.home(id);
        }
        self.push_view(View::Home(view));
        self.focus = Focus::Content;
    }

    /// A cover, or the drawn placeholder for items without one. Nothing with cover art off.
    pub fn cover(&mut self, url: Option<&str>, size: Size, round: bool) -> Option<&Picture> {
        self.image(url.unwrap_or(crate::art::PLACEHOLDER_COVER), size, round)
    }

    /// Starts drawing a frame.
    pub fn begin_frame(&mut self) {
        self.draws += 1;
        self.frame_started = Instant::now();
    }

    /// Finishes drawing a frame.
    pub fn end_frame(&mut self) {
        self.loader.frame_drawn(self.draws);
        // Covers on screen were requested while drawing, so the ones near them queue up after.
        let mut queue = std::mem::take(&mut self.prefetch);
        for (url, size, round) in queue.drain(..) {
            let key = image_key(&url, size);
            if self.images.contains_key(&key) {
                continue;
            }
            self.images.insert(key.clone(), ImageSlot::Loading(Arc::new(AtomicU64::new(self.draws))));
            // Counts as used once, so the cover is dropped again if it is never shown.
            self.image_used.insert(key.clone(), Instant::now());
            self.loader.prefetch_image(key, url, size, round);
        }
        self.prefetch = queue;
    }

    /// Asks for a cover that is likely shown soon, such as the next page of a shelf, so it is
    /// there when it scrolls into view. Loaded after the frame's visible covers.
    pub fn prefetch(&mut self, url: Option<&str>, size: Size, round: bool) {
        if !self.prefs.covers {
            return;
        }
        let url = url.unwrap_or(crate::art::PLACEHOLDER_COVER);
        self.prefetch.push((url.to_owned(), size, round));
    }

    /// What [`ImageProtocol::Auto`] draws with in this terminal, to show beside the choice.
    pub fn auto_image_protocol(&self) -> ImageProtocol {
        self.loader.auto_image_protocol()
    }

    /// Draws covers with `protocol` from now on, for the Settings page. Every cover is encoded
    /// again, from the disk cache; covers that loaded for the old protocol meanwhile are dropped.
    pub fn set_image_protocol(&mut self, protocol: ImageProtocol) {
        if protocol == self.loader.image_protocol() {
            return;
        }
        self.loader.set_image_protocol(protocol);
        for (_, slot) in self.images.drain() {
            if let ImageSlot::Ready(picture) = slot {
                self.terminal_out.extend(picture.terminal_delete());
            }
        }
        self.image_used.clear();
        if let Some(cover) = self.now.cover.take() {
            self.terminal_out.extend(cover.picture.terminal_delete());
            self.load_cover(cover.url);
        }
        if let Some(cover) = self.zen_cover.take() {
            self.terminal_out.extend(cover.picture.terminal_delete());
        }
        self.zen_cover_pending = None;
        self.zen_cover_failed = None;
    }

    /// The cover for `url` at `size`, loading it on first use.
    pub fn image(&mut self, url: &str, size: Size, round: bool) -> Option<&Picture> {
        if !self.prefs.covers {
            return None;
        }
        let key = image_key(url, size);
        self.image_used.insert(key.clone(), Instant::now());
        match self.images.get(&key) {
            Some(ImageSlot::Loading(drawn)) => drawn.store(self.draws, Ordering::Relaxed),
            Some(ImageSlot::Ready(_)) => {}
            Some(ImageSlot::Failed(at)) if at.elapsed() < IMAGE_RETRY => {}
            None | Some(ImageSlot::Failed(_)) => {
                let drawn = Arc::new(AtomicU64::new(self.draws));
                self.images.insert(key.clone(), ImageSlot::Loading(drawn.clone()));
                self.loader.image(key.clone(), url.to_owned(), size, round, drawn);
            }
        }
        match self.images.get(&key) {
            Some(ImageSlot::Ready(picture)) => Some(picture),
            _ => None,
        }
    }

    /// Drops images that have not been on screen for a while. Until drawn, each one holds its
    /// encoded pixels for the terminal, and they come back quickly from the disk cache.
    ///
    /// A drawn kitty image is different: cloofy keeps next to nothing of it, but kitty keeps
    /// its pixels until told to delete it, and drawing it again would mean another upload.
    /// Those stay until they take more than [`TERMINAL_IMAGE_BUDGET`] together; then the least
    /// recently drawn ones that are off screen are deleted from the terminal.
    fn evict_images(&mut self) {
        self.last_eviction = Instant::now();
        let mut stale = Vec::new();
        let mut held = 0;
        let mut off_screen = Vec::new();
        for (key, used) in &self.image_used {
            let Some(ImageSlot::Ready(picture)) = self.images.get(key) else { continue };
            if !picture.in_terminal() {
                if used.elapsed() >= IMAGE_TTL {
                    stale.push(key.clone());
                }
                continue;
            }
            held += picture.terminal_bytes;
            if *used < self.frame_started {
                off_screen.push((*used, key.clone(), picture.terminal_bytes));
            }
        }
        off_screen.sort_unstable_by_key(|(used, _, _)| *used);
        for (_, key, bytes) in off_screen {
            if held <= TERMINAL_IMAGE_BUDGET {
                break;
            }
            held -= bytes;
            stale.push(key);
        }
        if stale.is_empty() {
            return;
        }
        for key in &stale {
            if let Some(ImageSlot::Ready(picture)) = self.images.remove(key) {
                self.terminal_out.extend(picture.terminal_delete());
            }
            self.image_used.remove(key);
        }
        crate::release_freed_memory();
    }

    fn open_library_view(&mut self) {
        let id = self.next_id();
        let filter = self.library_filter;
        self.push_view(View::Library(LibraryView { id, filter, cursor: 0, columns: 1, top: 0, detached: false }));
        self.focus = Focus::Content;
    }

    /// Switches the filter tab of the full-page library.
    pub(crate) fn set_page_filter(&mut self, filter: LibraryFilter) {
        if let View::Library(v) = &mut self.view {
            v.filter = if v.filter == filter { LibraryFilter::All } else { filter };
            v.cursor = 0;
            v.top = 0;
        }
    }

    fn cycle_page_filter(&mut self, delta: isize) {
        let View::Library(v) = &self.view else { return };
        let filters: Vec<_> = std::iter::once(LibraryFilter::All).chain(self.library_filters()).collect();
        let current = filters.iter().position(|f| *f == v.filter).unwrap_or(0) as isize;
        let next = filters[(current + delta).rem_euclid(filters.len() as isize) as usize];
        if let View::Library(v) = &mut self.view {
            v.filter = next;
            v.cursor = 0;
            v.top = 0;
        }
    }

    fn open_search(&mut self) {
        if let View::Search(_) = self.view {
            self.focus = Focus::SearchInput;
            return;
        }
        let id = self.next_id();
        self.push_view(View::Search(SearchView {
            id,
            query: String::new(),
            searched: String::new(),
            entries: Vec::new(),
            loading: false,
            error: None,
            typed_at: None,
            list: ListCursor::default(),
            entry_rows: 1,
        }));
        self.focus = Focus::SearchInput;
    }

    fn open_tracks(&mut self, source: TrackSource) {
        if let View::Tracks(v) = &self.view
            && v.source.same(&source)
        {
            self.focus = Focus::Content;
            return;
        }
        let page_uri = match &source {
            TrackSource::Liked => None,
            TrackSource::Playlist(p) => Some(p.uri.clone()),
            TrackSource::Album(a) => Some(a.uri.clone()),
        };
        if let Some(uri) = page_uri.filter(|uri| !self.saved.contains_key(uri)) {
            self.loader.check_saved(vec![uri]);
        }
        let loaded = self.take_loaded_copy(&source).or_else(|| self.cached_copy(&source));
        self.retire_copies(&source);
        let view = match loaded {
            Some(view) if view.loading || view.fetched_at.is_some_and(|at| at.elapsed() < PAGE_FRESH) => view,
            Some(mut view) => {
                view.refresh(self.next_id(), |id, source| self.loader.tracks(id, source));
                view
            }
            None => {
                let mut view = TrackView::new(source);
                view.load(self.next_id(), |id, source| self.loader.tracks(id, source));
                view
            }
        };
        self.push_view(View::Tracks(view));
        self.focus = Focus::Content;
    }

    /// Takes the newest loaded copy of a list out of the history, to show it again at once.
    /// An emptied stand-in keeps its place, so Back still leads there.
    fn take_loaded_copy(&mut self, source: &TrackSource) -> Option<TrackView> {
        let index = self.history.iter().rposition(
            |view| matches!(view, View::Tracks(v) if v.source.same(source) && !v.stale && v.error.is_none()),
        )?;
        let View::Tracks(v) = &mut self.history[index] else { return None };
        let stand_in = TrackView { list: v.list.clone(), ..TrackView::new(v.source.clone()) };
        let loaded = std::mem::replace(v, stand_in);
        self.next_view_id += 1;
        v.retire(self.next_view_id);
        Some(loaded)
    }

    /// Empties earlier copies of a list in the history, so each list is held once however
    /// often it was opened. Back loads them again.
    fn retire_copies(&mut self, source: &TrackSource) {
        for view in &mut self.history {
            if let View::Tracks(v) = view
                && v.source.same(source)
                && !v.stale
            {
                self.next_view_id += 1;
                v.retire(self.next_view_id);
            }
        }
    }

    fn push_view(&mut self, view: View) {
        let previous = std::mem::replace(&mut self.view, view);
        self.history.push(previous);
        if self.history.len() > HISTORY_LIMIT {
            self.history.remove(0);
        }
        let quick = self.last_nav.is_some_and(|at| at.elapsed() < QUICK_NAV);
        self.last_nav = Some(Instant::now());
        if !quick {
            self.animate_content();
        }
    }

    fn back(&mut self) {
        match self.history.pop() {
            Some(view) => {
                self.view = view;
                if let View::Tracks(v) = &mut self.view
                    && v.stale
                {
                    self.next_view_id += 1;
                    v.load(self.next_view_id, |id, source| self.loader.tracks(id, source));
                }
                self.check_queue();
                self.focus = Focus::Content;
                // A page seen before comes back at once, without the rest of a transition.
                self.last_nav = Some(Instant::now());
                self.effects.cancel_unique_effect(Fx::Content);
            }
            None => self.focus = Focus::Nav,
        }
    }

    /// A new page's text fades in briefly, while its backgrounds and covers are there at once.
    fn animate_content(&mut self) {
        let p = theme::palette();
        let area = self.content_area;
        let effect = fx::fade_from_fg(p.bg, (120, Interpolation::QuadOut)).with_area(area);
        self.animate(Fx::Content, effect);
    }

    fn next_id(&mut self) -> u64 {
        self.next_view_id += 1;
        self.next_view_id
    }

    fn view_mut(&mut self, id: u64) -> Option<&mut View> {
        std::iter::once(&mut self.view).chain(self.history.iter_mut()).find(|v| v.id() == id)
    }

    // ── Playback ─────────────────────────────────────────────────────────────────────────

    /// Enter in the content area: play a track, or open an album or playlist.
    fn activate(&mut self) {
        match &self.view {
            View::Home(v) => {
                if let Some(item) = v.selected().cloned() {
                    self.open_home_item(item);
                }
            }
            View::Library(v) => {
                if let Some(entry) = self.library_entries_for(v.filter).get(v.cursor).copied() {
                    self.open_library_entry(entry);
                }
            }
            View::Artist(v) => match v.selected() {
                Some(ArtistSelection::Track(t)) => {
                    let (context, track) = (v.uri.clone(), t.track.clone());
                    self.play(context, Some(track.uri.clone()));
                    self.show_early(&track, None);
                }
                Some(ArtistSelection::Item(item)) => {
                    let item = item.clone();
                    self.open_home_item(item);
                }
                None => {}
            },
            View::Tracks(v) => {
                let Some(track) = v.tracks.get(v.list.selected).cloned() else { return };
                let (context, cover_url) = match &v.source {
                    TrackSource::Liked => match self.liked_context() {
                        Some(context) => (context, None),
                        None => return self.not_connected(),
                    },
                    TrackSource::Playlist(p) => (p.uri.clone(), None),
                    TrackSource::Album(a) => (a.uri.clone(), a.image_url.clone()),
                };
                self.play(context, Some(track.uri.clone()));
                self.show_early(&track, cover_url);
            }
            View::Search(v) => match v.selected().cloned() {
                Some(SearchEntry::Track(t)) => {
                    self.play(t.album_uri.clone(), Some(t.uri.clone()));
                    self.show_early(&t, None);
                }
                Some(SearchEntry::Artist(a)) => self.open_artist(a.uri, a.name),
                Some(SearchEntry::Album(a)) => self.open_tracks(TrackSource::Album(a)),
                Some(SearchEntry::Playlist(p)) => self.open_tracks(TrackSource::Playlist(p)),
                Some(SearchEntry::Header(_)) | None => {}
            },
            View::Settings(v) => {
                let setting = v.setting();
                self.change_setting(setting, true);
            }
            View::Queue(v) => {
                if let Some(row) = v.selected().cloned() {
                    self.play_queue_row(row);
                }
            }
        }
    }

    /// Opens playlists and albums; everything else starts playing.
    fn open_home_item(&mut self, item: HomeItem) {
        if backend::is_dj(&item.uri) {
            return self.run(Command::Dj);
        }
        let id = item.uri.rsplit(':').next().unwrap_or_default().to_owned();
        match item.kind {
            ItemKind::Playlist => self.open_tracks(TrackSource::Playlist(PlaylistInfo {
                id,
                uri: item.uri,
                name: item.title,
                owner_id: item.owner_id,
                owner_name: item.owner_name,
                collaborative: false,
                total: 0,
                image_url: item.image_url,
            })),
            ItemKind::Album => self.open_tracks(TrackSource::Album(AlbumInfo {
                id,
                uri: item.uri,
                name: item.title,
                artists: vec![item.subtitle],
                year: String::new(),
                total: 0,
                image_url: item.image_url,
            })),
            ItemKind::Artist => self.open_artist(item.uri, item.title),
            _ => self.play_item(item),
        }
    }

    fn open_artist(&mut self, uri: String, name: String) {
        if let View::Artist(v) = &self.view
            && v.uri == uri
        {
            self.focus = Focus::Content;
            return;
        }
        let id = self.next_id();
        // Artist pages are small, so a loaded one is copied rather than moved.
        let loaded = self.history.iter().rev().find_map(|view| match view {
            View::Artist(v) if v.uri == uri && v.page.is_some() => Some(v.clone()),
            _ => None,
        });
        let view = match loaded {
            Some(view) => {
                if !view.fetched_at.is_some_and(|at| at.elapsed() < PAGE_FRESH) {
                    self.loader.artist(id, uri);
                }
                ArtistView { id, ..view }
            }
            None => {
                self.loader.artist(id, uri.clone());
                ArtistView::new(id, uri, name)
            }
        };
        self.push_view(View::Artist(view));
        self.focus = Focus::Content;
    }

    fn play_home_item(&mut self) {
        if let View::Home(v) = &self.view
            && let Some(item) = v.selected().cloned()
        {
            self.play_item(item);
        }
    }

    fn play_item(&mut self, item: HomeItem) {
        match item.kind {
            ItemKind::Audiobook => self.error("cloofy can’t play audiobooks yet.".into()),
            ItemKind::Episode => self.play(item.context_uri, Some(item.uri)),
            _ => self.play(item.context_uri, None),
        }
    }

    /// Plays a context here, or on the other device that is currently playing.
    fn play(&mut self, context: String, track: Option<String>) {
        self.resume = None;
        self.last_context = Some(context.clone());
        let sent = if self.remote_active && self.plays_here(&context) {
            self.remote(Remote::PlayContext { context, track: track.clone(), shuffle: false });
            true
        } else {
            let uri = track.clone().unwrap_or_else(|| context.clone());
            self.start_on(&uri, |p| p.play(context, track.clone()))
        };
        if sent {
            self.now.start(track);
        }
    }

    /// A track is shown but nothing plays it: cloofy's own device is inactive and would
    /// ignore play, skip and seek commands. A start on its way activates it.
    fn idle(&self) -> bool {
        !self.local_active && !self.remote_active && self.now.track.is_some() && self.now.pending.is_none()
    }

    /// Plays the shown track on cloofy from where it stands, e.g. after another device stopped.
    fn play_shown(&mut self) {
        let Some(track) = &self.now.track else { return };
        let (context, uri, position_ms) = (self.last_context.clone(), track.uri.clone(), self.now.position_ms());
        if self.start_on(&uri, |p| p.resume(context, uri.clone(), position_ms)) {
            self.now.start(Some(uri));
        }
    }

    fn play_shuffled(&mut self, context: String) {
        self.resume = None;
        self.last_context = Some(context.clone());
        let sent = if self.remote_active && self.plays_here(&context) {
            self.remote(Remote::PlayContext { context, track: None, shuffle: true });
            true
        } else {
            self.start_on(&context.clone(), |p| p.play_shuffled(context))
        };
        if sent {
            self.now.start(None);
        }
    }

    /// Skips to the next or previous track; the play button spins until it plays. Like
    /// Spotify, previous restarts the track instead once it has played a few seconds.
    fn skip(&mut self, forward: bool) {
        let restart = !forward && self.now.position_ms() >= RESTART_AFTER_MS;
        let sent = match (self.remote_active, forward) {
            (true, true) => {
                self.remote(Remote::Next);
                true
            }
            (true, false) => {
                if restart {
                    self.remote_intent.seek = Some((0, Instant::now()));
                }
                self.remote(Remote::Previous);
                true
            }
            (false, true) => self.control(Player::next),
            (false, false) => self.control(Player::previous),
        };
        // An inactive player drops skips; with nothing shown there is nothing to wait for.
        if !sent || !(self.remote_active || self.local_active) {
            return;
        }
        if !restart {
            self.now.skip();
        } else if self.remote_active {
            self.now.set_position(0);
        } else {
            self.now.seek(0);
        }
    }

    /// Shows a picked track in the player bar right away, as the official client does,
    /// instead of only once its audio has loaded. `cover_url` is its album's cover, if known.
    fn show_early(&mut self, track: &Track, cover_url: Option<String>) {
        let before = self.now.track.as_ref().map(|t| t.uri.clone());
        self.now.show_early(CurrentTrack {
            uri: track.uri.clone(),
            name: track.name.clone(),
            artists: track.artists.clone(),
            artist_uris: track.artist_uris.clone(),
            album: track.album.clone(),
            album_uri: Some(track.album_uri.clone()).filter(|uri| !uri.is_empty()),
            duration_ms: track.duration_ms,
            cover_url: cover_url.clone(),
            cover_url_large: cover_url.clone(),
        });
        if self.now.track.as_ref().map(|t| &t.uri) == before.as_ref() {
            return;
        }
        if let Some(url) = cover_url {
            self.load_cover(url);
        }
        // Lyrics and the like state load alongside the audio.
        self.after_track_change(before);
    }

    /// Gives up on a track start, showing again what played before it.
    fn cancel_start(&mut self) {
        let before = self.now.track.as_ref().map(|t| t.uri.clone());
        self.now.cancel_start();
        self.after_track_change(before);
    }

    /// The play, shuffle, save and more buttons in page headers.
    fn header_button(&mut self, button: HeaderButton) {
        let Some(target) = self.page_target() else { return };
        match button {
            HeaderButton::Play => self.play(target.uri, None),
            HeaderButton::Shuffle => self.play_shuffled(target.uri),
            HeaderButton::Save => self.run_menu(crate::overlay::MenuAction::Save, target),
            HeaderButton::More => {
                // Right below the button, wherever the header puts it or has scrolled it to.
                let Some(&(area, _)) = self.hits.iter().find(|(_, hit)| *hit == Hit::Header(button)) else { return };
                let items = menu::page_menu_items(&target, self.is_saved(&target.uri));
                let anchor = (area.x, area.bottom());
                self.overlay = Some(Overlay::Menu(crate::overlay::Menu { target, anchor, items, cursor: 0 }));
                self.overlay_fx_pending = true;
            }
        }
    }

    /// Sends a command to the other device. Its answer schedules the poll that confirms it.
    fn remote(&mut self, command: Remote) {
        self.remote_intent.sent();
        self.last_command = Instant::now();
        self.loader.remote(self.active, command);
    }

    fn change_volume(&mut self, steps: i32) {
        let step = u16::MAX as i32 / 20;
        let volume = (self.now.volume as i32 + steps * step).clamp(0, u16::MAX as i32) as u16;
        self.set_volume(volume);
    }

    /// Moves the slider at once, so repeated steps add up; the device follows.
    fn set_volume(&mut self, volume: u16) {
        self.now.volume = volume;
        if self.remote_active {
            self.remote_intent.volume = Some(volume);
            self.remote(Remote::Volume((volume as u32 * 100 / u16::MAX as u32) as u8));
            return;
        }
        self.restored_volume = Some(volume);
        if self.local_active {
            if let Some(volume) = self.volume_sync.request(volume) {
                self.control(|p| p.set_volume(volume));
            }
        } else {
            self.volume_sync.defer(volume);
        }
    }

    fn seek_by(&mut self, delta_ms: i64) {
        let Some(track) = &self.now.track else { return };
        let target = (self.now.position_ms() as i64 + delta_ms).clamp(0, track.duration_ms as i64);
        self.seek_to(target as u32);
    }

    /// Moves the bar at once; repeated seeks add up instead of all starting from the same spot.
    fn seek_to(&mut self, position_ms: u32) {
        let Some(track) = &self.now.track else { return };
        // Spirc refuses positions past the end.
        let position_ms = if track.duration_ms > 0 { position_ms.min(track.duration_ms) } else { position_ms };
        if self.remote_active {
            self.now.set_position(position_ms);
            self.remote_intent.seek = Some((position_ms, Instant::now()));
            self.remote(Remote::Seek(position_ms));
        } else if self.local_active {
            self.now.seek(position_ms);
            self.control(|p| p.seek(position_ms));
        } else {
            // cloofy's inactive device ignores seeks; space starts from the new spot instead.
            self.now.set_position(position_ms);
            if let Some(last) = &mut self.resume {
                last.position_ms = position_ms;
            }
        }
    }

    /// The DJ panel: ask DJ for something, take one of Spotify's suggestions or jump to
    /// different picks.
    fn open_dj(&mut self) {
        let Some(track) = &self.now.track else { return };
        self.loader.dj_suggestions(track.uri.clone());
        self.status = None;
        let picker = Picker::free_text(dj_items("", &[]));
        self.open_overlay(Overlay::Dj { picker, suggestions: Vec::new(), loading: true });
    }

    fn choose_dj(&mut self, choice: DjChoice) {
        let Some(Overlay::Dj { picker, suggestions, .. }) = self.overlay.take() else { return };
        match choice {
            DjChoice::Jump => {
                if self.control(Player::next_segment) {
                    self.notice("Getting different DJ picks…".into());
                    self.now.start(None);
                }
            }
            DjChoice::Ask => self.ask_dj(picker.query.trim().to_owned(), false),
            DjChoice::Suggestion(index) => {
                if let Some(prompt) = suggestions.into_iter().nth(index) {
                    self.ask_dj(prompt, true);
                }
            }
        }
    }

    fn ask_dj(&mut self, prompt: String, suggested: bool) {
        let Some(player) = self.player() else { return self.not_connected() };
        match player.ask_dj(prompt, suggested) {
            Ok(answer) => {
                self.loader.ask_dj(answer);
                self.notice("Asking DJ…".into());
                self.now.start(None);
            }
            Err(err) => self.error(format!("DJ: {err:#}")),
        }
    }

    fn open_devices(&mut self, devices: Vec<Device>, refreshing: bool) {
        if devices.is_empty() && !refreshing {
            return self.error(format!("No {} devices found.", self.service_name()));
        }
        let items = device_items(&devices, self.device_id.as_deref());
        self.status = None;
        self.open_overlay(Overlay::Devices { devices, picker: Picker::new(items), refreshing });
    }

    /// Updates the open device picker with a fresh list, keeping the selected device.
    fn refresh_devices(&mut self, devices: Vec<Device>) {
        if devices.is_empty() {
            self.overlay = None;
            return self.error(format!("No {} devices found.", self.service_name()));
        }
        let items = device_items(&devices, self.device_id.as_deref());
        let Some(Overlay::Devices { devices: shown, picker, refreshing }) = &mut self.overlay else { return };
        let selected = picker.selected().and_then(|&index| shown.get(index)).map(|d| d.id.clone());
        picker.set_items(items, |&index| devices.get(index).map(|d| &d.id) == selected.as_ref());
        *shown = devices;
        *refreshing = false;
    }

    fn transfer_to(&mut self, index: usize) {
        let Some(Overlay::Devices { devices, .. }) = self.overlay.take() else { return };
        let Some(device) = devices.into_iter().nth(index) else { return };
        // The device label shows the move until the device turns up playing.
        self.transfer =
            Some(Transfer { device_id: device.id.clone(), name: device.name.clone(), since: Instant::now() });
        self.loader.transfer(self.service(), device);
        self.last_command = Instant::now();
    }

    /// The device playback is moving to, while it does.
    pub fn transferring_to(&self) -> Option<&str> {
        self.transfer.as_ref().map(|t| t.name.as_str())
    }

    /// Sends a command to cloofy's own player. Returns whether it went out.
    fn control(&mut self, command: impl FnOnce(&Player) -> anyhow::Result<()>) -> bool {
        let Some(player) = self.player() else {
            self.not_connected();
            return false;
        };
        if let Err(err) = command(player) {
            self.error(format!("{err:#}"));
            return false;
        }
        true
    }

    /// Starts playback of `uri` on the player of its service, which takes over from the one
    /// that played before. Returns whether the command went out.
    fn start_on(&mut self, uri: &str, command: impl FnOnce(&Player) -> anyhow::Result<()>) -> bool {
        match Service::of(uri) {
            Some(service) if self.players.contains_key(&service) => self.take_over(service),
            // Another service's player must not get it, while this one's isn't connected.
            Some(service) if !self.is_active(service) => {
                self.not_connected();
                return false;
            }
            _ => {}
        }
        self.control(command)
    }

    /// Makes `service`'s player the one that plays. Only one plays at a time, so the one that
    /// played before pauses, wherever it played.
    fn take_over(&mut self, service: Service) {
        let Some(previous) = self.active.replace(service).filter(|previous| *previous != service) else { return };
        if self.remote_active {
            self.loader.remote(Some(previous), Remote::Pause);
        } else if let Some(player) = self.players.get(&previous) {
            let _ = player.pause();
        }
        self.device_id = self.players.get(&service).and_then(Player::device_id);
        self.local_active = false;
        self.remote_active = false;
        self.active_device = None;
        self.now.device = None;
    }

    /// Whether `uri` plays on the service that plays now, rather than on another one.
    fn plays_here(&self, uri: &str) -> bool {
        Service::of(uri).is_none_or(|service| self.is_active(service))
    }

    /// Liked Songs of the service shown as a context to play, once its player knows the user.
    fn liked_context(&self) -> Option<String> {
        let service = self.loader.service()?;
        let user = self.players.get(&service).and_then(Player::user);
        backend::liked_context(service, user.as_deref())
    }

    fn not_connected(&mut self) {
        let text = match &self.player_error {
            Some(err) => format!("Playback unavailable: {err}"),
            None if self.demo => "This is the demo: nothing plays here".into(),
            None => format!("Still connecting to {}…", self.service_name()),
        };
        self.error(text);
    }

    pub(super) fn notice(&mut self, text: String) {
        self.status = Some(Status { text, error: false, since: Instant::now() });
    }

    fn error(&mut self, text: String) {
        self.status = Some(Status { text, error: true, since: Instant::now() });
    }
}

/// Images are kept per URL and cell size.
fn image_key(url: &str, size: Size) -> String {
    format!("{}x{}:{url}", size.width, size.height)
}

/// Picker rows for Spotify Connect devices; `own` is cloofy's device ID.
fn device_items(devices: &[Device], own: Option<&str>) -> Vec<PickItem<usize>> {
    devices
        .iter()
        .enumerate()
        .map(|(index, device)| {
            let icon = crate::icons::device(&device.kind);
            let this = if Some(device.id.as_str()) == own { " (this device)" } else { "" };
            let state = if device.active { "● playing" } else { device.kind.as_str() };
            PickItem {
                label: format!("{icon}  {}{this}", device.name),
                detail: state.to_owned(),
                image_url: None,
                value: index,
            }
        })
        .collect()
}

/// A picked overlay entry, unified across the different pickers.
#[derive(Debug, Clone, Copy)]
enum Choice {
    Palette(PaletteAction),
    Playlist(usize),
    Device(usize),
    Dj(DjChoice),
}
