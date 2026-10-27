//! YouTube Music for cloofy: the catalog through InnerTube (the API music.youtube.com itself
//! uses), playback through mpv with stream URLs from yt-dlp. Signed out is a first-class mode:
//! browsing and playing work without an account; liking and the library need the cookies of
//! a browser that is signed in.

pub mod auth;
mod cookies;
pub mod doctor;
mod home;
mod innertube;
mod library;
mod lyrics;
mod pages;
mod parse;
mod player;
mod search;
mod watch;

use std::sync::Arc;

use anyhow::Result;
use cloofy_core::{
    artist::ArtistPage,
    catalog::{AlbumInfo, ArtistInfo, Page, PlaylistInfo, SearchResults, Track, User},
    home::Home,
    lyrics::{Lyrics, LyricsQuery},
    paths::Paths,
};
use futures::Stream;
use serde_json::Value;

pub use crate::player::Player;
use crate::{innertube::InnerTube, library::Library};

/// What `tracks` lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The user's liked songs (`ytmusic:playlist:LM`).
    Liked,
    /// A playlist by URI.
    Playlist(String),
    /// An album by URI.
    Album(String),
}

/// The YouTube Music client. Cheap to clone; clones share the connection and the sign-in.
#[derive(Clone)]
pub struct YtMusic {
    inner: Arc<Inner>,
}

struct Inner {
    api: InnerTube,
    http: reqwest::Client,
    library: Library,
}

impl YtMusic {
    /// Signs in with the cookies saved by `cloofy login ytmusic`, if any; signed out otherwise.
    pub fn new(paths: &Paths) -> Result<Self> {
        let api = InnerTube::new(paths.ytmusic_cookies())?;
        let http = cloofy_core::http::client(concat!("cloofy/", env!("CARGO_PKG_VERSION"))).build()?;
        Ok(Self { inner: Arc::new(Inner { api, http, library: Library::default() }) })
    }

    pub fn signed_in(&self) -> bool {
        self.inner.api.signed_in()
    }

    /// The signed-in account; `None` when signed out.
    pub async fn me(&self) -> Result<Option<User>> {
        if !self.signed_in() {
            return Ok(None);
        }
        library::me(&self.inner.api).await.map(Some)
    }

    /// The Home feed's shelves, as music.youtube.com shows them. Home items have no kind for
    /// songs, so a song is its radio: a playlist (`ytmusic:playlist:RDAMVM<id>`) to open, whose
    /// `context_uri` is the song itself (`ytmusic:track:<id>`) to play.
    pub async fn home(&self) -> Result<Home> {
        home::home(&self.inner.api).await
    }

    /// Songs, albums, artists and playlists for `query`.
    pub async fn search(&self, query: &str) -> Result<SearchResults> {
        search::search(&self.inner.api, query).await
    }

    /// Completions for a query being typed.
    pub async fn suggestions(&self, query: &str) -> Result<Vec<String>> {
        search::suggestions(&self.inner.api, query).await
    }

    pub async fn artist(&self, uri: &str) -> Result<ArtistPage> {
        pages::artist(&self.inner.api, uri).await
    }

    pub async fn playlist_info(&self, uri: &str) -> Result<PlaylistInfo> {
        pages::playlist_info(&self.inner.api, uri).await
    }

    pub async fn album_info(&self, uri: &str) -> Result<AlbumInfo> {
        let (album, playlist) = pages::album_info(&self.inner.api, uri).await?;
        self.inner.library.remember_album(&album.uri, playlist);
        Ok(album)
    }

    /// The songs of a playlist, an album or the liked songs, a page at a time as YouTube sends
    /// them (100 per page for playlists). Every page but the last has `next_offset` set to the
    /// number of songs so far; `total` is the best count known by then.
    pub fn tracks(&self, source: Source) -> impl Stream<Item = Result<Page<Track>>> + Send + 'static {
        pages::tracks(self.clone(), source)
    }

    /// The playlists in the library, without Liked Music.
    pub async fn library_playlists(&self) -> Result<Vec<PlaylistInfo>> {
        library::playlists(&self.inner.api, &self.inner.library).await
    }

    pub async fn saved_albums(&self) -> Result<Vec<AlbumInfo>> {
        library::albums(&self.inner.api, &self.inner.library).await
    }

    /// The artists the user subscribed to.
    pub async fn followed_artists(&self) -> Result<Vec<ArtistInfo>> {
        library::artists(&self.inner.api, &self.inner.library).await
    }

    pub async fn liked_count(&self) -> Result<u32> {
        library::liked_count(&self.inner.api).await
    }

    /// Songs by URI, with their details, in the given order; unknown ones are left out.
    pub async fn tracks_by_uri(&self, uris: &[String]) -> Result<Vec<Track>> {
        watch::tracks_by_uri(&self.inner.api, uris).await
    }

    /// A song's radio: the song, then what YouTube Music would play after it.
    pub async fn radio(&self, track_uri: &str) -> Result<Vec<Track>> {
        watch::radio(&self.inner.api, track_uri).await
    }

    /// Time-synced lyrics where YouTube Music has them, else its plain ones, else LRCLIB's.
    pub async fn lyrics(&self, query: &LyricsQuery<'_>) -> Result<Option<Lyrics>> {
        lyrics::lyrics(&self.inner.api, &self.inner.http, query).await
    }

    /// Whether each URI is in the library: liked songs, saved albums and playlists, subscribed
    /// artists. Signed out, nothing is.
    pub async fn check_saved(&self, uris: &[String]) -> Result<Vec<(String, bool)>> {
        library::check_saved(&self.inner.api, &self.inner.library, uris).await
    }

    /// Likes or unlikes a song, saves or removes an album or playlist, subscribes to or
    /// unsubscribes from an artist.
    pub async fn set_saved(&self, uri: &str, saved: bool) -> Result<()> {
        library::set_saved(&self.inner.api, &self.inner.library, uri, saved).await
    }

    pub async fn add_to_playlist(&self, playlist_uri: &str, uris: &[String]) -> Result<()> {
        library::add_to_playlist(&self.inner.api, playlist_uri, uris).await
    }

    /// A raw InnerTube call, for diagnostics and the `ytm` example.
    #[doc(hidden)]
    pub async fn raw(&self, endpoint: &str, body: Value) -> Result<Value> {
        self.inner.api.call(endpoint, body).await
    }

    /// [`YtMusic::raw`] as the Android app, which gets time-synced lyrics.
    #[doc(hidden)]
    pub async fn raw_android(&self, endpoint: &str, body: Value) -> Result<Value> {
        self.inner.api.call_as(innertube::ClientKind::Android, endpoint, body).await
    }

    pub(crate) fn api(&self) -> &InnerTube {
        &self.inner.api
    }
}
