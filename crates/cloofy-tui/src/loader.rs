//! Background data loading. Every request runs in its own task and reports back as an event.

use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use cloofy_spotify::{
    artist::{self, ArtistPage},
    catalog::{self, AlbumInfo, ArtistInfo, Page, PlaylistInfo, SearchResults, ShowInfo, Track, User},
    connect::{self, Device, Playback},
    home::{self, Home},
    lyrics::{self, Lyrics, LyricsQuery},
    pathfinder::Pathfinder,
    webapi::WebApi,
};
use image::imageops::FilterType;
use librespot::core::session::Session;
use ratatui::layout::Size;
use ratatui_image::{Resize, picker::Picker, protocol::Protocol};
use tokio::sync::{OnceCell, Semaphore, mpsc::UnboundedSender, watch};

use crate::{
    app::AppEvent,
    art,
    playback::CurrentTrack,
    theme::{self, Rgb},
    views::TrackSource,
};

/// An encoded image plus, in snapshot mode, its source for pasting into PNG snapshots.
pub struct Picture {
    pub protocol: Protocol,
    pub source: Option<Arc<image::DynamicImage>>,
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
    Playlists(Result<Vec<PlaylistInfo>, String>),
    /// The rest of the library; each part may fail on its own.
    Library {
        liked: Result<u32, String>,
        albums: Result<Vec<AlbumInfo>, String>,
        artists: Result<Vec<ArtistInfo>, String>,
        shows: Result<Vec<ShowInfo>, String>,
    },
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
    /// What plays where, from Spotify Connect.
    Playback(Result<Option<Playback>, String>),
    Devices(Result<Vec<Device>, String>),
    Notice(String),
    Failed(String),
}

#[derive(Clone)]
pub struct Loader {
    api: Arc<WebApi>,
    tx: UnboundedSender<AppEvent>,
    session: watch::Receiver<Option<Session>>,
    http: reqwest::Client,
    picker: Picker,
    pathfinder: Arc<OnceCell<Pathfinder>>,
    pathfinder_hashes: PathBuf,
    /// Keep decoded images for dev snapshots.
    keep_sources: bool,
    image_cache: PathBuf,
    /// Turns for decoding and encoding covers. A screenful asks for dozens at once; run all
    /// together, they would hog every core and need a few megabytes of buffers each.
    encode_turns: Arc<Semaphore>,
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
        session: watch::Receiver<Option<Session>>,
        picker: Picker,
        pathfinder_hashes: PathBuf,
        image_cache: PathBuf,
        keep_sources: bool,
    ) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("cloofy/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        // Leave cores for the interface and audio.
        let turns = std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(2).clamp(1, 4));
        Self {
            api,
            tx,
            session,
            http,
            picker,
            pathfinder: Arc::default(),
            pathfinder_hashes,
            keep_sources,
            image_cache,
            encode_turns: Arc::new(Semaphore::new(turns)),
        }
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

    pub fn me(&self) {
        self.spawn(|this| async move {
            let result = catalog::me(&this.api).await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Me(result));
        });
    }

    pub fn home(&self, view: u64) {
        self.spawn(move |this| async move {
            let result = async { home::home(this.pathfinder().await?).await }.await;
            this.send(DataEvent::Home { view, result: result.map_err(|e| format!("{e:#}")) });
        });
    }

    pub fn artist(&self, view: u64, uri: String) {
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
    pub fn image(&self, key: String, url: String, size: Size, round: bool) {
        self.spawn(move |this| async move {
            let result = async {
                let source = this.fetch_image(&url).await?;
                // Downloads run freely; decoding waits for a turn, held until the encode is done.
                let _turn = this.encode_turns.acquire().await?;
                let mask = move |image: &mut image::DynamicImage| {
                    if round {
                        *image = circle(image);
                    }
                };
                this.encode_image(source, size, mask).await.map(|(picture, ())| picture)
            }
            .await;
            this.send(DataEvent::Image { key, result: result.map_err(|e| format!("{e:#}")) });
        });
    }

    pub fn library(&self) {
        self.spawn(|this| async move {
            let api = &this.api;
            let (liked, albums, artists, shows) = tokio::join!(
                catalog::liked_count(api),
                catalog::saved_albums(api),
                catalog::followed_artists(api),
                catalog::saved_shows(api),
            );
            let text = |e: anyhow::Error| format!("{e:#}");
            this.send(DataEvent::Library {
                liked: liked.map_err(text),
                albums: albums.map_err(text),
                artists: artists.map_err(text),
                shows: shows.map_err(text),
            });
        });
    }

    pub fn playlists(&self) {
        self.spawn(|this| async move {
            let result = catalog::playlists(&this.api).await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Playlists(result));
        });
    }

    /// Loads all tracks of a source, page by page.
    pub fn tracks(&self, view: u64, source: TrackSource, me: Option<String>) {
        self.spawn(move |this| async move {
            if let TrackSource::Playlist(playlist) = &source
                && me.as_deref() != Some(playlist.owner_id.as_str())
                && !playlist.collaborative
            {
                let page = this.internal_playlist(&playlist.id).await;
                this.send(DataEvent::Tracks { view, page: page.map_err(|e| format!("{e:#}")) });
                return;
            }

            let mut offset = Some(0);
            while let Some(current) = offset {
                let page = match &source {
                    TrackSource::Liked => catalog::liked_tracks(&this.api, current).await,
                    TrackSource::Playlist(p) => catalog::playlist_tracks(&this.api, &p.id, current).await,
                    TrackSource::Album(a) => catalog::album_tracks(&this.api, a, current).await,
                };
                offset = page.as_ref().ok().and_then(|p| p.next_offset);
                this.send(DataEvent::Tracks { view, page: page.map_err(|e| format!("{e:#}")) });
            }
        });
    }

    /// Followed and Spotify-made playlists only resolve through librespot.
    async fn internal_playlist(&self, id: &str) -> Result<Page<Track>> {
        let session = self.session().await?;
        let items = catalog::playlist_tracks_internal(&session, id).await?;
        Ok(Page { total: items.len() as u32, items, next_offset: None })
    }

    async fn session(&self) -> Result<Session> {
        let mut session = self.session.clone();
        let session = session.wait_for(Option::is_some).await?;
        Ok(session.clone().expect("checked by wait_for"))
    }

    pub fn search(&self, view: u64, query: String) {
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
            // Scale, not Fit: small covers (e.g. user-uploaded playlist images) must fill their slot too.
            let source = keep_source.then(|| Arc::new(image.clone()));
            // Triangle (bilinear) is several times faster than Lanczos and looks the same when shrinking.
            let protocol = picker.new_protocol(image, size, Resize::Scale(Some(FilterType::Triangle)))?;
            Ok((Picture { protocol, source }, extra))
        })
        .await?
    }

    /// Cover art is immutable per URL, so it is cached on disk for good.
    async fn download_image(&self, url: &str) -> Result<Vec<u8>> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        url.hash(&mut hasher);
        let path = self.image_cache.join(format!("{:016x}", hasher.finish()));
        if let Ok(bytes) = tokio::fs::read(&path).await {
            return Ok(bytes);
        }
        let bytes = self.http.get(url).send().await?.error_for_status()?.bytes().await?.to_vec();
        let _ = tokio::fs::create_dir_all(&self.image_cache).await;
        let _ = tokio::fs::write(&path, &bytes).await;
        Ok(bytes)
    }

    pub fn lyrics(&self, track: &CurrentTrack) {
        let uri = track.uri.clone();
        let title = track.name.clone();
        let artist = track.artists.first().cloned().unwrap_or_default();
        let album = track.album.clone();
        let duration_ms = track.duration_ms;
        self.spawn(move |this| async move {
            let session = this.session.borrow().clone();
            let query = LyricsQuery { uri: &uri, title: &title, artist: &artist, album: &album, duration_ms };
            let result = lyrics::fetch(session.as_ref(), &this.http, query).await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Lyrics { uri, result });
        });
    }

    pub fn check_saved(&self, uris: Vec<String>) {
        if uris.is_empty() {
            return;
        }
        self.spawn(move |this| async move {
            match catalog::contains(&this.api, &uris).await {
                Ok(flags) => this.send(DataEvent::Saved(uris.into_iter().zip(flags).collect())),
                Err(err) => tracing::warn!("library check failed: {err:#}"),
            }
        });
    }

    pub fn set_saved(&self, uri: String, name: String, saved: bool) {
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
        self.spawn(|this| async move {
            let result = connect::playback(&this.api).await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Playback(result));
        });
    }

    pub fn devices(&self) {
        self.spawn(|this| async move {
            let result = connect::devices(&this.api).await.map_err(|e| format!("{e:#}"));
            this.send(DataEvent::Devices(result));
        });
    }

    pub fn transfer(&self, device: Device) {
        self.spawn(move |this| async move {
            match connect::transfer(&this.api, &device.id).await {
                Ok(()) => this.send(DataEvent::Notice(format!("Playing on {}", device.name))),
                Err(err) => this.send(DataEvent::Failed(format!("Transfer: {err:#}"))),
            }
        });
    }

    /// Controls playback on another Spotify Connect device.
    pub fn remote(&self, command: connect::Command) {
        self.spawn(move |this| async move {
            if let Err(err) = connect::send(&this.api, command).await {
                this.send(DataEvent::Failed(format!("Remote control: {err:#}")));
            }
        });
    }

    pub fn queue(&self, uri: String, name: String) {
        self.spawn(move |this| async move {
            match catalog::queue(&this.api, &uri).await {
                Ok(()) => this.send(DataEvent::Notice(format!("“{name}” added to queue"))),
                Err(err) => this.send(DataEvent::Failed(format!("Queue: {err:#}"))),
            }
        });
    }

    pub fn add_to_playlist(&self, playlist: PlaylistInfo, uris: Vec<String>) {
        self.spawn(move |this| async move {
            match catalog::add_to_playlist(&this.api, &playlist.id, &uris).await {
                Ok(()) => this.send(DataEvent::Notice(format!("Added to “{}”", playlist.name))),
                Err(err) => this.send(DataEvent::Failed(format!("Playlist: {err:#}"))),
            }
        });
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
