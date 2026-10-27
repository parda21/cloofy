//! Background data loading. Every request runs in its own task and reports back as an event.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use cloofy_core::{
    artist::ArtistPage,
    catalog::{AlbumInfo, ArtistInfo, Page, PlaylistInfo, SearchResults, ShowInfo, Track, User},
    connect::{Device, Playback},
    home::Home,
    lyrics::{self, Lyrics, LyricsQuery},
    paths::Paths,
    player::CurrentTrack,
    update::{self, Release},
};
use futures::{StreamExt, stream};
use image::imageops::FilterType;
use ratatui::layout::Size;
use ratatui_image::{
    Resize,
    picker::{Picker, ProtocolType},
    protocol::Protocol,
};
use sha2::{Digest, Sha256};
use tokio::sync::{Semaphore, mpsc::UnboundedSender, watch};

use crate::{
    app::AppEvent,
    art,
    backend::{self, Catalog, Listed, Remote, Service},
    demo::Demo,
    graphics::{ImageProtocol, ImageTerminal, Inline},
    queue::Upcoming,
    theme::{self, Rgb},
    views::TrackSource,
};

/// Cached cover art unused for this long is deleted at startup.
const IMAGE_CACHE_AGE: Duration = Duration::from_secs(60 * 24 * 60 * 60);

/// A load running in the background; dropping it stops the load.
pub struct Fetch(tokio::task::AbortHandle);

impl Drop for Fetch {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// An encoded image plus, in snapshot mode, its source for pasting into PNG snapshots.
pub struct Picture {
    pub graphic: Graphic,
    pub source: Option<Arc<image::DynamicImage>>,
    /// Memory the terminal keeps for the image once drawn. Only kitty keeps images, until told
    /// to delete them; the other protocols draw them and forget.
    pub terminal_bytes: usize,
}

pub enum Graphic {
    /// Kitty placeholders or half blocks: cells that ratatui draws like text.
    Cells(Protocol),
    /// Sixel or iTerm2: escape sequences that cloofy writes after the frame.
    Inline(Arc<Inline>),
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
    /// Details of tracks in the queue, for the URIs in `asked`; tracks Spotify has none for are
    /// left out. With `reached` false no request got to Spotify, so they can be asked for again.
    QueueTracks {
        asked: Vec<String>,
        tracks: Vec<Track>,
        reached: bool,
    },
    /// The Web API's list of what plays after `playing`.
    Upcoming {
        playing: String,
        result: Result<Vec<Track>, String>,
    },
    /// The device took a skip to a track in the queue, or turned it down.
    SkippedTo(Result<(), String>),
    /// What Spotify suggests asking DJ for.
    DjSuggestions(Result<Vec<String>, String>),
    /// DJ took a request, or turned it down.
    DjAsked(Result<(), String>),
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
    /// What each service started has to show, asked by the URI's service.
    catalogs: Arc<[Catalog]>,
    /// The service whose Home, Library and Search show.
    service: Option<Service>,
    tx: UnboundedSender<AppEvent>,
    http: reqwest::Client,
    /// What the terminal said about images at startup.
    image_terminal: Arc<ImageTerminal>,
    /// The protocol chosen for covers, and what encodes them for it. It can change while
    /// cloofy runs; covers encoded before are dropped as they arrive.
    picker: Arc<watch::Sender<(ImageProtocol, Picker)>>,
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
    /// The sample `cloofy demo` shows instead of the account's data.
    demo: Option<Arc<Demo>>,
}

/// An image to decode: downloaded bytes, or artwork cloofy draws itself.
enum ImageSource {
    Downloaded(Vec<u8>),
    Drawn(fn() -> image::DynamicImage),
}

impl Loader {
    /// `service` is the one whose Home, Library and Search show first.
    pub fn new(
        catalogs: Vec<Catalog>,
        service: Option<Service>,
        tx: UnboundedSender<AppEvent>,
        image_terminal: ImageTerminal,
        image_cache: PathBuf,
        keep_sources: bool,
    ) -> Self {
        let http = cloofy_core::http::client(concat!("cloofy/", env!("CARGO_PKG_VERSION"))).build().unwrap_or_default();
        // Leave cores for the interface and audio.
        let turns = std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(2).clamp(1, 4));
        let picker = image_terminal.picker(ImageProtocol::Auto);
        Self {
            catalogs: catalogs.into(),
            service,
            tx,
            http,
            image_terminal: Arc::new(image_terminal),
            picker: Arc::new(watch::Sender::new((ImageProtocol::Auto, picker))),
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

    /// The protocol chosen for covers.
    pub fn image_protocol(&self) -> ImageProtocol {
        self.picker.borrow().0
    }

    /// What [`ImageProtocol::Auto`] draws with in this terminal.
    pub fn auto_image_protocol(&self) -> ImageProtocol {
        self.image_terminal.auto()
    }

    /// Encodes covers for `protocol` from now on.
    pub fn set_image_protocol(&self, protocol: ImageProtocol) {
        self.picker.send_replace((protocol, self.image_terminal.picker(protocol)));
    }

    /// What covers are encoded as now.
    pub fn image_kind(&self) -> ProtocolType {
        self.picker.borrow().1.protocol_type()
    }

    fn demo(&self) -> Option<&Demo> {
        self.demo.as_deref()
    }

    /// The service whose Home, Library and Search show.
    pub fn service(&self) -> Option<Service> {
        self.service
    }

    /// The catalog of `service`, if it was started.
    fn catalog(&self, service: Option<Service>) -> Result<Catalog> {
        let service = service.context("No music service to ask")?;
        let catalog = self.catalogs.iter().find(|catalog| catalog.service() == service);
        catalog.cloned().with_context(|| format!("{} isn't enabled", service.name()))
    }

    /// The catalog of the service `uri` belongs to.
    fn catalog_of(&self, uri: &str) -> Result<Catalog> {
        self.catalog(Service::of(uri))
    }

    /// The catalog of the service shown.
    fn shown(&self) -> Result<Catalog> {
        self.catalog(self.service)
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
            let result = async { this.shown()?.me().await }.await;
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
        let Some(variant) = backend::VARIANT else {
            return self.send(DataEvent::UpdateFailed("This cloofy has no release to update to".into()));
        };
        self.spawn(move |this| async move {
            let tx = this.tx.clone();
            let mut shown = None;
            let result = update::install(&release, variant, |done, total| {
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

    pub fn home(&self, view: u64) {
        if let Some(demo) = self.demo() {
            return self.send(DataEvent::Home { view, result: Ok(demo.home.clone()) });
        }
        self.spawn(move |this| async move {
            let result = async { this.shown()?.home().await }.await;
            this.send(DataEvent::Home { view, result: result.map_err(|e| format!("{e:#}")) });
        });
    }

    pub fn artist(&self, view: u64, uri: String) {
        if let Some(demo) = self.demo() {
            let result = demo.artist_pages.get(&uri).cloned().ok_or_else(|| "This artist isn't in the demo".to_owned());
            return self.send(DataEvent::Artist { view, result });
        }
        self.spawn(move |this| async move {
            let result = async { this.catalog_of(&uri)?.artist(&uri).await }.await;
            this.send(DataEvent::Artist { view, result: result.map_err(|e| format!("{e:#}")) });
        });
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
            let result = async { this.shown()?.liked_count().await }.await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::LikedCount(result));
        });
    }

    pub fn albums(&self) {
        if let Some(demo) = self.demo() {
            return self.send(DataEvent::Albums(Ok(demo.albums.clone())));
        }
        self.spawn(|this| async move {
            let result = async { this.shown()?.saved_albums().await }.await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Albums(result));
        });
    }

    pub fn artists(&self) {
        if let Some(demo) = self.demo() {
            return self.send(DataEvent::Artists(Ok(demo.artists.clone())));
        }
        self.spawn(|this| async move {
            let result = async { this.shown()?.followed_artists().await }.await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Artists(result));
        });
    }

    pub fn shows(&self) {
        if let Some(demo) = self.demo() {
            return self.send(DataEvent::Shows(Ok(demo.shows.clone())));
        }
        self.spawn(|this| async move {
            let result = async { this.shown()?.saved_shows().await }.await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Shows(result));
        });
    }

    pub fn playlists(&self) {
        if let Some(demo) = self.demo() {
            return self.send(DataEvent::Playlists(Ok(demo.playlists.clone())));
        }
        self.spawn(|this| async move {
            let result = async { this.shown()?.playlists().await }.await.map_err(|e| format!("{e:#}"));
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
            let service = match &source {
                TrackSource::Liked => this.service,
                TrackSource::Playlist(p) => Service::of(&p.uri),
                TrackSource::Album(a) => Service::of(&a.uri),
            };
            let mut pages = match this.catalog(service) {
                Ok(catalog) => catalog.tracks(source),
                Err(err) => stream::iter([Err(err)]).boxed(),
            };
            while let Some(page) = pages.next().await {
                let failed = page.is_err();
                this.send(DataEvent::Tracks { view, page: page.map_err(|e| format!("{e:#}")) });
                if failed {
                    return;
                }
            }
        })
    }

    pub fn search(&self, view: u64, query: String) {
        if let Some(demo) = self.demo() {
            let result = Ok(demo.search(&query));
            return self.send(DataEvent::Search { view, query, result });
        }
        self.spawn(move |this| async move {
            let result = async { this.shown()?.search(&query).await }.await.map_err(|e| format!("{e:#}"));
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
        let picker = self.picker.borrow().1.clone();
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
            let (graphic, terminal_bytes) = match picker.protocol_type() {
                ProtocolType::Kitty => {
                    // Kitty fits an image into its cells itself, so it is only shrunk to the
                    // pixels the cells have, never enlarged: that would only add pixels to
                    // encode, send and store, such as a 640 px cover blown up to fill Now Playing.
                    let font = picker.font_size();
                    let width = u32::from(size.width) * u32::from(font.width);
                    let height = u32::from(size.height) * u32::from(font.height);
                    if image.width() > width || image.height() > height {
                        image = image.resize(width, height, FilterType::Triangle);
                    }
                    let terminal_bytes = image.width() as usize * image.height() as usize * 4;
                    (Graphic::Cells(picker.new_protocol_cells(image, size)?), terminal_bytes)
                }
                kind @ (ProtocolType::Sixel | ProtocolType::Iterm2) => {
                    let inline = Inline::new(kind, &image, size, picker.font_size(), picker.tmux_detected())?;
                    (Graphic::Inline(Arc::new(inline)), 0)
                }
                // Scale, not Fit: small covers (e.g. user-uploaded playlist images) must fill their slot too.
                ProtocolType::Halfblocks => {
                    (Graphic::Cells(picker.new_protocol(image, size, Resize::Scale(Some(FilterType::Triangle)))?), 0)
                }
            };
            Ok((Picture { graphic, source, terminal_bytes }, extra))
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
            let query = LyricsQuery { uri: &uri, title: &title, artist: &artist, album: &album, duration_ms };
            // A service that isn't on leaves LRCLIB.
            let result = match this.catalog_of(&uri) {
                Ok(catalog) => catalog.lyrics(query).await,
                Err(_) => lyrics::lrclib(&this.http, &query).await,
            };
            this.send(DataEvent::Lyrics { uri, result: result.map_err(|e| format!("{e:#}")) });
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
            for (service, uris) in by_service(uris) {
                let Ok(catalog) = this.catalog(Some(service)) else { continue };
                let mut checks = catalog.check_saved(uris);
                while let Some(result) = checks.next().await {
                    match result {
                        Ok(flags) => this.send(DataEvent::Saved(flags)),
                        Err(err) => tracing::warn!("library check failed: {err:#}"),
                    }
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
            let result = async { this.catalog_of(&uri)?.set_saved(&uri, saved).await }.await;
            match result {
                Ok(()) => {
                    let notice = if saved { format!("“{name}” saved") } else { format!("“{name}” removed") };
                    this.send(DataEvent::SaveDone { uri, notice });
                }
                Err(err) => this.send(DataEvent::SaveFailed { uri, saved: !saved, error: format!("{err:#}") }),
            }
        });
    }

    /// What plays on `service`'s devices.
    pub fn playback(&self, service: Option<Service>) {
        if self.demo.is_some() {
            return self.send(DataEvent::Playback(Ok(None)));
        }
        self.spawn(move |this| async move {
            let result = async { this.catalog(service)?.playback().await }.await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Playback(result));
        });
    }

    pub fn devices(&self, service: Option<Service>) {
        if self.demo.is_some() {
            return self.send(DataEvent::Devices(Ok(Vec::new())));
        }
        self.spawn(move |this| async move {
            let result = async { this.catalog(service)?.devices().await }.await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Devices(result));
        });
    }

    pub fn transfer(&self, service: Option<Service>, device: Device) {
        if self.demo.is_some() {
            return;
        }
        self.spawn(move |this| async move {
            let result = async { this.catalog(service)?.transfer(&device.id).await }.await;
            this.send(DataEvent::Transferred { name: device.name, result: result.map_err(|e| format!("{e:#}")) });
        });
    }

    /// What the service suggests asking DJ for while `track_uri` plays. Answers with
    /// `DjSuggestions`.
    pub fn dj_suggestions(&self, track_uri: String) {
        if self.demo.is_some() {
            return self.send(DataEvent::DjSuggestions(Ok(Vec::new())));
        }
        self.spawn(move |this| async move {
            let suggestions = async { this.catalog_of(&track_uri)?.dj_suggestions(&track_uri).await };
            let result = suggestions.await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::DjSuggestions(result));
        });
    }

    /// Waits for DJ to take a request. Answers with `DjAsked`.
    pub fn ask_dj(&self, answer: impl Future<Output = anyhow::Result<()>> + Send + 'static) {
        self.spawn(move |this| async move {
            let result = answer.await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::DjAsked(result));
        });
    }

    /// Controls playback on another of `service`'s devices. Answers with `RemoteDone`.
    pub fn remote(&self, service: Option<Service>, command: Remote) {
        if self.demo.is_some() {
            return;
        }
        if let Ok(catalog) = self.catalog(service) {
            catalog.remote(command);
        }
    }

    /// Reports only failures; the app confirms right away.
    pub fn queue(&self, uri: String) {
        if self.demo.is_some() {
            return;
        }
        self.spawn(move |this| async move {
            if let Err(err) = async { this.catalog_of(&uri)?.add_to_queue(&uri).await }.await {
                this.send(DataEvent::Failed(format!("Queue: {err:#}")));
            }
        });
    }

    /// Details of tracks in the queue, a batch at a time. Answers with `QueueTracks`.
    pub fn queue_tracks(&self, uris: Vec<String>) {
        if let Some(demo) = self.demo() {
            let tracks = uris.iter().filter_map(|uri| demo.track(uri)).cloned().collect();
            return self.send(DataEvent::QueueTracks { asked: uris, tracks, reached: true });
        }
        self.spawn(move |this| async move {
            let (known, unknown): (Vec<_>, Vec<_>) =
                uris.into_iter().partition(|uri| Service::of(uri).is_some_and(|s| this.catalog(Some(s)).is_ok()));
            // No service to ask has nothing to say about these.
            if !unknown.is_empty() {
                this.send(DataEvent::QueueTracks { asked: unknown, tracks: Vec::new(), reached: true });
            }
            for (service, uris) in by_service(known) {
                let Ok(catalog) = this.catalog(Some(service)) else { continue };
                let mut found = catalog.queue_tracks(uris);
                while let Some(found) = found.next().await {
                    let (asked, tracks, reached) = (found.asked, found.tracks, found.reached);
                    this.send(DataEvent::QueueTracks { asked, tracks, reached });
                }
            }
        });
    }

    /// The Web API's list of what plays next, for when Spotify hasn't said yet in its own way.
    /// Answers with `Upcoming`.
    pub fn upcoming(&self, playing: String) {
        if self.demo.is_some() {
            return;
        }
        self.spawn(move |this| async move {
            let result = async { this.catalog_of(&playing)?.upcoming().await }.await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Upcoming { playing, result });
        });
    }

    /// Skips ahead to a track in the queue of `device`. Answers with `SkippedTo`.
    pub fn skip_to(&self, device: String, entry: Upcoming) {
        if self.demo.is_some() {
            return;
        }
        self.spawn(move |this| async move {
            let skip = async { this.catalog_of(&entry.uri)?.skip_to(&device, &entry).await };
            let result = skip.await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::SkippedTo(result));
        });
    }

    /// Hands `device` of `service` its queue back with a change. Reports only failures; the
    /// device's next state shows the change.
    pub fn set_queue(&self, service: Option<Service>, device: String, listed: Listed) {
        if self.demo.is_some() {
            return;
        }
        self.spawn(move |this| async move {
            let set = async { this.catalog(service)?.set_queue(&device, &listed).await };
            if let Err(err) = set.await {
                this.send(DataEvent::Failed(format!("Queue: {err:#}")));
            }
        });
    }

    /// Reports only failures; the app confirms right away.
    pub fn add_to_playlist(&self, playlist: &PlaylistInfo, uris: Vec<String>) {
        if self.demo.is_some() {
            return;
        }
        let (playlist_uri, playlist_id) = (playlist.uri.clone(), playlist.id.clone());
        self.spawn(move |this| async move {
            let added = async { this.catalog_of(&playlist_uri)?.add_to_playlist(&playlist_id, &uris).await };
            if let Err(err) = added.await {
                let error = format!("Playlist: {err:#}");
                this.send(DataEvent::AddFailed { playlist_id, count: uris.len() as u32, error });
            }
        });
    }
}

/// `uris` by the service each belongs to, in order; ones of no service are left out.
fn by_service(uris: Vec<String>) -> Vec<(Service, Vec<String>)> {
    let mut groups: Vec<(Service, Vec<String>)> = Vec::new();
    for uri in uris {
        let Some(service) = Service::of(&uri) else { continue };
        match groups.iter_mut().find(|(s, _)| *s == service) {
            Some((_, group)) => group.push(uri),
            None => groups.push((service, vec![uri])),
        }
    }
    groups
}

/// The cache file of an image URL: its SHA-256 in hex. Not std's hasher, whose output may change
/// with any Rust release and would orphan the whole cache.
fn cache_name(url: &str) -> String {
    Sha256::digest(url.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

impl Picture {
    /// In cells.
    pub fn size(&self) -> Size {
        match &self.graphic {
            Graphic::Cells(protocol) => protocol.size(),
            Graphic::Inline(inline) => inline.size(),
        }
    }

    /// The protocol it was encoded for.
    pub fn kind(&self) -> ProtocolType {
        match &self.graphic {
            Graphic::Cells(Protocol::Kitty(_)) => ProtocolType::Kitty,
            Graphic::Cells(Protocol::Sixel(_)) => ProtocolType::Sixel,
            Graphic::Cells(Protocol::ITerm2(_)) => ProtocolType::Iterm2,
            Graphic::Cells(Protocol::Halfblocks(_)) => ProtocolType::Halfblocks,
            Graphic::Inline(inline) => inline.kind(),
        }
    }

    /// Whether the terminal holds the image: a kitty image that has been drawn.
    pub fn in_terminal(&self) -> bool {
        matches!(&self.graphic, Graphic::Cells(Protocol::Kitty(kitty)) if kitty.transmitted())
    }

    /// The escape sequence that deletes the image from the terminal, if it holds it.
    pub fn terminal_delete(&self) -> Option<String> {
        match &self.graphic {
            Graphic::Cells(Protocol::Kitty(kitty)) if kitty.transmitted() => Some(kitty.delete_sequence()),
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
    use super::*;

    #[test]
    fn requests_go_to_the_service_of_each_uri() {
        let uris = ["spotify:track:1", "ytmusic:track:2", "file:3", "spotify:track:4"].map(String::from).to_vec();
        let groups = by_service(uris);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0], (Service::Spotify, vec!["spotify:track:1".to_owned(), "spotify:track:4".to_owned()]));
        assert_eq!(groups[1], (Service::YouTubeMusic, vec!["ytmusic:track:2".to_owned()]));
    }
}
