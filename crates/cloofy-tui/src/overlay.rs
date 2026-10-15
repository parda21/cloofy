use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nucleo_matcher::{
    Config, Matcher,
    pattern::{CaseMatching, Normalization, Pattern},
};

use crate::{commands::Command, views::ListCursor};

pub enum Overlay {
    Help,
    Palette(Picker<PaletteAction>),
    AddToPlaylist { uris: Vec<String>, picker: Picker<usize> },
    Devices { devices: Vec<cloofy_spotify::connect::Device>, picker: Picker<usize> },
    Menu(Menu),
}

/// The right-click menu of one item.
pub struct Menu {
    pub target: ContextTarget,
    /// Screen position the menu opens at.
    pub anchor: (u16, u16),
    pub items: Vec<(MenuAction, String)>,
    pub cursor: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Open,
    Play,
    Shuffle,
    Queue,
    Save,
    AddToPlaylist,
    GoToAlbum,
    GoToArtist,
    CopyLink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Track,
    Album,
    Playlist,
    Artist,
    Show,
    Episode,
}

/// Something a menu or button can act on, with what it needs to open or play it.
#[derive(Debug, Clone)]
pub struct ContextTarget {
    pub uri: String,
    pub name: String,
    pub kind: TargetKind,
    /// The context a track plays in: its playlist, album or Liked Songs.
    pub context: Option<String>,
    /// Album URI and name.
    pub album: Option<(String, String)>,
    /// First artist's URI and name.
    pub artist: Option<(String, String)>,
    pub image_url: Option<String>,
    /// Playlists the user owns (or Liked Songs) can't be followed.
    pub owned: bool,
    pub liked_songs: bool,
}

impl Overlay {
    pub fn picker_title(&self) -> Option<&'static str> {
        match self {
            Overlay::Help => None,
            Overlay::Palette(_) => Some("Command or playlist…"),
            Overlay::AddToPlaylist { .. } => Some("Add to which playlist?"),
            Overlay::Devices { .. } => Some("Transfer playback to…"),
            Overlay::Menu(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PaletteAction {
    Run(Command),
    OpenHome,
    OpenLiked,
    OpenPlaylist(usize),
}

pub struct PickItem<T> {
    pub label: String,
    pub detail: String,
    /// Shown as a thumbnail; pickers with images get two-line rows.
    pub image_url: Option<String>,
    pub value: T,
}

pub enum PickerOutcome<T> {
    Stay,
    Close,
    Select(T),
}

impl<T> PickerOutcome<T> {
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> PickerOutcome<U> {
        match self {
            PickerOutcome::Stay => PickerOutcome::Stay,
            PickerOutcome::Close => PickerOutcome::Close,
            PickerOutcome::Select(value) => PickerOutcome::Select(f(value)),
        }
    }
}

/// A filterable list: type to fuzzy-match, arrows to move, enter to pick.
pub struct Picker<T> {
    pub query: String,
    pub items: Vec<PickItem<T>>,
    /// Indices into `items`, best match first.
    pub matches: Vec<usize>,
    pub cursor: ListCursor,
    matcher: Matcher,
}

struct Candidate<'a> {
    index: usize,
    text: &'a str,
}

impl AsRef<str> for Candidate<'_> {
    fn as_ref(&self) -> &str {
        self.text
    }
}

impl<T: Copy> Picker<T> {
    pub fn new(items: Vec<PickItem<T>>) -> Self {
        let matches = (0..items.len()).collect();
        Self {
            query: String::new(),
            items,
            matches,
            cursor: ListCursor::default(),
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> PickerOutcome<T> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return PickerOutcome::Close,
            KeyCode::Enter => return self.select(self.cursor.selected),
            KeyCode::Up => self.cursor.move_by(-1, self.matches.len()),
            KeyCode::Down | KeyCode::Tab => self.cursor.move_by(1, self.matches.len()),
            KeyCode::Char('p') if ctrl => self.cursor.move_by(-1, self.matches.len()),
            KeyCode::Char('n') if ctrl => self.cursor.move_by(1, self.matches.len()),
            KeyCode::Char('u' | 'w') if ctrl => self.set_query(String::new()),
            KeyCode::Char(c) if !ctrl => {
                let mut query = std::mem::take(&mut self.query);
                query.push(c);
                self.set_query(query);
            }
            KeyCode::Backspace => {
                let mut query = std::mem::take(&mut self.query);
                query.pop();
                self.set_query(query);
            }
            _ => {}
        }
        PickerOutcome::Stay
    }

    /// Picks the match at `position` (as shown, best match first).
    pub fn select(&self, position: usize) -> PickerOutcome<T> {
        match self.matches.get(position) {
            Some(&index) => PickerOutcome::Select(self.items[index].value),
            None => PickerOutcome::Stay,
        }
    }

    fn set_query(&mut self, query: String) {
        self.query = query;
        let candidates = self.items.iter().enumerate().map(|(index, item)| Candidate { index, text: &item.label });
        let pattern = Pattern::parse(&self.query, CaseMatching::Ignore, Normalization::Smart);
        self.matches = pattern
            .match_list(candidates, &mut self.matcher)
            .into_iter()
            .map(|(candidate, _)| candidate.index)
            .collect();
        self.cursor = ListCursor::default();
    }
}
