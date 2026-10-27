//! "Your Library" in memory: fresh lists from Spotify, and likes, saves and follows shown at
//! once while Spotify confirms them.

use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use cloofy_core::{
    catalog::{AlbumInfo, ArtistInfo, PlaylistInfo, ShowInfo, Track},
    home::{HomeItem, ItemKind},
};

use super::{App, NavItem};
use crate::{
    overlay::{Overlay, PaletteAction},
    playback::CurrentTrack,
    views::{QueueRow, SearchEntry, TrackSource, View},
};

/// Confirmed changes are followed by a reload of their list, once things have been quiet for
/// this long, so a burst of saves costs one reload.
const RELOAD_DELAY: Duration = Duration::from_secs(1);

/// Something the user can add to their library.
#[derive(Clone)]
pub(super) enum LibraryItem {
    Playlist(PlaylistInfo),
    Album(AlbumInfo),
    Artist(ArtistInfo),
    /// A song in Liked Songs.
    Track(Track),
}

/// A list of the library that can be loaded again on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum LibraryPart {
    Playlists,
    Albums,
    Artists,
}

impl App {
    pub(super) fn set_playlists(&mut self, fresh: Vec<PlaylistInfo>) {
        self.edit_library(|app| {
            let old = std::mem::take(&mut app.playlists);
            app.playlists = app.reconcile(old, fresh, |p| &p.uri);
        });
    }

    pub(super) fn set_albums(&mut self, fresh: Vec<AlbumInfo>) {
        self.edit_library(|app| {
            let old = std::mem::take(&mut app.albums);
            app.albums = app.reconcile(old, fresh, |a| &a.uri);
        });
    }

    pub(super) fn set_artists(&mut self, fresh: Vec<ArtistInfo>) {
        self.edit_library(|app| {
            let old = std::mem::take(&mut app.artists);
            app.artists = app.reconcile(old, fresh, |a| &a.uri);
        });
    }

    pub(super) fn set_shows(&mut self, fresh: Vec<ShowInfo>) {
        self.edit_library(|app| app.shows = fresh);
    }

    /// Takes a freshly loaded list as the truth, except for changes Spotify has not confirmed
    /// yet, which it may not reflect. Items that left the library no longer count as saved.
    fn reconcile<T: Clone>(&mut self, old: Vec<T>, mut fresh: Vec<T>, uri: impl Fn(&T) -> &str) -> Vec<T> {
        let unconfirmed = |app: &Self, item: &T| app.save_in_progress(uri(item));
        let mut list: Vec<T> = old
            .iter()
            .filter(|item| unconfirmed(self, item) && self.is_saved(uri(item)))
            .filter(|item| !fresh.iter().any(|f| uri(f) == uri(item)))
            .cloned()
            .collect();
        fresh.retain(|item| !unconfirmed(self, item) || self.is_saved(uri(item)));
        list.extend(fresh);
        let kept: HashSet<&str> = list.iter().map(&uri).collect();
        for item in &old {
            if !unconfirmed(self, item) && !kept.contains(uri(item)) {
                self.saved.insert(uri(item).to_owned(), false);
            }
        }
        self.saved.extend(list.iter().map(|item| (uri(item).to_owned(), true)));
        list
    }

    /// Changes the library lists while keeping what points into them by position (the
    /// sidebar cursor, the library page cursor, open pickers) on the same items.
    pub(super) fn edit_library(&mut self, edit: impl FnOnce(&mut Self)) {
        let nav = self.nav_items().get(self.nav.selected).and_then(|item| self.nav_uri(*item));
        let page = match &self.view {
            View::Library(v) => self.library_entries_for(v.filter).get(v.cursor).and_then(|e| self.entry_uri(*e)),
            _ => None,
        };
        let playlists: Vec<String> = self.playlists.iter().map(|p| p.uri.clone()).collect();

        edit(self);
        self.data_dirty = true;

        if let Some(uri) = nav
            && let Some(index) = self.nav_items().iter().position(|item| self.nav_uri(*item).as_ref() == Some(&uri))
        {
            self.nav.selected = index;
        }
        // While the library is still arriving, a restored cursor may point past what is there.
        if !self.playlists_loading && !self.library_loading {
            self.nav.selected = self.nav.selected.min(self.nav_len().saturating_sub(1));
        }
        let filter = match &self.view {
            View::Library(v) => Some(v.filter),
            _ => None,
        };
        if let (Some(uri), Some(filter)) = (page, filter) {
            let index = self.library_entries_for(filter).iter().position(|e| self.entry_uri(*e).as_ref() == Some(&uri));
            if let (Some(index), View::Library(v)) = (index, &mut self.view) {
                v.cursor = index;
            }
        }
        self.remap_playlist_pickers(&playlists);
    }

    fn nav_uri(&self, item: NavItem) -> Option<String> {
        match item {
            NavItem::Library(entry) => self.entry_uri(entry),
            NavItem::Home | NavItem::Search => None,
        }
    }

    /// Open pickers refer to playlists by position; points them at the same playlists after
    /// the list changed, or at nothing when one is gone.
    fn remap_playlist_pickers(&mut self, old: &[String]) {
        let position = |index: usize| {
            old.get(index).and_then(|uri| self.playlists.iter().position(|p| &p.uri == uri)).unwrap_or(usize::MAX)
        };
        match &mut self.overlay {
            Some(Overlay::AddToPlaylist { picker, .. }) => {
                for item in &mut picker.items {
                    item.value = position(item.value);
                }
            }
            Some(Overlay::Palette(picker)) => {
                for item in &mut picker.items {
                    if let PaletteAction::OpenPlaylist(index) = &mut item.value {
                        *index = position(*index);
                    }
                }
            }
            _ => {}
        }
    }

    /// Shows a like, save or follow (or undoing one) at once in the sidebar, the library page
    /// and Liked Songs. `saved` is the new state.
    pub(super) fn show_library_change(&mut self, uri: &str, saved: bool) {
        if uri.starts_with("spotify:track:") {
            return self.show_liked_change(uri, saved);
        }
        let restore = if saved { self.unsaved.remove(uri) } else { None };
        let item = restore.or_else(|| if saved { self.find_library_item(uri).map(|item| (0, item)) } else { None });
        self.edit_library(|app| match item {
            Some((index, item)) => app.insert_library_item(index, item),
            None if !saved => {
                if let Some(removed) = app.take_library_item(uri) {
                    app.unsaved.insert(uri.to_owned(), removed);
                }
            }
            None => {}
        });
    }

    fn show_liked_change(&mut self, uri: &str, liked: bool) {
        self.liked_total = self.liked_total.map(|total| if liked { total + 1 } else { total.saturating_sub(1) });
        self.data_dirty = true;
        let restore = if liked { self.unsaved.remove(uri) } else { None };
        let (index, track) = match restore {
            Some((index, LibraryItem::Track(track))) => (index, Some(track)),
            _ => (0, if liked { self.find_track(uri) } else { None }),
        };
        let mut removed = None;
        for view in std::iter::once(&mut self.view).chain(&mut self.history) {
            // A list still loading gets the change from Spotify itself.
            let View::Tracks(v) = view else { continue };
            if !matches!(v.source, TrackSource::Liked) || v.loading {
                continue;
            }
            match &track {
                Some(track) if liked => v.insert_track(index, track.clone()),
                _ if !liked => removed = v.remove_track(uri).or(removed),
                _ => {}
            }
        }
        if let Some((index, track)) = removed {
            self.unsaved.insert(uri.to_owned(), (index, LibraryItem::Track(track)));
        }
    }

    fn insert_library_item(&mut self, index: usize, item: LibraryItem) {
        fn insert<T>(list: &mut Vec<T>, index: usize, item: T, same: impl Fn(&T) -> bool) {
            if !list.iter().any(same) {
                list.insert(index.min(list.len()), item);
            }
        }
        match item {
            LibraryItem::Playlist(p) => {
                let uri = p.uri.clone();
                insert(&mut self.playlists, index, p, |q| q.uri == uri);
            }
            LibraryItem::Album(a) => {
                let uri = a.uri.clone();
                insert(&mut self.albums, index, a, |b| b.uri == uri);
            }
            LibraryItem::Artist(a) => {
                let uri = a.uri.clone();
                insert(&mut self.artists, index, a, |b| b.uri == uri);
            }
            LibraryItem::Track(_) => {}
        }
    }

    /// Removes a playlist, album or artist from the library lists; returns where it was.
    fn take_library_item(&mut self, uri: &str) -> Option<(usize, LibraryItem)> {
        if let Some(index) = self.playlists.iter().position(|p| p.uri == uri) {
            return Some((index, LibraryItem::Playlist(self.playlists.remove(index))));
        }
        if let Some(index) = self.albums.iter().position(|a| a.uri == uri) {
            return Some((index, LibraryItem::Album(self.albums.remove(index))));
        }
        let index = self.artists.iter().position(|a| a.uri == uri)?;
        Some((index, LibraryItem::Artist(self.artists.remove(index))))
    }

    /// What a newly saved playlist, album or artist looks like, from the page it was saved on.
    fn find_library_item(&self, uri: &str) -> Option<LibraryItem> {
        match &self.view {
            View::Tracks(v) => match &v.source {
                TrackSource::Playlist(p) if p.uri == uri => return Some(LibraryItem::Playlist(p.clone())),
                TrackSource::Album(a) if a.uri == uri => return Some(LibraryItem::Album(a.clone())),
                _ => {}
            },
            View::Search(v) => {
                let found = v.entries.iter().find_map(|entry| match entry {
                    SearchEntry::Playlist(p) if p.uri == uri => Some(LibraryItem::Playlist(p.clone())),
                    SearchEntry::Album(a) if a.uri == uri => Some(LibraryItem::Album(a.clone())),
                    SearchEntry::Artist(a) if a.uri == uri => Some(LibraryItem::Artist(a.clone())),
                    _ => None,
                });
                if found.is_some() {
                    return found;
                }
            }
            View::Artist(v) => {
                let page = v.page.as_ref();
                if v.uri == uri {
                    let image_url = page.and_then(|p| p.image_url.clone());
                    return Some(LibraryItem::Artist(ArtistInfo {
                        uri: v.uri.clone(),
                        name: v.name.clone(),
                        image_url,
                    }));
                }
                // Releases on an artist page are subtitled with year and type, not artists.
                let item = page.into_iter().flat_map(|p| &p.shelves).flat_map(|s| &s.items).find(|i| i.uri == uri);
                if let Some(item) = item {
                    return library_item(item, Some(&v.name));
                }
            }
            View::Home(v) => {
                let item = v.home.iter().flat_map(|h| &h.shelves).flat_map(|s| &s.items).find(|i| i.uri == uri);
                if let Some(item) = item {
                    return library_item(item, None);
                }
            }
            View::Library(_) | View::Settings(_) | View::Queue(_) => {}
        }
        let item = self.home_cache.iter().flat_map(|h| &h.shelves).flat_map(|s| &s.items).find(|i| i.uri == uri)?;
        library_item(item, None)
    }

    /// A song the user just liked, from the page it was liked on or the player.
    fn find_track(&self, uri: &str) -> Option<Track> {
        let listed = match &self.view {
            View::Tracks(v) => v.tracks.iter().find(|t| t.uri == uri).cloned(),
            View::Search(v) => v.entries.iter().find_map(|entry| match entry {
                SearchEntry::Track(t) if t.uri == uri => Some(t.clone()),
                _ => None,
            }),
            View::Artist(v) => {
                v.page.iter().flat_map(|p| &p.top_tracks).find(|t| t.track.uri == uri).map(|t| t.track.clone())
            }
            View::Queue(v) => v.rows.iter().filter_map(QueueRow::track).find(|t| t.uri == uri).cloned(),
            View::Home(_) | View::Library(_) | View::Settings(_) => None,
        };
        listed.or_else(|| self.now.track.as_ref().filter(|t| t.uri == uri).map(CurrentTrack::to_track))
    }

    /// Loads the list a confirmed change touched again, as Spotify's final word on it.
    pub(super) fn schedule_reload(&mut self, uri: &str) {
        let part = match uri.split(':').nth(1) {
            Some("playlist") => LibraryPart::Playlists,
            Some("album") => LibraryPart::Albums,
            Some("artist") => LibraryPart::Artists,
            _ => return,
        };
        self.library_reloads.insert(part, Instant::now() + RELOAD_DELAY);
    }

    pub(super) fn reload_library(&mut self) {
        let now = Instant::now();
        let due: Vec<LibraryPart> =
            self.library_reloads.iter().filter(|(_, at)| **at <= now).map(|(p, _)| *p).collect();
        for part in due {
            self.library_reloads.remove(&part);
            match part {
                LibraryPart::Playlists => self.loader.playlists(),
                LibraryPart::Albums => self.loader.albums(),
                LibraryPart::Artists => self.loader.artists(),
            }
        }
    }
}

/// A Home or artist page card as a library item; `artist` names the artist page it is on.
fn library_item(item: &HomeItem, artist: Option<&str>) -> Option<LibraryItem> {
    let id = item.uri.rsplit(':').next().unwrap_or_default().to_owned();
    let (uri, name, image_url) = (item.uri.clone(), item.title.clone(), item.image_url.clone());
    Some(match item.kind {
        ItemKind::Playlist => LibraryItem::Playlist(PlaylistInfo {
            id,
            uri,
            name,
            owner_id: item.owner_id.clone(),
            owner_name: item.owner_name.clone(),
            collaborative: false,
            total: 0,
            image_url,
        }),
        ItemKind::Album => LibraryItem::Album(AlbumInfo {
            id,
            uri,
            name,
            artists: vec![artist.unwrap_or(&item.subtitle).to_owned()],
            year: String::new(),
            total: 0,
            image_url,
        }),
        ItemKind::Artist => LibraryItem::Artist(ArtistInfo { uri, name, image_url }),
        ItemKind::Audiobook | ItemKind::Podcast | ItemKind::Episode => return None,
    })
}
