use std::{collections::HashMap, ops::Range, time::Instant};

use cloofy_core::{
    artist::{ArtistPage, TopTrack},
    catalog::{AlbumInfo, ArtistInfo, PlaylistInfo, SearchResults, Track},
    home::{Home, HomeItem},
};

use crate::{loader::Fetch, preferences::Setting, queue::Upcoming};

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

impl TrackSource {
    /// Names the list: Liked Songs, or the playlist or album URI.
    pub fn key(&self) -> &str {
        match self {
            TrackSource::Liked => "liked",
            TrackSource::Playlist(p) => &p.uri,
            TrackSource::Album(a) => &a.uri,
        }
    }

    /// Whether both show the same list, however they describe it.
    pub fn same(&self, other: &TrackSource) -> bool {
        self.key() == other.key()
    }
}

pub struct TrackView {
    pub id: u64,
    pub source: TrackSource,
    pub tracks: Vec<Track>,
    pub total: Option<u32>,
    pub loading: bool,
    pub error: Option<String>,
    pub list: ListCursor,
    /// Emptied to save memory because the list was opened again later; it loads again when
    /// Back returns to it.
    pub stale: bool,
    /// The load filling the list; dropping it stops the load.
    pub fetch: Option<Fetch>,
    /// When the list was last loaded completely.
    pub fetched_at: Option<Instant>,
    /// A fresh copy on its way while the songs on screen stay; it replaces them once complete.
    pub incoming: Option<Vec<Track>>,
}

impl TrackView {
    /// An empty list; [`Self::load`] fills it.
    pub fn new(source: TrackSource) -> Self {
        Self {
            id: 0,
            source,
            tracks: Vec::new(),
            total: None,
            loading: false,
            error: None,
            list: ListCursor::default(),
            stale: false,
            fetch: None,
            fetched_at: None,
            incoming: None,
        }
    }

    /// Loads the list from scratch under a new `id`, so pages of an earlier load can't land.
    pub fn load(&mut self, id: u64, fetch: impl FnOnce(u64, TrackSource) -> Fetch) {
        self.id = id;
        self.tracks = Vec::new();
        self.total = None;
        self.loading = true;
        self.error = None;
        self.stale = false;
        self.incoming = None;
        self.fetch = Some(fetch(id, self.source.clone()));
    }

    /// Loads a fresh copy under a new `id` while the songs on screen stay.
    pub fn refresh(&mut self, id: u64, fetch: impl FnOnce(u64, TrackSource) -> Fetch) {
        self.id = id;
        self.incoming = Some(Vec::new());
        self.fetch = Some(fetch(id, self.source.clone()));
    }

    /// Swaps in a fresh copy of the list, keeping the cursor and the view on the same song.
    pub fn replace_tracks(&mut self, tracks: Vec<Track>) {
        let selected = self.tracks.get(self.list.selected).map(|t| t.uri.clone());
        self.tracks = tracks;
        if let Some(index) = selected.and_then(|uri| self.tracks.iter().position(|t| t.uri == uri)) {
            self.list.offset = (self.list.offset + index).saturating_sub(self.list.selected);
            self.list.selected = index;
        }
        self.list.selected = self.list.selected.min(self.tracks.len().saturating_sub(1));
    }

    /// Drops the songs and stops loading them; the cursor stays for when the list comes
    /// back. The new `id` turns away pages that are already on their way.
    pub fn retire(&mut self, id: u64) {
        self.id = id;
        self.tracks = Vec::new();
        self.total = None;
        self.loading = false;
        self.error = None;
        self.stale = true;
        self.fetch = None;
        self.fetched_at = None;
        self.incoming = None;
    }

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
    /// Screen rows per result at the last draw, so the mouse wheel scrolls by rows.
    pub entry_rows: u16,
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
        section("Songs", results.tracks.into_iter().map(SearchEntry::Track).collect());
        section("Artists", results.artists.into_iter().map(SearchEntry::Artist).collect());
        section("Albums", results.albums.into_iter().map(SearchEntry::Album).collect());
        section("Playlists", results.playlists.into_iter().map(SearchEntry::Playlist).collect());
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
        let entries = &self.entries;
        move_past_headers(&mut self.list, delta, entries.len(), |index| {
            matches!(entries.get(index), Some(SearchEntry::Header(_)))
        });
    }
}

/// Moves a list's cursor by `delta`, on past the rows `is_header` names.
fn move_past_headers(list: &mut ListCursor, delta: isize, len: usize, is_header: impl Fn(usize) -> bool) {
    let step = if delta < 0 { -1 } else { 1 };
    list.move_by(delta, len);
    while is_header(list.selected) {
        let before = list.selected;
        list.move_by(step, len);
        if list.selected == before {
            // Stuck at the top on a header: go the other way instead.
            list.move_by(-step, len);
            if list.selected == before {
                break;
            }
        }
    }
}

/// A row of the queue page.
#[derive(Debug, Clone)]
pub enum QueueRow {
    Header(String),
    /// The song that plays now.
    Playing(Track),
    Next {
        track: Track,
        entry: Upcoming,
    },
}

impl QueueRow {
    pub fn track(&self) -> Option<&Track> {
        match self {
            QueueRow::Header(_) => None,
            QueueRow::Playing(track) | QueueRow::Next { track, .. } => Some(track),
        }
    }

    /// What names the row from one list of what plays next to another.
    fn key(&self) -> Option<(&str, &str)> {
        match self {
            QueueRow::Header(_) => None,
            QueueRow::Playing(_) => Some(("", "")),
            QueueRow::Next { entry, .. } => Some((&entry.uri, &entry.uid)),
        }
    }
}

/// What plays next: the playing song, the songs the user queued, then the rest of the context.
pub struct QueueView {
    pub id: u64,
    pub rows: Vec<QueueRow>,
    pub list: ListCursor,
    /// Spotify hasn't said yet what plays next.
    pub loading: bool,
    /// Screen rows per entry at the last draw, so the mouse wheel scrolls by rows.
    pub entry_rows: u16,
}

impl QueueView {
    pub fn new(id: u64) -> Self {
        Self { id, rows: Vec::new(), list: ListCursor::default(), loading: false, entry_rows: 1 }
    }

    /// Shows the queue as it is now, keeping the cursor on the song it was on.
    pub fn set_rows(&mut self, rows: Vec<QueueRow>) {
        let selected =
            self.rows.get(self.list.selected).and_then(QueueRow::key).map(|(a, b)| (a.to_owned(), b.to_owned()));
        let found = selected.and_then(|(uri, uid)| rows.iter().position(|r| r.key() == Some((&uri, &uid))));
        self.rows = rows;
        if let Some(index) = found {
            self.list.offset = (self.list.offset + index).saturating_sub(self.list.selected);
            self.list.selected = index;
        }
        self.move_by(0);
    }

    pub fn selected(&self) -> Option<&QueueRow> {
        self.rows.get(self.list.selected).filter(|row| !matches!(row, QueueRow::Header(_)))
    }

    /// Moves the cursor, skipping section headers.
    pub fn move_by(&mut self, delta: isize) {
        let rows = &self.rows;
        move_past_headers(&mut self.list, delta, rows.len(), |index| {
            matches!(rows.get(index), Some(QueueRow::Header(_)))
        });
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

    /// Shows a newer Home. Shelves and cards are found again by URI, so the selection and
    /// how far each shelf was paged survive the update.
    pub fn set_home(&mut self, home: Home) {
        let old = self.home.take();
        let shelves = old.as_ref().map_or(&[][..], |h| &h.shelves[..]);
        let offsets: HashMap<&str, usize> = shelves
            .iter()
            .zip(&self.offsets)
            .filter(|(shelf, _)| !shelf.uri.is_empty())
            .map(|(shelf, offset)| (shelf.uri.as_str(), *offset))
            .collect();
        self.offsets = home.shelves.iter().map(|s| offsets.get(s.uri.as_str()).copied().unwrap_or(0)).collect();
        if let Some(selected) = shelves.get(self.shelf).filter(|s| !s.uri.is_empty())
            && let Some(shelf) = home.shelves.iter().position(|s| s.uri == selected.uri)
        {
            let card = selected.items.get(self.item).map(|i| i.uri.as_str());
            self.item =
                home.shelves[shelf].items.iter().position(|i| Some(i.uri.as_str()) == card).unwrap_or(self.item);
            self.shelf = shelf;
        }
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
#[derive(Clone)]
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
    /// When the page last arrived.
    pub fetched_at: Option<Instant>,
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
            fetched_at: None,
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
        // A refreshed page keeps its place unless its shelves changed.
        let refresh = self.page.as_ref().is_some_and(|old| old.shelves.len() == page.shelves.len());
        if !refresh {
            self.offsets = vec![0; page.shelves.len()];
        }
        self.name = page.name.clone();
        self.page = Some(page);
        self.fetched_at = Some(Instant::now());
        if refresh {
            self.row = self.row.min(self.rows().saturating_sub(1));
            self.clamp_item();
        }
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

/// The Settings page.
pub struct SettingsView {
    pub id: u64,
    /// Index into [`Setting::all`].
    pub selected: usize,
    /// Screen rows scrolled past at the top, when the page is taller than the window.
    pub top: usize,
    /// Scrolled with the mouse: the page stays put instead of following the selection.
    pub detached: bool,
}

impl SettingsView {
    pub fn new(id: u64) -> Self {
        Self { id, selected: 0, top: 0, detached: false }
    }

    pub fn setting(&self) -> Setting {
        Setting::all().nth(self.selected).unwrap_or(Setting::Crossfade)
    }

    pub fn move_by(&mut self, delta: isize) {
        let last = Setting::all().count() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
        self.detached = false;
    }
}

pub enum View {
    Home(HomeView),
    Library(LibraryView),
    Artist(ArtistView),
    Tracks(TrackView),
    Search(SearchView),
    Settings(SettingsView),
    Queue(QueueView),
}

impl View {
    pub fn id(&self) -> u64 {
        match self {
            View::Home(v) => v.id,
            View::Library(v) => v.id,
            View::Artist(v) => v.id,
            View::Tracks(v) => v.id,
            View::Search(v) => v.id,
            View::Settings(v) => v.id,
            View::Queue(v) => v.id,
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
            image_url: None,
        }
    }

    fn liked(count: u32, selected: usize) -> TrackView {
        TrackView {
            tracks: (0..count).map(track).collect(),
            total: Some(count),
            list: ListCursor { selected, ..ListCursor::default() },
            ..TrackView::new(TrackSource::Liked)
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
    fn a_fresh_copy_keeps_the_cursor_on_its_song() {
        let mut view = liked(5, 3);
        view.list.offset = 2;
        view.replace_tracks([9, 8, 0, 1, 2, 3, 4].map(track).to_vec());
        assert_eq!(view.list.selected, 5);
        assert_eq!(view.list.offset, 4);
    }

    fn shelf(uri: &str, items: &[&str]) -> cloofy_core::home::Shelf {
        let item = |uri: &&str| HomeItem {
            uri: uri.to_string(),
            kind: cloofy_core::home::ItemKind::Playlist,
            title: uri.to_string(),
            subtitle: String::new(),
            image_url: None,
            owner_id: String::new(),
            owner_name: String::new(),
            context_uri: uri.to_string(),
        };
        cloofy_core::home::Shelf {
            uri: uri.into(),
            title: uri.into(),
            shortcuts: false,
            items: items.iter().map(item).collect(),
        }
    }

    #[test]
    fn a_newer_home_keeps_the_selected_card_and_paging() {
        let home = |shelves| Home { greeting: String::new(), shelves };
        let mut view = HomeView::new(1, Some(home(vec![shelf("a", &["1", "2"]), shelf("b", &["3", "4", "5"])])));
        (view.shelf, view.item, view.offsets) = (1, 2, vec![0, 1]);
        view.set_home(home(vec![shelf("new", &["0"]), shelf("b", &["6", "3", "4", "5"]), shelf("a", &["1"])]));
        assert_eq!((view.shelf, view.item), (1, 3));
        assert_eq!(view.offsets, [0, 1, 0]);
    }

    fn next(n: u32, provider: crate::queue::Provider) -> QueueRow {
        let entry = Upcoming { uri: format!("spotify:track:{n}"), uid: format!("u{n}"), provider };
        QueueRow::Next { track: track(n), entry }
    }

    fn queue_rows(queued: &[u32], rest: &[u32]) -> Vec<QueueRow> {
        use crate::queue::Provider;
        let mut rows = vec![QueueRow::Header("Now playing".into()), QueueRow::Playing(track(0))];
        if !queued.is_empty() {
            rows.push(QueueRow::Header("Next in queue".into()));
            rows.extend(queued.iter().map(|&n| next(n, Provider::Queue)));
        }
        rows.push(QueueRow::Header("Next from: Late Night Coding".into()));
        rows.extend(rest.iter().map(|&n| next(n, Provider::Context)));
        rows
    }

    #[test]
    fn the_queue_cursor_passes_over_section_titles() {
        let mut view = QueueView::new(1);
        view.set_rows(queue_rows(&[1], &[2, 3]));
        // It starts on the playing song, under the first title.
        assert!(matches!(view.selected(), Some(QueueRow::Playing(_))));
        view.move_by(1);
        assert_eq!(view.selected().and_then(QueueRow::track).map(|t| t.uri.as_str()), Some("spotify:track:1"));
        view.move_by(1);
        assert_eq!(view.selected().and_then(QueueRow::track).map(|t| t.uri.as_str()), Some("spotify:track:2"));
        view.move_by(isize::MIN / 2);
        assert!(matches!(view.selected(), Some(QueueRow::Playing(_))));
    }

    #[test]
    fn a_newer_queue_keeps_the_cursor_on_its_song() {
        let mut view = QueueView::new(1);
        view.set_rows(queue_rows(&[1, 4], &[2, 3]));
        view.move_by(4);
        assert_eq!(view.selected().and_then(QueueRow::track).map(|t| t.uri.as_str()), Some("spotify:track:2"));
        // The first queued song started playing: the rest moves up, the cursor stays on its song.
        view.set_rows(queue_rows(&[4], &[2, 3]));
        assert_eq!(view.selected().and_then(QueueRow::track).map(|t| t.uri.as_str()), Some("spotify:track:2"));
        // A song that's gone leaves the cursor on a song nearby, never on a title.
        view.set_rows(queue_rows(&[], &[3]));
        assert!(view.selected().is_some());
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
