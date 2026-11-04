//! Each service's own pages: who is signed in, the library, Home and the pages opened, with
//! their history. The service shown keeps them in [`App`]'s fields, where everything else
//! looks; the others' wait in a [`Browsing`] each, and a switch trades them, so each service
//! comes back as it was left, at once.

use std::{collections::HashMap, time::Instant};

use cloofy_core::{
    catalog::{AlbumInfo, ArtistInfo, PlaylistInfo, ShowInfo, User},
    home::{Home, ItemKind},
};

use super::{App, Focus, LIBRARY_PARTS, LibraryFilter};
use crate::{
    backend::Service,
    datacache,
    state::Page,
    views::{HomeView, ListCursor, TrackSource, View},
};

/// A service's pages while another one shows. Mirrors the fields of [`App`] that
/// [`App::swap_browsing`] trades.
pub(super) struct Browsing {
    me: Option<User>,
    playlists: Vec<PlaylistInfo>,
    playlists_loading: bool,
    albums: Vec<AlbumInfo>,
    artists: Vec<ArtistInfo>,
    shows: Vec<ShowInfo>,
    liked_total: Option<u32>,
    library_loading: bool,
    library_pending: u8,
    library_filter: LibraryFilter,
    signed_out: bool,
    problem: Option<String>,
    shown_before: bool,
    home_cache: Option<Home>,
    home_fetched_at: Option<Instant>,
    data_dirty: bool,
    cached_tracks: Option<datacache::Tracks>,
    cached_tracks_version: Option<(u64, Instant)>,
    focus: Focus,
    nav: ListCursor,
    view: View,
    history: Vec<View>,
}

impl Browsing {
    /// Pages not shown yet: an empty Home under view `id`, filled on the first switch to them.
    pub(super) fn new(id: u64) -> Self {
        Self {
            me: None,
            playlists: Vec::new(),
            playlists_loading: true,
            albums: Vec::new(),
            artists: Vec::new(),
            shows: Vec::new(),
            liked_total: None,
            library_loading: true,
            library_pending: LIBRARY_PARTS,
            library_filter: LibraryFilter::All,
            signed_out: false,
            problem: None,
            shown_before: false,
            home_cache: None,
            home_fetched_at: None,
            data_dirty: false,
            cached_tracks: None,
            cached_tracks_version: None,
            focus: Focus::Content,
            nav: ListCursor::default(),
            view: View::Home(HomeView::new(id, None)),
            history: Vec::new(),
        }
    }

    /// The views it holds, the open one first.
    pub(super) fn views_mut(&mut self) -> impl Iterator<Item = &mut View> {
        std::iter::once(&mut self.view).chain(self.history.iter_mut())
    }

    /// Where its pages were left, for the next start.
    fn page(&self) -> Page {
        super::persist::page(&self.view, &self.history, self.focus, &self.nav, self.library_filter)
    }
}

/// What a service's pages hold, shown or not, to name what plays from them.
pub(super) struct Known<'a> {
    playlists: &'a [PlaylistInfo],
    albums: &'a [AlbumInfo],
    artists: &'a [ArtistInfo],
    home: Option<&'a Home>,
    views: Vec<&'a View>,
}

impl<'a> Known<'a> {
    /// What kind of item `uri` is, and its name: from the library, Home, or a page opened.
    pub(super) fn name(&self, uri: &str) -> Option<(&'static str, &'a str)> {
        if let Some(p) = self.playlists.iter().find(|p| p.uri == uri) {
            return Some(("Playlist", &p.name));
        }
        if let Some(a) = self.albums.iter().find(|a| a.uri == uri) {
            return Some(("Album", &a.name));
        }
        if let Some(a) = self.artists.iter().find(|a| a.uri == uri) {
            return Some(("Artist", &a.name));
        }
        // Home's cards, and those of the artist pages opened.
        let artists = self.views.iter().filter_map(|view| match *view {
            View::Artist(v) => v.page.as_ref(),
            _ => None,
        });
        let shelves = self.home.iter().flat_map(|home| &home.shelves).chain(artists.flat_map(|page| &page.shelves));
        if let Some(item) = shelves.flat_map(|shelf| &shelf.items).find(|item| item.uri == uri) {
            let kind = match item.kind {
                ItemKind::Album => "Album",
                ItemKind::Artist => "Artist",
                _ => "Playlist",
            };
            return Some((kind, &item.title));
        }
        // A page it was played from, such as an album found by searching.
        self.views.iter().find_map(|view| match *view {
            View::Tracks(v) if v.source.key() == uri => match &v.source {
                TrackSource::Album(a) => Some(("Album", a.name.as_str())),
                TrackSource::Playlist(p) => Some(("Playlist", p.name.as_str())),
                TrackSource::Liked => None,
            },
            View::Artist(v) if v.uri == uri => Some(("Artist", v.name.as_str())),
            _ => None,
        })
    }
}

impl App {
    /// What `service`'s pages hold; the shown service's for a URI of none.
    pub(super) fn known(&self, service: Option<Service>) -> Known<'_> {
        match service.and_then(|service| self.parked.get(&service)) {
            Some(parked) => Known {
                playlists: &parked.playlists,
                albums: &parked.albums,
                artists: &parked.artists,
                home: parked.home_cache.as_ref(),
                views: std::iter::once(&parked.view).chain(&parked.history).collect(),
            },
            None => Known {
                playlists: &self.playlists,
                albums: &self.albums,
                artists: &self.artists,
                home: self.home_cache.as_ref(),
                views: std::iter::once(&self.view).chain(&self.history).collect(),
            },
        }
    }

    /// The services to switch between; just one, or none, means there is no switching.
    pub fn services(&self) -> Vec<Service> {
        self.loader.services()
    }

    /// The service whose Home, library and pages show.
    pub fn shown_service(&self) -> Option<Service> {
        self.loader.service()
    }

    /// Trades the shown service's pages for `other`'s.
    fn swap_browsing(&mut self, other: &mut Browsing) {
        use std::mem::swap;
        swap(&mut self.me, &mut other.me);
        swap(&mut self.playlists, &mut other.playlists);
        swap(&mut self.playlists_loading, &mut other.playlists_loading);
        swap(&mut self.albums, &mut other.albums);
        swap(&mut self.artists, &mut other.artists);
        swap(&mut self.shows, &mut other.shows);
        swap(&mut self.liked_total, &mut other.liked_total);
        swap(&mut self.library_loading, &mut other.library_loading);
        swap(&mut self.library_pending, &mut other.library_pending);
        swap(&mut self.library_filter, &mut other.library_filter);
        swap(&mut self.signed_out, &mut other.signed_out);
        swap(&mut self.problem, &mut other.problem);
        swap(&mut self.shown_before, &mut other.shown_before);
        swap(&mut self.home_cache, &mut other.home_cache);
        swap(&mut self.home_fetched_at, &mut other.home_fetched_at);
        swap(&mut self.data_dirty, &mut other.data_dirty);
        swap(&mut self.cached_tracks, &mut other.cached_tracks);
        swap(&mut self.cached_tracks_version, &mut other.cached_tracks_version);
        swap(&mut self.focus, &mut other.focus);
        swap(&mut self.nav, &mut other.nav);
        swap(&mut self.view, &mut other.view);
        swap(&mut self.history, &mut other.history);
    }

    /// Runs `f` on `service`'s pages, shown or not, as what arrives for a service may arrive
    /// after a switch away from it. Nothing runs for a service not in use.
    pub(super) fn with_service(&mut self, service: Service, f: impl FnOnce(&mut Self)) {
        let shown = self.loader.service();
        if shown == Some(service) {
            return f(self);
        }
        let Some(mut parked) = self.parked.remove(&service) else { return };
        self.swap_browsing(&mut parked);
        self.loader.set_service(Some(service));
        // Menus and pickers belong to the pages on screen.
        let overlay = self.overlay.take();
        f(self);
        self.overlay = overlay;
        self.loader.set_service(shown);
        self.swap_browsing(&mut parked);
        self.parked.insert(service, parked);
    }

    /// Makes room for the pages of every service in use besides the one shown.
    pub(super) fn park_others(&mut self) {
        for service in self.services() {
            if Some(service) != self.loader.service() && !self.parked.contains_key(&service) {
                let id = self.next_id();
                self.parked.insert(service, Browsing::new(id));
            }
        }
    }

    /// Shows `service`'s pages as they were left, filling them the first time.
    pub(super) fn show_service(&mut self, service: Service) {
        let Some(shown) = self.loader.service().filter(|shown| *shown != service) else { return };
        let Some(mut parked) = self.parked.remove(&service) else { return };
        if self.data_dirty {
            self.save_library(true);
        }
        self.save_open_tracks(true);
        self.swap_browsing(&mut parked);
        self.parked.insert(shown, parked);
        self.loader.set_service(Some(service));
        self.zen = false;
        if !self.shown_before {
            self.open_browsing();
        }
        // Until something plays, the player of the service shown is the one that would.
        if self.now.track.is_none() && self.players.contains_key(&service) {
            self.take_over(service);
        }
        self.check_queue();
        self.last_nav = Some(Instant::now());
        self.animate_content();
    }

    /// Shows the next service in use, as the switch key does.
    pub(super) fn switch_service(&mut self) {
        if let Some(next) = self.next_service() {
            self.show_service(next);
        }
    }

    /// Shows the pages of the service `uri` belongs to, for a page of it to open there.
    pub(super) fn show_service_of(&mut self, uri: &str) {
        if let Some(service) = Service::of(uri) {
            self.show_service(service);
        }
    }

    /// The next service in use after the one shown, which the switch shows.
    pub fn next_service(&self) -> Option<Service> {
        let services = self.services();
        let index = services.iter().position(|s| Some(*s) == self.loader.service())?;
        services.get((index + 1) % services.len()).copied().filter(|next| Some(*next) != self.loader.service())
    }

    /// Fills the shown service's pages the first time they show: with why it couldn't start,
    /// or with what the last run left, while fresh data loads.
    pub(super) fn open_browsing(&mut self) {
        self.shown_before = true;
        let Some(service) = self.loader.service() else { return };
        let page = self.saved_pages.remove(&service);
        if let Some(problem) = self.loader.problem(service).map(str::to_owned) {
            self.playlists_loading = false;
            self.library_loading = false;
            self.library_pending = 0;
            if let View::Home(v) = &mut self.view {
                v.loading = false;
                v.error = Some(problem.clone());
            }
            self.problem = Some(problem);
            // Kept for when it can start again.
            self.saved_pages.extend(page.map(|page| (service, page)));
            return;
        }
        self.signed_out = !self.loader.signed_in();
        if self.signed_out {
            self.playlists_loading = false;
            self.library_loading = false;
            self.library_pending = 0;
        } else {
            self.loader.me();
            self.loader.playlists();
            self.loader.library();
        }
        if let View::Home(v) = &self.view {
            self.loader.home(v.id);
        }
        if !self.demo {
            self.load_cache();
        }
        // A page of another service's, from a file that mixed them up, would switch over.
        if let Some(page) = page.filter(|page| page.view.uri().is_none_or(|uri| Service::of(uri) == Some(service))) {
            self.restore_page(page);
        }
        self.cached_tracks = None;
    }

    /// Where every service's pages were left; ones not shown this time stay as they were.
    pub(super) fn parked_pages(&self) -> HashMap<Service, Page> {
        let mut pages: HashMap<Service, Page> =
            self.saved_pages.iter().map(|(service, page)| (*service, page.clone())).collect();
        pages.extend(
            self.parked
                .iter()
                .filter(|(_, parked)| parked.shown_before)
                .map(|(service, parked)| (*service, parked.page())),
        );
        if let Some(shown) = self.loader.service() {
            pages.remove(&shown);
        }
        pages
    }

    /// Writes what every service's pages hold for the next start, when quitting.
    pub(super) fn save_parked(&mut self) {
        let services: Vec<Service> = self.parked.iter().filter(|(_, p)| p.data_dirty).map(|(s, _)| *s).collect();
        for service in services {
            self.with_service(service, |app| app.save_library(false));
        }
    }

    /// Whether `service` couldn't start, so its pages only say why.
    pub fn failed_to_start(&self, service: Service) -> bool {
        self.loader.problem(service).is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use cloofy_core::{
        paths::Paths,
        player::{PlayerEvent, QueueTrack},
    };
    use tokio::sync::mpsc;

    use super::*;
    use crate::{
        app::Start,
        commands::Command,
        loader::Loader,
        preferences::Preferences,
        queue::Queue,
        state::{SavedView, State},
    };

    const BOTH: [Service; 2] = [Service::Spotify, Service::YouTubeMusic];

    /// An app with `services` in use, none of them connected, after a last session `state`.
    fn app(name: &str, services: &[Service], problems: &[(Service, &str)], state: State) -> App {
        let root = std::env::temp_dir().join(format!("cloofy-browsing-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths =
            Paths { config: root.join("config"), data: root.join("data"), cache: root.join("cache"), keychain: false };
        let (tx, _) = mpsc::unbounded_channel();
        let loader = Loader::for_tests(services, problems, tx);
        App::new(loader, Preferences::default(), Start::Restore(Box::new(state)), paths, None, None)
    }

    fn page(view: SavedView) -> Page {
        Page { view, ..Page::default() }
    }

    #[tokio::test]
    async fn each_service_keeps_its_own_pages() {
        let mut app = app("switch", &BOTH, &[], State::default());
        assert_eq!(app.shown_service(), Some(Service::Spotify));
        app.run(Command::Library);
        app.run(Command::SwitchService);
        assert_eq!(app.shown_service(), Some(Service::YouTubeMusic));
        assert!(matches!(app.view, View::Home(_)) && !app.can_go_back(), "YouTube Music starts at its Home");
        app.run(Command::Search);
        app.run(Command::SwitchService);
        assert_eq!(app.shown_service(), Some(Service::Spotify));
        assert!(matches!(app.view, View::Library(_)) && app.can_go_back(), "Spotify is where it was left");

        let state = app.snapshot();
        assert_eq!(state.service.as_deref(), Some("spotify"));
        assert!(matches!(state.page.view, SavedView::Library { .. }));
        assert!(matches!(state.pages["ytmusic"].view, SavedView::Search { .. }));
    }

    #[tokio::test]
    async fn the_last_session_comes_back_for_every_service() {
        let state = State {
            page: page(SavedView::Search { query: "daft punk".into() }),
            service: Some("ytmusic".into()),
            pages: BTreeMap::from([("spotify".into(), page(SavedView::Library { filter: LibraryFilter::Albums }))]),
            ..State::default()
        };
        let mut app = app("restore", &BOTH, &[], state);
        assert_eq!(app.shown_service(), Some(Service::YouTubeMusic));
        assert!(matches!(&app.view, View::Search(v) if v.query == "daft punk"));
        // Spotify's page waits until it shows.
        assert_eq!(app.snapshot().pages["spotify"].view, SavedView::Library { filter: LibraryFilter::Albums });
        app.run(Command::SwitchService);
        assert!(matches!(&app.view, View::Library(v) if v.filter == LibraryFilter::Albums));
    }

    #[tokio::test]
    async fn a_service_that_cant_start_leaves_the_other_to_use() {
        // A file from before YouTube Music, with Spotify's page.
        let artist = SavedView::Artist { uri: "spotify:artist:1".into(), name: "Daft Punk".into() };
        let state = State { page: page(artist.clone()), ..State::default() };
        let problem = "Not logged in yet – run `cloofy login` first.";
        let mut app = app("problem", &BOTH, &[(Service::Spotify, problem)], state);
        assert_eq!(app.shown_service(), Some(Service::YouTubeMusic));
        let status = app.status.as_ref().map(|status| status.text.as_str()).unwrap_or_default();
        assert!(status.contains("Spotify isn't set up yet") && status.contains("cloofy login spotify"), "{status}");

        app.run(Command::SwitchService);
        assert!(matches!(&app.view, View::Home(v) if v.error.as_deref() == Some(problem)));
        assert!(app.library_entries().is_empty());
        assert!(app.library_note().is_some_and(|note| note.contains(problem)));
        // Its page stays for when it can start again.
        assert_eq!(app.snapshot().page.view, artist);
    }

    #[tokio::test]
    async fn the_queue_is_the_one_of_the_service_that_plays() {
        let mut app = app("queue", &BOTH, &[], State::default());
        let report = |context: &str, current: &str, next: &str| PlayerEvent::QueueChanged {
            context_uri: context.into(),
            current: Some(current.into()),
            next: vec![QueueTrack { uri: next.into(), provider: "context".into() }],
        };
        app.take_over(Service::YouTubeMusic);
        app.on_player(Service::YouTubeMusic, report("ytmusic:album:a", "ytmusic:track:3", "ytmusic:track:4"));
        assert!(app.queue.knows("ytmusic:track:3"));
        // Spotify starts playing, and says what plays next there.
        app.on_player(Service::Spotify, PlayerEvent::Playing { position_ms: 0 });
        assert_eq!(app.queue, Queue::default(), "YouTube Music's queue goes");
        app.on_player(Service::Spotify, report("spotify:album:b", "spotify:track:1", "spotify:track:2"));
        assert!(app.queue.knows("spotify:track:1"));
        // The same album plays on YouTube Music again: Spotify's queue isn't shown for it.
        app.take_over(Service::YouTubeMusic);
        assert_eq!(app.queue, Queue::default());
    }

    #[tokio::test]
    async fn a_single_service_has_nothing_to_switch_to() {
        let mut app = app("single", &[Service::YouTubeMusic], &[], State::default());
        assert!(!app.offers(Command::SwitchService));
        app.run(Command::SwitchService);
        assert_eq!(app.shown_service(), Some(Service::YouTubeMusic));
        assert!(app.library_note().is_some_and(|note| note.contains("cloofy login ytmusic")), "signed out");
    }
}
