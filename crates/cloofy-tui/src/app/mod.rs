mod menu;
mod persist;

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use cloofy_spotify::{
    catalog::{AlbumInfo, ArtistInfo, Page, PlaylistInfo, ShowInfo, Track, User},
    connect::{self, Device, Playback},
    home::{Home, HomeItem, ItemKind},
    lyrics::Lyrics,
    paths::Paths,
    session,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use librespot::playback::player::PlayerEvent;
use ratatui::layout::{Position, Rect, Size};
use tachyonfx::{CellFilter, Effect, EffectManager, Interpolation, Motion, fx};
use tokio::sync::{mpsc::UnboundedSender, watch};

use crate::{
    commands::Command,
    loader::{DataEvent, Loader, Picture, SessionState},
    overlay::{MenuAction, Overlay, PaletteAction, PickItem, Picker, PickerOutcome},
    playback::{self, Controller, Cover, NowPlaying, Repeat},
    snapshot::Placement,
    theme::{Accent, BG, RAISED},
    views::{
        ArtistSelection, ArtistView, HomeView, LibraryView, ListCursor, SearchEntry, SearchView, TrackSource,
        TrackView, View,
    },
};

/// Cover art size in the player bar, in cells (roughly square in most fonts).
pub const COVER_SIZE: Size = Size { width: 6, height: 3 };
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
const STATUS_TTL: Duration = Duration::from_secs(4);
const SEEK_STEP_MS: i64 = 5_000;
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
/// Spotify's DJ; loading it without a track asks for a fresh set.
const DJ_URI: &str = "spotify:playlist:37i9dQZF1EYkqdzj48dyYq";
const CROSSFADE_STEPS: [u64; 5] = [0, 3, 6, 9, 12];
/// The character Kitty's unicode-placeholder images are drawn with.
const KITTY_PLACEHOLDER: char = '\u{10EEEE}';
/// How often to check which device plays, and how soon after a remote command.
const POLL_INTERVAL: Duration = Duration::from_secs(3);
const QUICK_POLL: Duration = Duration::from_millis(500);
/// Session state is also saved periodically, in case cloofy gets killed.
const STATE_SAVE_INTERVAL: Duration = Duration::from_secs(20);
/// Rapid like/unlike toggles collapse into one request after this pause.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(350);
/// Animation frame length (spinners, equalizer, progress bar).
const FRAME: Duration = Duration::from_millis(120);
/// Parts of the library besides playlists, each loaded on its own.
const LIBRARY_PARTS: u8 = 4;
/// Waits before starting the player again after it failed to connect, longer after each
/// failure in a row.
const PLAYER_RETRY: [Duration; 4] =
    [Duration::from_secs(2), Duration::from_secs(5), Duration::from_secs(15), Duration::from_secs(60)];

// Events move through a channel once each; boxing the big ones would only add allocations.
#[allow(clippy::large_enum_variant)]
pub enum AppEvent {
    Data(DataEvent),
    Player(PlayerEvent),
    PlayerReady(Box<Controller>),
    PlayerFailed(String),
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
    /// The `artist`th artist of track `row` in the open track list.
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
    restored_volume: Option<u16>,
    controller: Option<Controller>,
    /// cloofy's own Spotify Connect device ID.
    device_id: Option<String>,
    /// Playback runs on another device; commands go through Spotify Connect.
    remote_active: bool,
    /// Seconds consecutive tracks overlap.
    crossfade: u64,
    next_poll: Instant,
    polling: bool,
    session_tx: watch::Sender<SessionState>,
    /// For starting the player again after it failed to connect.
    paths: Paths,
    events: UnboundedSender<AppEvent>,
    /// Failed player starts in a row, for backing off between retries.
    player_failures: usize,
    player_retry_at: Option<Instant>,
    pub player_error: Option<String>,
    pub now: NowPlaying,
    pub accent: Accent,
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
    /// F12 was pressed; the next frame gets written as a PNG.
    pub snapshot_requested: bool,
    last_click: Option<(Instant, Hit)>,
    pub effects: EffectManager<Fx>,
    pub last_draw: Instant,
    pub content_area: Rect,
    pub screen: Rect,
    /// Rows of the content list at the last draw, for paging.
    pub page_height: usize,
    started: Instant,
    last_frame: u64,
    pub quit: bool,
}

impl App {
    pub fn new(
        loader: Loader,
        session_tx: watch::Sender<SessionState>,
        events: UnboundedSender<AppEvent>,
        crossfade: u64,
        state: crate::state::State,
        paths: Paths,
    ) -> Self {
        loader.me();
        loader.playlists();
        loader.library();
        loader.home(0);
        let mut app = Self {
            loader,
            state_path: paths.state_file(),
            last_save: Instant::now(),
            resume: None,
            last_context: None,
            restored_volume: None,
            controller: None,
            device_id: None,
            remote_active: false,
            crossfade,
            next_poll: Instant::now(),
            polling: false,
            session_tx,
            paths,
            events,
            player_failures: 0,
            player_retry_at: None,
            player_error: None,
            now: NowPlaying::new(),
            accent: Accent::new(),
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
            snapshot_requested: false,
            mouse: None,
            hovered: None,
            dragging: None,
            seek_preview: None,
            last_click: None,
            effects: EffectManager::default(),
            last_draw: Instant::now(),
            content_area: Rect::default(),
            screen: Rect::default(),
            page_height: 10,
            started: Instant::now(),
            last_frame: 0,
            quit: false,
        };
        app.restore(state);
        app
    }

    /// The last session's track is shown and ␣ would continue it.
    pub fn can_resume(&self) -> bool {
        self.resume.is_some()
    }

    pub fn is_connected(&self) -> bool {
        self.controller.is_some()
    }

    /// Animation frame counter, shared by all animated elements.
    pub fn frame(&self) -> u64 {
        (self.started.elapsed().as_millis() / FRAME.as_millis()) as u64
    }

    pub fn is_saved(&self, uri: &str) -> bool {
        self.saved.get(uri).copied().unwrap_or(false)
    }

    pub fn shutdown(&mut self) {
        self.save_state();
        if let Some(controller) = &self.controller {
            controller.shutdown();
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
            _ => false,
        }
    }

    /// Returns whether the screen needs a redraw.
    pub fn on_event(&mut self, event: AppEvent) -> bool {
        match event {
            AppEvent::Player(event) => return self.on_player(event),
            AppEvent::PlayerReady(controller) => {
                let _ = self.session_tx.send(SessionState::Ready(controller.session.clone()));
                self.player_failures = 0;
                self.device_id = Some(controller.session.device_id().to_owned());
                if let Some(volume) = self.restored_volume {
                    let _ = controller.set_volume(volume);
                }
                controller.set_crossfade(Duration::from_secs(self.crossfade));
                self.controller = Some(*controller);
                self.reload_failed_view();
            }
            AppEvent::PlayerFailed(err) => self.on_player_failed(err),
            AppEvent::Data(data) => return self.on_data(data),
        }
        true
    }

    /// Lets requests waiting for the session fail, and schedules another try.
    fn on_player_failed(&mut self, err: String) {
        tracing::warn!("player: {err}");
        let _ = self.session_tx.send(SessionState::Failed(err.clone()));
        // Being offline or a server hiccup passes; a missing login does not.
        if session::is_logged_in(&self.paths).unwrap_or(false) {
            let delay = PLAYER_RETRY[self.player_failures.min(PLAYER_RETRY.len() - 1)];
            self.player_failures += 1;
            self.player_retry_at = Some(Instant::now() + delay);
        }
        self.player_error = Some(err);
    }

    fn retry_player(&mut self) {
        self.player_retry_at = None;
        self.player_error = None;
        let _ = self.session_tx.send(SessionState::Connecting);
        playback::spawn(self.paths.clone(), Duration::from_secs(self.crossfade), self.events.clone());
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
                v.error = None;
                v.loading = true;
                v.tracks.clear();
                self.loader.tracks(v.id, v.source.clone());
            }
            _ => {}
        }
    }

    fn on_player(&mut self, event: PlayerEvent) -> bool {
        if matches!(event, PlayerEvent::TrackChanged { .. } | PlayerEvent::Playing { .. }) {
            self.resume = None;
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
        let before = self.now.track.as_ref().map(|t| t.uri.clone());
        if let Some(url) = self.now.apply(event) {
            self.loader.cover(url, COVER_SIZE);
        }
        self.after_track_change(before);
        true
    }

    /// Returns whether anything shown changed: while cloofy plays or nothing does, the poll
    /// mostly confirms what is on screen.
    fn on_playback(&mut self, playback: Option<Playback>) -> bool {
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
                if let Some(url) = self.now.apply_remote(&p) {
                    self.loader.cover(url, COVER_SIZE);
                }
                self.after_track_change(before);
                true
            }
            None if self.remote_active => {
                self.remote_active = false;
                self.now.device = None;
                self.now.playing = false;
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
                Ok(user) => self.me = Some(user),
                Err(err) => self.error(err),
            },
            DataEvent::Playlists(result) => {
                self.playlists_loading = false;
                match result {
                    Ok(playlists) => {
                        self.saved.extend(playlists.iter().map(|p| (p.uri.clone(), true)));
                        self.playlists = playlists;
                    }
                    Err(err) => self.error(err),
                }
            }
            DataEvent::LikedCount(result) => {
                self.library_part_done();
                match result {
                    Ok(total) => self.liked_total = Some(total),
                    Err(err) => tracing::warn!("liked songs: {err}"),
                }
            }
            DataEvent::Albums(result) => {
                self.library_part_done();
                match result {
                    Ok(list) => {
                        self.saved.extend(list.iter().map(|a| (a.uri.clone(), true)));
                        self.albums = list;
                    }
                    Err(err) => tracing::warn!("albums: {err}"),
                }
            }
            DataEvent::Artists(result) => {
                self.library_part_done();
                match result {
                    Ok(list) => {
                        self.saved.extend(list.iter().map(|a| (a.uri.clone(), true)));
                        self.artists = list;
                    }
                    Err(err) => tracing::warn!("artists: {err}"),
                }
            }
            DataEvent::Shows(result) => {
                self.library_part_done();
                match result {
                    Ok(list) => self.shows = list,
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
                if current != Some(url.as_str()) {
                    return false;
                }
                match result {
                    Ok((picture, accent)) => {
                        if let Some(old) = self.now.cover.replace(Cover { url, picture }) {
                            self.terminal_out.extend(old.picture.terminal_delete());
                        }
                        self.accent.set(accent);
                    }
                    Err(err) => tracing::warn!("cover {url}: {err}"),
                }
            }
            DataEvent::LargeCover { url, size, result } => {
                if self.zen_cover_pending.as_ref().is_some_and(|(u, s)| *u == url && *s == size) {
                    self.zen_cover_pending = None;
                }
                let current = self.now.track.as_ref().and_then(|t| t.cover_url_large.as_deref());
                let wanted = self.zen && self.zen_cover_want == Some(size) && current == Some(url.as_str());
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
                    self.notice(notice);
                }
            }
            DataEvent::SaveFailed { uri, saved, error } => {
                self.saving.remove(&uri);
                if !self.pending_saves.contains_key(&uri) {
                    self.saved.insert(uri, saved);
                    self.error(error);
                }
            }
            DataEvent::Playback(result) => {
                self.polling = false;
                match result {
                    Ok(playback) => return self.on_playback(playback),
                    Err(err) => {
                        tracing::warn!("playback state: {err}");
                        return false;
                    }
                }
            }
            DataEvent::Devices(result) => match result {
                Ok(devices) => self.open_devices(devices),
                Err(err) => self.error(format!("Devices: {err}")),
            },
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
        let Some(View::Tracks(v)) = self.view_mut(view) else { return };
        let page = match page {
            Ok(page) => page,
            Err(err) => {
                v.loading = false;
                v.error = Some(err);
                return;
            }
        };
        let uris: Vec<String> = page.items.iter().map(|t| t.uri.clone()).collect();
        let liked = matches!(v.source, TrackSource::Liked);
        v.tracks.extend(page.items);
        v.total = Some(page.total);
        v.loading = page.next_offset.is_some();
        if liked {
            self.saved.extend(uris.into_iter().map(|uri| (uri, true)));
        } else {
            self.loader.check_saved(uris);
        }
    }

    /// Advances timers. Returns whether anything visible changed.
    pub fn on_tick(&mut self) -> bool {
        let mut redraw = false;

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
        if self.player_retry_at.is_some_and(|at| Instant::now() >= at) {
            self.retry_player();
            redraw = true;
        }
        if self.last_save.elapsed() >= STATE_SAVE_INTERVAL {
            self.save_state();
        }
        if self.last_eviction.elapsed() >= EVICTION_INTERVAL {
            self.evict_images();
        }
        if self.trim_due.is_some_and(|due| Instant::now() >= due) {
            self.trim_due = None;
            crate::release_freed_memory();
        }

        if self.status.as_ref().is_some_and(|s| s.since.elapsed() >= STATUS_TTL) {
            self.status = None;
            redraw = true;
        }

        if self.controller.is_some() && !self.polling && Instant::now() >= self.next_poll {
            self.polling = true;
            self.next_poll = Instant::now() + POLL_INTERVAL;
            self.loader.playback();
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
            View::Library(_) => false,
            View::Artist(v) => v.page.is_none() && v.error.is_none(),
            View::Tracks(v) => v.loading,
            View::Search(v) => v.loading,
        };
        let lyrics_loading = self.zen && self.lyrics.as_ref().is_some_and(|l| l.result.is_none());
        self.now.playing
            || loading
            || lyrics_loading
            || self.playlists_loading
            || self.library_loading
            || (self.controller.is_none() && self.player_error.is_none())
    }

    /// Requests the big cover once the Now Playing layout knows its size.
    fn request_zen_cover(&mut self) {
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
        if distance.abs() < 0.02 {
            self.lyrics_scroll = target;
            return false;
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
            _ => {}
        }
    }

    /// Mouse wheel: moves the view, not the selection.
    fn scroll_content(&mut self, delta: isize) {
        match &mut self.view {
            View::Tracks(v) => v.list.scroll(delta),
            View::Search(v) => v.list.scroll(delta),
            View::Home(v) => v.scroll_by(delta),
            View::Artist(v) => v.scroll_by(delta),
            View::Library(v) => {
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
                    self.click(hit, mouse.column);
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                self.overlay = None;
                if let Some(hit @ (Hit::Row(_) | Hit::Card { .. } | Hit::Nav(_))) = hit {
                    self.select(hit);
                    self.open_menu((mouse.column, mouse.row));
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
                    View::Artist(v) => {
                        v.row = index;
                        v.item = 0;
                        v.detached = true;
                    }
                    View::Library(v) => {
                        v.cursor = index;
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
            },
            Hit::Header(button) => self.header_button(button, column),
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
            Hit::OverlayItem(position) if matches!(self.overlay, Some(Overlay::Menu(_))) => {
                self.choose_menu_item(position);
            }
            Hit::OverlayItem(position) => {
                let outcome = match &self.overlay {
                    Some(Overlay::Palette(picker)) => picker.select(position).map(Choice::Palette),
                    Some(Overlay::AddToPlaylist { picker, .. }) => picker.select(position).map(Choice::Playlist),
                    Some(Overlay::Devices { picker, .. }) => picker.select(position).map(Choice::Device),
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
                | Hit::Header(_),
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
            Command::PlayPause if self.remote_active => {
                let playing = !self.now.playing;
                self.now.playing = playing;
                self.remote(if playing { connect::Command::Play } else { connect::Command::Pause });
            }
            Command::PlayPause if self.resume.is_some() && self.controller.is_some() => {
                if let Some(last) = self.resume.take() {
                    self.control(|c| c.resume(last.context, last.uri, last.position_ms));
                }
            }
            Command::PlayPause => {
                let has_track = self.now.track.is_some();
                self.control(|c| c.toggle(has_track));
            }
            Command::Next if self.remote_active => self.remote(connect::Command::Next),
            Command::Next => self.control(|c| c.next()),
            Command::Previous if self.remote_active => self.remote(connect::Command::Previous),
            Command::Previous => self.control(|c| c.previous()),
            Command::SeekBack => self.seek_by(-SEEK_STEP_MS),
            Command::SeekForward => self.seek_by(SEEK_STEP_MS),
            Command::VolumeUp => self.change_volume(1),
            Command::VolumeDown => self.change_volume(-1),
            Command::Shuffle => {
                let on = !self.now.shuffle;
                if self.remote_active {
                    self.now.shuffle = on;
                    self.remote(connect::Command::Shuffle(on));
                } else {
                    self.control(|c| c.shuffle(on));
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
                    let mode = match next {
                        Repeat::Off => "off",
                        Repeat::Context => "context",
                        Repeat::Track => "track",
                    };
                    self.remote(connect::Command::Repeat(mode));
                } else {
                    self.control(|c| c.repeat(next));
                }
            }
            Command::Search => {
                if self.zen {
                    self.toggle_zen();
                }
                self.open_search();
            }
            Command::NowPlaying => self.toggle_zen(),
            Command::Like => self.toggle_saved(),
            Command::Queue => match self.target() {
                Some(target) if target.is_track => self.loader.queue(target.uri, target.name),
                Some(_) => self.error("Only songs can be added to the queue.".into()),
                None => self.error("No song selected.".into()),
            },
            Command::AddToPlaylist => self.open_playlist_picker(),
            Command::Menu => {
                let area = self.content_area;
                self.open_menu((area.x + area.width / 3, area.y + area.height / 3));
            }
            Command::Dj => {
                self.notice("Starting DJ…".into());
                self.play(DJ_URI.into(), None);
            }
            Command::Crossfade => {
                let next = CROSSFADE_STEPS.iter().copied().find(|s| *s > self.crossfade).unwrap_or(0);
                self.crossfade = next;
                if let Some(controller) = &self.controller {
                    controller.set_crossfade(Duration::from_secs(next));
                }
                let text = if next == 0 { "Crossfade off".to_owned() } else { format!("Crossfade: {next} s") };
                self.notice(text);
            }
            Command::Devices => {
                self.notice("Finding devices…".into());
                self.loader.devices();
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
            .map(|&command| PickItem {
                label: command.label().to_owned(),
                detail: command.key().to_owned(),
                image_url: None,
                value: PaletteAction::Run(command),
            })
            .collect();
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
            self.loader.add_to_playlist(playlist.clone(), uris);
        }
    }

    fn open_overlay(&mut self, overlay: Overlay) {
        self.overlay = Some(overlay);
        self.overlay_fx_pending = true;
    }

    /// Called by the first draw of an overlay, which knows where it ended up.
    pub fn animate_overlay(&mut self, area: Rect) {
        self.overlay_fx_pending = false;
        let effect = fx::sweep_in(Motion::UpToDown, 6, 0, RAISED, (180, Interpolation::QuadOut)).with_area(area);
        self.animate(Fx::Overlay, effect);
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
            };
            if selected.is_some() {
                return selected;
            }
        }
        self.now.track.as_ref().map(|t| track(&t.uri, &t.name, true))
    }

    fn toggle_zen(&mut self) {
        self.zen = !self.zen;
        let screen = self.screen;
        self.animate(Fx::Screen, fx::fade_from_fg(BG, (320, Interpolation::QuadOut)).with_area(screen));
    }

    /// Starts an animation; the next frame measures time from now.
    fn animate(&mut self, slot: Fx, effect: Effect) {
        // Kitty images are drawn through placeholder cells whose color encodes the image;
        // recoloring them mid-animation would show the wrong picture.
        let effect = effect.with_filter(CellFilter::eval_cell(|cell| !cell.symbol().starts_with(KITTY_PLACEHOLDER)));
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

    fn follow_link(&mut self, link: Link) {
        let (uri, name) = match link {
            Link::TrackArtist { row, artist } => {
                let View::Tracks(v) = &self.view else { return };
                let Some(track) = v.tracks.get(row) else { return };
                match (track.artist_uris.get(artist), track.artists.get(artist)) {
                    (Some(uri), Some(name)) => (uri.clone(), name.clone()),
                    _ => return,
                }
            }
            Link::TrackAlbum(row) => {
                let View::Tracks(v) = &self.view else { return };
                let Some(track) = v.tracks.get(row).filter(|t| !t.album_uri.is_empty()) else { return };
                let (uri, name, artists) = (track.album_uri.clone(), track.album.clone(), track.artists.clone());
                return self.open_album(uri, name, artists, None);
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
        self.loader.home(id);
        self.push_view(View::Home(HomeView::new(id, self.home_cache.clone())));
        self.focus = Focus::Content;
    }

    /// A cover, or the drawn placeholder for items without one.
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
        let url = url.unwrap_or(crate::art::PLACEHOLDER_COVER);
        self.prefetch.push((url.to_owned(), size, round));
    }

    /// The cover for `url` at `size`, loading it on first use.
    pub fn image(&mut self, url: &str, size: Size, round: bool) -> Option<&Picture> {
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
        }));
        self.focus = Focus::SearchInput;
    }

    fn open_tracks(&mut self, source: TrackSource) {
        let page_uri = match &source {
            TrackSource::Liked => None,
            TrackSource::Playlist(p) => Some(p.uri.clone()),
            TrackSource::Album(a) => Some(a.uri.clone()),
        };
        if let Some(uri) = page_uri.filter(|uri| !self.saved.contains_key(uri)) {
            self.loader.check_saved(vec![uri]);
        }
        let id = self.next_id();
        self.loader.tracks(id, source.clone());
        self.push_view(View::Tracks(TrackView {
            id,
            source,
            tracks: Vec::new(),
            total: None,
            loading: true,
            error: None,
            list: ListCursor::default(),
        }));
        self.focus = Focus::Content;
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
        let area = self.content_area;
        let effect = fx::fade_from_fg(BG, (120, Interpolation::QuadOut)).with_area(area);
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
                    let (context, track) = (v.uri.clone(), t.track.uri.clone());
                    self.play(context, Some(track));
                }
                Some(ArtistSelection::Item(item)) => {
                    let item = item.clone();
                    self.open_home_item(item);
                }
                None => {}
            },
            View::Tracks(v) => {
                let Some(track) = v.tracks.get(v.list.selected) else { return };
                let context = match &v.source {
                    TrackSource::Liked => match &self.controller {
                        Some(c) => format!("spotify:user:{}:collection", c.session.username()),
                        None => return self.not_connected(),
                    },
                    TrackSource::Playlist(p) => p.uri.clone(),
                    TrackSource::Album(a) => a.uri.clone(),
                };
                let track = track.uri.clone();
                self.play(context, Some(track));
            }
            View::Search(v) => match v.selected().cloned() {
                Some(SearchEntry::Track(t)) => self.play(t.album_uri, Some(t.uri)),
                Some(SearchEntry::Artist(a)) => self.open_artist(a.uri, a.name),
                Some(SearchEntry::Album(a)) => self.open_tracks(TrackSource::Album(a)),
                Some(SearchEntry::Playlist(p)) => self.open_tracks(TrackSource::Playlist(p)),
                Some(SearchEntry::Header(_)) | None => {}
            },
        }
    }

    /// Opens playlists and albums; everything else starts playing.
    fn open_home_item(&mut self, item: HomeItem) {
        if item.uri == DJ_URI {
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
        let id = self.next_id();
        self.loader.artist(id, uri.clone());
        self.push_view(View::Artist(ArtistView::new(id, uri, name)));
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
        if self.remote_active {
            self.remote(connect::Command::PlayContext { context, track, shuffle: false });
        } else {
            self.control(|c| c.play(context, track));
        }
    }

    fn play_shuffled(&mut self, context: String) {
        self.resume = None;
        self.last_context = Some(context.clone());
        if self.remote_active {
            self.remote(connect::Command::PlayContext { context, track: None, shuffle: true });
        } else {
            self.control(|c| c.play_shuffled(context));
        }
    }

    /// The play, shuffle, save and more buttons in page headers.
    fn header_button(&mut self, button: HeaderButton, column: u16) {
        let Some(target) = self.page_target() else { return };
        match button {
            HeaderButton::Play => self.play(target.uri, None),
            HeaderButton::Shuffle => self.play_shuffled(target.uri),
            HeaderButton::Save => self.run_menu(crate::overlay::MenuAction::Save, target),
            HeaderButton::More => {
                let items = menu::page_menu_items(&target, self.is_saved(&target.uri));
                let anchor = (column, self.content_area.y + 8);
                self.overlay = Some(Overlay::Menu(crate::overlay::Menu { target, anchor, items, cursor: 0 }));
                self.overlay_fx_pending = true;
            }
        }
    }

    fn remote(&mut self, command: connect::Command) {
        self.loader.remote(command);
        self.next_poll = Instant::now() + QUICK_POLL;
    }

    fn change_volume(&mut self, steps: i32) {
        let step = u16::MAX as i32 / 20;
        let volume = (self.now.volume as i32 + steps * step).clamp(0, u16::MAX as i32) as u16;
        self.set_volume(volume);
    }

    fn set_volume(&mut self, volume: u16) {
        if self.remote_active {
            self.now.volume = volume;
            self.remote(connect::Command::Volume((volume as u32 * 100 / u16::MAX as u32) as u8));
        } else {
            self.control(|c| c.set_volume(volume));
        }
    }

    fn seek_by(&mut self, delta_ms: i64) {
        let Some(track) = &self.now.track else { return };
        let target = (self.now.position_ms() as i64 + delta_ms).clamp(0, track.duration_ms as i64);
        self.seek_to(target as u32);
    }

    fn seek_to(&mut self, position_ms: u32) {
        if self.remote_active {
            self.remote(connect::Command::Seek(position_ms));
        } else {
            self.control(|c| c.seek(position_ms));
        }
    }

    fn open_devices(&mut self, devices: Vec<Device>) {
        if devices.is_empty() {
            return self.error("No Spotify devices found.".into());
        }
        let own = self.device_id.clone();
        let items = devices
            .iter()
            .enumerate()
            .map(|(index, device)| {
                let icon = crate::icons::device(&device.kind);
                let this = if Some(&device.id) == own.as_ref() { " (this device)" } else { "" };
                let state = if device.active { "● playing" } else { device.kind.as_str() };
                PickItem {
                    label: format!("{icon}  {}{this}", device.name),
                    detail: state.to_owned(),
                    image_url: None,
                    value: index,
                }
            })
            .collect();
        self.status = None;
        self.open_overlay(Overlay::Devices { devices, picker: Picker::new(items) });
    }

    fn transfer_to(&mut self, index: usize) {
        let Some(Overlay::Devices { devices, .. }) = self.overlay.take() else { return };
        let Some(device) = devices.into_iter().nth(index) else { return };
        self.loader.transfer(device);
        self.next_poll = Instant::now() + QUICK_POLL;
    }

    fn control(&mut self, command: impl FnOnce(&Controller) -> anyhow::Result<()>) {
        match &self.controller {
            Some(controller) => {
                if let Err(err) = command(controller) {
                    self.error(format!("{err:#}"));
                }
            }
            None => self.not_connected(),
        }
    }

    fn not_connected(&mut self) {
        let text = match &self.player_error {
            Some(err) => format!("Playback unavailable: {err}"),
            None => "Still connecting to Spotify…".into(),
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

/// A picked overlay entry, unified across the different pickers.
#[derive(Debug, Clone, Copy)]
enum Choice {
    Palette(PaletteAction),
    Playlist(usize),
    Device(usize),
}
