//! Builds the sample that `cloofy demo` shows, from public Spotify content:
//!
//!     cargo run --example demo_fixture
//!
//! It reads `crates/cloofy-tui/demo/plan.json` and writes `crates/cloofy-tui/demo/sample.json`.
//! Albums, artists and songs are looked up with the signed-in account, but nothing of the
//! account itself ends up in the sample: the user, the playlists and the library are the plan's.

use std::{collections::HashMap, path::Path};

use anyhow::{Context, Result};
use cloofy_core::{
    artist::ArtistPage,
    catalog::{AlbumInfo, ArtistInfo, PlaylistInfo, Track, User},
    config::Config,
    home::{Home, HomeItem, ItemKind, Shelf},
    lyrics::{Lyrics, LyricsQuery},
    paths::Paths,
};
use cloofy_spotify::{artist, catalog, lyrics, pathfinder::Pathfinder, session, webapi::WebApi};
use cloofy_tui::demo::{Demo, Playing};
use serde::Deserialize;

/// Spotify's own Liked Songs artwork.
const LIKED_IMAGE: &str = "https://misc.scdn.co/liked-songs/liked-songs-300.png";
/// Cover art file names start with their size: 300 px, and 640 px for Now Playing.
const SMALL: &str = "ab67616d00001e02";
const LARGE: &str = "ab67616d0000b273";

#[derive(Deserialize)]
struct Plan {
    user: String,
    albums: Vec<String>,
    artists: Vec<String>,
    playlists: Vec<PlanPlaylist>,
    liked: Vec<String>,
    searches: Vec<String>,
    playing: PlanPlaying,
    home: PlanHome,
}

#[derive(Deserialize)]
struct PlanPlaylist {
    name: String,
    /// In the user's library, or only on Home like Spotify's mixes.
    #[serde(default = "yes")]
    library: bool,
    owner: Option<String>,
    tracks: Vec<String>,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct PlanPlaying {
    track: String,
    context: String,
    position_ms: u32,
    /// Songs in the queue, before the rest of the context.
    queued: Vec<String>,
}

#[derive(Deserialize)]
struct PlanHome {
    greeting: String,
    shortcuts: Vec<String>,
    shelves: Vec<PlanShelf>,
}

#[derive(Deserialize)]
struct PlanShelf {
    title: String,
    items: Vec<String>,
}

struct Builder {
    api: WebApi,
    found: HashMap<String, Track>,
}

impl Builder {
    /// The first song Spotify finds for `query`, like "Daft Punk Around the World".
    async fn track(&mut self, query: &str) -> Result<Track> {
        if let Some(track) = self.found.get(query) {
            return Ok(track.clone());
        }
        let results = catalog::search(&self.api, query).await?;
        let track = results.tracks.into_iter().next().with_context(|| format!("no song for “{query}”"))?;
        println!("  {query} → {} · {}", track.name, track.artists.join(", "));
        self.found.insert(query.to_owned(), track.clone());
        Ok(track)
    }

    async fn tracks(&mut self, queries: &[String]) -> Result<Vec<Track>> {
        let mut tracks = Vec::new();
        for query in queries {
            tracks.push(self.track(query).await?);
        }
        Ok(tracks)
    }

    async fn album(&self, query: &str) -> Result<(AlbumInfo, Vec<Track>)> {
        let results = catalog::search(&self.api, query).await?;
        let album = results.albums.into_iter().next().with_context(|| format!("no album for “{query}”"))?;
        println!("  {query} → {} · {}", album.name, album.artists.join(", "));
        let mut tracks = Vec::new();
        let mut offset = Some(0);
        while let Some(at) = offset {
            let page = catalog::album_tracks(&self.api, &album, at).await?;
            tracks.extend(page.items);
            offset = page.next_offset;
        }
        Ok((album, tracks))
    }

    async fn artist(&self, name: &str) -> Result<ArtistInfo> {
        let results = catalog::search(&self.api, name).await?;
        let mut artists = results.artists.into_iter();
        let first = artists.next().with_context(|| format!("no artist “{name}”"))?;
        let exact = std::iter::once(first.clone()).chain(artists).find(|a| a.name.eq_ignore_ascii_case(name));
        Ok(exact.unwrap_or(first))
    }
}

/// Spotify's cover for a playlist of others' songs: the covers of its first four albums.
fn mosaic(tracks: &[Track]) -> Option<String> {
    let mut ids: Vec<&str> = Vec::new();
    for url in tracks.iter().filter_map(|t| t.image_url.as_deref()) {
        let id = url.rsplit('/').next().unwrap_or_default();
        if id.starts_with(SMALL) && !ids.contains(&id) {
            ids.push(id);
        }
    }
    match ids.len() {
        4.. => Some(format!("https://mosaic.scdn.co/300/{}", ids[..4].concat())),
        _ => tracks.first().and_then(|t| t.image_url.clone()),
    }
}

fn slug(name: &str) -> String {
    name.to_lowercase().chars().filter(char::is_ascii_alphanumeric).collect()
}

/// "Artist, Artist and more" from the first songs of a mix.
fn artists_of(tracks: &[Track]) -> String {
    let mut names: Vec<&str> = Vec::new();
    for name in tracks.iter().filter_map(|t| t.artists.first()) {
        if !names.contains(&name.as_str()) {
            names.push(name);
        }
    }
    match names.as_slice() {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [a, b] => format!("{a} and {b}"),
        [a, b, ..] => format!("{a}, {b} and more"),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../cloofy-tui/demo");
    let plan: Plan = serde_json::from_slice(&std::fs::read(dir.join("plan.json"))?)?;

    let mut paths = Paths::resolve()?;
    let config = Config::load(&paths)?;
    paths.keychain = config.keychain()?;
    let api = WebApi::new(config.client_id(&paths)?, paths.webapi_token())?;
    let session = session::prepare(&paths)?.connect().await?;
    let pathfinder = Pathfinder::new(session.clone(), paths.pathfinder_hashes());
    let mut builder = Builder { api, found: HashMap::new() };
    let user = User { id: slug(&plan.user), name: plan.user.clone() };

    println!("Albums");
    let mut albums = Vec::new();
    let mut tracks: HashMap<String, Vec<Track>> = HashMap::new();
    for query in &plan.albums {
        let (album, songs) = builder.album(query).await?;
        tracks.insert(album.uri.clone(), songs);
        albums.push(album);
    }

    println!("Artists");
    let mut artists = Vec::new();
    let mut artist_pages: HashMap<String, ArtistPage> = HashMap::new();
    for name in &plan.artists {
        let artist = builder.artist(name).await?;
        let mut page = artist::artist(&pathfinder, &artist.uri).await?;
        page.saved = true;
        println!("  {name} → {} ({} songs, {} shelves)", page.name, page.top_tracks.len(), page.shelves.len());
        artist_pages.insert(artist.uri.clone(), page);
        artists.push(artist);
    }

    println!("Playlists");
    let mut playlists = Vec::new();
    let mut every_playlist: HashMap<String, PlaylistInfo> = HashMap::new();
    for planned in &plan.playlists {
        let songs = builder.tracks(&planned.tracks).await?;
        let id = format!("cloofydemo{}", slug(&planned.name));
        let (owner_id, owner_name) = match &planned.owner {
            Some(owner) => (slug(owner), owner.clone()),
            None => (user.id.clone(), user.name.clone()),
        };
        let playlist = PlaylistInfo {
            uri: format!("spotify:playlist:{id}"),
            id,
            name: planned.name.clone(),
            owner_id,
            owner_name,
            collaborative: false,
            total: songs.len() as u32,
            image_url: mosaic(&songs),
        };
        tracks.insert(playlist.uri.clone(), songs);
        if planned.library {
            playlists.push(playlist.clone());
        }
        every_playlist.insert(planned.name.clone(), playlist);
    }

    println!("Liked Songs");
    let liked = builder.tracks(&plan.liked).await?;
    let liked_uri = format!("spotify:user:{}:collection", user.id);
    tracks.insert(liked_uri.clone(), liked.clone());

    println!("Searches");
    let mut searches = HashMap::new();
    for query in &plan.searches {
        let mut results = catalog::search(&builder.api, query).await?;
        // Playlists of other users would show their names; keep the artists' own.
        results.playlists.retain(|p| artists.iter().any(|a| a.name == p.owner_name));
        searches.insert(query.to_lowercase(), results);
    }

    println!("Now playing");
    let track = builder.track(&plan.playing.track).await?;
    let queued = builder.tracks(&plan.playing.queued).await?;
    let context = every_playlist.get(&plan.playing.context).map(|p| p.uri.clone());
    let cover_url_large = track.image_url.as_ref().map(|url| url.replace(SMALL, LARGE));
    let mut lyrics: HashMap<String, Lyrics> = HashMap::new();
    let http = reqwest::Client::new();
    let artist_name = track.artists.first().cloned().unwrap_or_default();
    let query = LyricsQuery {
        uri: &track.uri,
        title: &track.name,
        artist: &artist_name,
        album: &track.album,
        duration_ms: track.duration_ms,
    };
    match lyrics::fetch(Some(&session), &http, query).await? {
        Some(found) => {
            println!("  lyrics: {} lines from {}", found.lines.len(), found.provider);
            lyrics.insert(track.uri.clone(), found);
        }
        None => println!("  no lyrics"),
    }

    let item = |reference: &str| -> Result<HomeItem> {
        let (kind, name) = reference.split_once(':').unwrap_or((reference, ""));
        let item = match kind {
            "liked" => HomeItem {
                uri: liked_uri.clone(),
                kind: ItemKind::Playlist,
                title: "Liked Songs".into(),
                subtitle: format!("{} songs", liked.len()),
                image_url: Some(LIKED_IMAGE.into()),
                owner_id: user.id.clone(),
                owner_name: user.name.clone(),
                context_uri: liked_uri.clone(),
            },
            "playlist" => {
                let playlist = every_playlist.get(name).with_context(|| format!("no playlist “{name}”"))?;
                let subtitle = if playlist.owner_id == user.id {
                    format!("By {}", user.name)
                } else {
                    artists_of(&tracks[&playlist.uri])
                };
                HomeItem {
                    uri: playlist.uri.clone(),
                    kind: ItemKind::Playlist,
                    title: playlist.name.clone(),
                    subtitle,
                    image_url: playlist.image_url.clone(),
                    owner_id: playlist.owner_id.clone(),
                    owner_name: playlist.owner_name.clone(),
                    context_uri: playlist.uri.clone(),
                }
            }
            "album" => {
                // Spotify names some editions longer, like "Since I Left You (Deluxe Edition)".
                let album = albums
                    .iter()
                    .find(|a| a.name == name || a.name.starts_with(name))
                    .with_context(|| format!("no album “{name}”"))?;
                HomeItem {
                    uri: album.uri.clone(),
                    kind: ItemKind::Album,
                    title: album.name.clone(),
                    subtitle: album.artists.join(", "),
                    image_url: album.image_url.clone(),
                    owner_id: String::new(),
                    owner_name: String::new(),
                    context_uri: album.uri.clone(),
                }
            }
            "artist" => {
                let artist = artists.iter().find(|a| a.name == name).with_context(|| format!("no artist “{name}”"))?;
                HomeItem {
                    uri: artist.uri.clone(),
                    kind: ItemKind::Artist,
                    title: artist.name.clone(),
                    subtitle: "Artist".into(),
                    image_url: artist.image_url.clone(),
                    owner_id: String::new(),
                    owner_name: String::new(),
                    context_uri: artist.uri.clone(),
                }
            }
            _ => anyhow::bail!("unknown Home item “{reference}”"),
        };
        Ok(item)
    };
    let mut shelves = vec![Shelf {
        uri: "spotify:section:shortcuts".into(),
        title: String::new(),
        shortcuts: true,
        items: plan.home.shortcuts.iter().map(|r| item(r)).collect::<Result<_>>()?,
    }];
    for (index, shelf) in plan.home.shelves.iter().enumerate() {
        shelves.push(Shelf {
            uri: format!("spotify:section:demo{index}"),
            title: shelf.title.clone(),
            shortcuts: false,
            items: shelf.items.iter().map(|r| item(r)).collect::<Result<_>>()?,
        });
    }

    let demo = Demo {
        user: user.clone(),
        playlists,
        albums,
        artists,
        shows: Vec::new(),
        liked,
        home: Home { greeting: plan.home.greeting.clone(), shelves },
        tracks,
        artist_pages,
        searches,
        lyrics,
        playing: Playing { track, cover_url_large, context, position_ms: plan.playing.position_ms, queued },
    };
    let out = dir.join("sample.json");
    std::fs::write(&out, serde_json::to_vec(&demo)?)?;
    println!("Wrote {} ({} KB)", out.display(), std::fs::metadata(&out)?.len() / 1024);
    session.shutdown();
    Ok(())
}
