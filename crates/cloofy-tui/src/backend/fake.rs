//! A music service for tests. It stands in for Spotify or YouTube Music with a small made-up
//! catalog, and its player does what it is told at once, reporting it in the order YouTube
//! Music's player does. Tests keep a [`Fake`] to read the commands it got and to send what a
//! real player would, like a lost connection or a refused song.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use anyhow::{Result, bail};
use cloofy_core::{
    artist::ArtistPage,
    catalog::{AlbumInfo, ArtistInfo, Page, PlaylistInfo, SearchResults, ShowInfo, Track, User},
    home::{Chip, Home, HomeItem, ItemKind, Shelf},
    lyrics::{Lyrics, LyricsQuery},
    player::{CurrentTrack, PlayerEvent, QueueTrack},
    service::Service,
};
use futures::stream::{self, BoxStream, StreamExt};
use tokio::sync::mpsc::UnboundedSender;

use super::{Found, Player, is_track};
use crate::{app::AppEvent, clock::Instant, playback::Repeat, views::TrackSource};

#[derive(Clone)]
pub struct Fake(Arc<Inner>);

struct Inner {
    service: Service,
    tx: UnboundedSender<AppEvent>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    commands: Vec<String>,
    liked: HashSet<String>,
    playing: Option<Playing>,
    /// Home fails to load from now on.
    home_down: bool,
}

struct Playing {
    context: String,
    tracks: Vec<Track>,
    index: usize,
    queued: Vec<Track>,
    /// Where the track was when it last started, paused or seeked, and since when it plays on.
    position_ms: u32,
    since: Option<Instant>,
}

impl Playing {
    fn track(&self) -> &Track {
        &self.tracks[self.index]
    }

    fn position_ms(&self) -> u32 {
        let elapsed = self.since.map_or(0, |since| since.elapsed().as_millis() as u32);
        (self.position_ms + elapsed).min(self.track().duration_ms)
    }
}

impl Fake {
    pub fn new(service: Service, tx: UnboundedSender<AppEvent>) -> Self {
        let liked = album_tracks(service).into_iter().take(3).map(|t| t.uri).collect();
        let state = State { liked, ..State::default() };
        Self(Arc::new(Inner { service, tx, state: Mutex::new(state) }))
    }

    pub fn service(&self) -> Service {
        self.0.service
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap()
    }

    // ── For tests ────────────────────────────────────────────────────────────────────────

    /// The player is up, as a real one says once it has connected.
    pub fn connect(&self) {
        let _ = self.0.tx.send(AppEvent::PlayerReady(Box::new(Player::Fake(self.clone()))));
    }

    /// The player's connection dropped.
    pub fn lose_connection(&self) {
        let _ = self.0.tx.send(AppEvent::PlayerLost(self.0.service));
    }

    /// Home fails to load from now on, as when the service doesn't answer.
    pub fn take_home_down(&self) {
        self.state().home_down = true;
    }

    /// Sends what a real player would report.
    pub fn send(&self, event: PlayerEvent) {
        let _ = self.0.tx.send(AppEvent::Player(self.0.service, event));
    }

    /// The commands the player and the catalog got, oldest first.
    pub fn commands(&self) -> Vec<String> {
        self.state().commands.clone()
    }

    /// Goes on to the next track where the one playing has ended, as time passes.
    pub fn catch_up(&self) {
        let ended = self
            .state()
            .playing
            .as_ref()
            .is_some_and(|p| p.since.is_some() && p.position_ms() >= p.track().duration_ms);
        if ended {
            self.skip(1);
        }
    }

    // ── Player ───────────────────────────────────────────────────────────────────────────

    fn record(&self, command: String) {
        self.state().commands.push(command);
    }

    fn start(&self, context: String, track: Option<&str>, position_ms: u32) -> Result<()> {
        let mut tracks = tracks_of(self.0.service, &context);
        let index = match track {
            Some(uri) => match tracks.iter().position(|t| t.uri == uri) {
                Some(index) => index,
                None => {
                    let Some(single) = all_tracks(self.0.service).into_iter().find(|t| t.uri == uri) else {
                        bail!("The fake has no {uri}");
                    };
                    tracks = vec![single];
                    0
                }
            },
            None if tracks.is_empty() => bail!("The fake has nothing in {context}"),
            None => 0,
        };
        let playing = Playing { context, tracks, index, queued: Vec::new(), position_ms, since: Some(Instant::now()) };
        self.state().playing = Some(playing);
        self.send(PlayerEvent::Activated);
        self.announce_track();
        Ok(())
    }

    /// Reports the track that plays now, from its start or where it was left.
    fn announce_track(&self) {
        let (track, position_ms, queue) = {
            let state = self.state();
            let Some(playing) = &state.playing else { return };
            (playing.track().clone(), playing.position_ms(), queue_of(playing))
        };
        self.send(PlayerEvent::Loading { uri: track.uri.clone(), position_ms });
        self.send(PlayerEvent::TrackChanged { track: current(&track) });
        self.send(queue);
        self.send(PlayerEvent::Playing { position_ms });
    }

    fn skip(&self, delta: isize) {
        let stopped = {
            let mut state = self.state();
            let Some(playing) = state.playing.as_mut() else { return };
            if delta > 0 && !playing.queued.is_empty() {
                let next = playing.queued.remove(0);
                playing.tracks.insert(playing.index + 1, next);
            }
            let index = playing.index as isize + delta;
            if index < 0 || index as usize >= playing.tracks.len() {
                state.playing = None;
                true
            } else {
                playing.index = index as usize;
                (playing.position_ms, playing.since) = (0, Some(Instant::now()));
                false
            }
        };
        if stopped {
            self.send(PlayerEvent::Stopped);
        } else {
            self.announce_track();
        }
    }

    fn announce_queue(&self) {
        let queue = self.state().playing.as_ref().map(queue_of);
        if let Some(queue) = queue {
            self.send(queue);
        }
    }

    pub fn play(&self, context_uri: String, track_uri: Option<String>) -> Result<()> {
        let at = track_uri.as_ref().map(|t| format!(" at {t}")).unwrap_or_default();
        self.record(format!("play {context_uri}{at}"));
        self.start(context_uri, track_uri.as_deref(), 0)
    }

    pub fn resume(&self, context: Option<String>, track: String, position_ms: u32) -> Result<()> {
        self.record(format!("resume {track} at {position_ms} ms"));
        self.start(context.unwrap_or_else(|| track.clone()), Some(&track), position_ms)
    }

    pub fn play_shuffled(&self, context_uri: String) -> Result<()> {
        self.record(format!("shuffle {context_uri}"));
        self.send(PlayerEvent::ShuffleChanged { shuffle: true });
        self.start(context_uri, None, 0)
    }

    pub fn toggle(&self, _has_track: bool) -> Result<()> {
        self.record("toggle".into());
        let event = {
            let mut state = self.state();
            let Some(playing) = state.playing.as_mut() else { return Ok(()) };
            let position_ms = playing.position_ms();
            playing.position_ms = position_ms;
            match playing.since.take() {
                Some(_) => PlayerEvent::Paused { position_ms },
                None => {
                    playing.since = Some(Instant::now());
                    PlayerEvent::Playing { position_ms }
                }
            }
        };
        self.send(event);
        Ok(())
    }

    pub fn pause(&self) -> Result<()> {
        self.record("pause".into());
        let position_ms = {
            let mut state = self.state();
            let Some(playing) = state.playing.as_mut() else { return Ok(()) };
            playing.position_ms = playing.position_ms();
            playing.since = None;
            playing.position_ms
        };
        self.send(PlayerEvent::Paused { position_ms });
        Ok(())
    }

    pub fn next(&self) -> Result<()> {
        self.record("next".into());
        self.skip(1);
        Ok(())
    }

    pub fn previous(&self) -> Result<()> {
        self.record("previous".into());
        self.skip(-1);
        Ok(())
    }

    pub fn seek(&self, position_ms: u32) -> Result<()> {
        self.record(format!("seek {position_ms} ms"));
        if let Some(playing) = self.state().playing.as_mut() {
            playing.position_ms = position_ms;
            playing.since = playing.since.map(|_| Instant::now());
        }
        self.send(PlayerEvent::Seeked { position_ms });
        Ok(())
    }

    pub fn set_volume(&self, volume: u16) -> Result<()> {
        self.record(format!("volume {volume}"));
        self.send(PlayerEvent::VolumeChanged { volume });
        Ok(())
    }

    pub fn shuffle(&self, on: bool) -> Result<()> {
        self.record(format!("shuffle {on}"));
        self.send(PlayerEvent::ShuffleChanged { shuffle: on });
        Ok(())
    }

    pub fn repeat(&self, mode: Repeat) -> Result<()> {
        self.record(format!("repeat {mode:?}"));
        self.send(PlayerEvent::RepeatChanged { context: mode != Repeat::Off, track: mode == Repeat::Track });
        Ok(())
    }

    pub fn clear_queue(&self) -> Result<()> {
        self.record("clear queue".into());
        if let Some(playing) = self.state().playing.as_mut() {
            playing.queued.clear();
        }
        self.announce_queue();
        Ok(())
    }

    pub fn add_to_queue(&self, uri: &str) -> Result<()> {
        self.record(format!("queue {uri}"));
        let Some(track) = all_tracks(self.0.service).into_iter().find(|t| t.uri == uri) else {
            bail!("The fake has no {uri}");
        };
        if let Some(playing) = self.state().playing.as_mut() {
            playing.queued.push(track);
        }
        self.announce_queue();
        Ok(())
    }

    pub fn shutdown(&self) {
        self.record("shutdown".into());
    }

    // ── Catalog ──────────────────────────────────────────────────────────────────────────

    pub async fn me(&self) -> Result<User> {
        Ok(User { id: "tester".into(), name: "Tester".into() })
    }

    /// A shelf of cards, and on YouTube Music chips and Quick picks too. A chip's Home has a
    /// shelf named after it.
    pub async fn home(&self, chip: Option<&str>) -> Result<Home> {
        if self.state().home_down {
            bail!("The fake's Home is down");
        }
        let service = self.0.service;
        let (album, playlist) = (album(service), playlist(service));
        let item = |uri: &str, kind, title: &str| HomeItem {
            uri: uri.into(),
            kind,
            title: title.into(),
            subtitle: String::new(),
            image_url: None,
            owner_id: String::new(),
            owner_name: String::new(),
            context_uri: uri.into(),
        };
        let items = vec![
            item(&playlist.uri, ItemKind::Playlist, &playlist.name),
            item(&album.uri, ItemKind::Album, &album.name),
        ];
        let shelf =
            Shelf { uri: format!("{}:shelf:1", service.key()), title: "For you".into(), items, ..Shelf::default() };
        let greeting = format!("Hello from {}", service.name());
        if service != Service::YouTubeMusic {
            return Ok(Home { greeting, shelves: vec![shelf], ..Home::default() });
        }
        let chips: Vec<_> = ["Relax", "Workout"]
            .into_iter()
            .map(|title| Chip {
                title: title.into(),
                id: format!("chip-{title}"),
                selected: chip == Some(&*format!("chip-{title}")),
            })
            .collect();
        let songs = all_tracks(service);
        let picks = Shelf {
            uri: "ytmusic:shelf:Quick picks".into(),
            title: "Quick picks".into(),
            songs: true,
            play_all: Some(format!(
                "ytmusic:songs:{}",
                songs.iter().map(|t| &t.uri["ytmusic:track:".len()..]).collect::<Vec<_>>().join(",")
            )),
            items: songs
                .iter()
                .map(|t| HomeItem {
                    uri: format!("ytmusic:playlist:RDAMVM{}", &t.uri["ytmusic:track:".len()..]),
                    kind: ItemKind::Playlist,
                    title: t.name.clone(),
                    subtitle: t.artists.join(", "),
                    image_url: None,
                    owner_id: String::new(),
                    owner_name: String::new(),
                    context_uri: t.uri.clone(),
                })
                .collect(),
            ..Shelf::default()
        };
        let mut shelves = vec![picks, Shelf { strapline: "MADE FOR YOU".into(), ..shelf }];
        if let Some(chip) = chips.iter().find(|c| c.selected) {
            shelves.insert(
                0,
                Shelf {
                    uri: format!("ytmusic:shelf:{}", chip.title),
                    title: format!("{} mixes", chip.title),
                    ..shelves[1].clone()
                },
            );
        }
        Ok(Home { greeting, chips, shelves })
    }

    pub async fn artist(&self, _uri: &str) -> Result<ArtistPage> {
        bail!("The fake has no artists")
    }

    pub async fn search(&self, query: &str) -> Result<SearchResults> {
        let query = query.to_lowercase();
        let tracks =
            all_tracks(self.0.service).into_iter().filter(|t| t.name.to_lowercase().contains(&query)).collect();
        Ok(SearchResults { tracks, albums: Vec::new(), artists: Vec::new(), playlists: Vec::new() })
    }

    pub fn tracks(&self, source: TrackSource) -> BoxStream<'static, Result<Page<Track>>> {
        let items = match source {
            TrackSource::Liked => {
                let liked = self.state().liked.clone();
                all_tracks(self.0.service).into_iter().filter(|t| liked.contains(&t.uri)).collect()
            }
            TrackSource::Playlist(playlist) => tracks_of(self.0.service, &playlist.uri),
            TrackSource::Album(album) => tracks_of(self.0.service, &album.uri),
        };
        let total = items.len() as u32;
        stream::iter([Ok(Page { items, total, next_offset: None })]).boxed()
    }

    pub async fn playlists(&self) -> Result<Vec<PlaylistInfo>> {
        Ok(vec![playlist(self.0.service)])
    }

    pub async fn liked_count(&self) -> Result<u32> {
        Ok(self.state().liked.len() as u32)
    }

    pub async fn saved_albums(&self) -> Result<Vec<AlbumInfo>> {
        Ok(vec![album(self.0.service)])
    }

    pub async fn followed_artists(&self) -> Result<Vec<ArtistInfo>> {
        Ok(Vec::new())
    }

    pub async fn saved_shows(&self) -> Result<Vec<ShowInfo>> {
        Ok(Vec::new())
    }

    pub async fn lyrics(&self, _query: LyricsQuery<'_>) -> Result<Option<Lyrics>> {
        Ok(None)
    }

    pub fn check_saved(&self, uris: Vec<String>) -> BoxStream<'static, Result<Vec<(String, bool)>>> {
        let liked = self.state().liked.clone();
        let saved = uris.into_iter().map(|uri| (uri.clone(), liked.contains(&uri))).collect();
        stream::iter([Ok(saved)]).boxed()
    }

    pub async fn set_saved(&self, uri: &str, saved: bool) -> Result<()> {
        let mut state = self.state();
        state.commands.push(format!("{} {uri}", if saved { "save" } else { "unsave" }));
        if saved {
            state.liked.insert(uri.into());
        } else {
            state.liked.remove(uri);
        }
        Ok(())
    }

    pub async fn add_to_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<()> {
        self.record(format!("add {} to {playlist_id}", uris.join(", ")));
        Ok(())
    }

    pub fn queue_tracks(&self, uris: Vec<String>) -> BoxStream<'static, Found> {
        let all = all_tracks(self.0.service);
        let tracks = all.into_iter().filter(|t| uris.contains(&t.uri)).collect();
        stream::iter([Found { asked: uris, tracks, reached: true }]).boxed()
    }

    /// Spotify's Web API lists what plays next; YouTube Music's player only says it itself.
    pub async fn upcoming(&self) -> Result<Vec<Track>> {
        if self.0.service == Service::YouTubeMusic {
            bail!("YouTube Music's player says what plays next itself");
        }
        let state = self.state();
        Ok(state.playing.as_ref().map(|p| p.tracks[p.index + 1..].to_vec()).unwrap_or_default())
    }
}

/// The fake as a catalog, whose songs added to the queue go to its player.
#[derive(Clone)]
pub struct Catalog(pub Fake);

impl std::ops::Deref for Catalog {
    type Target = Fake;
    fn deref(&self) -> &Fake {
        &self.0
    }
}

impl Catalog {
    pub async fn add_to_queue(&self, uri: &str) -> Result<()> {
        self.0.add_to_queue(uri)
    }
}

fn queue_of(playing: &Playing) -> PlayerEvent {
    let queued = playing.queued.iter().map(|t| QueueTrack { uri: t.uri.clone(), provider: "queue".into() });
    let rest = playing.tracks[playing.index + 1..]
        .iter()
        .map(|t| QueueTrack { uri: t.uri.clone(), provider: "context".into() });
    PlayerEvent::QueueChanged {
        context_uri: playing.context.clone(),
        current: Some(playing.track().uri.clone()),
        next: queued.chain(rest).collect(),
    }
}

fn current(track: &Track) -> CurrentTrack {
    CurrentTrack {
        uri: track.uri.clone(),
        name: track.name.clone(),
        artists: track.artists.clone(),
        artist_uris: track.artist_uris.clone(),
        album: track.album.clone(),
        album_uri: Some(track.album_uri.clone()),
        duration_ms: track.duration_ms,
        cover_url: None,
        cover_url_large: None,
    }
}

// ── The catalog ──────────────────────────────────────────────────────────────────────────────
//
// Every service has the same album of five songs and playlist of four, named after it, so a
// screen shows which service a song came from.

/// How long every fake song lasts.
pub const SONG: Duration = Duration::from_secs(180);

fn album(service: Service) -> AlbumInfo {
    let name = service.name();
    AlbumInfo {
        id: "a1".into(),
        uri: format!("{}:album:a1", service.key()),
        name: format!("{name} Album"),
        artists: vec![format!("{name} Band")],
        year: "2024".into(),
        total: 5,
        image_url: None,
    }
}

fn playlist(service: Service) -> PlaylistInfo {
    PlaylistInfo {
        id: "p1".into(),
        uri: format!("{}:playlist:p1", service.key()),
        name: format!("{} Mix", service.name()),
        owner_id: "tester".into(),
        owner_name: "Tester".into(),
        collaborative: false,
        total: Some(4),
        image_url: None,
    }
}

fn song(service: Service, id: String, name: String) -> Track {
    let album = album(service);
    Track {
        uri: format!("{}:track:{id}", service.key()),
        name,
        artists: album.artists.clone(),
        artist_uris: Vec::new(),
        album: album.name,
        album_uri: album.uri,
        duration_ms: SONG.as_millis() as u32,
        image_url: None,
    }
}

fn album_tracks(service: Service) -> Vec<Track> {
    (1..=5).map(|i| song(service, format!("a1-{i}"), format!("{} Song {i}", service.name()))).collect()
}

fn playlist_tracks(service: Service) -> Vec<Track> {
    (1..=4).map(|i| song(service, format!("p1-{i}"), format!("{} Mix Song {i}", service.name()))).collect()
}

fn all_tracks(service: Service) -> Vec<Track> {
    album_tracks(service).into_iter().chain(playlist_tracks(service)).collect()
}

fn tracks_of(service: Service, context: &str) -> Vec<Track> {
    if let Some(ids) = context.strip_prefix("ytmusic:songs:") {
        let all = all_tracks(service);
        ids.split(',').filter_map(|id| all.iter().find(|t| t.uri == format!("ytmusic:track:{id}")).cloned()).collect()
    } else if is_track(context) {
        all_tracks(service).into_iter().filter(|t| t.uri == context).collect()
    } else if context == album(service).uri {
        album_tracks(service)
    } else if context == playlist(service).uri {
        playlist_tracks(service)
    } else {
        Vec::new()
    }
}
