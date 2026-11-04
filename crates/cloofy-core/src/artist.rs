//! Artist pages: popular tracks, then discography and related artists as shelves.

use serde::{Deserialize, Serialize};

use crate::{catalog::Track, home::Shelf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistPage {
    pub uri: String,
    pub name: String,
    pub image_url: Option<String>,
    pub monthly_listeners: Option<u64>,
    pub followers: Option<u64>,
    /// Whether the user follows the artist.
    pub saved: bool,
    pub top_tracks: Vec<TopTrack>,
    /// Discography, related artists and appearances, shown like Home shelves.
    pub shelves: Vec<Shelf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopTrack {
    pub track: Track,
    pub playcount: Option<u64>,
}
