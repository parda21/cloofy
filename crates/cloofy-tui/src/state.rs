//! What cloofy remembers between runs: the open view, the selection and the last track.

use std::{fs, io, path::Path};

use serde::{Deserialize, Serialize};

use crate::app::LibraryFilter;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub view: SavedView,
    pub selection: Selection,
    pub sidebar_focused: bool,
    pub sidebar_selected: usize,
    pub library_filter: LibraryFilter,
    pub last_track: Option<LastTrack>,
    pub volume: Option<u16>,
    /// Only in files from before settings moved to `config.toml`.
    pub crossfade: Option<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SavedView {
    #[default]
    Home,
    Library {
        filter: LibraryFilter,
    },
    Liked,
    Playlist {
        id: String,
        uri: String,
        name: String,
        owner_id: String,
        owner_name: String,
        #[serde(default)]
        collaborative: bool,
        image_url: Option<String>,
    },
    Album {
        id: String,
        uri: String,
        name: String,
        artists: Vec<String>,
        image_url: Option<String>,
    },
    Artist {
        uri: String,
        name: String,
    },
    Search {
        query: String,
    },
}

/// Cursor position within a view: a list index, or a shelf and card.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct Selection {
    pub index: usize,
    pub shelf: usize,
    pub item: usize,
}

/// The track to resume, with where it was and what it played in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastTrack {
    pub uri: String,
    pub name: String,
    pub artists: Vec<String>,
    #[serde(default)]
    pub artist_uris: Vec<String>,
    pub album: String,
    #[serde(default)]
    pub album_uri: Option<String>,
    pub duration_ms: u32,
    pub cover_url: Option<String>,
    pub cover_url_large: Option<String>,
    /// Playlist, album or artist the track played in.
    pub context: Option<String>,
    pub position_ms: u32,
}

/// Missing or unreadable state just means a fresh start.
pub fn load(path: &Path) -> State {
    fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

pub fn save(path: &Path, state: &State) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // Write and rename, so a crash mid-write never leaves a broken file behind.
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, serde_json::to_vec_pretty(state)?)?;
    fs::rename(temp, path)
}
