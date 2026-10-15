//! The personalized Home feed, as the official clients show it.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::pathfinder::{self, Pathfinder};

const SECTION_ITEMS: u32 = 20;
/// Preferred cover width; Spotify offers roughly 64, 300 and 640 px.
const IMAGE_WIDTH: u64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Home {
    pub greeting: String,
    pub shelves: Vec<Shelf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shelf {
    pub uri: String,
    pub title: String,
    /// The compact grid of shortcuts at the top of Home.
    pub shortcuts: bool,
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
    /// Playlist owner, to tell own playlists from Spotify's.
    pub owner_id: String,
    pub owner_name: String,
    /// What to play: the show for an episode, the item itself otherwise.
    pub context_uri: String,
}

pub async fn home(pathfinder: &Pathfinder) -> Result<Home> {
    let variables = json!({
        "timeZone": pathfinder::time_zone(),
        "sp_t": "",
        "facet": "",
        "sectionItemsLimit": SECTION_ITEMS,
        "homeEndUserIntegration": "INTEGRATION_WEB_PLAYER",
        "includeHomeChapterVideoCards": false,
        "includeEpisodeContentRatingsV2": false,
    });
    let data = pathfinder.query("home", variables).await?;
    parse(&data)
}

fn parse(data: &Value) -> Result<Home> {
    let home = data.get("home").context("Home: unexpected response")?;
    if home["__typename"] != "HomeResponsePayload" {
        anyhow::bail!("Spotify did not return Home ({})", home["__typename"]);
    }
    let shelves = home
        .pointer("/sectionContainer/sections/items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(shelf)
        .collect();
    Ok(Home { greeting: text(home, "/greeting/transformedLabel"), shelves })
}

fn shelf(section: &Value) -> Option<Shelf> {
    let items: Vec<HomeItem> = section
        .pointer("/sectionItems/items")?
        .as_array()?
        .iter()
        .filter_map(|item| item.pointer("/content/data"))
        .filter_map(item)
        .collect();
    if items.is_empty() {
        return None;
    }
    Some(Shelf {
        uri: text(section, "/uri"),
        title: text(section, "/data/title/transformedLabel"),
        shortcuts: section.pointer("/data/__typename").and_then(Value::as_str) == Some("HomeShortsSectionData"),
        items,
    })
}

fn item(data: &Value) -> Option<HomeItem> {
    let uri = text(data, "/uri");
    let mut item = HomeItem {
        context_uri: uri.clone(),
        uri,
        kind: ItemKind::Playlist,
        title: text(data, "/name"),
        subtitle: String::new(),
        image_url: None,
        owner_id: String::new(),
        owner_name: String::new(),
    };
    match data.get("__typename")?.as_str()? {
        "Playlist" => {
            item.owner_name = text(data, "/ownerV2/data/name");
            item.owner_id = text(data, "/ownerV2/data/uri").rsplit(':').next().unwrap_or_default().to_owned();
            let description = strip_html(&text(data, "/description"));
            item.subtitle = if description.is_empty() { format!("By {}", item.owner_name) } else { description };
            item.image_url = image(data.pointer("/images/items/0/sources"));
        }
        "Album" => {
            item.kind = ItemKind::Album;
            item.subtitle = names(data.pointer("/artists/items"), "/profile/name");
            item.image_url = image(data.pointer("/coverArt/sources"));
        }
        "Artist" => {
            item.kind = ItemKind::Artist;
            item.title = text(data, "/profile/name");
            item.subtitle = "Artist".into();
            item.image_url = image(data.pointer("/visuals/avatarImage/sources"));
        }
        "Audiobook" => {
            item.kind = ItemKind::Audiobook;
            item.subtitle = names(data.get("authorsV2"), "/name");
            item.image_url = image(data.pointer("/coverArt/sources"));
        }
        "Podcast" => {
            item.kind = ItemKind::Podcast;
            item.subtitle = text(data, "/publisher/name");
            item.image_url = image(data.pointer("/coverArt/sources"));
        }
        "Episode" => {
            item.kind = ItemKind::Episode;
            item.subtitle = text(data, "/podcastV2/data/name");
            item.context_uri = text(data, "/podcastV2/data/uri");
            item.image_url = image(data.pointer("/coverArt/sources"))
                .or_else(|| image(data.pointer("/podcastV2/data/coverArt/sources")));
        }
        _ => return None,
    }
    (!item.uri.is_empty() && !item.title.is_empty()).then_some(item)
}

/// The source closest to the preferred width.
fn image(sources: Option<&Value>) -> Option<String> {
    sources?
        .as_array()?
        .iter()
        .min_by_key(|s| s["width"].as_u64().unwrap_or(IMAGE_WIDTH).abs_diff(IMAGE_WIDTH))
        .and_then(|s| s["url"].as_str())
        .map(str::to_owned)
}

fn names(list: Option<&Value>, pointer: &str) -> String {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.pointer(pointer).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(", ")
}

fn text(value: &Value, pointer: &str) -> String {
    value.pointer(pointer).and_then(Value::as_str).unwrap_or_default().to_owned()
}

/// Playlist descriptions contain links like `<a href=…>Artist</a>`.
fn strip_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&#x27;", "'").replace("&quot;", "\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_links_from_descriptions() {
        assert_eq!(strip_html(r#"With <a href="spotify:artist:1">Daft Punk</a> &amp; more"#), "With Daft Punk & more");
    }

    #[test]
    fn parses_sections_and_skips_unknown_items() {
        let data = json!({ "home": {
            "__typename": "HomeResponsePayload",
            "greeting": { "transformedLabel": "Good morning" },
            "sectionContainer": { "sections": { "items": [{
                "uri": "spotify:section:1",
                "data": { "__typename": "HomeGenericSectionData", "title": { "transformedLabel": "Made For You" } },
                "sectionItems": { "items": [
                    { "content": { "data": { "__typename": "Album", "uri": "spotify:album:1", "name": "Discovery",
                        "artists": { "items": [{ "profile": { "name": "Daft Punk" } }] },
                        "coverArt": { "sources": [{ "url": "small", "width": 64 }, { "url": "medium", "width": 300 }] } } } },
                    { "content": { "data": { "__typename": "NotFound" } } }
                ] }
            }] } }
        }});
        let home = parse(&data).unwrap();
        assert_eq!(home.greeting, "Good morning");
        let shelf = &home.shelves[0];
        assert_eq!((shelf.title.as_str(), shelf.items.len()), ("Made For You", 1));
        assert_eq!(shelf.items[0].subtitle, "Daft Punk");
        assert_eq!(shelf.items[0].image_url.as_deref(), Some("medium"));
    }
}
