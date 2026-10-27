//! YouTube Music: InnerTube for the catalog and the library, mpv for playback. Signed out, it
//! browses and plays; the library and likes need the sign-in `cloofy login ytmusic` borrows
//! from a browser.

use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result, bail};
use cloofy_core::{
    artist::ArtistPage,
    catalog::{AlbumInfo, ArtistInfo, Page, PlaylistInfo, SearchResults, ShowInfo, Track, User},
    home::Home,
    lyrics::{Lyrics, LyricsQuery},
    paths::Paths,
    service::Service,
};
use cloofy_ytmusic::{Source, YtMusic};
use futures::{
    StreamExt,
    stream::{self, BoxStream},
};
use tokio::sync::mpsc;

use super::{Found, Launch};
use crate::{app::AppEvent, backend::Player, playback::Repeat, views::TrackSource};

/// Where the volume starts without a last session's: half, as for Spotify.
const DEFAULT_VOLUME: u16 = u16::MAX / 2;
/// Songs asked about at once: whether they are liked, or their details for the queue.
const BATCH: usize = 50;
/// Batches of library checks running at the same time.
const SAVED_CONCURRENCY: usize = 2;

/// YouTube Music, signed in or not.
pub struct Account {
    client: YtMusic,
}

impl Account {
    /// Reads the sign-in, if there is one; signed out is fine.
    pub fn open(paths: &Paths) -> Result<Self> {
        Ok(Self { client: YtMusic::new(paths)? })
    }

    /// Starts mpv in the background; the catalog works without it.
    pub fn start(self, launch: Launch) -> Catalog {
        let player = Arc::new(OnceLock::new());
        let catalog = Catalog { client: self.client.clone(), player: player.clone() };
        let Launch { paths, volume, tx, .. } = launch;
        tokio::spawn(async move {
            let (events, mut received) = mpsc::unbounded_channel();
            let volume = volume.unwrap_or(DEFAULT_VOLUME);
            let spawned = cloofy_ytmusic::Player::spawn(self.client, &paths, volume, events).await;
            let started = match spawned {
                Ok(started) => started,
                Err(err) => {
                    let _ = tx.send(AppEvent::PlayerFailed(Service::YouTubeMusic, format!("{err:#}")));
                    return;
                }
            };
            let _ = player.set(started.clone());
            if tx.send(AppEvent::PlayerReady(Box::new(Player::YouTubeMusic(Controller(started))))).is_err() {
                return;
            }
            while let Some(event) = received.recv().await {
                if tx.send(AppEvent::Player(Service::YouTubeMusic, event)).is_err() {
                    break;
                }
            }
        });
        catalog
    }
}

/// cloofy's own YouTube Music player: mpv, and the queue it plays from.
pub struct Controller(cloofy_ytmusic::Player);

impl Controller {
    pub fn play(&self, context_uri: String, track_uri: Option<String>) -> Result<()> {
        self.0.play(context_uri, track_uri)
    }

    pub fn resume(&self, context: Option<String>, track: String, position_ms: u32) -> Result<()> {
        self.0.resume(context, track, position_ms)
    }

    pub fn play_shuffled(&self, context_uri: String) -> Result<()> {
        self.0.play_shuffled(context_uri)
    }

    pub fn toggle(&self, has_track: bool) -> Result<()> {
        self.0.toggle(has_track)
    }

    pub fn pause(&self) -> Result<()> {
        self.0.pause()
    }

    pub fn next(&self) -> Result<()> {
        self.0.next()
    }

    pub fn previous(&self) -> Result<()> {
        self.0.previous()
    }

    pub fn seek(&self, position_ms: u32) -> Result<()> {
        self.0.seek(position_ms)
    }

    pub fn set_volume(&self, volume: u16) -> Result<()> {
        self.0.set_volume(volume)
    }

    pub fn shuffle(&self, on: bool) -> Result<()> {
        self.0.shuffle(on)
    }

    pub fn repeat(&self, mode: Repeat) -> Result<()> {
        self.0.repeat(mode != Repeat::Off, mode == Repeat::Track)
    }

    pub fn clear_queue(&self) -> Result<()> {
        self.0.clear_queue()
    }

    pub fn add_to_queue(&self, uri: &str) -> Result<()> {
        self.0.add_to_queue(uri)
    }

    /// The queue is cloofy's own, so the player jumps there itself.
    pub fn skip_to(&self, uri: &str) -> Result<bool> {
        self.0.skip_to(uri).map(|()| true)
    }

    pub fn shutdown(&self) {
        self.0.shutdown();
    }
}

#[derive(Clone)]
pub struct Catalog {
    client: YtMusic,
    /// The player once mpv is up, which songs added to the queue go to.
    player: Arc<OnceLock<cloofy_ytmusic::Player>>,
}

impl Catalog {
    pub fn signed_in(&self) -> bool {
        self.client.signed_in()
    }

    pub async fn me(&self) -> Result<User> {
        self.client.me().await?.context("Not signed in to YouTube Music")
    }

    pub async fn home(&self) -> Result<Home> {
        self.client.home().await
    }

    pub async fn artist(&self, uri: &str) -> Result<ArtistPage> {
        self.client.artist(uri).await
    }

    pub async fn search(&self, query: &str) -> Result<SearchResults> {
        self.client.search(query).await
    }

    pub fn tracks(&self, source: TrackSource) -> BoxStream<'static, Result<Page<Track>>> {
        let source = match source {
            TrackSource::Liked => Source::Liked,
            TrackSource::Playlist(playlist) => Source::Playlist(playlist.uri),
            TrackSource::Album(album) => Source::Album(album.uri),
        };
        self.client.tracks(source).boxed()
    }

    pub async fn playlists(&self) -> Result<Vec<PlaylistInfo>> {
        self.client.library_playlists().await
    }

    pub async fn liked_count(&self) -> Result<u32> {
        self.client.liked_count().await
    }

    pub async fn saved_albums(&self) -> Result<Vec<AlbumInfo>> {
        self.client.saved_albums().await
    }

    pub async fn followed_artists(&self) -> Result<Vec<ArtistInfo>> {
        self.client.followed_artists().await
    }

    /// cloofy plays no podcasts from YouTube Music.
    pub async fn saved_shows(&self) -> Result<Vec<ShowInfo>> {
        Ok(Vec::new())
    }

    /// YouTube Music's own lyrics, time-synced where it has them, else LRCLIB's.
    pub async fn lyrics(&self, query: LyricsQuery<'_>) -> Result<Option<Lyrics>> {
        self.client.lyrics(&query).await
    }

    /// A batch at a time, so the first rows of a long list get their hearts early.
    pub fn check_saved(&self, uris: Vec<String>) -> BoxStream<'static, Result<Vec<(String, bool)>>> {
        let client = self.client.clone();
        let batches: Vec<Vec<String>> = uris.chunks(BATCH).map(<[_]>::to_vec).collect();
        stream::iter(batches)
            .map(move |batch| {
                let client = client.clone();
                async move { client.check_saved(&batch).await }
            })
            .buffered(SAVED_CONCURRENCY)
            .boxed()
    }

    pub async fn set_saved(&self, uri: &str, saved: bool) -> Result<()> {
        self.client.set_saved(uri, saved).await
    }

    pub async fn add_to_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<()> {
        self.client.add_to_playlist(&format!("ytmusic:playlist:{playlist_id}"), uris).await
    }

    /// cloofy's player keeps the queue, so the song goes there.
    pub async fn add_to_queue(&self, uri: &str) -> Result<()> {
        let player = self.player.get().context("YouTube Music's player isn't running")?;
        player.add_to_queue(uri)
    }

    pub fn queue_tracks(&self, uris: Vec<String>) -> BoxStream<'static, Found> {
        let client = self.client.clone();
        let batches: Vec<Vec<String>> = uris.chunks(BATCH).map(<[_]>::to_vec).collect();
        stream::iter(batches)
            .then(move |asked| {
                let client = client.clone();
                async move {
                    match client.tracks_by_uri(&asked).await {
                        Ok(tracks) => Found { asked, tracks, reached: true },
                        Err(err) => {
                            tracing::warn!("songs of the queue: {err:#}");
                            Found { asked, tracks: Vec::new(), reached: false }
                        }
                    }
                }
            })
            .boxed()
    }

    /// The player always says what plays next itself.
    pub async fn upcoming(&self) -> Result<Vec<Track>> {
        bail!("YouTube Music's player says what plays next itself")
    }
}
