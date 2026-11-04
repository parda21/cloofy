//! What cloofy remembers between runs: each service's open page and selection, which service
//! shows, and the last track.

use std::{collections::BTreeMap, fs, io, path::Path};

use serde::{Deserialize, Serialize};

use crate::app::LibraryFilter;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// The page of the service shown, where files from before YouTube Music kept Spotify's.
    #[serde(flatten)]
    pub page: Page,
    /// The service shown, by key; missing in files from before YouTube Music.
    pub service: Option<String>,
    /// The pages of the other services, by key.
    pub pages: BTreeMap<String, Page>,
    pub last_track: Option<LastTrack>,
    pub volume: Option<u16>,
    /// Only in files from before settings moved to `config.toml`.
    pub crossfade: Option<u64>,
}

/// Where a service's pages were left: the open one, and the sidebar.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Page {
    pub view: SavedView,
    pub selection: Selection,
    pub sidebar_focused: bool,
    pub sidebar_selected: usize,
    pub library_filter: LibraryFilter,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
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

impl SavedView {
    /// The playlist, album or artist it shows.
    pub fn uri(&self) -> Option<&str> {
        match self {
            SavedView::Playlist { uri, .. } | SavedView::Album { uri, .. } | SavedView::Artist { uri, .. } => Some(uri),
            SavedView::Home | SavedView::Library { .. } | SavedView::Liked | SavedView::Search { .. } => None,
        }
    }
}

/// Cursor position within a view: a list index, or a shelf and card.
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_from_before_youtube_music_holds_the_page_of_its_one_service() {
        let old = r#"{"view":{"kind":"artist","uri":"spotify:artist:1","name":"Daft Punk"},
            "selection":{"index":2,"shelf":0,"item":1},"sidebar_focused":true,"sidebar_selected":3,
            "library_filter":"Albums","last_track":null,"volume":30000}"#;
        let state: State = serde_json::from_str(old).unwrap();
        assert_eq!(state.service, None);
        assert!(state.pages.is_empty());
        assert_eq!(state.page.view, SavedView::Artist { uri: "spotify:artist:1".into(), name: "Daft Punk".into() });
        assert_eq!((state.page.selection.index, state.page.sidebar_selected), (2, 3));
        assert_eq!(state.page.library_filter, LibraryFilter::Albums);
        assert_eq!(state.volume, Some(30000));
    }

    #[test]
    fn keeps_every_services_page() {
        let dir = std::env::temp_dir().join(format!("cloofy-state-{}", std::process::id()));
        let path = dir.join("state.json");
        let search = Page { view: SavedView::Search { query: "daft punk".into() }, ..Page::default() };
        let library = Page { view: SavedView::Library { filter: LibraryFilter::Artists }, ..Page::default() };
        let state = State {
            page: search.clone(),
            service: Some("ytmusic".into()),
            pages: BTreeMap::from([("spotify".to_owned(), library.clone())]),
            ..State::default()
        };
        save(&path, &state).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.service.as_deref(), Some("ytmusic"));
        assert_eq!(loaded.page, search);
        assert_eq!(loaded.pages.get("spotify"), Some(&library));
        // Older cloofys read the page at the top as theirs and skip the rest.
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""view""#) && text.contains("daft punk"), "{text}");
        fs::remove_dir_all(dir).unwrap();
    }
}
