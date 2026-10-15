//! A showcase that needs no Spotify account: `cloofy demo` fills the app with the library, Home
//! and playing track of a built-in sample made of public Spotify content. Nothing plays, nothing
//! is saved, and only cover art comes from the network.

use std::collections::HashMap;

use anyhow::{Context, Result};
use cloofy_spotify::{
    artist::ArtistPage,
    catalog::{AlbumInfo, ArtistInfo, PlaylistInfo, SearchResults, ShowInfo, Track, User},
    home::Home,
    lyrics::Lyrics,
};
use serde::{Deserialize, Serialize};

/// Built by `cargo run --example demo_fixture` from `demo/plan.json`.
const SAMPLE: &str = include_str!("../demo/sample.json");

#[derive(Serialize, Deserialize)]
pub struct Demo {
    pub user: User,
    pub playlists: Vec<PlaylistInfo>,
    pub albums: Vec<AlbumInfo>,
    pub artists: Vec<ArtistInfo>,
    pub shows: Vec<ShowInfo>,
    pub liked: Vec<Track>,
    pub home: Home,
    /// Track lists of playlists and albums, by URI. Playlists that only appear on Home are here
    /// too.
    pub tracks: HashMap<String, Vec<Track>>,
    pub artist_pages: HashMap<String, ArtistPage>,
    /// Results for the searches the sample was made with; other searches look through it.
    pub searches: HashMap<String, SearchResults>,
    pub lyrics: HashMap<String, Lyrics>,
    pub playing: Playing,
}

/// The track shown playing.
#[derive(Serialize, Deserialize)]
pub struct Playing {
    pub track: Track,
    pub cover_url_large: Option<String>,
    /// The playlist or album it plays from.
    pub context: Option<String>,
    pub position_ms: u32,
}

impl Demo {
    pub fn sample() -> Result<Self> {
        serde_json::from_str(SAMPLE).context("The built-in demo is broken")
    }

    pub fn tracks(&self, uri: &str) -> Vec<Track> {
        if uri.ends_with(":collection") {
            return self.liked.clone();
        }
        self.tracks.get(uri).cloned().unwrap_or_default()
    }

    /// The results saved for `query`, else whatever in the sample matches it.
    pub fn search(&self, query: &str) -> SearchResults {
        let query = query.trim().to_lowercase();
        if let Some(results) = self.searches.get(&query) {
            return results.clone();
        }
        let hit = |text: &str| text.to_lowercase().contains(&query);
        let mut tracks: Vec<Track> = self.tracks.values().flatten().cloned().collect();
        tracks.sort_by(|a, b| a.uri.cmp(&b.uri));
        tracks.dedup_by(|a, b| a.uri == b.uri);
        SearchResults {
            tracks: tracks.into_iter().filter(|t| hit(&t.name) || t.artists.iter().any(|a| hit(a))).take(10).collect(),
            albums: self.albums.iter().filter(|a| hit(&a.name) || a.artists.iter().any(|n| hit(n))).cloned().collect(),
            artists: self.artists.iter().filter(|a| hit(&a.name)).cloned().collect(),
            playlists: self.playlists.iter().filter(|p| hit(&p.name)).cloned().collect(),
        }
    }

    /// Whether `uri` is in the sample's library: liked, saved or followed.
    pub fn is_saved(&self, uri: &str) -> bool {
        self.liked.iter().any(|t| t.uri == uri)
            || self.albums.iter().any(|a| a.uri == uri)
            || self.artists.iter().any(|a| a.uri == uri)
            || self.playlists.iter().any(|p| p.uri == uri)
    }
}
