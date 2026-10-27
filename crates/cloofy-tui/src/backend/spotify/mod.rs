//! Spotify: the Web API for the library and catalog, librespot for playback and for what only
//! Spotify's own apps get, like Home, artist pages, DJ and Spotify-made playlists.

mod player;
mod queue;
mod remote;

use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Result, bail};
use cloofy_core::{
    artist::ArtistPage,
    catalog::{AlbumInfo, ArtistInfo, Page, PlaylistInfo, SearchResults, ShowInfo, Track, User},
    config::Config,
    connect::{Device, Playback},
    home::Home,
    lyrics::{Lyrics, LyricsQuery},
    paths::Paths,
};
use cloofy_spotify::{artist, catalog, connect, home, lyrics, pathfinder::Pathfinder, session, webapi::WebApi};
use futures::{
    StreamExt, future,
    stream::{self, BoxStream},
};
use librespot::core::{SpotifyUri, session::Session};
use tokio::sync::{
    OnceCell,
    mpsc::{self, UnboundedSender},
    watch,
};

use self::player::SessionState;
pub use self::{player::Controller, queue::Listed};
use super::{Found, Launch, Remote};
use crate::{
    app::AppEvent,
    queue::{Provider, Upcoming},
    views::TrackSource,
};

/// Track metadata requests running at the same time for one playlist.
const METADATA_BATCHES: usize = 3;
/// Library checks running at the same time for one list.
const SAVED_CONCURRENCY: usize = 2;
/// How long lyrics wait for the session, so a track restored at startup still gets Spotify's
/// synced lyrics instead of going straight to LRCLIB.
const LYRICS_SESSION_WAIT: Duration = Duration::from_secs(5);

/// Signed in to both the Web API and librespot.
pub struct Account {
    api: Arc<WebApi>,
}

impl Account {
    pub async fn open(paths: &Paths, config: &Config) -> Result<Self> {
        let api = Arc::new(WebApi::new(config.client_id(paths)?, paths.webapi_token())?);
        if !api.is_logged_in().await || !session::is_logged_in(paths)? {
            bail!("Not logged in yet – run `cloofy login` first.");
        }
        Ok(Self { api })
    }

    /// Connects the player in the background; the catalog's requests that need its session
    /// wait for it.
    pub fn start(self, launch: Launch) -> Catalog {
        let state = Arc::new(watch::Sender::new(SessionState::Connecting));
        let volume = launch.volume.unwrap_or(cloofy_player::DEFAULT_VOLUME);
        let hashes = launch.paths.pathfinder_hashes();
        player::spawn(launch.paths, launch.crossfade, volume, launch.retry_now, launch.tx.clone(), state.clone());
        Catalog::new(self.api, state, hashes, launch.tx)
    }
}

#[derive(Clone)]
pub struct Catalog {
    api: Arc<WebApi>,
    session: Arc<watch::Sender<SessionState>>,
    pathfinder: Arc<OnceCell<Pathfinder>>,
    pathfinder_hashes: PathBuf,
    /// The user's Spotify ID, to tell their own playlists from followed ones.
    user_id: Arc<OnceCell<String>>,
    /// For LRCLIB, where lyrics come from when Spotify has none.
    http: reqwest::Client,
    remote: UnboundedSender<connect::Command>,
}

impl Catalog {
    fn new(
        api: Arc<WebApi>,
        session: Arc<watch::Sender<SessionState>>,
        pathfinder_hashes: PathBuf,
        tx: UnboundedSender<AppEvent>,
    ) -> Self {
        let http = cloofy_core::http::client(concat!("cloofy/", env!("CARGO_PKG_VERSION"))).build().unwrap_or_default();
        let (remote, commands) = mpsc::unbounded_channel();
        tokio::spawn(remote::send_remote(api.clone(), tx, commands));
        Self { api, session, pathfinder: Arc::default(), pathfinder_hashes, user_id: Arc::default(), http, remote }
    }

    /// The librespot session, once the player has connected.
    async fn session(&self) -> Result<Session> {
        let mut session = self.session.subscribe();
        let state = session.wait_for(|state| !matches!(state, SessionState::Connecting)).await?;
        match &*state {
            SessionState::Ready(session) => Ok(session.clone()),
            SessionState::Failed(err) => bail!("No connection to Spotify: {err}"),
            SessionState::Connecting => unreachable!("checked by wait_for"),
        }
    }

    /// Spotify's internal API client, created once the session is up.
    async fn pathfinder(&self) -> Result<&Pathfinder> {
        let session = self.session().await?;
        let hashes = self.pathfinder_hashes.clone();
        Ok(self.pathfinder.get_or_init(|| async move { Pathfinder::new(session, hashes) }).await)
    }

    /// The user's ID; asks Spotify itself when `/me` has not answered yet, as happens for a
    /// playlist restored at startup.
    async fn user_id(&self) -> Option<String> {
        let id = self.user_id.get_or_try_init(|| async { catalog::me(&self.api).await.map(|user| user.id) }).await;
        id.ok().cloned()
    }

    pub async fn me(&self) -> Result<User> {
        let user = catalog::me(&self.api).await?;
        let _ = self.user_id.set(user.id.clone());
        Ok(user)
    }

    pub async fn home(&self) -> Result<Home> {
        home::home(self.pathfinder().await?).await
    }

    pub async fn artist(&self, uri: &str) -> Result<ArtistPage> {
        artist::artist(self.pathfinder().await?, uri).await
    }

    pub async fn search(&self, query: &str) -> Result<SearchResults> {
        catalog::search(&self.api, query).await
    }

    /// Pages after the first are fetched a few at a time and arrive in order.
    pub fn tracks(&self, source: TrackSource) -> BoxStream<'static, Result<Page<Track>>> {
        let this = self.clone();
        stream::once(async move {
            if let TrackSource::Playlist(playlist) = &source
                && !playlist.collaborative
                && this.user_id().await.as_deref() != Some(playlist.owner_id.as_str())
            {
                return this.internal_playlist(playlist.id.clone()).await;
            }
            let api = this.api.clone();
            let fetch = move |offset| {
                let (api, source) = (api.clone(), source.clone());
                async move {
                    match &source {
                        TrackSource::Liked => catalog::liked_tracks(&api, offset).await,
                        TrackSource::Playlist(p) => catalog::playlist_tracks(&api, &p.id, offset).await,
                        TrackSource::Album(a) => catalog::album_tracks(&api, a, offset).await,
                    }
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
            stream::iter([first]).chain(rest).boxed()
        })
        .flatten()
        .boxed()
    }

    /// Followed and Spotify-made playlists only resolve through librespot. Their tracks arrive
    /// in batches, so the first rows show while the rest load.
    async fn internal_playlist(self, id: String) -> BoxStream<'static, Result<Page<Track>>> {
        let listed = async {
            let session = self.session().await?;
            let uris = catalog::playlist_track_uris(&session, &id).await?;
            anyhow::Ok((session, uris))
        };
        let (session, uris) = match listed.await {
            Ok(listed) => listed,
            Err(err) => return stream::iter([Err(err)]).boxed(),
        };
        if uris.is_empty() {
            return stream::iter([Ok(Page { items: Vec::new(), total: 0, next_offset: None })]).boxed();
        }
        let count = uris.len();
        let batches: Vec<_> = uris.chunks(catalog::METADATA_BATCH).map(<[_]>::to_vec).collect();
        stream::iter(batches)
            .map(move |batch| {
                let session = session.clone();
                async move { (batch.len(), catalog::tracks_metadata(&session, &batch).await) }
            })
            .buffered(METADATA_BATCHES)
            .scan((0, 0), move |(requested, delivered), (len, items)| {
                *requested += len;
                *delivered += items.len();
                let last = *requested == count;
                // Tracks without details are left out, so the final count is what actually arrived.
                let total = if last { *delivered } else { count } as u32;
                let next_offset = (!last).then_some(*requested as u32);
                future::ready(Some(Ok(Page { items, total, next_offset })))
            })
            .boxed()
    }

    pub async fn playlists(&self) -> Result<Vec<PlaylistInfo>> {
        catalog::playlists(&self.api).await
    }

    pub async fn liked_count(&self) -> Result<u32> {
        catalog::liked_count(&self.api).await
    }

    pub async fn saved_albums(&self) -> Result<Vec<AlbumInfo>> {
        catalog::saved_albums(&self.api).await
    }

    pub async fn followed_artists(&self) -> Result<Vec<ArtistInfo>> {
        catalog::followed_artists(&self.api).await
    }

    pub async fn saved_shows(&self) -> Result<Vec<ShowInfo>> {
        catalog::saved_shows(&self.api).await
    }

    /// Spotify's own lyrics, else LRCLIB's.
    pub async fn lyrics(&self, query: LyricsQuery<'_>) -> Result<Option<Lyrics>> {
        let session = tokio::time::timeout(LYRICS_SESSION_WAIT, self.session()).await.ok().and_then(Result::ok);
        lyrics::fetch(session.as_ref(), &self.http, query).await
    }

    /// Each batch reports as soon as it is known, so the first rows of a long list get their
    /// hearts without waiting for the rest.
    pub fn check_saved(&self, uris: Vec<String>) -> BoxStream<'static, Result<Vec<(String, bool)>>> {
        let api = self.api.clone();
        let batches: Vec<Vec<String>> = uris.chunks(catalog::LIBRARY_BATCH).map(<[_]>::to_vec).collect();
        stream::iter(batches)
            .map(move |batch| {
                let api = api.clone();
                async move {
                    let flags = catalog::contains(&api, &batch).await?;
                    Ok(batch.into_iter().zip(flags).collect())
                }
            })
            .buffered(SAVED_CONCURRENCY)
            .boxed()
    }

    pub async fn set_saved(&self, uri: &str, saved: bool) -> Result<()> {
        let uris = [uri.to_owned()];
        if saved { catalog::save(&self.api, &uris).await } else { catalog::unsave(&self.api, &uris).await }
    }

    pub async fn add_to_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<()> {
        catalog::add_to_playlist(&self.api, playlist_id, uris).await
    }

    /// Through the Web API, so it reaches whichever device plays.
    pub async fn add_to_queue(&self, uri: &str) -> Result<()> {
        catalog::queue(&self.api, uri).await
    }

    /// From librespot's metadata, a batch at a time. Without a session, nothing reaches
    /// Spotify; URIs that aren't Spotify's are reported at once, without details.
    pub fn queue_tracks(&self, uris: Vec<String>) -> BoxStream<'static, Found> {
        let this = self.clone();
        stream::once(async move {
            let Ok(session) = this.session().await else {
                return stream::iter([Found { asked: uris, tracks: Vec::new(), reached: false }]).boxed();
            };
            let (parsed, unknown): (Vec<_>, Vec<_>) =
                uris.into_iter().map(|uri| (SpotifyUri::from_uri(&uri).ok(), uri)).partition(|(id, _)| id.is_some());
            let unknown = (!unknown.is_empty()).then(|| Found {
                asked: unknown.into_iter().map(|(_, uri)| uri).collect(),
                tracks: Vec::new(),
                reached: true,
            });
            let batches: Vec<Vec<_>> = parsed.chunks(catalog::METADATA_BATCH).map(<[_]>::to_vec).collect();
            let found = stream::iter(batches).then(move |batch| {
                let session = session.clone();
                async move {
                    let ids: Vec<SpotifyUri> = batch.iter().filter_map(|(id, _)| id.clone()).collect();
                    let tracks = catalog::tracks_metadata(&session, &ids).await;
                    Found { asked: batch.into_iter().map(|(_, uri)| uri).collect(), tracks, reached: true }
                }
            });
            stream::iter(unknown).chain(found).boxed()
        })
        .flatten()
        .boxed()
    }

    /// The Web API's list of what plays next.
    pub async fn upcoming(&self) -> Result<Vec<Track>> {
        catalog::upcoming(&self.api).await
    }

    pub async fn dj_suggestions(&self, track_uri: &str) -> Result<Vec<String>> {
        Ok(self.session().await?.spclient().get_dj_suggestions(track_uri).await?)
    }

    pub async fn playback(&self) -> Result<Option<Playback>> {
        connect::playback(&self.api).await
    }

    pub async fn devices(&self) -> Result<Vec<Device>> {
        connect::devices(&self.api).await
    }

    pub async fn transfer(&self, device_id: &str) -> Result<()> {
        connect::transfer(&self.api, device_id).await
    }

    pub fn remote(&self, command: Remote) {
        let _ = self.remote.send(remote::command(command));
    }

    /// Through Spotify Connect, as cloofy's own device.
    pub async fn skip_to(&self, device: &str, entry: &Upcoming) -> Result<()> {
        let session = self.session().await?;
        connect::skip_to(&session, device, &entry.uri, &entry.uid, provider_name(entry.provider)).await
    }

    /// Through Spotify Connect, as cloofy's own device.
    pub async fn set_queue(&self, device: &str, listed: &Listed) -> Result<()> {
        let session = self.session().await?;
        connect::set_queue(&session, device, &listed.next, &listed.previous, &listed.revision).await
    }
}

/// Spotify's name for where an entry comes from, as commands about the entry name it.
fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Queue => "queue",
        Provider::Context | Provider::Unknown => "context",
    }
}
