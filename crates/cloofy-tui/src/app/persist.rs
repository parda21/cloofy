//! Remembering the session: each service's open page and selections, the service shown and
//! the last track survive a restart.

use std::{
    collections::HashMap,
    io,
    time::{Duration, Instant},
};

use cloofy_core::catalog::{AlbumInfo, PlaylistInfo};
use tachyonfx::EffectManager;

use super::{App, Focus, LIBRARY_PARTS, LibraryFilter};
use crate::{
    backend::Service,
    datacache,
    preferences::Setting,
    state::{self, LastTrack, Page, SavedView, Selection, State},
    views::{HomeView, ListCursor, TrackSource, TrackView, View},
};

/// A cached greeting older than this ("Good morning" in the evening) is left out.
const GREETING_FRESH: Duration = Duration::from_secs(60 * 60);

impl App {
    /// Shows the library, Home and open list the last run left for the service shown, until
    /// fresh ones arrive.
    pub(super) fn load_cache(&mut self) {
        let (cache, Some(service)) = (&self.paths.cache, self.loader.service()) else { return };
        self.cached_tracks = datacache::load(&datacache::tracks_path(cache, service)).map(|(tracks, _)| tracks);
        let Some((library, age)) = datacache::load::<datacache::Library>(&datacache::library_path(cache, service))
        else {
            return;
        };
        // Signed out since, the library is gone; Home is the same for everyone.
        if !self.signed_out {
            let uris = library.playlists.iter().map(|p| &p.uri);
            let uris = uris.chain(library.albums.iter().map(|a| &a.uri)).chain(library.artists.iter().map(|a| &a.uri));
            self.saved.extend(uris.map(|uri| (uri.clone(), true)));
            self.me = Some(library.user);
            self.playlists = library.playlists;
            self.albums = library.albums;
            self.artists = library.artists;
            self.shows = library.shows;
            self.liked_total = library.liked_total;
        }
        if let Some(mut home) = library.home {
            if age > GREETING_FRESH {
                home.greeting.clear();
            }
            if let View::Home(v) = &mut self.view {
                *v = HomeView::new(v.id, Some(home.clone()));
            }
            self.home_cache = Some(home);
        }
    }

    /// The list the last run left open, to show at once while it loads again.
    pub(super) fn cached_copy(&mut self, source: &TrackSource) -> Option<TrackView> {
        let cached = self.cached_tracks.take().filter(|cached| cached.key == source.key())?;
        if matches!(source, TrackSource::Liked) {
            self.saved.extend(cached.tracks.iter().map(|t| (t.uri.clone(), true)));
        }
        Some(TrackView { tracks: cached.tracks, total: Some(cached.total), ..TrackView::new(source.clone()) })
    }

    /// Drops cached data of the account logged in last time and loads this one's.
    pub(super) fn forget_library(&mut self) {
        self.edit_library(|app| {
            app.playlists.clear();
            app.albums.clear();
            app.artists.clear();
            app.shows.clear();
        });
        self.liked_total = None;
        self.saved.clear();
        self.home_cache = None;
        self.home_fetched_at = None;
        self.playlists_loading = true;
        self.library_loading = true;
        self.library_pending = LIBRARY_PARTS;
        self.loader.playlists();
        self.loader.library();
    }

    /// Writes the library and Home of the service shown for the next start; in the background
    /// unless quitting.
    pub(super) fn save_library(&mut self, background: bool) {
        self.data_dirty = false;
        if self.demo {
            return;
        }
        self.last_cache_save = Instant::now();
        let (Some(user), Some(service)) = (self.me.clone(), self.loader.service()) else { return };
        let library = datacache::Library {
            user,
            playlists: self.playlists.clone(),
            albums: self.albums.clone(),
            artists: self.artists.clone(),
            shows: self.shows.clone(),
            liked_total: self.liked_total,
            home: self.home_cache.clone(),
        };
        let path = datacache::library_path(&self.paths.cache, service);
        write_cache(background, move || datacache::save(&path, &library));
    }

    /// Writes the open track list for the next start, once it is complete and if it changed.
    pub(super) fn save_open_tracks(&mut self, background: bool) {
        let Some(service) = self.loader.service().filter(|_| !self.demo) else { return };
        let View::Tracks(v) = &self.view else { return };
        let Some(fetched_at) = v.fetched_at.filter(|_| v.incoming.is_none()) else { return };
        if self.cached_tracks_version == Some((v.id, fetched_at)) {
            return;
        }
        self.cached_tracks_version = Some((v.id, fetched_at));
        let tracks = datacache::Tracks {
            key: v.source.key().to_owned(),
            total: v.total.unwrap_or(v.tracks.len() as u32),
            tracks: v.tracks.clone(),
        };
        let path = datacache::tracks_path(&self.paths.cache, service);
        write_cache(background, move || datacache::save(&path, &tracks));
    }

    /// Reopens what was open last time: the service shown then, as far as it started, with
    /// its page, and the track to resume. Called once, right after startup.
    pub(super) fn restore(&mut self, state: State) {
        // Crossfade used to be kept here; the last one chosen moves to the config file.
        if let Some(crossfade) = state.crossfade.filter(|c| *c != self.prefs.crossfade) {
            self.prefs.crossfade = crossfade.min(12);
            self.save_setting(Setting::Crossfade);
        }
        self.restored_volume = state.volume;
        // The player starts at this volume too, so the slider can show it right away.
        if let Some(volume) = state.volume {
            self.now.volume = volume;
        }

        // Files from before YouTube Music hold Spotify's page only; without a file, the first
        // service in use shows.
        let saved = match state.service.as_deref() {
            Some(key) => Service::from_key(key),
            None if state.page == Page::default() && state.last_track.is_none() => None,
            None => Some(Service::Spotify),
        };
        let mut pages: HashMap<Service, Page> =
            state.pages.into_iter().filter_map(|(key, page)| Some((Service::from_key(&key)?, page))).collect();
        pages.extend(saved.map(|service| (service, state.page)));
        let services = self.services();
        let working: Vec<Service> =
            services.iter().copied().filter(|service| self.loader.problem(*service).is_none()).collect();
        let shown = saved
            .filter(|service| working.contains(service))
            .or(working.first().copied())
            .or(services.first().copied());
        self.saved_pages = pages;
        self.loader.set_service(shown);
        self.park_others();
        self.open_browsing();

        // Only a track that can play again is offered.
        let last =
            state.last_track.filter(|last| Service::of(&last.uri).is_some_and(|service| working.contains(&service)));
        if let Some(last) = last {
            self.now.restore(&last);
            if let Some(url) = &last.cover_url {
                self.load_cover(url.clone());
            }
            self.last_context = last.context.clone();
            // Its player is the one that plays, once it is up.
            self.active = Service::of(&last.uri);
            self.notice(format!("Resume “{}” · press space", last.name));
            self.resume = Some(last);
            self.after_track_change(None);
        }
        // A service that can't start, while another one shows: say how to set it up.
        let down = services.iter().find(|service| Some(**service) != shown && !working.contains(service));
        if let Some(down) = down.filter(|_| shown.is_some_and(|shown| working.contains(&shown))) {
            self.notice(format!("{} isn't set up yet · run `cloofy login {}` in a terminal", down.name(), down.key()));
        }
        // Coming back should look like you never left: no entrance animations.
        self.effects = EffectManager::default();
    }

    /// Why `service` couldn't start, said with its name.
    pub(super) fn problem_of(&self, service: Service) -> Option<String> {
        self.loader.problem(service).map(|problem| format!("{}: {problem}", service.name()))
    }

    /// Reopens a service's page from the last run.
    pub(super) fn restore_page(&mut self, page: Page) {
        self.library_filter = page.library_filter;
        self.nav.selected = page.sidebar_selected;
        let selection = page.selection;
        match page.view {
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
            SavedView::Playlist { id, uri, name, owner_id, owner_name, collaborative, image_url } => {
                self.open_tracks(TrackSource::Playlist(PlaylistInfo {
                    id,
                    uri,
                    name,
                    owner_id,
                    owner_name,
                    collaborative,
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
        self.focus = if page.sidebar_focused { Focus::Nav } else { Focus::Content };
    }

    pub(super) fn snapshot(&self) -> State {
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
        let shown = self.loader.service();
        // A service that couldn't start keeps the page it had.
        let page = match shown.filter(|_| self.problem.is_some()).and_then(|s| self.saved_pages.get(&s)) {
            Some(page) => page.clone(),
            None => page(&self.view, &self.history, self.focus, &self.nav, self.library_filter),
        };
        State {
            page,
            service: shown.map(|service| service.key().to_owned()),
            pages: self.parked_pages().into_iter().map(|(service, page)| (service.key().to_owned(), page)).collect(),
            last_track,
            volume,
            crossfade: None,
        }
    }

    pub fn save_state(&mut self) {
        self.last_save = Instant::now();
        if self.demo {
            return;
        }
        if let Err(err) = state::save(&self.state_path, &self.snapshot()) {
            tracing::warn!("saving state: {err}");
        }
    }
}

fn write_cache(background: bool, save: impl FnOnce() -> io::Result<()> + Send + 'static) {
    let save = move || {
        if let Err(err) = save() {
            tracing::warn!("saving the data cache: {err}");
        }
    };
    if background {
        tokio::task::spawn_blocking(save);
    } else {
        save();
    }
}

/// Where a service's pages are: the page open, or the one the queue or Settings were opened
/// from, which are detours, and the sidebar.
pub(super) fn page(view: &View, history: &[View], focus: Focus, nav: &ListCursor, filter: LibraryFilter) -> Page {
    let page = match view {
        View::Queue(_) | View::Settings(_) => {
            history.iter().rev().find(|view| !matches!(view, View::Queue(_) | View::Settings(_)))
        }
        view => Some(view),
    };
    let (view, selection) = match page {
        None | Some(View::Queue(_) | View::Settings(_)) => (SavedView::Home, Selection::default()),
        Some(View::Home(v)) => (SavedView::Home, Selection { shelf: v.shelf, item: v.item, ..Selection::default() }),
        Some(View::Library(v)) => {
            (SavedView::Library { filter: v.filter }, Selection { index: v.cursor, ..Selection::default() })
        }
        Some(View::Tracks(v)) => {
            let view = match &v.source {
                TrackSource::Liked => SavedView::Liked,
                TrackSource::Playlist(p) => SavedView::Playlist {
                    id: p.id.clone(),
                    uri: p.uri.clone(),
                    name: p.name.clone(),
                    owner_id: p.owner_id.clone(),
                    owner_name: p.owner_name.clone(),
                    collaborative: p.collaborative,
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
        Some(View::Artist(v)) => (
            SavedView::Artist { uri: v.uri.clone(), name: v.name.clone() },
            Selection { index: v.row, item: v.item, ..Selection::default() },
        ),
        Some(View::Search(v)) => (SavedView::Search { query: v.query.clone() }, Selection::default()),
    };
    Page {
        view,
        selection,
        sidebar_focused: focus == Focus::Nav,
        sidebar_selected: nav.selected,
        library_filter: filter,
    }
}
