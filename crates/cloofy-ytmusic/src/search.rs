//! Search, one filtered search per kind at the same time: the unfiltered one has only a few
//! results of each kind.

use anyhow::Result;
use cloofy_core::catalog::{AlbumInfo, ArtistInfo, PlaylistInfo, SearchResults, Track};
use serde_json::{Value, json};

use crate::{
    innertube::InnerTube,
    parse::{self, ALBUM, ARTIST, Byline, IMAGE_SIZE, Link, PLAYLIST, find_all, list, runs, text},
};

/// The `params` of music.youtube.com's filter chips.
const SONGS: &str = "EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D";
const ALBUMS: &str = "EgWKAQIYAWoKEAkQBRAKEAMQBA%3D%3D";
const ARTISTS: &str = "EgWKAQIgAWoKEAkQBRAKEAMQBA%3D%3D";
const FEATURED_PLAYLISTS: &str = "EgeKAQQoADgBagwQDhAKEAMQBRAJEAQ%3D";
const COMMUNITY_PLAYLISTS: &str = "EgeKAQQoAEABagoQAxAEEAoQCRAF";

pub(crate) async fn search(api: &InnerTube, query: &str) -> Result<SearchResults> {
    let filtered = |params: &'static str| api.call("search", json!({ "query": query, "params": params }));
    let (songs, albums, artists, featured, community) = tokio::join!(
        filtered(SONGS),
        filtered(ALBUMS),
        filtered(ARTISTS),
        filtered(FEATURED_PLAYLISTS),
        filtered(COMMUNITY_PLAYLISTS),
    );
    let songs = songs?;
    // The other kinds are a bonus: a failure there still leaves the songs.
    let rows = |page: Result<Value>| match page {
        Ok(page) => rows(&page),
        Err(err) => {
            tracing::warn!("search: {err:#}");
            Vec::new()
        }
    };
    let mut playlists: Vec<PlaylistInfo> = rows(featured).iter().filter_map(playlist).collect();
    playlists.extend(rows(community).iter().filter_map(playlist));
    Ok(SearchResults {
        tracks: rows(Ok(songs)).iter().filter_map(track).collect(),
        albums: rows(albums).iter().filter_map(album).collect(),
        artists: rows(artists).iter().filter_map(artist).collect(),
        playlists,
    })
}

pub(crate) async fn suggestions(api: &InnerTube, query: &str) -> Result<Vec<String>> {
    let page = api.call("music/get_search_suggestions", json!({ "input": query })).await?;
    Ok(parse_suggestions(&page))
}

fn parse_suggestions(page: &Value) -> Vec<String> {
    let mut found = Vec::new();
    find_all(page, "searchSuggestionRenderer", &mut found);
    found.into_iter().map(|s| text(&s["suggestion"])).filter(|s| !s.is_empty()).collect()
}

/// The rows of the result shelves.
fn rows(page: &Value) -> Vec<Value> {
    let mut shelves = Vec::new();
    find_all(page, "musicShelfRenderer", &mut shelves);
    shelves
        .iter()
        .flat_map(|shelf| list(&shelf["contents"]))
        .filter_map(|item| item.get("musicResponsiveListItemRenderer").cloned())
        .collect()
}

fn track(row: &Value) -> Option<Track> {
    parse::list_item_track(row)
}

/// The title, where the row links to, and its byline.
fn row_parts(row: &Value) -> Option<(String, Link, Byline)> {
    let columns: Vec<&[Value]> = list(&row["flexColumns"])
        .iter()
        .map(|c| runs(&c["musicResponsiveListItemFlexColumnRenderer"]["text"]))
        .collect();
    let title: String = columns.first()?.iter().filter_map(|r| r["text"].as_str()).collect();
    let link = Link::of(row.get("navigationEndpoint")?)?;
    Some((title, link, Byline::parse(columns.into_iter().skip(1))))
}

fn album(row: &Value) -> Option<AlbumInfo> {
    let (name, Link::Album(id), byline) = row_parts(row)? else { return None };
    Some(AlbumInfo {
        uri: format!("{ALBUM}{id}"),
        id,
        name,
        artists: byline.artist_names(),
        year: byline.year.unwrap_or_default(),
        total: 0,
        image_url: parse::image(&row["thumbnail"], IMAGE_SIZE),
    })
}

fn artist(row: &Value) -> Option<ArtistInfo> {
    let (name, Link::Artist(id), _) = row_parts(row)? else { return None };
    Some(ArtistInfo { uri: format!("{ARTIST}{id}"), name, image_url: parse::image(&row["thumbnail"], IMAGE_SIZE) })
}

fn playlist(row: &Value) -> Option<PlaylistInfo> {
    let (name, Link::Playlist(id), byline) = row_parts(row)? else { return None };
    let (owner_id, owner_name) = match byline.artists.first() {
        Some((name, id)) => (id.clone().unwrap_or_default(), name.clone()),
        None => (String::new(), byline.other.first().cloned().unwrap_or_default()),
    };
    let total = byline.counts.iter().find(|c| c.ends_with("songs") || c.ends_with("tracks") || c.ends_with("song"));
    Some(PlaylistInfo {
        uri: format!("{PLAYLIST}{id}"),
        id,
        name,
        owner_id,
        owner_name,
        collaborative: false,
        total: total.and_then(|t| parse::leading_number(t)).unwrap_or(0),
        image_url: parse::image(&row["thumbnail"], IMAGE_SIZE),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn parses_songs() {
        let rows = rows(&fixture("search_songs"));
        let song = track(&rows[0]).unwrap();
        assert_eq!(song.uri, "ytmusic:track:d7woYjGWs6k");
        assert_eq!(song.name, "Da Funk");
        assert_eq!(song.artists, ["Daft Punk"]);
        assert_eq!(song.artist_uris, ["ytmusic:artist:UCRr1xG_2WIDs18a6cIiCxeA"]);
        assert_eq!((song.album.as_str(), song.album_uri.as_str()), ("Homework", "ytmusic:album:MPREb_dMHztPsrDgN"));
        assert_eq!(song.duration_ms, 329_000);
        assert!(song.image_url.is_some());
    }

    #[test]
    fn parses_albums_artists_and_playlists() {
        let albums: Vec<_> = rows(&fixture("search_albums")).iter().filter_map(album).collect();
        assert_eq!(albums[0].name, "Random Access Memories");
        assert_eq!(albums[0].artists, ["Daft Punk"]);
        assert_eq!(albums[0].year, "2013");
        assert!(albums[0].uri.starts_with("ytmusic:album:MPREb_"));

        let artists: Vec<_> = rows(&fixture("search_artists")).iter().filter_map(artist).collect();
        assert_eq!(artists[0].name, "Daft Punk");
        assert_eq!(artists[0].uri, "ytmusic:artist:UCRr1xG_2WIDs18a6cIiCxeA");

        let featured: Vec<_> = rows(&fixture("search_featured")).iter().filter_map(playlist).collect();
        assert_eq!(featured[0].name, "Presenting Daft Punk");
        assert_eq!(featured[0].owner_name, "YouTube Music");
        assert!(featured[0].total > 0);
        assert!(!featured[0].uri.contains(":VL"), "playlist URIs leave out the browse prefix");

        let community: Vec<_> = rows(&fixture("search_community")).iter().filter_map(playlist).collect();
        assert!(community[0].owner_id.starts_with("UC"));
    }

    #[test]
    fn parses_suggestions() {
        let suggestions = parse_suggestions(&fixture("suggestions"));
        assert!(suggestions.iter().any(|s| s.starts_with("daft punk")), "{suggestions:?}");
    }
}
