//! The Home feed: shelves of playlists, albums, artists and podcasts picked for the user.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Home {
    pub greeting: String,
    /// Moods and kinds of music that narrow Home down, shown above its shelves the way
    /// music.youtube.com shows them. Empty for services without.
    #[serde(default)]
    pub chips: Vec<Chip>,
    pub shelves: Vec<Shelf>,
}

impl Home {
    /// This Home without the shelves titled as one of `hidden`, whatever the case. The
    /// shortcuts at the top have no title to hide them by.
    pub fn without_shelves(mut self, hidden: &[String]) -> Self {
        let hidden: Vec<String> = hidden.iter().map(|title| title.trim().to_lowercase()).collect();
        self.shelves.retain(|shelf| shelf.shortcuts || !hidden.contains(&shelf.title.trim().to_lowercase()));
        self
    }

    /// The chip Home is narrowed to, if any.
    pub fn chip(&self) -> Option<&Chip> {
        self.chips.iter().find(|chip| chip.selected)
    }
}

/// A mood or kind of music Home can be narrowed to, e.g. "Relax".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chip {
    pub title: String,
    /// What the service needs to show Home for this chip.
    pub id: String,
    /// Whether this Home is the chip's.
    pub selected: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Shelf {
    pub uri: String,
    pub title: String,
    /// A line above the title, e.g. "THE BEST HITS FOR INSTANT MORNING ENERGY.".
    #[serde(default)]
    pub strapline: String,
    /// The compact grid of shortcuts at the top of Home.
    pub shortcuts: bool,
    /// Songs in columns of a few rows, like YouTube Music's Quick picks, instead of cards.
    #[serde(default)]
    pub songs: bool,
    /// What the shelf's Play all button plays, when it has one.
    #[serde(default)]
    pub play_all: Option<String>,
    pub items: Vec<HomeItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemKind {
    Playlist,
    Album,
    Artist,
    Audiobook,
    Podcast,
    Episode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeItem {
    pub uri: String,
    pub kind: ItemKind,
    pub title: String,
    pub subtitle: String,
    pub image_url: Option<String>,
    /// Playlist owner, to tell own playlists from the service's.
    pub owner_id: String,
    pub owner_name: String,
    /// What to play: the show for an episode, the song for a song's radio, the item itself
    /// otherwise.
    pub context_uri: String,
}

impl HomeItem {
    /// A song shown as its radio: it plays at once, as in the services' own apps, and opens as
    /// the radio.
    pub fn is_song(&self) -> bool {
        self.kind == ItemKind::Playlist && self.context_uri != self.uri
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_shelves_go_by_title_and_the_shortcuts_stay() {
        let shelf = |title: &str, shortcuts| Shelf { title: title.into(), shortcuts, ..Shelf::default() };
        let home = Home {
            shelves: vec![shelf("", true), shelf("Covers and remixes", false), shelf("Quick picks", false)],
            ..Home::default()
        };
        let shown = home.clone().without_shelves(&["covers and Remixes ".into(), String::new()]);
        let titles: Vec<&str> = shown.shelves.iter().map(|shelf| shelf.title.as_str()).collect();
        assert_eq!(titles, ["", "Quick picks"]);
        assert_eq!(home.without_shelves(&[]).shelves.len(), 3);
    }
}
