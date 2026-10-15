//! Remembering the session: the open view, selections and the last track survive a restart.

use std::time::Instant;

use cloofy_spotify::catalog::{AlbumInfo, PlaylistInfo};
use tachyonfx::EffectManager;

use super::{App, COVER_SIZE, Focus};
use crate::{
    state::{self, LastTrack, SavedView, Selection, State},
    views::{TrackSource, View},
};

impl App {
    /// Reopens what was open last time. Called once, right after startup.
    pub(super) fn restore(&mut self, state: State) {
        self.library_filter = state.library_filter;
        self.nav.selected = state.sidebar_selected;
        if let Some(crossfade) = state.crossfade {
            self.crossfade = crossfade;
        }
        self.restored_volume = state.volume;
        // Playback starts at this volume too, see `playback::spawn`.
        if let Some(volume) = state.volume {
            self.now.volume = volume;
        }

        let selection = state.selection;
        match state.view {
            SavedView::Home => {
                if let View::Home(v) = &mut self.view {
                    v.shelf = selection.shelf;
                    v.item = selection.item;
                }
            }
            SavedView::Library { filter } => {
                self.open_library_view();
                if let View::Library(v) = &mut self.view {
                    v.filter = filter;
                    v.cursor = selection.index;
                }
            }
            SavedView::Liked => self.open_tracks(TrackSource::Liked),
            SavedView::Playlist { id, uri, name, owner_id, owner_name, image_url } => {
                self.open_tracks(TrackSource::Playlist(PlaylistInfo {
                    id,
                    uri,
                    name,
                    owner_id,
                    owner_name,
                    collaborative: false,
                    total: 0,
                    image_url,
                }));
            }
            SavedView::Album { id, uri, name, artists, image_url } => {
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
            SavedView::Artist { uri, name } => self.open_artist(uri, name),
            SavedView::Search { query } => {
                self.open_search();
                if let View::Search(v) = &mut self.view {
                    v.query = query;
                    v.typed_at = Some(Instant::now());
                }
            }
        }
        match &mut self.view {
            View::Tracks(v) => v.list.selected = selection.index,
            View::Artist(v) => {
                v.row = selection.index;
                v.item = selection.item;
            }
            _ => {}
        }
        self.focus = if state.sidebar_focused { Focus::Nav } else { Focus::Content };

        if let Some(last) = state.last_track {
            self.now.restore(&last);
            if let Some(url) = &last.cover_url {
                self.loader.cover(url.clone(), COVER_SIZE);
            }
            self.last_context = last.context.clone();
            self.notice(format!("Resume “{}” · press space", last.name));
            self.resume = Some(last);
            self.after_track_change(None);
        }
        // Coming back should look like you never left: no entrance animations.
        self.effects = EffectManager::default();
    }

    pub(super) fn snapshot(&self) -> State {
        let (view, selection) = match &self.view {
            View::Home(v) => (SavedView::Home, Selection { shelf: v.shelf, item: v.item, ..Selection::default() }),
            View::Library(v) => {
                (SavedView::Library { filter: v.filter }, Selection { index: v.cursor, ..Selection::default() })
            }
            View::Tracks(v) => {
                let view = match &v.source {
                    TrackSource::Liked => SavedView::Liked,
                    TrackSource::Playlist(p) => SavedView::Playlist {
                        id: p.id.clone(),
                        uri: p.uri.clone(),
                        name: p.name.clone(),
                        owner_id: p.owner_id.clone(),
                        owner_name: p.owner_name.clone(),
                        image_url: p.image_url.clone(),
                    },
                    TrackSource::Album(a) => SavedView::Album {
                        id: a.id.clone(),
                        uri: a.uri.clone(),
                        name: a.name.clone(),
                        artists: a.artists.clone(),
                        image_url: a.image_url.clone(),
                    },
                };
                (view, Selection { index: v.list.selected, ..Selection::default() })
            }
            View::Artist(v) => (
                SavedView::Artist { uri: v.uri.clone(), name: v.name.clone() },
                Selection { index: v.row, item: v.item, ..Selection::default() },
            ),
            View::Search(v) => (SavedView::Search { query: v.query.clone() }, Selection::default()),
        };
        let last_track = self.now.track.as_ref().map(|t| LastTrack {
            uri: t.uri.clone(),
            name: t.name.clone(),
            artists: t.artists.clone(),
            artist_uris: t.artist_uris.clone(),
            album: t.album.clone(),
            album_uri: t.album_uri.clone(),
            duration_ms: t.duration_ms,
            cover_url: t.cover_url.clone(),
            cover_url_large: t.cover_url_large.clone(),
            context: self.last_context.clone(),
            position_ms: self.now.position_ms(),
        });
        // While another device plays, the shown volume is that device's, not cloofy's.
        let volume = if self.now.device.is_none() { Some(self.now.volume) } else { self.restored_volume };
        State {
            view,
            selection,
            sidebar_focused: self.focus == Focus::Nav,
            sidebar_selected: self.nav.selected,
            library_filter: self.library_filter,
            last_track,
            volume,
            crossfade: Some(self.crossfade),
        }
    }

    pub fn save_state(&mut self) {
        self.last_save = Instant::now();
        if let Err(err) = state::save(&self.state_path, &self.snapshot()) {
            tracing::warn!("saving state: {err}");
        }
    }
}
