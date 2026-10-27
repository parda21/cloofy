//! The catalog as cloofy shows it: tracks, playlists, albums, artists and podcasts, whichever
//! service they come from.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub uri: String,
    pub name: String,
    pub artists: Vec<String>,
    /// Same order as `artists`; empty where the service doesn't say.
    pub artist_uris: Vec<String>,
    pub album: String,
    pub album_uri: String,
    pub duration_ms: u32,
    /// The album cover, where the service sends one along.
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistInfo {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub owner_id: String,
    pub owner_name: String,
    pub collaborative: bool,
    pub total: u32,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumInfo {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub artists: Vec<String>,
    pub year: String,
    pub total: u32,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistInfo {
    pub uri: String,
    pub name: String,
    pub image_url: Option<String>,
}

/// A podcast in the user's library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowInfo {
    pub uri: String,
    pub name: String,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchResults {
    pub tracks: Vec<Track>,
    pub albums: Vec<AlbumInfo>,
    pub artists: Vec<ArtistInfo>,
    pub playlists: Vec<PlaylistInfo>,
}

/// One page of a long list, and where the next one starts.
#[derive(Debug, Clone)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: u32,
    pub next_offset: Option<u32>,
}
