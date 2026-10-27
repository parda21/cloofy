//! The Home feed. music.youtube.com sends a few shelves first and more as the page scrolls;
//! cloofy asks for a few more pages straight away so Home has as many shelves as Spotify's.

use anyhow::{Context, Result};
use cloofy_core::home::{Home, Shelf};
use serde_json::Value;

use crate::{
    innertube::InnerTube,
    parse::{self, find, list, text},
};

/// Home asks for more pages until it has this many shelves…
const MIN_SHELVES: usize = 10;
/// …but no more pages than this.
const MAX_PAGES: usize = 5;

pub(crate) async fn home(api: &InnerTube) -> Result<Home> {
    let first = api.browse("FEmusic_home").await?;
    let sections = find(&first, "sectionListRenderer").context("Home: unexpected response")?;
    let mut shelves = parse_shelves(sections);
    let mut token = parse::continuation(sections);
    let mut pages = 1;
    while let Some(next) = token.take().filter(|_| shelves.len() < MIN_SHELVES && pages < MAX_PAGES) {
        pages += 1;
        let page = match api.continuation("browse", &next).await {
            Ok(page) => page,
            // What arrived so far is still a Home.
            Err(err) => {
                tracing::warn!("more of Home: {err:#}");
                break;
            }
        };
        let Some(sections) = page.pointer("/continuationContents/sectionListContinuation") else { break };
        shelves.extend(parse_shelves(sections));
        token = parse::continuation(sections);
    }
    Ok(Home { greeting: String::new(), shelves })
}

/// The shelves of a section list; ones with nothing cloofy can show are left out.
pub(crate) fn parse_shelves(sections: &Value) -> Vec<Shelf> {
    list(&sections["contents"]).iter().filter_map(shelf).filter(|shelf| !shelf.items.is_empty()).collect()
}

fn shelf(section: &Value) -> Option<Shelf> {
    let carousel =
        section.get("musicCarouselShelfRenderer").or_else(|| section.get("musicImmersiveCarouselShelfRenderer"))?;
    let header = carousel["header"]
        .get("musicCarouselShelfBasicHeaderRenderer")
        .or_else(|| carousel["header"].get("musicImmersiveCarouselShelfBasicHeaderRenderer"))?;
    let title = text(&header["title"]);
    if title.is_empty() {
        return None;
    }
    let items = list(&carousel["contents"]).iter().filter_map(parse::home_item).collect();
    Some(Shelf { uri: format!("ytmusic:shelf:{title}"), title, shortcuts: false, items })
}

#[cfg(test)]
mod tests {
    use cloofy_core::home::ItemKind;

    use super::*;

    #[test]
    fn parses_the_shelves_of_home() {
        let page: Value = serde_json::from_str(include_str!("../tests/fixtures/home.json")).unwrap();
        let sections = find(&page, "sectionListRenderer").unwrap();
        let shelves = parse_shelves(sections);
        assert_eq!(shelves[0].title, "Quick picks");
        let song = &shelves[0].items[0];
        assert_eq!(song.kind, ItemKind::Playlist, "a song opens as its radio");
        assert_eq!(song.uri, "ytmusic:playlist:RDAMVMxukbqwRuN5w");
        assert_eq!(song.context_uri, "ytmusic:track:xukbqwRuN5w");
        assert_eq!(song.title, "Choosin' Texas");
        assert_eq!(song.subtitle, "Song • Ella Langley");
        assert!(song.image_url.as_deref().is_some_and(|url| url.contains("=w300-h300")), "{:?}", song.image_url);

        let playlists = &shelves[1];
        assert_eq!(playlists.title, "100% Hits");
        let first = &playlists.items[0];
        assert_eq!(first.kind, ItemKind::Playlist);
        assert!(first.uri.starts_with("ytmusic:playlist:RDCLAK5uy_"), "{}", first.uri);
        assert_eq!(first.context_uri, first.uri);
        assert!(parse::continuation(sections).is_some());
    }

    #[test]
    fn continuation_pages_add_shelves() {
        let page: Value = serde_json::from_str(include_str!("../tests/fixtures/home_continuation.json")).unwrap();
        let sections = page.pointer("/continuationContents/sectionListContinuation").unwrap();
        let shelves = parse_shelves(sections);
        assert!(!shelves.is_empty());
        let kinds: Vec<_> = shelves.iter().flat_map(|s| s.items.iter().map(|i| i.kind)).collect();
        assert!(kinds.iter().all(|k| matches!(k, ItemKind::Playlist | ItemKind::Album | ItemKind::Artist)));
    }

    #[test]
    fn unknown_shelves_are_skipped() {
        let sections = serde_json::json!({ "contents": [
            { "musicTastebuilderShelfRenderer": {} },
            { "somethingNewRenderer": { "contents": [1, 2, 3] } },
            { "musicCarouselShelfRenderer": { "header": {}, "contents": [] } },
        ] });
        assert!(parse_shelves(&sections).is_empty());
    }
}
