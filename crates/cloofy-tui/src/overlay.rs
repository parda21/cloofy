use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nucleo_matcher::{
    Config, Matcher,
    pattern::{CaseMatching, Normalization, Pattern},
};

use crate::{commands::Command, views::ListCursor};

pub enum Overlay {
    Help,
    Palette(Picker<PaletteAction>),
    AddToPlaylist {
        uris: Vec<String>,
        picker: Picker<usize>,
    },
    Devices {
        devices: Vec<cloofy_spotify::connect::Device>,
        picker: Picker<usize>,
        refreshing: bool,
    },
    Menu(Menu),
    /// Asks DJ for something or jumps to different picks, like the DJ button in Spotify's apps.
    Dj {
        picker: Picker<DjChoice>,
        suggestions: Vec<String>,
        loading: bool,
    },
}

/// A row of the DJ panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DjChoice {
    /// Asks for what was typed.
    Ask,
    /// Jumps to different picks.
    Jump,
    /// Asks for one of Spotify's suggestions.
    Suggestion(usize),
}

/// The DJ panel's rows: what was typed, different picks, then Spotify's suggestions.
pub fn dj_items(query: &str, suggestions: &[String]) -> Vec<PickItem<DjChoice>> {
    let item = |label: String, detail: &str, value| PickItem { label, detail: detail.into(), image_url: None, value };
    let query = query.trim();
    let ask = (!query.is_empty()).then(|| item(format!("Ask for “{query}”"), "enter", DjChoice::Ask));
    ask.into_iter()
        .chain([item("Pick something for me".into(), "new picks", DjChoice::Jump)])
        .chain(suggestions.iter().enumerate().map(|(index, s)| item(s.clone(), "idea", DjChoice::Suggestion(index))))
        .collect()
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
            Overlay::Dj { .. } => Some("What should your DJ play?"),
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
    /// Whether typing narrows the list; otherwise the text is free and every item stays.
    filter: bool,
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
            filter: true,
        }
    }

    /// A picker whose text is free, like a request, rather than a filter of the list.
    pub fn free_text(items: Vec<PickItem<T>>) -> Self {
        Self { filter: false, ..Self::new(items) }
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

    /// Swaps in fresh items, keeping the query and the selection on the item `selected` finds.
    pub fn set_items(&mut self, items: Vec<PickItem<T>>, selected: impl Fn(&T) -> bool) {
        self.items = items;
        let query = std::mem::take(&mut self.query);
        self.set_query(query);
        if let Some(position) = self.matches.iter().position(|&index| selected(&self.items[index].value)) {
            self.cursor.selected = position;
        }
    }

    /// The item under the cursor.
    pub fn selected(&self) -> Option<&T> {
        self.matches.get(self.cursor.selected).map(|&index| &self.items[index].value)
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
        self.cursor = ListCursor::default();
        if !self.filter {
            self.matches = (0..self.items.len()).collect();
            return;
        }
        let candidates = self.items.iter().enumerate().map(|(index, item)| Candidate { index, text: &item.label });
        let pattern = Pattern::parse(&self.query, CaseMatching::Ignore, Normalization::Smart);
        self.matches = pattern
            .match_list(candidates, &mut self.matcher)
            .into_iter()
            .map(|(candidate, _)| candidate.index)
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(labels: &[&str]) -> Vec<PickItem<usize>> {
        labels
            .iter()
            .enumerate()
            .map(|(value, label)| PickItem { label: (*label).into(), detail: String::new(), image_url: None, value })
            .collect()
    }

    #[test]
    fn fresh_items_keep_the_query_and_the_selected_item() {
        let mut picker = Picker::new(items(&["Phone", "Kitchen speaker", "Living room speaker"]));
        for c in "speaker".chars() {
            picker.on_key(KeyEvent::from(KeyCode::Char(c)));
        }
        picker.on_key(KeyEvent::from(KeyCode::Down));
        let kept = picker.items[*picker.selected().unwrap()].label.clone();
        // A new device shows up and the order changes.
        let fresh = ["Living room speaker", "TV", "Kitchen speaker", "Phone"];
        picker.set_items(items(&fresh), |&value| fresh[value] == kept);
        assert_eq!(picker.query, "speaker");
        assert_eq!(picker.matches.len(), 2);
        assert_eq!(fresh[*picker.selected().unwrap()], kept);
    }

    #[test]
    fn a_request_to_dj_is_free_text_offered_first() {
        let suggestions = vec!["Chicago House".to_owned()];
        let mut picker = Picker::free_text(dj_items("", &suggestions));
        assert_eq!(picker.selected(), Some(&DjChoice::Jump));
        for c in "jazz".chars() {
            picker.on_key(KeyEvent::from(KeyCode::Char(c)));
            let query = picker.query.clone();
            picker.set_items(dj_items(&query, &suggestions), |_| false);
        }
        // Nothing is filtered away, and enter asks for what was typed.
        assert_eq!(picker.matches.len(), 3);
        assert!(matches!(picker.on_key(KeyEvent::from(KeyCode::Enter)), PickerOutcome::Select(DjChoice::Ask)));
    }
}
