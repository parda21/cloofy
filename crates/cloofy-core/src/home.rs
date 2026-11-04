//! The Home feed: shelves of playlists, albums, artists and podcasts picked for the user.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Home {
    pub greeting: String,
    pub shelves: Vec<Shelf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shelf {
    pub uri: String,
    pub title: String,
    /// The compact grid of shortcuts at the top of Home.
    pub shortcuts: bool,
    pub items: Vec<HomeItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemKind {
    Playlist,
    Album,
    Artist,
    Audiobook,
    Podcast,
    Episode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeItem {
    pub uri: String,
    pub kind: ItemKind,
    pub title: String,
    pub subtitle: String,
    pub image_url: Option<String>,
    /// Playlist owner, to tell own playlists from the service's.
    pub owner_id: String,
    pub owner_name: String,
    /// What to play: the show for an episode, the item itself otherwise.
    pub context_uri: String,
}
