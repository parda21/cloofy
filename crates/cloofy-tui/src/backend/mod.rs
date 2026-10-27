//! The music services cloofy plays from, behind one interface. This is the only part of the
//! interface that names a service's crate: everything else asks here, and a request goes to the
//! service its URI belongs to, or to the one whose Home, Library and Search are shown.
//!
//! Each service built in is a variant of the enums below, so a build without a service holds
//! none of its code. A service plugs in as a module next to `spotify` behind its Cargo feature:
//! listed in [`COMPILED`], a variant in [`Account`], [`Catalog`] and [`Player`], and an arm in
//! [`dispatch!`] and in the `match`es that name each service. Its catalog and player types have
//! the methods [`dispatch!`] calls; what only some services can do (devices, DJ, crossfade)
//! falls back to an error or to nothing for the others. Without any service, the enums are
//! empty and their methods can never be called; only the demo runs.

// What only one service does leaves its arguments unused in a build without that service, and
// with no service built in, nothing below has a variant to hand its arguments to.
#![cfg_attr(not(all(feature = "spotify", feature = "ytmusic")), allow(unused_variables))]

#[cfg(feature = "spotify")]
pub mod spotify;
#[cfg(feature = "ytmusic")]
pub mod ytmusic;

use std::{sync::Arc, time::Duration};

use anyhow::{Result, bail};
pub use cloofy_core::service::Service;
use cloofy_core::{
    artist::ArtistPage,
    catalog::{AlbumInfo, ArtistInfo, Page, PlaylistInfo, SearchResults, ShowInfo, Track, User},
    config::Config,
    connect::{Device, Playback},
    home::Home,
    lyrics::{Lyrics, LyricsQuery},
    paths::Paths,
};
use futures::{future::BoxFuture, stream::BoxStream};
use tokio::sync::{Notify, mpsc::UnboundedSender};

use crate::{app::AppEvent, playback::Repeat, queue::Upcoming, views::TrackSource};

/// The services this build has.
pub const COMPILED: &[Service] = &[
    #[cfg(feature = "spotify")]
    Service::Spotify,
    #[cfg(feature = "ytmusic")]
    Service::YouTubeMusic,
];

/// The release this build belongs to, as its archive is named: `spotify`, `ytmusic`, or `full`
/// with both. None without a service: such a build runs the demo only and is never released,
/// so there is nothing to update it to.
pub const VARIANT: Option<&str> = match (cfg!(feature = "spotify"), cfg!(feature = "ytmusic")) {
    (true, true) => Some("full"),
    (true, false) => Some("spotify"),
    (false, true) => Some("ytmusic"),
    (false, false) => None,
};

/// Runs `$body` with `$inner` bound to the service's own value inside `$value`, an `&` of one of
/// the enums below, whichever service it is. With no service built in, the `match` has no arms
/// and never returns.
macro_rules! dispatch {
    ($value:expr, $enum:ident, $inner:ident => $body:expr) => {
        match *$value {
            #[cfg(feature = "spotify")]
            $enum::Spotify(ref $inner) => $body,
            #[cfg(feature = "ytmusic")]
            $enum::YouTubeMusic(ref $inner) => $body,
        }
    };
}

// ── URIs ─────────────────────────────────────────────────────────────────────────────────────
//
// What a URI looks like is known for every service, built in or not: the demo shows Spotify's
// sample in any build.

/// Whether a player can play `uri` by itself: a song or an episode, not a marker such as the
/// ones Spotify keeps between the pages of a context.
pub fn is_playable(uri: &str) -> bool {
    match Service::of(uri) {
        Some(Service::Spotify) => uri.starts_with("spotify:track:") || uri.starts_with("spotify:episode:"),
        Some(Service::YouTubeMusic) => uri.starts_with("ytmusic:track:"),
        None => false,
    }
}

/// Whether `uri` is a song, which liking adds to the service's liked songs.
pub fn is_track(uri: &str) -> bool {
    Service::of(uri).is_some_and(|service| uri[service.key().len()..].starts_with(":track:"))
}

/// Whether `uri` is a podcast episode.
pub fn is_episode(uri: &str) -> bool {
    uri.starts_with("spotify:episode:")
}

/// Liked Songs as a context to play from. Spotify's names the user; `None` without one.
pub fn liked_context(service: Service, user: Option<&str>) -> Option<String> {
    match service {
        Service::Spotify => Some(format!("spotify:user:{}:collection", user?)),
        // The "Liked Music" playlist, the same for everyone signed in.
        Service::YouTubeMusic => Some("ytmusic:playlist:LM".into()),
    }
}

/// Whether `uri` is one of the contexts [`liked_context`] gives.
pub fn is_liked_context(uri: &str) -> bool {
    match Service::of(uri) {
        Some(Service::Spotify) => uri.ends_with(":collection"),
        Some(Service::YouTubeMusic) => uri == "ytmusic:playlist:LM",
        None => false,
    }
}

/// Spotify's DJ; loading it without a track asks for a fresh set.
const SPOTIFY_DJ: &str = "spotify:playlist:37i9dQZF1EYkqdzj48dyYq";

/// The service's DJ as a context to play, where it has one.
pub fn dj_context(service: Service) -> Option<&'static str> {
    (service == Service::Spotify).then_some(SPOTIFY_DJ)
}

/// Whether `uri` is a DJ, which cloofy treats as a feature rather than a playlist.
pub fn is_dj(uri: &str) -> bool {
    uri == SPOTIFY_DJ
}

/// The address of `uri` on the service's website, to share; the URI itself where it has none.
pub fn web_link(uri: &str) -> String {
    let mut parts = uri.split(':');
    let (Some(service), Some(kind), Some(id)) = (parts.next().and_then(Service::from_key), parts.next(), parts.next())
    else {
        return uri.to_owned();
    };
    match (service, kind) {
        (Service::Spotify, _) => format!("https://open.spotify.com/{kind}/{id}"),
        (Service::YouTubeMusic, "track") => format!("https://music.youtube.com/watch?v={id}"),
        (Service::YouTubeMusic, "playlist") => format!("https://music.youtube.com/playlist?list={id}"),
        (Service::YouTubeMusic, "album") => format!("https://music.youtube.com/browse/{id}"),
        (Service::YouTubeMusic, "artist") => format!("https://music.youtube.com/channel/{id}"),
        (Service::YouTubeMusic, _) => uri.to_owned(),
    }
}

// ── What a service offers ────────────────────────────────────────────────────────────────────
//
// Playing, browsing and the library work alike everywhere; the interface hides the rest where
// the service it plays from lacks it.

/// Spotify's DJ, which talks between songs and takes requests.
pub fn supports_dj(service: Service) -> bool {
    service == Service::Spotify
}

/// Playing on the account's other devices, and moving playback between them.
pub fn supports_devices(service: Service) -> bool {
    service == Service::Spotify
}

/// Songs fading into each other.
pub fn supports_crossfade(service: Service) -> bool {
    service == Service::Spotify
}

// ── Signing in and starting ──────────────────────────────────────────────────────────────────

/// A service the user is signed in to, ready to start.
pub enum Account {
    #[cfg(feature = "spotify")]
    Spotify(spotify::Account),
    #[cfg(feature = "ytmusic")]
    YouTubeMusic(ytmusic::Account),
}

/// What a service's player starts with.
pub struct Launch {
    pub paths: Paths,
    pub crossfade: Duration,
    /// Where the last session left the volume, if anywhere.
    pub volume: Option<u16>,
    /// Cuts a wait between attempts to connect short, as when the computer wakes up.
    pub retry_now: Arc<Notify>,
    pub tx: UnboundedSender<AppEvent>,
}

impl Account {
    /// Checks that the user is signed in to `service`, which this build must have.
    pub async fn open(service: Service, paths: &Paths, config: &Config) -> Result<Self> {
        match service {
            #[cfg(feature = "spotify")]
            Service::Spotify => Ok(Account::Spotify(spotify::Account::open(paths, config).await?)),
            #[cfg(feature = "ytmusic")]
            Service::YouTubeMusic => Ok(Account::YouTubeMusic(ytmusic::Account::open(paths)?)),
            #[allow(unreachable_patterns)]
            other => bail!("{} isn't built into this cloofy", other.name()),
        }
    }

    /// Starts the service's player in the background, which reports with
    /// [`AppEvent::PlayerReady`] once it is up, and returns what the loader asks for the rest.
    pub fn start(self, launch: Launch) -> Catalog {
        match self {
            #[cfg(feature = "spotify")]
            Account::Spotify(account) => Catalog::Spotify(account.start(launch)),
            #[cfg(feature = "ytmusic")]
            Account::YouTubeMusic(account) => Catalog::YouTubeMusic(account.start(launch)),
        }
    }
}

// ── Playing ──────────────────────────────────────────────────────────────────────────────────

/// cloofy's own player for a service. Commands return at once; what they lead to comes back as
/// [`AppEvent::Player`] events.
pub enum Player {
    #[cfg(feature = "spotify")]
    Spotify(spotify::Controller),
    #[cfg(feature = "ytmusic")]
    YouTubeMusic(ytmusic::Controller),
}

impl Player {
    pub fn service(&self) -> Service {
        match *self {
            #[cfg(feature = "spotify")]
            Player::Spotify(_) => Service::Spotify,
            #[cfg(feature = "ytmusic")]
            Player::YouTubeMusic(_) => Service::YouTubeMusic,
        }
    }

    /// Plays `context_uri`, at `track_uri` if given.
    pub fn play(&self, context_uri: String, track_uri: Option<String>) -> Result<()> {
        dispatch!(self, Player, player => player.play(context_uri, track_uri))
    }

    /// Continues a track from the last session at `position_ms`, in its context if known.
    pub fn resume(&self, context: Option<String>, track: String, position_ms: u32) -> Result<()> {
        dispatch!(self, Player, player => player.resume(context, track, position_ms))
    }

    pub fn play_shuffled(&self, context_uri: String) -> Result<()> {
        dispatch!(self, Player, player => player.play_shuffled(context_uri))
    }

    /// Plays or pauses; with nothing loaded, takes over what plays elsewhere, if anything.
    pub fn toggle(&self, has_track: bool) -> Result<()> {
        dispatch!(self, Player, player => player.toggle(has_track))
    }

    pub fn pause(&self) -> Result<()> {
        dispatch!(self, Player, player => player.pause())
    }

    pub fn next(&self) -> Result<()> {
        dispatch!(self, Player, player => player.next())
    }

    pub fn previous(&self) -> Result<()> {
        dispatch!(self, Player, player => player.previous())
    }

    pub fn seek(&self, position_ms: u32) -> Result<()> {
        dispatch!(self, Player, player => player.seek(position_ms))
    }

    /// From 0 to 65535.
    pub fn set_volume(&self, volume: u16) -> Result<()> {
        dispatch!(self, Player, player => player.set_volume(volume))
    }

    pub fn shuffle(&self, on: bool) -> Result<()> {
        dispatch!(self, Player, player => player.shuffle(on))
    }

    pub fn repeat(&self, mode: Repeat) -> Result<()> {
        dispatch!(self, Player, player => player.repeat(mode))
    }

    /// Empties the queue; the rest of the context stays.
    pub fn clear_queue(&self) -> Result<()> {
        dispatch!(self, Player, player => player.clear_queue())
    }

    /// Adds a track after the ones queued before.
    pub fn add_to_queue(&self, uri: &str) -> Result<()> {
        dispatch!(self, Player, player => player.add_to_queue(uri))
    }

    pub fn shutdown(&self) {
        dispatch!(self, Player, player => player.shutdown())
    }

    // What only some services can do; the others say so, or do nothing.

    /// Jumps ahead to `uri` in what plays next, where the player keeps that list itself.
    /// `false` where it doesn't: the app gets there with the queue's commands instead.
    pub fn skip_to(&self, uri: &str) -> Result<bool> {
        match self {
            #[cfg(feature = "ytmusic")]
            Player::YouTubeMusic(player) => player.skip_to(uri),
            #[allow(unreachable_patterns)]
            _ => Ok(false),
        }
    }

    /// Jumps DJ to its next segment of picks.
    pub fn next_segment(&self) -> Result<()> {
        match self {
            #[cfg(feature = "spotify")]
            Player::Spotify(player) => player.next_segment(),
            #[allow(unreachable_patterns)]
            _ => bail!("{} has no DJ", self.service().name()),
        }
    }

    /// Asks DJ for something. The future resolves once DJ took the request; its answer plays
    /// seconds later.
    pub fn ask_dj(&self, prompt: String, suggested: bool) -> Result<BoxFuture<'static, Result<()>>> {
        match self {
            #[cfg(feature = "spotify")]
            Player::Spotify(player) => player.ask_dj(prompt, suggested),
            #[allow(unreachable_patterns)]
            _ => bail!("{} has no DJ", self.service().name()),
        }
    }

    /// Elsewhere songs follow each other as they are.
    pub fn set_crossfade(&self, crossfade: Duration) {
        match self {
            #[cfg(feature = "spotify")]
            Player::Spotify(player) => player.set_crossfade(crossfade),
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }

    /// cloofy's ID among the account's devices, where the service has devices.
    pub fn device_id(&self) -> Option<String> {
        match self {
            #[cfg(feature = "spotify")]
            Player::Spotify(player) => Some(player.device_id()),
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }

    /// The account the player plays for, where the service needs it to name the user's own
    /// contexts, like Spotify's Liked Songs.
    pub fn user(&self) -> Option<String> {
        match self {
            #[cfg(feature = "spotify")]
            Player::Spotify(player) => Some(player.user()),
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }

    /// Drops the connection to the service for a fresh one, as after the computer slept, where
    /// the player has one to lose.
    pub fn reconnect(&self) {
        match self {
            #[cfg(feature = "spotify")]
            Player::Spotify(player) => player.reconnect(),
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }

    pub fn supports_devices(&self) -> bool {
        supports_devices(self.service())
    }
}

/// A command for another of the account's devices, sent through the service. Only where
/// [`supports_devices`].
#[derive(Debug, Clone)]
pub enum Remote {
    Play,
    Pause,
    Next,
    Previous,
    Seek(u32),
    /// In percent.
    Volume(u8),
    Shuffle(bool),
    Repeat(Repeat),
    /// Starts a context on the device that plays, optionally at a given track or shuffled.
    PlayContext {
        context: String,
        track: Option<String>,
        shuffle: bool,
    },
}

/// What plays next on another device, as the service listed it, so it can go back with a
/// change.
#[derive(Debug, Clone, PartialEq)]
pub enum Listed {
    #[cfg(feature = "spotify")]
    Spotify(spotify::Listed),
}

impl Listed {
    /// The list without `entry`, if it is in it.
    pub fn without(&self, entry: &Upcoming) -> Option<Listed> {
        match *self {
            #[cfg(feature = "spotify")]
            Listed::Spotify(ref listed) => listed.without(entry).map(Listed::Spotify),
        }
    }
}

// ── Browsing ─────────────────────────────────────────────────────────────────────────────────

/// Some of the tracks asked for in the queue; see [`crate::loader::DataEvent::QueueTracks`].
pub struct Found {
    pub asked: Vec<String>,
    pub tracks: Vec<Track>,
    /// Whether the service was asked at all.
    pub reached: bool,
}

/// What a service has to show and do besides playing: its catalog, the user's library, and
/// for some, the account's other devices. Cheap to clone.
#[derive(Clone)]
pub enum Catalog {
    #[cfg(feature = "spotify")]
    Spotify(spotify::Catalog),
    #[cfg(feature = "ytmusic")]
    YouTubeMusic(ytmusic::Catalog),
}

impl Catalog {
    pub fn service(&self) -> Service {
        match *self {
            #[cfg(feature = "spotify")]
            Catalog::Spotify(_) => Service::Spotify,
            #[cfg(feature = "ytmusic")]
            Catalog::YouTubeMusic(_) => Service::YouTubeMusic,
        }
    }

    /// Whether the service knows who listens. Without, it browses and plays, but has no
    /// library, likes or user to show.
    pub fn signed_in(&self) -> bool {
        match self {
            #[cfg(feature = "ytmusic")]
            Catalog::YouTubeMusic(catalog) => catalog.signed_in(),
            #[allow(unreachable_patterns)]
            _ => true,
        }
    }

    pub async fn me(&self) -> Result<User> {
        dispatch!(self, Catalog, catalog => catalog.me().await)
    }

    pub async fn home(&self) -> Result<Home> {
        dispatch!(self, Catalog, catalog => catalog.home().await)
    }

    pub async fn artist(&self, uri: &str) -> Result<ArtistPage> {
        dispatch!(self, Catalog, catalog => catalog.artist(uri).await)
    }

    pub async fn search(&self, query: &str) -> Result<SearchResults> {
        dispatch!(self, Catalog, catalog => catalog.search(query).await)
    }

    /// The songs of a list, in pages as they arrive, in order. Every page but the last has
    /// `next_offset`; a failed one ends the list.
    pub fn tracks(&self, source: TrackSource) -> BoxStream<'static, Result<Page<Track>>> {
        dispatch!(self, Catalog, catalog => catalog.tracks(source))
    }

    pub async fn playlists(&self) -> Result<Vec<PlaylistInfo>> {
        dispatch!(self, Catalog, catalog => catalog.playlists().await)
    }

    pub async fn liked_count(&self) -> Result<u32> {
        dispatch!(self, Catalog, catalog => catalog.liked_count().await)
    }

    pub async fn saved_albums(&self) -> Result<Vec<AlbumInfo>> {
        dispatch!(self, Catalog, catalog => catalog.saved_albums().await)
    }

    pub async fn followed_artists(&self) -> Result<Vec<ArtistInfo>> {
        dispatch!(self, Catalog, catalog => catalog.followed_artists().await)
    }

    pub async fn saved_shows(&self) -> Result<Vec<ShowInfo>> {
        dispatch!(self, Catalog, catalog => catalog.saved_shows().await)
    }

    /// `None` when the song has no lyrics anywhere, LRCLIB included.
    pub async fn lyrics(&self, query: LyricsQuery<'_>) -> Result<Option<Lyrics>> {
        dispatch!(self, Catalog, catalog => catalog.lyrics(query).await)
    }

    /// Which `uris` are in the library, a batch at a time.
    pub fn check_saved(&self, uris: Vec<String>) -> BoxStream<'static, Result<Vec<(String, bool)>>> {
        dispatch!(self, Catalog, catalog => catalog.check_saved(uris))
    }

    /// Likes, saves or follows `uri`, or undoes it.
    pub async fn set_saved(&self, uri: &str, saved: bool) -> Result<()> {
        dispatch!(self, Catalog, catalog => catalog.set_saved(uri, saved).await)
    }

    pub async fn add_to_playlist(&self, playlist_id: &str, uris: &[String]) -> Result<()> {
        dispatch!(self, Catalog, catalog => catalog.add_to_playlist(playlist_id, uris).await)
    }

    /// Adds `uri` to the queue of the device that plays.
    pub async fn add_to_queue(&self, uri: &str) -> Result<()> {
        dispatch!(self, Catalog, catalog => catalog.add_to_queue(uri).await)
    }

    /// Details of tracks in the queue, a batch at a time.
    pub fn queue_tracks(&self, uris: Vec<String>) -> BoxStream<'static, Found> {
        dispatch!(self, Catalog, catalog => catalog.queue_tracks(uris))
    }

    /// The service's own list of what plays next, for when the player hasn't said.
    pub async fn upcoming(&self) -> Result<Vec<Track>> {
        dispatch!(self, Catalog, catalog => catalog.upcoming().await)
    }

    // What only some services can do; the others say so, or do nothing.

    /// What the service suggests asking DJ for while `track_uri` plays.
    pub async fn dj_suggestions(&self, track_uri: &str) -> Result<Vec<String>> {
        match self {
            #[cfg(feature = "spotify")]
            Catalog::Spotify(catalog) => catalog.dj_suggestions(track_uri).await,
            #[allow(unreachable_patterns)]
            _ => bail!("{} has no DJ", self.service().name()),
        }
    }

    /// What plays on the account's devices; `None` when nothing does.
    pub async fn playback(&self) -> Result<Option<Playback>> {
        match self {
            #[cfg(feature = "spotify")]
            Catalog::Spotify(catalog) => catalog.playback().await,
            #[allow(unreachable_patterns)]
            _ => Ok(None),
        }
    }

    pub async fn devices(&self) -> Result<Vec<Device>> {
        match self {
            #[cfg(feature = "spotify")]
            Catalog::Spotify(catalog) => catalog.devices().await,
            #[allow(unreachable_patterns)]
            _ => bail!("{} doesn't play on other devices", self.service().name()),
        }
    }

    /// Moves playback to the device and keeps it playing.
    pub async fn transfer(&self, device_id: &str) -> Result<()> {
        match self {
            #[cfg(feature = "spotify")]
            Catalog::Spotify(catalog) => catalog.transfer(device_id).await,
            #[allow(unreachable_patterns)]
            _ => bail!("{} doesn't play on other devices", self.service().name()),
        }
    }

    /// Sends a command to the other device that plays. The answer comes as
    /// [`crate::loader::DataEvent::RemoteDone`].
    pub fn remote(&self, command: Remote) {
        match self {
            #[cfg(feature = "spotify")]
            Catalog::Spotify(catalog) => catalog.remote(command),
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }

    /// Skips ahead to an entry of what plays next on `device`.
    pub async fn skip_to(&self, device: &str, entry: &Upcoming) -> Result<()> {
        match self {
            #[cfg(feature = "spotify")]
            Catalog::Spotify(catalog) => catalog.skip_to(device, entry).await,
            #[allow(unreachable_patterns)]
            _ => bail!("{} doesn't play on other devices", self.service().name()),
        }
    }

    /// Hands `device` its list of what plays next back with a change.
    pub async fn set_queue(&self, device: &str, listed: &Listed) -> Result<()> {
        match (self, listed) {
            #[cfg(feature = "spotify")]
            (Catalog::Spotify(catalog), Listed::Spotify(listed)) => catalog.set_queue(device, listed).await,
            #[allow(unreachable_patterns)]
            _ => bail!("{} can't change what plays next there", self.service().name()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_songs_and_episodes_from_markers() {
        assert!(is_playable("spotify:track:4uLU6hMCjMI75M1A2tKUQC"));
        assert!(is_playable("spotify:episode:1"));
        assert!(!is_playable("spotify:delimiter"));
        assert!(is_playable("ytmusic:track:dQw4w9WgXcQ"));
        assert!(!is_playable("ytmusic:album:MPREb_1"));
        assert!(is_track("spotify:track:1") && is_track("ytmusic:track:1"));
        assert!(!is_track("spotify:episode:1") && !is_track("ytmusic:playlist:LM") && !is_track("track:1"));
    }

    #[test]
    fn builds_web_links() {
        assert_eq!(web_link("spotify:track:abc"), "https://open.spotify.com/track/abc");
        assert_eq!(web_link("spotify:album:abc"), "https://open.spotify.com/album/abc");
        assert_eq!(web_link("ytmusic:track:dQw4w9WgXcQ"), "https://music.youtube.com/watch?v=dQw4w9WgXcQ");
        assert_eq!(web_link("ytmusic:playlist:PL1"), "https://music.youtube.com/playlist?list=PL1");
        assert_eq!(web_link("ytmusic:album:MPREb_1"), "https://music.youtube.com/browse/MPREb_1");
        assert_eq!(web_link("ytmusic:artist:UC1"), "https://music.youtube.com/channel/UC1");
        assert_eq!(web_link("local:file"), "local:file");
    }

    #[test]
    fn liked_songs_are_a_context_of_their_own() {
        let spotify = liked_context(Service::Spotify, Some("sam")).unwrap();
        assert_eq!(spotify, "spotify:user:sam:collection");
        assert!(is_liked_context(&spotify));
        assert_eq!(liked_context(Service::Spotify, None), None);
        assert!(is_liked_context(&liked_context(Service::YouTubeMusic, None).unwrap()));
        assert!(!is_liked_context("spotify:playlist:37i9dQZF1EYkqdzj48dyYq"));
    }
}
