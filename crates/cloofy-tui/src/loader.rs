//! Background data loading. Every request runs in its own task and reports back as an event.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, bail};
use cloofy_spotify::{
    artist::{self, ArtistPage},
    catalog::{self, AlbumInfo, ArtistInfo, Page, PlaylistInfo, SearchResults, ShowInfo, Track, User},
    connect::{self, Device, Playback},
    home::{self, Home},
    lyrics::{self, Lyrics, LyricsQuery},
    pathfinder::Pathfinder,
    paths::Paths,
    update::{self, Release},
    webapi::WebApi,
};
use futures::{StreamExt, stream};
use image::imageops::FilterType;
use librespot::core::session::Session;
use ratatui::layout::Size;
use ratatui_image::{
    Resize,
    picker::{Picker, ProtocolType},
    protocol::Protocol,
};
use sha2::{Digest, Sha256};
use tokio::sync::{
    OnceCell, Semaphore,
    mpsc::{self, UnboundedSender},
    watch,
};

use crate::{
    app::AppEvent,
    art,
    demo::Demo,
    playback::CurrentTrack,
    theme::{self, Rgb},
    views::TrackSource,
};

/// Cached cover art unused for this long is deleted at startup.
const IMAGE_CACHE_AGE: Duration = Duration::from_secs(60 * 24 * 60 * 60);

/// Track metadata requests running at the same time for one playlist.
const METADATA_BATCHES: usize = 3;
/// Library checks running at the same time for one list.
const SAVED_CONCURRENCY: usize = 2;
/// How long lyrics wait for the session, so a track restored at startup still gets Spotify's
/// synced lyrics instead of going straight to LRCLIB.
const LYRICS_SESSION_WAIT: Duration = Duration::from_secs(5);

/// The librespot session as the loader sees it.
#[derive(Clone)]
pub enum SessionState {
    Connecting,
    Ready(Session),
    /// Connecting failed; requests that need the session fail too instead of waiting forever.
    Failed(String),
}

/// A load running in the background; dropping it stops the load.
pub struct Fetch(tokio::task::AbortHandle);

impl Drop for Fetch {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// An encoded image plus, in snapshot mode, its source for pasting into PNG snapshots.
pub struct Picture {
    pub protocol: Protocol,
    pub source: Option<Arc<image::DynamicImage>>,
    /// Memory the terminal keeps for the image once drawn. Only kitty keeps images, until told
    /// to delete them; the other protocols draw them and forget.
    pub terminal_bytes: usize,
}

pub enum DataEvent {
    Me(Result<User, String>),
    Home {
        view: u64,
        result: Result<Home, String>,
    },
    Artist {
        view: u64,
        result: Result<ArtistPage, String>,
    },
    Image {
        key: String,
        result: Result<Picture, String>,
    },
    /// The image was off screen by its turn and was not decoded.
    ImageSkipped {
        key: String,
    },
    Playlists(Result<Vec<PlaylistInfo>, String>),
    /// The rest of the library; each part arrives, or fails, on its own.
    LikedCount(Result<u32, String>),
    Albums(Result<Vec<AlbumInfo>, String>),
    Artists(Result<Vec<ArtistInfo>, String>),
    Shows(Result<Vec<ShowInfo>, String>),
    Tracks {
        view: u64,
        page: Result<Page<Track>, String>,
    },
    Search {
        view: u64,
        query: String,
        result: Result<SearchResults, String>,
    },
    Cover {
        url: String,
        result: Result<(Picture, Rgb), String>,
    },
    LargeCover {
        url: String,
        size: Size,
        result: Result<Picture, String>,
    },
    /// A cover as an encoded image, for the system's media controls.
    Artwork {
        url: String,
        result: Result<Arc<[u8]>, String>,
    },
    Lyrics {
        uri: String,
        result: Result<Option<Lyrics>, String>,
    },
    /// Library state of URIs (liked songs, saved albums, followed playlists …).
    Saved(Vec<(String, bool)>),
    /// Spotify confirmed a library change.
    SaveDone {
        uri: String,
        notice: String,
    },
    /// A library change failed; `saved` is the state to roll back to.
    SaveFailed {
        uri: String,
        saved: bool,
        error: String,
    },
    /// Adding `count` songs to a playlist failed; its song count goes back down.
    AddFailed {
        playlist_id: String,
        count: u32,
        error: String,
    },
    /// What plays where, from Spotify Connect.
    Playback(Result<Option<Playback>, String>),
    /// Spotify answered a remote command, which stood for `commands` collapsed ones.
    RemoteDone {
        commands: usize,
        result: Result<(), String>,
    },
    Devices(Result<Vec<Device>, String>),
    /// Spotify answered a transfer of playback to the device named `name`.
    Transferred {
        name: String,
        result: Result<(), String>,
    },
    /// A release newer than this cloofy.
    UpdateAvailable(Release),
    /// How much of the update has downloaded, in percent.
    UpdateProgress(u8),
    /// The update took the place of the binary at this path; it runs after a restart.
    UpdateInstalled(PathBuf),
    UpdateFailed(String),
    Notice(String),
    Failed(String),
}

#[derive(Clone)]
pub struct Loader {
    api: Arc<WebApi>,
    tx: UnboundedSender<AppEvent>,
    session: watch::Receiver<SessionState>,
    http: reqwest::Client,
    picker: Picker,
    pathfinder: Arc<OnceCell<Pathfinder>>,
    pathfinder_hashes: PathBuf,
    /// The user's Spotify ID, to tell their own playlists from followed ones.
    user_id: Arc<OnceCell<String>>,
    /// Keep decoded images for dev snapshots.
    keep_sources: bool,
    image_cache: PathBuf,
    /// Turns for decoding and encoding covers. A screenful asks for dozens at once; run all
    /// together, they would hog every core and need a few megabytes of buffers each.
    encode_turns: Arc<Semaphore>,
    /// Turns for covers loaded ahead of being shown, separate so they never hold up the ones on
    /// screen.
    prefetch_turns: Arc<Semaphore>,
    /// The number of the last frame drawn; see [`Loader::frame_drawn`].
    frames_drawn: Arc<AtomicU64>,
    remote: UnboundedSender<connect::Command>,
    /// The sample `cloofy demo` shows instead of the account's data.
    demo: Option<Arc<Demo>>,
}

/// An image to decode: downloaded bytes, or artwork cloofy draws itself.
enum ImageSource {
    Downloaded(Vec<u8>),
    Drawn(fn() -> image::DynamicImage),
}

impl Loader {
    pub fn new(
        api: Arc<WebApi>,
        tx: UnboundedSender<AppEvent>,
        session: watch::Receiver<SessionState>,
        picker: Picker,
        pathfinder_hashes: PathBuf,
        image_cache: PathBuf,
        keep_sources: bool,
    ) -> Self {
        let http =
            cloofy_spotify::http::client(concat!("cloofy/", env!("CARGO_PKG_VERSION"))).build().unwrap_or_default();
        // Leave cores for the interface and audio.
        let turns = std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(2).clamp(1, 4));
        let (remote, commands) = mpsc::unbounded_channel();
        tokio::spawn(send_remote(api.clone(), tx.clone(), commands));
        Self {
            remote,
            api,
            tx,
            session,
            http,
            picker,
            pathfinder: Arc::default(),
            pathfinder_hashes,
            user_id: Arc::default(),
            keep_sources,
            image_cache,
            encode_turns: Arc::new(Semaphore::new(turns)),
            prefetch_turns: Arc::new(Semaphore::new(2)),
            frames_drawn: Arc::default(),
            demo: None,
        }
    }

    /// Answers every request from the demo's sample instead of Spotify.
    pub fn with_demo(mut self, demo: Option<Arc<Demo>>) -> Self {
        self.demo = demo;
        self
    }

    fn demo(&self) -> Option<&Demo> {
        self.demo.as_deref()
    }

    /// Notes that frame `number` is on screen. An image request whose image the frame did not
    /// draw has scrolled out of view.
    pub fn frame_drawn(&self, number: u64) {
        self.frames_drawn.store(number, Ordering::Relaxed);
    }

    fn send(&self, event: DataEvent) {
        let _ = self.tx.send(AppEvent::Data(event));
    }

    /// Runs `task` in the background and reports its error, if any.
    fn spawn<F>(&self, task: impl FnOnce(Self) -> F + Send + 'static)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        tokio::spawn(task(self.clone()));
    }

    /// Like [`Self::spawn`], for loads that may become pointless before they finish.
    fn spawn_fetch<F>(&self, task: impl FnOnce(Self) -> F + Send + 'static) -> Fetch
    where
        F: Future<Output = ()> + Send + 'static,
    {
        Fetch(tokio::spawn(task(self.clone())).abort_handle())
    }

    pub fn me(&self) {
        if let Some(demo) = self.demo() {
            return self.send(DataEvent::Me(Ok(demo.user.clone())));
        }
        self.spawn(|this| async move {
            let result = catalog::me(&this.api).await;
            if let Ok(user) = &result {
                let _ = this.user_id.set(user.id.clone());
            }
            this.send(DataEvent::Me(result.map_err(|e| format!("{e:#}"))));
        });
    }

    /// Looks for a newer cloofy on GitHub, at most every few hours unless `asked`. Only a look
    /// the user asked for reports finding nothing, or failing.
    pub fn check_update(&self, paths: Paths, asked: bool) {
        self.spawn(move |this| async move {
            match update::check(&paths, asked).await {
                Ok(Some(release)) => this.send(DataEvent::UpdateAvailable(release)),
                Ok(None) if asked => {
                    this.send(DataEvent::Notice(format!("cloofy {} is the latest version", update::current_version())))
                }
                Ok(None) => {}
                Err(err) if asked => this.send(DataEvent::Failed(format!("Couldn't look for updates: {err:#}"))),
                Err(err) => tracing::warn!("Update check failed: {err:#}"),
            }
        });
    }

    /// Downloads `release` and puts it in place of the running binary.
    pub fn install_update(&self, release: Release) {
        self.spawn(move |this| async move {
            let tx = this.tx.clone();
            let mut shown = None;
            let result = update::install(&release, |done, total| {
                let percent = total.filter(|total| *total > 0).map(|total| (done * 100 / total) as u8);
                if percent != shown {
                    shown = percent;
                    if let Some(percent) = percent {
                        let _ = tx.send(AppEvent::Data(DataEvent::UpdateProgress(percent)));
                    }
                }
            })
            .await;
            this.send(match result {
                Ok(binary) => DataEvent::UpdateInstalled(binary),
                Err(err) => DataEvent::UpdateFailed(format!("{err:#}")),
            });
        });
    }

    /// The user's ID; asks Spotify itself when `/me` has not answered yet, as happens for a
    /// playlist restored at startup.
    async fn user_id(&self) -> Option<String> {
        let id = self.user_id.get_or_try_init(|| async { catalog::me(&self.api).await.map(|user| user.id) }).await;
        id.ok().cloned()
    }

    pub fn home(&self, view: u64) {
        if let Some(demo) = self.demo() {
            return self.send(DataEvent::Home { view, result: Ok(demo.home.clone()) });
        }
        self.spawn(move |this| async move {
            let result = async { home::home(this.pathfinder().await?).await }.await;
            this.send(DataEvent::Home { view, result: result.map_err(|e| format!("{e:#}")) });
        });
    }

    pub fn artist(&self, view: u64, uri: String) {
        if let Some(demo) = self.demo() {
            let result = demo.artist_pages.get(&uri).cloned().ok_or_else(|| "This artist isn't in the demo".to_owned());
            return self.send(DataEvent::Artist { view, result });
        }
        self.spawn(move |this| async move {
            let result = async { artist::artist(this.pathfinder().await?, &uri).await }.await;
            this.send(DataEvent::Artist { view, result: result.map_err(|e| format!("{e:#}")) });
        });
    }

    /// Spotify's internal API client, created once the session is up.
    async fn pathfinder(&self) -> Result<&Pathfinder> {
        let session = self.session().await?;
        let hashes = self.pathfinder_hashes.clone();
        Ok(self.pathfinder.get_or_init(|| async move { Pathfinder::new(session, hashes) }).await)
    }

    /// Loads an image at a fixed cell size; `round` masks it to a circle (artist portraits).
    /// `drawn` is the number of the last frame that drew the image.
    pub fn image(&self, key: String, url: String, size: Size, round: bool, drawn: Arc<AtomicU64>) {
        self.spawn(move |this| async move {
            // A fast scroll asks for every cover it passes. Ones gone from the screen by their
            // turn are skipped, so the covers where the scroll stops come first; they are asked
            // for again if they come back.
            let on_screen = || drawn.load(Ordering::Relaxed) >= this.frames_drawn.load(Ordering::Relaxed);
            let result = async {
                let source = this.fetch_image(&url).await?;
                if !on_screen() {
                    return Ok(None);
                }
                // Downloads run freely; decoding waits for a turn, held until the encode is done.
                let _turn = this.encode_turns.acquire().await?;
                if !on_screen() {
                    return Ok(None);
                }
                this.encode_image(source, size, mask(round)).await.map(|(picture, ())| Some(picture))
            }
            .await;
            this.send(match result {
                Ok(Some(picture)) => DataEvent::Image { key, result: Ok(picture) },
                Ok(None) => DataEvent::ImageSkipped { key },
                Err(err) => DataEvent::Image { key, result: Err(format!("{err:#}")) },
            });
        });
    }

    /// Loads an image the interface is likely to show soon, like [`Loader::image`] but one of
    /// two at a time, download included, and whether on screen or not.
    pub fn prefetch_image(&self, key: String, url: String, size: Size, round: bool) {
        self.spawn(move |this| async move {
            let result = async {
                let _turn = this.prefetch_turns.acquire().await?;
                let source = this.fetch_image(&url).await?;
                this.encode_image(source, size, mask(round)).await.map(|(picture, ())| picture)
            }
            .await;
            this.send(DataEvent::Image { key, result: result.map_err(|e| format!("{e:#}")) });
        });
    }

    /// Everything in the library except playlists, each part shown as soon as it is complete.
    pub fn library(&self) {
        self.liked_count();
        self.albums();
        self.artists();
        self.shows();
    }

    pub fn liked_count(&self) {
        if let Some(demo) = self.demo() {
            return self.send(DataEvent::LikedCount(Ok(demo.liked.len() as u32)));
        }
        self.spawn(|this| async move {
            let result = catalog::liked_count(&this.api).await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::LikedCount(result));
        });
    }

    pub fn albums(&self) {
        if let Some(demo) = self.demo() {
            return self.send(DataEvent::Albums(Ok(demo.albums.clone())));
        }
        self.spawn(|this| async move {
            let result = catalog::saved_albums(&this.api).await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Albums(result));
        });
    }

    pub fn artists(&self) {
        if let Some(demo) = self.demo() {
            return self.send(DataEvent::Artists(Ok(demo.artists.clone())));
        }
        self.spawn(|this| async move {
            let result = catalog::followed_artists(&this.api).await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Artists(result));
        });
    }

    pub fn shows(&self) {
        if let Some(demo) = self.demo() {
            return self.send(DataEvent::Shows(Ok(demo.shows.clone())));
        }
        self.spawn(|this| async move {
            let result = catalog::saved_shows(&this.api).await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Shows(result));
        });
    }

    pub fn playlists(&self) {
        if let Some(demo) = self.demo() {
            return self.send(DataEvent::Playlists(Ok(demo.playlists.clone())));
        }
        self.spawn(|this| async move {
            let result = catalog::playlists(&this.api).await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Playlists(result));
        });
    }

    /// Loads all tracks of a source. Pages after the first are fetched a few at a time and
    /// arrive in order.
    pub fn tracks(&self, view: u64, source: TrackSource) -> Fetch {
        if let Some(demo) = self.demo() {
            let items = match &source {
                TrackSource::Liked => demo.liked.clone(),
                TrackSource::Playlist(playlist) => demo.tracks(&playlist.uri),
                TrackSource::Album(album) => demo.tracks(&album.uri),
            };
            let total = items.len() as u32;
            self.send(DataEvent::Tracks { view, page: Ok(Page { items, total, next_offset: None }) });
            return self.spawn_fetch(|_| async {});
        }
        self.spawn_fetch(move |this| async move {
            if let TrackSource::Playlist(playlist) = &source
                && !playlist.collaborative
                && this.user_id().await.as_deref() != Some(playlist.owner_id.as_str())
            {
                if let Err(err) = this.internal_playlist(view, &playlist.id).await {
                    this.send(DataEvent::Tracks { view, page: Err(format!("{err:#}")) });
                }
                return;
            }

            let (api, source) = (&this.api, &source);
            let fetch = |offset| async move {
                match source {
                    TrackSource::Liked => catalog::liked_tracks(api, offset).await,
                    TrackSource::Playlist(p) => catalog::playlist_tracks(api, &p.id, offset).await,
                    TrackSource::Album(a) => catalog::album_tracks(api, a, offset).await,
                }
            };
            let first = fetch(0).await;
            let offsets: Vec<u32> = match &first {
                Ok(Page { next_offset: Some(next), total, .. }) => {
                    (*next..*total).step_by(catalog::PAGE as usize).collect()
                }
                _ => Vec::new(),
            };
            let rest = stream::iter(offsets).map(fetch).buffered(catalog::PAGE_CONCURRENCY);
            let mut pages = std::pin::pin!(stream::iter([first]).chain(rest));
            while let Some(page) = pages.next().await {
                let failed = page.is_err();
                this.send(DataEvent::Tracks { view, page: page.map_err(|e| format!("{e:#}")) });
                if failed {
                    return;
                }
            }
        })
    }

    /// Followed and Spotify-made playlists only resolve through librespot. Their tracks arrive
    /// in batches, so the first rows show while the rest load.
    async fn internal_playlist(&self, view: u64, id: &str) -> Result<()> {
        let session = self.session().await?;
        let uris = catalog::playlist_track_uris(&session, id).await?;
        if uris.is_empty() {
            let page = Page { items: Vec::new(), total: 0, next_offset: None };
            self.send(DataEvent::Tracks { view, page: Ok(page) });
            return Ok(());
        }
        let session = &session;
        let batches: Vec<_> = uris.chunks(catalog::METADATA_BATCH).map(<[_]>::to_vec).collect();
        let mut batches = stream::iter(batches)
            .map(|batch| async move { (batch.len(), catalog::tracks_metadata(session, &batch).await) })
            .buffered(METADATA_BATCHES);
        let (mut requested, mut delivered) = (0, 0);
        while let Some((len, items)) = batches.next().await {
            requested += len;
            delivered += items.len();
            let last = requested == uris.len();
            // Tracks without details are left out, so the final count is what actually arrived.
            let total = if last { delivered } else { uris.len() } as u32;
            let next_offset = (!last).then_some(requested as u32);
            self.send(DataEvent::Tracks { view, page: Ok(Page { items, total, next_offset }) });
        }
        Ok(())
    }

    async fn session(&self) -> Result<Session> {
        let mut session = self.session.clone();
        let state = session.wait_for(|state| !matches!(state, SessionState::Connecting)).await?;
        match &*state {
            SessionState::Ready(session) => Ok(session.clone()),
            SessionState::Failed(err) => bail!("No connection to Spotify: {err}"),
            SessionState::Connecting => unreachable!("checked by wait_for"),
        }
    }

    pub fn search(&self, view: u64, query: String) {
        if let Some(demo) = self.demo() {
            let result = Ok(demo.search(&query));
            return self.send(DataEvent::Search { view, query, result });
        }
        self.spawn(move |this| async move {
            let result = catalog::search(&this.api, &query).await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Search { view, query, result });
        });
    }

    /// Downloads cover art, picks the accent color and encodes it for the terminal.
    pub fn cover(&self, url: String, size: Size) {
        self.spawn(move |this| async move {
            let result = this.load_image(&url, size, |image| theme::accent_from_image(image)).await;
            this.send(DataEvent::Cover { url, result: result.map_err(|e| format!("{e:#}")) });
        });
    }

    pub fn artwork(&self, url: String) {
        self.spawn(move |this| async move {
            let result = this.download_image(&url).await.map(Arc::from).map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Artwork { url, result });
        });
    }

    pub fn large_cover(&self, url: String, size: Size) {
        self.spawn(move |this| async move {
            let result = this.load_image(&url, size, |_| ()).await.map(|(picture, ())| picture);
            this.send(DataEvent::LargeCover { url, size, result: result.map_err(|e| format!("{e:#}")) });
        });
    }

    /// Fetches and encodes an image without waiting for a turn: the player bar and Now Playing
    /// show one cover each, which must not queue behind a screenful of thumbnails.
    async fn load_image<T: Send + 'static>(
        &self,
        url: &str,
        size: Size,
        process: impl FnOnce(&mut image::DynamicImage) -> T + Send + 'static,
    ) -> Result<(Picture, T)> {
        let source = self.fetch_image(url).await?;
        self.encode_image(source, size, process).await
    }

    async fn fetch_image(&self, url: &str) -> Result<ImageSource> {
        Ok(match url {
            art::LIKED_COVER => ImageSource::Drawn(art::liked_cover),
            art::PLACEHOLDER_COVER => ImageSource::Drawn(art::placeholder_cover),
            _ => ImageSource::Downloaded(self.download_image(url).await?),
        })
    }

    /// Decodes, processes and encodes an image for the terminal, on the blocking pool.
    async fn encode_image<T: Send + 'static>(
        &self,
        source: ImageSource,
        size: Size,
        process: impl FnOnce(&mut image::DynamicImage) -> T + Send + 'static,
    ) -> Result<(Picture, T)> {
        let picker = self.picker.clone();
        let keep_source = self.keep_sources;
        tokio::task::spawn_blocking(move || {
            let mut image = match source {
                ImageSource::Downloaded(bytes) => image::load_from_memory(&bytes)?,
                ImageSource::Drawn(draw) => draw(),
            };
            // Every slot is square, so other shapes are cut to their middle square, as Spotify's
            // client does; scaled whole, they would leave a strip beside or below them.
            let (width, height) = (image.width(), image.height());
            if width != height {
                let side = width.min(height);
                image = image.crop_imm((width - side) / 2, (height - side) / 2, side, side);
            }
            let extra = process(&mut image);
            let source = keep_source.then(|| Arc::new(image.clone()));
            // Triangle (bilinear) is several times faster than Lanczos and looks the same when shrinking.
            let (protocol, terminal_bytes) = if picker.protocol_type() == ProtocolType::Kitty {
                // Kitty fits an image into its cells itself, so it is only shrunk to the pixels
                // the cells have, never enlarged: that would only add pixels to encode, send and
                // store, such as a 640 px cover blown up to fill Now Playing.
                let font = picker.font_size();
                let width = u32::from(size.width) * u32::from(font.width);
                let height = u32::from(size.height) * u32::from(font.height);
                if image.width() > width || image.height() > height {
                    image = image.resize(width, height, FilterType::Triangle);
                }
                let terminal_bytes = image.width() as usize * image.height() as usize * 4;
                (picker.new_protocol_cells(image, size)?, terminal_bytes)
            } else {
                // Scale, not Fit: small covers (e.g. user-uploaded playlist images) must fill their slot too.
                (picker.new_protocol(image, size, Resize::Scale(Some(FilterType::Triangle)))?, 0)
            };
            Ok((Picture { protocol, source, terminal_bytes }, extra))
        })
        .await?
    }

    /// Cover art is immutable per URL, so it is cached on disk until unused for a while.
    async fn download_image(&self, url: &str) -> Result<Vec<u8>> {
        let path = self.image_cache.join(cache_name(url));
        if let Ok(bytes) = tokio::fs::read(&path).await {
            return Ok(bytes);
        }
        let bytes = self.http.get(url).send().await?.error_for_status()?.bytes().await?.to_vec();
        let _ = tokio::fs::create_dir_all(&self.image_cache).await;
        let _ = tokio::fs::write(&path, &bytes).await;
        Ok(bytes)
    }

    /// Deletes cached cover art unused for [`IMAGE_CACHE_AGE`], and files named the way older
    /// versions did, which are never read again. Runs in the background.
    pub fn prune_image_cache(&self) {
        let dir = self.image_cache.clone();
        tokio::task::spawn_blocking(move || {
            let Ok(entries) = std::fs::read_dir(&dir) else { return };
            for entry in entries.flatten() {
                if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                    continue;
                }
                let current = entry.file_name().to_str().is_some_and(|name| {
                    name.len() == 64 && name.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                });
                // Reading a file updates its access time, at most once a day on most systems.
                let age = entry.metadata().ok().and_then(|meta| {
                    let modified = meta.modified().ok()?;
                    meta.accessed().map_or(modified, |accessed| accessed.max(modified)).elapsed().ok()
                });
                if !current || age.is_some_and(|age| age > IMAGE_CACHE_AGE) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        });
    }

    pub fn lyrics(&self, track: &CurrentTrack) {
        if let Some(demo) = self.demo() {
            let result = Ok(demo.lyrics.get(&track.uri).cloned());
            return self.send(DataEvent::Lyrics { uri: track.uri.clone(), result });
        }
        let uri = track.uri.clone();
        let title = track.name.clone();
        let artist = track.artists.first().cloned().unwrap_or_default();
        let album = track.album.clone();
        let duration_ms = track.duration_ms;
        self.spawn(move |this| async move {
            let session = tokio::time::timeout(LYRICS_SESSION_WAIT, this.session()).await.ok().and_then(Result::ok);
            let query = LyricsQuery { uri: &uri, title: &title, artist: &artist, album: &album, duration_ms };
            let result = lyrics::fetch(session.as_ref(), &this.http, query).await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Lyrics { uri, result });
        });
    }

    /// Checks which URIs are in the library. Each batch reports as soon as it is known, so the
    /// first rows of a long list get their hearts without waiting for the rest.
    pub fn check_saved(&self, uris: Vec<String>) {
        if uris.is_empty() {
            return;
        }
        if let Some(demo) = self.demo() {
            let flags = uris.into_iter().map(|uri| {
                let saved = demo.is_saved(&uri);
                (uri, saved)
            });
            return self.send(DataEvent::Saved(flags.collect()));
        }
        self.spawn(move |this| async move {
            let api = &this.api;
            let batches: Vec<Vec<String>> = uris.chunks(catalog::LIBRARY_BATCH).map(<[_]>::to_vec).collect();
            let mut checks = stream::iter(batches)
                .map(|batch| async move {
                    let result = catalog::contains(api, &batch).await;
                    (batch, result)
                })
                .buffered(SAVED_CONCURRENCY);
            while let Some((batch, result)) = checks.next().await {
                match result {
                    Ok(flags) => this.send(DataEvent::Saved(batch.into_iter().zip(flags).collect())),
                    Err(err) => tracing::warn!("library check failed: {err:#}"),
                }
            }
        });
    }

    pub fn set_saved(&self, uri: String, name: String, saved: bool) {
        if self.demo.is_some() {
            let notice = if saved { format!("“{name}” saved") } else { format!("“{name}” removed") };
            return self.send(DataEvent::SaveDone { uri, notice });
        }
        self.spawn(move |this| async move {
            let uris = [uri.clone()];
            let result =
                if saved { catalog::save(&this.api, &uris).await } else { catalog::unsave(&this.api, &uris).await };
            match result {
                Ok(()) => {
                    let notice = if saved { format!("“{name}” saved") } else { format!("“{name}” removed") };
                    this.send(DataEvent::SaveDone { uri, notice });
                }
                Err(err) => this.send(DataEvent::SaveFailed { uri, saved: !saved, error: format!("{err:#}") }),
            }
        });
    }

    pub fn playback(&self) {
        if self.demo.is_some() {
            return self.send(DataEvent::Playback(Ok(None)));
        }
        self.spawn(|this| async move {
            let result = connect::playback(&this.api).await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Playback(result));
        });
    }

    pub fn devices(&self) {
        if self.demo.is_some() {
            return self.send(DataEvent::Devices(Ok(Vec::new())));
        }
        self.spawn(|this| async move {
            let result = connect::devices(&this.api).await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Devices(result));
        });
    }

    pub fn transfer(&self, device: Device) {
        if self.demo.is_some() {
            return;
        }
        self.spawn(move |this| async move {
            let result = connect::transfer(&this.api, &device.id).await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Transferred { name: device.name, result });
        });
    }

    /// Controls playback on another Spotify Connect device. Answers with `RemoteDone`.
    pub fn remote(&self, command: connect::Command) {
        if self.demo.is_some() {
            return;
        }
        let _ = self.remote.send(command);
    }

    /// Reports only failures; the app confirms right away.
    pub fn queue(&self, uri: String) {
        if self.demo.is_some() {
            return;
        }
        self.spawn(move |this| async move {
            if let Err(err) = catalog::queue(&this.api, &uri).await {
                this.send(DataEvent::Failed(format!("Queue: {err:#}")));
            }
        });
    }

    /// Reports only failures; the app confirms right away.
    pub fn add_to_playlist(&self, playlist_id: String, uris: Vec<String>) {
        if self.demo.is_some() {
            return;
        }
        self.spawn(move |this| async move {
            if let Err(err) = catalog::add_to_playlist(&this.api, &playlist_id, &uris).await {
                let error = format!("Playlist: {err:#}");
                this.send(DataEvent::AddFailed { playlist_id, count: uris.len() as u32, error });
            }
        });
    }
}

/// The cache file of an image URL: its SHA-256 in hex. Not std's hasher, whose output may change
/// with any Rust release and would orphan the whole cache.
fn cache_name(url: &str) -> String {
    Sha256::digest(url.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

impl Picture {
    /// Whether the terminal holds the image: a kitty image that has been drawn.
    pub fn in_terminal(&self) -> bool {
        matches!(&self.protocol, Protocol::Kitty(kitty) if kitty.transmitted())
    }

    /// The escape sequence that deletes the image from the terminal, if it holds it.
    pub fn terminal_delete(&self) -> Option<String> {
        match &self.protocol {
            Protocol::Kitty(kitty) if kitty.transmitted() => Some(kitty.delete_sequence()),
            _ => None,
        }
    }
}

/// Masks artist portraits to a circle; other images stay as they are.
fn mask(round: bool) -> impl FnOnce(&mut image::DynamicImage) + Send + 'static {
    move |image| {
        if round {
            *image = circle(image);
        }
    }
}

/// Sends remote commands one at a time, so they reach Spotify in the order they were given.
/// Volume and seek commands that pile up meanwhile collapse into the newest one.
async fn send_remote(
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

/// Makes everything outside the inscribed circle transparent, with a soft edge.
fn circle(image: &image::DynamicImage) -> image::DynamicImage {
    let mut rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let radius = width.min(height) as f32 / 2.0;
    let (cx, cy) = (width as f32 / 2.0, height as f32 / 2.0);
    for (x, y, pixel) in rgba.enumerate_pixels_mut() {
        let distance = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
        let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
        pixel.0[3] = (pixel.0[3] as f32 * coverage) as u8;
    }
    rgba.into()
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
