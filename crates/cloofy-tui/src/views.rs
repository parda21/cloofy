use std::{ops::Range, time::Instant};

use cloofy_spotify::{
    artist::{ArtistPage, TopTrack},
    catalog::{AlbumInfo, ArtistInfo, PlaylistInfo, SearchResults, Track},
    home::{Home, HomeItem},
};

/// Cursor and scroll position of a list.
#[derive(Debug, Default, Clone)]
pub struct ListCursor {
    pub selected: usize,
    pub offset: usize,
    /// Scrolled with the mouse: the view stays put instead of following the selection.
    pub detached: bool,
}

impl ListCursor {
    /// Scrolls the view without moving the selection, like a mouse wheel should.
    pub fn scroll(&mut self, delta: isize) {
        self.offset = (self.offset as isize + delta).max(0) as usize;
        self.detached = true;
    }

    pub fn move_by(&mut self, delta: isize, len: usize) {
        self.detached = false;
        self.selected = if len == 0 { 0 } else { (self.selected as isize + delta).clamp(0, len as isize - 1) as usize };
    }

    /// Scrolls so the selection stays visible with some context, and returns the visible rows.
    pub fn visible(&mut self, height: usize, len: usize) -> Range<usize> {
        if height == 0 {
            return 0..0;
        }
        if self.detached {
            self.offset = self.offset.min(len.saturating_sub(height));
            return self.offset..(self.offset + height).min(len);
        }
        let margin = 2.min(height / 4);
        if self.selected < self.offset + margin {
            self.offset = self.selected.saturating_sub(margin);
        }
        if self.selected + margin >= self.offset + height {
            self.offset = (self.selected + margin + 1).saturating_sub(height);
        }
        self.offset = self.offset.min(len.saturating_sub(height));
        self.offset..(self.offset + height).min(len)
    }
}

#[derive(Debug, Clone)]
pub enum TrackSource {
    Liked,
    Playlist(PlaylistInfo),
    Album(AlbumInfo),
}

pub struct TrackView {
    pub id: u64,
    pub source: TrackSource,
    pub tracks: Vec<Track>,
    pub total: Option<u32>,
    pub loading: bool,
    pub error: Option<String>,
    pub list: ListCursor,
}

impl TrackView {
    /// Takes a song out of the list, keeping the cursor on the song it was on. Returns where
    /// it was.
    pub fn remove_track(&mut self, uri: &str) -> Option<(usize, Track)> {
        let index = self.tracks.iter().position(|t| t.uri == uri)?;
        let track = self.tracks.remove(index);
        if index < self.list.selected {
            self.list.selected -= 1;
        }
        self.list.selected = self.list.selected.min(self.tracks.len().saturating_sub(1));
        self.total = self.total.map(|total| total.saturating_sub(1));
        Some((index, track))
    }

    /// Adds a song unless it is listed already, keeping the cursor on the song it was on.
    pub fn insert_track(&mut self, index: usize, track: Track) {
        if self.tracks.iter().any(|t| t.uri == track.uri) {
            return;
        }
        let index = index.min(self.tracks.len());
        if index <= self.list.selected && !self.tracks.is_empty() {
            self.list.selected += 1;
        }
        self.tracks.insert(index, track);
        self.total = self.total.map(|total| total + 1);
    }
}

#[derive(Debug, Clone)]
pub enum SearchEntry {
    Header(&'static str),
    Track(Track),
    Album(AlbumInfo),
    Artist(ArtistInfo),
    Playlist(PlaylistInfo),
}

pub struct SearchView {
    pub id: u64,
    pub query: String,
    /// The query the current entries belong to.
    pub searched: String,
    pub entries: Vec<SearchEntry>,
    pub loading: bool,
    pub error: Option<String>,
    /// Set while typing; the search starts once the input rests for a moment.
    pub typed_at: Option<Instant>,
    pub list: ListCursor,
}

impl SearchView {
    pub fn set_results(&mut self, query: String, results: SearchResults) {
        let mut entries = Vec::new();
        let mut section = |title, items: Vec<SearchEntry>| {
            if !items.is_empty() {
                entries.push(SearchEntry::Header(title));
                entries.extend(items);
            }
        };
        section("SONGS", results.tracks.into_iter().map(SearchEntry::Track).collect());
        section("ARTISTS", results.artists.into_iter().map(SearchEntry::Artist).collect());
        section("ALBUMS", results.albums.into_iter().map(SearchEntry::Album).collect());
        section("PLAYLISTS", results.playlists.into_iter().map(SearchEntry::Playlist).collect());
        self.entries = entries;
        self.searched = query;
        self.list = ListCursor::default();
        self.move_by(0);
    }

    pub fn selected(&self) -> Option<&SearchEntry> {
        self.entries.get(self.list.selected).filter(|e| !matches!(e, SearchEntry::Header(_)))
    }

    /// Moves the cursor, skipping section headers.
    pub fn move_by(&mut self, delta: isize) {
        let len = self.entries.len();
        let step = if delta < 0 { -1 } else { 1 };
        self.list.move_by(delta, len);
        while matches!(self.entries.get(self.list.selected), Some(SearchEntry::Header(_))) {
            let before = self.list.selected;
            self.list.move_by(step, len);
            if self.list.selected == before {
                // Stuck at the top on a header: go the other way instead.
                self.list.move_by(-step, len);
                if self.list.selected == before {
                    break;
                }
            }
        }
    }
}

/// The Home feed: shelves of cards, navigated in two dimensions.
pub struct HomeView {
    pub id: u64,
    pub home: Option<Home>,
    pub loading: bool,
    pub error: Option<String>,
    pub shelf: usize,
    pub item: usize,
    /// First visible card per shelf.
    pub offsets: Vec<usize>,
    /// Cards that fit next to each other at the last draw.
    pub cards: usize,
    /// Vertical scroll in rows.
    pub scroll: u16,
    /// Scrolled with the mouse: the view stays put instead of following the selection.
    pub detached: bool,
}

/// Pages a shelf of `len` cards sideways; a selected card moves along to stay in view.
fn page_shelf(offset: &mut usize, len: usize, cards: usize, delta: isize, item: Option<&mut usize>) {
    let cards = cards.max(1);
    *offset = (*offset as isize + delta).clamp(0, len.saturating_sub(cards) as isize) as usize;
    if let Some(item) = item {
        *item = (*item).clamp(*offset, (*offset + cards - 1).min(len.saturating_sub(1)));
    }
}

impl HomeView {
    pub fn new(id: u64, cached: Option<Home>) -> Self {
        let offsets = cached.as_ref().map_or_else(Vec::new, |home| vec![0; home.shelves.len()]);
        Self {
            id,
            home: cached,
            loading: true,
            error: None,
            shelf: 0,
            item: 0,
            offsets,
            cards: 1,
            scroll: 0,
            detached: false,
        }
    }

    pub fn scroll_by(&mut self, rows: isize) {
        self.scroll = (self.scroll as isize + rows).max(0) as u16;
        self.detached = true;
    }

    pub fn scroll_shelf(&mut self, shelf: usize, delta: isize) {
        let Some(home) = &self.home else { return };
        let Some(len) = home.shelves.get(shelf).filter(|s| !s.shortcuts).map(|s| s.items.len()) else { return };
        let Some(offset) = self.offsets.get_mut(shelf) else { return };
        page_shelf(offset, len, self.cards, delta, (self.shelf == shelf).then_some(&mut self.item));
    }

    pub fn set_home(&mut self, home: Home) {
        self.offsets = vec![0; home.shelves.len()];
        self.home = Some(home);
        self.loading = false;
        self.error = None;
        self.clamp();
    }

    pub fn selected(&self) -> Option<&HomeItem> {
        self.home.as_ref()?.shelves.get(self.shelf)?.items.get(self.item)
    }

    pub fn move_shelf(&mut self, delta: isize) {
        self.detached = false;
        let shelves = self.home.as_ref().map_or(0, |h| h.shelves.len());
        if shelves > 0 {
            self.shelf = (self.shelf as isize + delta).clamp(0, shelves as isize - 1) as usize;
            self.clamp();
        }
    }

    /// Moves within the current shelf. Returns `false` at the left edge, so the caller can
    /// hand focus to the sidebar.
    pub fn move_item(&mut self, delta: isize) -> bool {
        if delta < 0 && self.item == 0 {
            return false;
        }
        self.detached = false;
        self.item = (self.item as isize + delta).max(0) as usize;
        self.clamp();
        true
    }

    fn clamp(&mut self) {
        let Some(home) = &self.home else { return };
        self.shelf = self.shelf.min(home.shelves.len().saturating_sub(1));
        let items = home.shelves.get(self.shelf).map_or(0, |s| s.items.len());
        self.item = self.item.min(items.saturating_sub(1));
    }
}

/// An artist page: popular tracks first, then one row per shelf.
pub struct ArtistView {
    pub id: u64,
    pub uri: String,
    pub name: String,
    pub page: Option<ArtistPage>,
    pub error: Option<String>,
    pub row: usize,
    pub item: usize,
    /// First visible card per shelf.
    pub offsets: Vec<usize>,
    /// Cards that fit next to each other at the last draw.
    pub cards: usize,
    /// Vertical scroll in rows.
    pub scroll: u16,
    /// Scrolled with the mouse: the view stays put instead of following the selection.
    pub detached: bool,
}

pub enum ArtistSelection<'a> {
    Track(&'a TopTrack),
    Item(&'a HomeItem),
}

impl ArtistView {
    pub fn new(id: u64, uri: String, name: String) -> Self {
        Self {
            id,
            uri,
            name,
            page: None,
            error: None,
            row: 0,
            item: 0,
            offsets: Vec::new(),
            cards: 1,
            scroll: 0,
            detached: false,
        }
    }

    pub fn scroll_by(&mut self, rows: isize) {
        self.scroll = (self.scroll as isize + rows).max(0) as u16;
        self.detached = true;
    }

    /// Pages a shelf sideways; `row` counts the top tracks first, like [`Self::row`].
    pub fn scroll_shelf(&mut self, row: usize, delta: isize) {
        let Some(page) = &self.page else { return };
        let Some(index) = row.checked_sub(page.top_tracks.len()) else { return };
        let Some(len) = page.shelves.get(index).map(|s| s.items.len()) else { return };
        let Some(offset) = self.offsets.get_mut(index) else { return };
        page_shelf(offset, len, self.cards, delta, (self.row == row).then_some(&mut self.item));
    }

    pub fn set_page(&mut self, page: ArtistPage) {
        self.offsets = vec![0; page.shelves.len()];
        self.name = page.name.clone();
        self.page = Some(page);
    }

    pub fn track_count(&self) -> usize {
        self.page.as_ref().map_or(0, |p| p.top_tracks.len())
    }

    fn rows(&self) -> usize {
        self.page.as_ref().map_or(0, |p| p.top_tracks.len() + p.shelves.len())
    }

    pub fn selected(&self) -> Option<ArtistSelection<'_>> {
        let page = self.page.as_ref()?;
        match self.row.checked_sub(page.top_tracks.len()) {
            None => page.top_tracks.get(self.row).map(ArtistSelection::Track),
            Some(shelf) => page.shelves.get(shelf)?.items.get(self.item).map(ArtistSelection::Item),
        }
    }

    pub fn move_row(&mut self, delta: isize) {
        self.detached = false;
        let rows = self.rows();
        if rows > 0 {
            self.row = (self.row as isize + delta).clamp(0, rows as isize - 1) as usize;
            self.clamp_item();
        }
    }

    /// Moves within a shelf. Returns `false` when there is nowhere to go to the left.
    pub fn move_item(&mut self, delta: isize) -> bool {
        if self.row < self.track_count() || (delta < 0 && self.item == 0) {
            return delta > 0;
        }
        self.item = (self.item as isize + delta).max(0) as usize;
        self.clamp_item();
        true
    }

    fn clamp_item(&mut self) {
        let Some(page) = &self.page else { return };
        let items = self
            .row
            .checked_sub(page.top_tracks.len())
            .and_then(|shelf| page.shelves.get(shelf))
            .map_or(0, |s| s.items.len());
        self.item = self.item.min(items.saturating_sub(1));
    }
}

/// "Your Library" as a full page: a grid of covers with filter tabs.
pub struct LibraryView {
    pub id: u64,
    pub filter: crate::app::LibraryFilter,
    pub cursor: usize,
    /// Cards per row at the last draw, for moving up and down.
    pub columns: usize,
    /// Screen rows scrolled past at the top of the grid.
    pub top: usize,
    /// Scrolled with the mouse: the view stays put instead of following the cursor.
    pub detached: bool,
}

pub enum View {
    Home(HomeView),
    Library(LibraryView),
    Artist(ArtistView),
    Tracks(TrackView),
    Search(SearchView),
}

impl View {
    pub fn id(&self) -> u64 {
        match self {
            View::Home(v) => v.id,
            View::Library(v) => v.id,
            View::Artist(v) => v.id,
            View::Tracks(v) => v.id,
            View::Search(v) => v.id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(n: u32) -> Track {
        Track {
            uri: format!("spotify:track:{n}"),
            name: format!("Song {n}"),
            artists: Vec::new(),
            artist_uris: Vec::new(),
            album: String::new(),
            album_uri: String::new(),
            duration_ms: 1,
        }
    }

    fn liked(count: u32, selected: usize) -> TrackView {
        TrackView {
            id: 1,
            source: TrackSource::Liked,
            tracks: (0..count).map(track).collect(),
            total: Some(count),
            loading: false,
            error: None,
            list: ListCursor { selected, ..ListCursor::default() },
        }
    }

    #[test]
    fn removing_a_song_keeps_the_cursor_on_its_song() {
        let mut view = liked(4, 2);
        assert_eq!(view.remove_track("spotify:track:0").map(|(index, _)| index), Some(0));
        assert_eq!(view.tracks[view.list.selected].uri, "spotify:track:2");
        assert_eq!(view.total, Some(3));
        assert!(view.remove_track("spotify:track:9").is_none());
    }

    #[test]
    fn adding_a_song_keeps_the_cursor_and_skips_duplicates() {
        let mut view = liked(3, 1);
        view.insert_track(0, track(7));
        view.insert_track(0, track(7));
        assert_eq!(view.tracks.len(), 4);
        assert_eq!(view.tracks[view.list.selected].uri, "spotify:track:1");
        assert_eq!(view.total, Some(4));
    }
}
