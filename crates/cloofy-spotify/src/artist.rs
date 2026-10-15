//! Artist pages: popular tracks, discography and related artists in one query.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::{
    catalog::Track,
    home::{HomeItem, ItemKind, Shelf},
    pathfinder::Pathfinder,
};

const IMAGE_WIDTH: u64 = 300;

#[derive(Debug, Clone)]
pub struct ArtistPage {
    pub uri: String,
    pub name: String,
    pub image_url: Option<String>,
    pub monthly_listeners: Option<u64>,
    pub followers: Option<u64>,
    /// Whether the user follows the artist.
    pub saved: bool,
    pub top_tracks: Vec<TopTrack>,
    /// Discography, related artists and appearances, shown like Home shelves.
    pub shelves: Vec<Shelf>,
}

#[derive(Debug, Clone)]
pub struct TopTrack {
    pub track: Track,
    pub playcount: Option<u64>,
}

pub async fn artist(pathfinder: &Pathfinder, uri: &str) -> Result<ArtistPage> {
    let variables = json!({ "uri": uri, "locale": "", "includePrerelease": true });
    let data = pathfinder.query("queryArtistOverview", variables).await?;
    parse(&data)
}

fn parse(data: &Value) -> Result<ArtistPage> {
    let artist = data.get("artistUnion").context("Artist: unexpected response")?;
    if artist["__typename"] != "Artist" {
        bail!("Artist not found ({})", artist["__typename"]);
    }
    let discography = &artist["discography"];
    let related = &artist["relatedContent"];
    let shelf = |title: &str, items: Vec<HomeItem>| Shelf {
        uri: String::new(),
        title: title.to_owned(),
        shortcuts: false,
        items,
    };
    let shelves = [
        shelf("Popular releases", releases(&discography["popularReleasesAlbums"], false)),
        shelf("Albums", releases(&discography["albums"], true)),
        shelf("Singles and EPs", releases(&discography["singles"], true)),
        shelf("Compilations", releases(&discography["compilations"], true)),
        shelf("Fans also like", artists(&related["relatedArtists"])),
        shelf("Appears On", releases(&related["appearsOn"], true)),
    ]
    .into_iter()
    .filter(|shelf| !shelf.items.is_empty())
    .collect();

    Ok(ArtistPage {
        uri: text(artist, "/uri"),
        name: text(artist, "/profile/name"),
        image_url: image(artist.pointer("/visuals/avatarImage/sources")),
        monthly_listeners: artist.pointer("/stats/monthlyListeners").and_then(Value::as_u64),
        followers: artist.pointer("/stats/followers").and_then(Value::as_u64),
        saved: artist["saved"].as_bool().unwrap_or(false),
        top_tracks: list(discography.pointer("/topTracks/items"))
            .filter_map(|item| top_track(&item["track"]))
            .collect(),
        shelves,
    })
}

fn top_track(track: &Value) -> Option<TopTrack> {
    let uri = text(track, "/uri");
    if uri.is_empty() {
        return None;
    }
    Some(TopTrack {
        track: Track {
            uri,
            name: text(track, "/name"),
            artists: names(track.pointer("/artists/items")),
            artist_uris: list(track.pointer("/artists/items"))
                .filter_map(|a| a["uri"].as_str().map(str::to_owned))
                .collect(),
            album: text(track, "/albumOfTrack/name"),
            album_uri: text(track, "/albumOfTrack/uri"),
            duration_ms: track.pointer("/duration/totalMilliseconds").and_then(Value::as_u64).unwrap_or(0) as u32,
        },
        playcount: track["playcount"].as_str().and_then(|n| n.parse().ok()).or(track["playcount"].as_u64()),
    })
}

/// Albums and singles; grouped sections wrap each release in `releases.items[0]`.
fn releases(section: &Value, grouped: bool) -> Vec<HomeItem> {
    list(section.get("items"))
        .filter_map(|item| if grouped { item.pointer("/releases/items/0") } else { Some(item) })
        .filter_map(|release| {
            let uri = text(release, "/uri");
            let name = text(release, "/name");
            if uri.is_empty() || name.is_empty() {
                return None;
            }
            let kind = match release["type"].as_str().unwrap_or_default() {
                "SINGLE" => "Single",
                "EP" => "EP",
                "COMPILATION" => "Compilation",
                _ => "Album",
            };
            let year = release.pointer("/date/year").and_then(Value::as_u64);
            let artists = names(release.pointer("/artists/items"));
            let subtitle = match (year, artists.is_empty()) {
                (Some(year), true) => format!("{year} · {kind}"),
                (Some(year), false) => format!("{year} · {}", artists.join(", ")),
                (None, _) => kind.to_owned(),
            };
            Some(HomeItem {
                context_uri: uri.clone(),
                uri,
                kind: ItemKind::Album,
                title: name,
                subtitle,
                image_url: image(release.pointer("/coverArt/sources")),
                owner_id: String::new(),
                owner_name: String::new(),
            })
        })
        .collect()
}

fn artists(section: &Value) -> Vec<HomeItem> {
    list(section.get("items"))
        .filter_map(|artist| {
            let uri = text(artist, "/uri");
            let name = text(artist, "/profile/name");
            (!uri.is_empty() && !name.is_empty()).then(|| HomeItem {
                context_uri: uri.clone(),
                uri,
                kind: ItemKind::Artist,
                title: name,
                subtitle: "Artist".into(),
                image_url: image(artist.pointer("/visuals/avatarImage/sources")),
                owner_id: String::new(),
                owner_name: String::new(),
            })
        })
        .collect()
}

fn list(value: Option<&Value>) -> impl Iterator<Item = &Value> {
    value.and_then(Value::as_array).into_iter().flatten()
}

fn names(list: Option<&Value>) -> Vec<String> {
    self::list(list)
        .filter_map(|entry| entry.pointer("/profile/name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn image(sources: Option<&Value>) -> Option<String> {
    list(sources)
        .min_by_key(|s| s["width"].as_u64().unwrap_or(IMAGE_WIDTH).abs_diff(IMAGE_WIDTH))
        .and_then(|s| s["url"].as_str())
        .map(str::to_owned)
}

fn text(value: &Value, pointer: &str) -> String {
    value.pointer(pointer).and_then(Value::as_str).unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_artist_overview() {
        let data = json!({ "artistUnion": {
            "__typename": "Artist",
            "uri": "spotify:artist:1",
            "profile": { "name": "Daft Punk" },
            "stats": { "monthlyListeners": 28_000_000, "followers": 11_000_000 },
            "saved": true,
            "visuals": { "avatarImage": { "sources": [{ "url": "avatar", "width": 320 }] } },
            "discography": {
                "topTracks": { "items": [{ "track": {
                    "uri": "spotify:track:1", "name": "One More Time", "playcount": "986916378",
                    "duration": { "totalMilliseconds": 320357 },
                    "artists": { "items": [{ "profile": { "name": "Daft Punk" } }] },
                    "albumOfTrack": { "uri": "spotify:album:1" }
                } }] },
                "albums": { "items": [{ "releases": { "items": [{
                    "uri": "spotify:album:1", "name": "Discovery", "type": "ALBUM", "date": { "year": 2001 }
                }] } }] }
            },
            "relatedContent": {}
        }});
        let page = parse(&data).unwrap();
        assert_eq!(page.name, "Daft Punk");
        assert_eq!(page.top_tracks[0].playcount, Some(986_916_378));
        assert_eq!(page.top_tracks[0].track.duration_ms, 320_357);
        assert_eq!(page.shelves.len(), 1);
        assert_eq!(page.shelves[0].items[0].subtitle, "2001 · Album");
    }
}
