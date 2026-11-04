//! Context menus: what can be done with the selected item, reachable by right-click or `.`.

use std::io::Write;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use cloofy_core::{
    catalog::{AlbumInfo, PlaylistInfo, Track},
    home::{HomeItem, ItemKind},
    player::CurrentTrack,
};

use super::{App, Focus, LibraryEntry, NavItem};
use crate::{
    backend::{self, Player, Service},
    icons::icons,
    overlay::{ContextTarget, Menu, MenuAction, Overlay, TargetKind},
    queue::Provider,
    views::{ArtistSelection, QueueRow, SearchEntry, TrackSource, View},
};

impl App {
    /// Opens the menu for the current selection at `anchor`.
    pub(super) fn open_menu(&mut self, anchor: (u16, u16)) {
        let Some(target) = self.context_target() else { return };
        self.show_menu(target, anchor);
    }

    /// Opens the menu for the playing track at `anchor`, as a right-click on the player does.
    pub(super) fn open_playing_menu(&mut self, anchor: (u16, u16)) {
        let Some(track) = &self.now.track else { return };
        let target = current_track_target(track, self.playing_context().map(str::to_owned));
        self.show_menu(target, anchor);
    }

    fn show_menu(&mut self, target: ContextTarget, anchor: (u16, u16)) {
        let items = menu_items(&target, self.is_saved(&target.uri));
        self.overlay = Some(Overlay::Menu(Menu { target, anchor, items, cursor: 0 }));
        self.overlay_fx_pending = true;
    }

    /// The selected item in the focused pane.
    pub(super) fn context_target(&self) -> Option<ContextTarget> {
        if self.focus == Focus::Nav {
            let NavItem::Library(entry) = *self.nav_items().get(self.nav.selected)? else { return None };
            return self.library_target(entry);
        }
        match &self.view {
            View::Library(v) => {
                let entry = *self.library_entries_for(v.filter).get(v.cursor)?;
                self.library_target(entry)
            }
            View::Tracks(v) => {
                let context = match &v.source {
                    TrackSource::Liked => self.liked_uri(),
                    TrackSource::Playlist(p) => Some(p.uri.clone()),
                    TrackSource::Album(a) => Some(a.uri.clone()),
                };
                v.tracks.get(v.list.selected).map(|t| track_target(t, context))
            }
            View::Search(v) => match v.selected()? {
                SearchEntry::Track(t) => Some(track_target(t, Some(t.album_uri.clone()).filter(|uri| !uri.is_empty()))),
                SearchEntry::Album(a) => Some(album_target(a)),
                SearchEntry::Playlist(p) => Some(self.playlist_target(p)),
                SearchEntry::Artist(a) => Some(target(&a.uri, &a.name, TargetKind::Artist, a.image_url.clone())),
                SearchEntry::Header(_) => None,
            },
            View::Home(v) => v.selected().map(|item| self.home_target(item)),
            View::Artist(v) => match v.selected()? {
                ArtistSelection::Track(t) => Some(track_target(&t.track, Some(v.uri.clone()))),
                ArtistSelection::Item(item) => Some(self.home_target(item)),
            },
            View::Settings(_) => None,
            View::Queue(v) => match v.selected()? {
                QueueRow::Playing(_) => {
                    let track = self.now.track.as_ref()?;
                    Some(current_track_target(track, self.playing_context().map(str::to_owned)))
                }
                QueueRow::Next { track, entry } => {
                    Some(ContextTarget { upcoming: Some(entry.clone()), ..track_target(track, self.queue_context()) })
                }
                QueueRow::Header(_) => None,
            },
        }
    }

    /// The playlist, album or artist a page shows, for its header buttons.
    pub(super) fn page_target(&self) -> Option<ContextTarget> {
        match &self.view {
            View::Tracks(v) => Some(match &v.source {
                TrackSource::Liked => ContextTarget {
                    liked_songs: true,
                    owned: true,
                    ..target(&self.liked_uri()?, "Liked Songs", TargetKind::Playlist, None)
                },
                TrackSource::Playlist(p) => self.playlist_target(p),
                TrackSource::Album(a) => album_target(a),
            }),
            View::Artist(v) => Some(target(&v.uri, &v.name, TargetKind::Artist, None)),
            _ => None,
        }
    }

    pub(super) fn library_target(&self, entry: LibraryEntry) -> Option<ContextTarget> {
        Some(match entry {
            LibraryEntry::Liked => ContextTarget {
                liked_songs: true,
                owned: true,
                ..target(&self.liked_uri()?, "Liked Songs", TargetKind::Playlist, None)
            },
            LibraryEntry::Playlist(i) => self.playlist_target(self.playlists.get(i)?),
            LibraryEntry::Album(i) => album_target(self.albums.get(i)?),
            LibraryEntry::Artist(i) => {
                let a = self.artists.get(i)?;
                target(&a.uri, &a.name, TargetKind::Artist, a.image_url.clone())
            }
            LibraryEntry::Show(i) => {
                let s = self.shows.get(i)?;
                target(&s.uri, &s.name, TargetKind::Show, s.image_url.clone())
            }
        })
    }

    fn playlist_target(&self, playlist: &PlaylistInfo) -> ContextTarget {
        ContextTarget {
            owned: self.owns(playlist),
            ..target(&playlist.uri, &playlist.name, TargetKind::Playlist, playlist.image_url.clone())
        }
    }

    fn home_target(&self, item: &HomeItem) -> ContextTarget {
        let kind = match item.kind {
            ItemKind::Playlist => TargetKind::Playlist,
            ItemKind::Album => TargetKind::Album,
            ItemKind::Artist => TargetKind::Artist,
            ItemKind::Episode => TargetKind::Episode,
            ItemKind::Audiobook | ItemKind::Podcast => TargetKind::Show,
        };
        let owned = self.me.as_ref().is_some_and(|me| me.id == item.owner_id);
        ContextTarget {
            context: (item.kind == ItemKind::Episode).then(|| item.context_uri.clone()),
            owned,
            ..target(&item.uri, &item.title, kind, item.image_url.clone())
        }
    }

    /// Liked Songs of the service shown as a playable context.
    pub(super) fn liked_uri(&self) -> Option<String> {
        let service = self.loader.service()?;
        let user = self.players.get(&service).and_then(Player::user).or_else(|| Some(self.me.as_ref()?.id.clone()));
        backend::liked_context(service, user.as_deref())
    }

    pub(super) fn run_menu(&mut self, action: MenuAction, target: ContextTarget) {
        // A song of the queue page plays from where it is in the queue.
        if action == MenuAction::Play
            && let Some(entry) = target.upcoming
        {
            return self.skip_to_entry(entry);
        }
        match action {
            MenuAction::Open => self.open_target(target),
            MenuAction::Play => match target.kind {
                TargetKind::Track | TargetKind::Episode => {
                    let context = target.context.or(target.album.map(|(uri, _)| uri));
                    match context {
                        Some(context) => self.play(context, Some(target.uri)),
                        None => self.play(target.uri, None),
                    }
                }
                _ => self.play(target.uri, None),
            },
            MenuAction::Shuffle => self.play_shuffled(target.uri),
            MenuAction::Queue => self.queue(target.uri, target.name),
            MenuAction::RemoveFromQueue => {
                if let Some(entry) = target.upcoming {
                    self.remove_from_queue(target.name, entry);
                }
            }
            MenuAction::Save => self.toggle_library(target.uri, target.name),
            MenuAction::AddToPlaylist => self.open_playlist_picker_for(target.uri),
            MenuAction::GoToAlbum => {
                if let Some((uri, name)) = target.album {
                    let artists = target.artist.map(|(_, name)| vec![name]).unwrap_or_default();
                    self.open_album(uri, name, artists, target.image_url);
                }
            }
            MenuAction::GoToArtist => {
                if let Some((uri, name)) = target.artist {
                    self.open_artist(uri, name);
                }
            }
            MenuAction::CopyLink => {
                copy_to_clipboard(&backend::web_link(&target.uri));
                self.notice("Link copied".into());
            }
        }
    }

    /// The Spotify URI behind a library entry.
    pub fn entry_uri(&self, entry: LibraryEntry) -> Option<String> {
        self.library_target(entry).map(|target| target.uri)
    }

    /// A short name for a playing context, like “Playlist · Deep Focus”, from the pages of the
    /// service it belongs to, shown or not.
    pub fn context_label(&self, uri: &str) -> Option<String> {
        if Some(uri) == self.liked_uri().as_deref() || backend::is_liked_context(uri) {
            return Some("Liked Songs".into());
        }
        if backend::is_dj(uri) {
            return Some("DJ".into());
        }
        let (kind, name) = self.known(Service::of(uri)).name(uri)?;
        (!name.is_empty()).then(|| format!("{kind} · {name}"))
    }

    /// Opens the page of a playlist, album or artist known only by its URI.
    pub(super) fn open_context(&mut self, uri: String) {
        if backend::is_liked_context(&uri) {
            return self.open_library_entry(LibraryEntry::Liked);
        }
        let kind = match uri.split(':').nth(1) {
            Some("album") => TargetKind::Album,
            Some("artist") => TargetKind::Artist,
            Some("playlist") => TargetKind::Playlist,
            _ => return,
        };
        if let Some(p) = self.playlists.iter().find(|p| p.uri == uri).cloned() {
            return self.open_tracks(TrackSource::Playlist(p));
        }
        let name = self
            .context_label(&uri)
            .and_then(|label| label.split_once(" · ").map(|(_, name)| name.to_owned()))
            .unwrap_or_default();
        self.open_target(target(&uri, &name, kind, None));
    }

    fn open_target(&mut self, target: ContextTarget) {
        match target.kind {
            TargetKind::Playlist if target.liked_songs => self.open_library_entry(LibraryEntry::Liked),
            TargetKind::Playlist => {
                let id = target.uri.rsplit(':').next().unwrap_or_default().to_owned();
                let owner_id = if target.owned {
                    self.me.as_ref().map(|m| m.id.clone()).unwrap_or_default()
                } else {
                    String::new()
                };
                self.open_tracks(TrackSource::Playlist(PlaylistInfo {
                    id,
                    uri: target.uri,
                    name: target.name,
                    owner_id,
                    owner_name: String::new(),
                    collaborative: false,
                    total: 0,
                    image_url: target.image_url,
                }));
            }
            TargetKind::Album => self.open_album(target.uri, target.name, Vec::new(), target.image_url),
            TargetKind::Artist => self.open_artist(target.uri, target.name),
            TargetKind::Track | TargetKind::Show | TargetKind::Episode => {
                self.run_menu(MenuAction::Play, target);
            }
        }
    }

    pub(super) fn open_album(&mut self, uri: String, name: String, artists: Vec<String>, image_url: Option<String>) {
        let id = uri.rsplit(':').next().unwrap_or_default().to_owned();
        self.open_tracks(TrackSource::Album(AlbumInfo {
            id,
            uri,
            name,
            artists,
            year: String::new(),
            total: 0,
            image_url,
        }));
    }
}

fn target(uri: &str, name: &str, kind: TargetKind, image_url: Option<String>) -> ContextTarget {
    ContextTarget {
        uri: uri.to_owned(),
        name: name.to_owned(),
        kind,
        context: None,
        album: None,
        artist: None,
        image_url,
        owned: false,
        liked_songs: false,
        upcoming: None,
    }
}

fn track_target(track: &Track, context: Option<String>) -> ContextTarget {
    ContextTarget {
        context,
        album: (!track.album_uri.is_empty()).then(|| (track.album_uri.clone(), track.album.clone())),
        artist: first_artist(&track.artist_uris, &track.artists),
        ..target(&track.uri, &track.name, TargetKind::Track, None)
    }
}

/// The playing track or episode; `context` is what it plays from.
fn current_track_target(track: &CurrentTrack, context: Option<String>) -> ContextTarget {
    let kind = if backend::is_episode(&track.uri) { TargetKind::Episode } else { TargetKind::Track };
    ContextTarget {
        context,
        album: track.album_uri.clone().map(|uri| (uri, track.album.clone())),
        artist: first_artist(&track.artist_uris, &track.artists),
        ..target(&track.uri, &track.name, kind, track.cover_url.clone())
    }
}

/// The first artist's URI and name, where Spotify named one.
fn first_artist(uris: &[String], names: &[String]) -> Option<(String, String)> {
    let uri = uris.first().filter(|uri| !uri.is_empty())?;
    Some((uri.clone(), names.first().cloned().unwrap_or_default()))
}

fn album_target(album: &AlbumInfo) -> ContextTarget {
    target(&album.uri, &album.name, TargetKind::Album, album.image_url.clone())
}

/// The menu behind a page's "more" button: everything except opening the page itself.
pub(super) fn page_menu_items(target: &ContextTarget, saved: bool) -> Vec<(MenuAction, String)> {
    menu_items(target, saved).into_iter().filter(|(action, _)| *action != MenuAction::Open).collect()
}

/// The actions offered for a target, in menu order.
fn menu_items(target: &ContextTarget, saved: bool) -> Vec<(MenuAction, String)> {
    let i = icons();
    let mut items = Vec::new();
    let mut add = |action, icon: &str, label: &str| items.push((action, format!("{icon}  {label}")));
    match target.kind {
        TargetKind::Track => {
            add(MenuAction::Play, i.play, "Play");
            add(MenuAction::Queue, i.queue, "Add to queue");
            if target.upcoming.as_ref().is_some_and(|entry| entry.provider == Provider::Queue) {
                add(MenuAction::RemoveFromQueue, i.remove, "Remove from queue");
            }
            if saved {
                add(MenuAction::Save, i.saved, "Remove from your Liked Songs");
            } else {
                add(MenuAction::Save, i.save, "Save to your Liked Songs");
            }
            add(MenuAction::AddToPlaylist, i.playlist, "Add to playlist…");
            if target.album.is_some() {
                add(MenuAction::GoToAlbum, i.album, "Go to album");
            }
            if target.artist.is_some() {
                add(MenuAction::GoToArtist, i.artist, "Go to artist");
            }
        }
        TargetKind::Album | TargetKind::Playlist => {
            add(MenuAction::Open, i.open, "Open");
            add(MenuAction::Play, i.play, "Play");
            add(MenuAction::Shuffle, i.shuffle, "Shuffle");
            if !target.owned {
                if saved {
                    add(MenuAction::Save, i.saved, "Remove from Your Library");
                } else {
                    add(MenuAction::Save, i.save, "Add to Your Library");
                }
            }
        }
        TargetKind::Artist => {
            add(MenuAction::Open, i.open, "Open");
            add(MenuAction::Play, i.play, "Play");
            if saved {
                add(MenuAction::Save, i.saved, "Unfollow");
            } else {
                add(MenuAction::Save, i.save, "Follow");
            }
        }
        TargetKind::Show | TargetKind::Episode => add(MenuAction::Play, i.play, "Play"),
    }
    if !target.liked_songs {
        add(MenuAction::CopyLink, i.link, "Copy link");
    }
    items
}

/// OSC 52: the terminal puts the text on the clipboard, which also works over SSH.
fn copy_to_clipboard(text: &str) {
    let mut stdout = std::io::stdout();
    let _ = write!(stdout, "\x1b]52;c;{}\x07", STANDARD.encode(text));
    let _ = stdout.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_menu_offers_library_and_navigation() {
        let track = Track {
            uri: "spotify:track:1".into(),
            name: "One More Time".into(),
            artists: vec!["Daft Punk".into()],
            artist_uris: vec!["spotify:artist:1".into()],
            album: "Discovery".into(),
            album_uri: "spotify:album:1".into(),
            duration_ms: 1,
            image_url: None,
        };
        let actions: Vec<_> = menu_items(&track_target(&track, None), true).into_iter().map(|(a, _)| a).collect();
        assert!(actions.contains(&MenuAction::GoToArtist) && actions.contains(&MenuAction::GoToAlbum));
        let labels = menu_items(&track_target(&track, None), true);
        assert!(labels.iter().any(|(_, l)| l.contains("Remove")));
        assert!(!actions.contains(&MenuAction::RemoveFromQueue));
    }

    #[test]
    fn only_queued_songs_can_be_removed_from_the_queue() {
        use crate::queue::Upcoming;
        let track = Track {
            uri: "spotify:track:1".into(),
            name: "Digital Love".into(),
            artists: Vec::new(),
            artist_uris: Vec::new(),
            album: String::new(),
            album_uri: String::new(),
            duration_ms: 1,
            image_url: None,
        };
        let actions = |provider| {
            let entry = Upcoming { uri: track.uri.clone(), uid: "q0".into(), provider };
            let target = ContextTarget { upcoming: Some(entry), ..track_target(&track, None) };
            menu_items(&target, false).into_iter().map(|(action, _)| action).collect::<Vec<_>>()
        };
        assert!(actions(Provider::Queue).contains(&MenuAction::RemoveFromQueue));
        assert!(!actions(Provider::Context).contains(&MenuAction::RemoveFromQueue));
    }
}
