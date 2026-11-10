//! The Home feed. music.youtube.com sends a few shelves first and more as the page scrolls;
//! cloofy asks for a few more pages straight away so Home has as many shelves as Spotify's.
//! Above the shelves are chips for moods like "Relax", which show a Home of their own.

use anyhow::{Context, Result};
use cloofy_core::home::{Chip, Home, Shelf};
use serde_json::{Value, json};

use crate::{
    innertube::InnerTube,
    parse::{self, PLAYLIST, SONGS, TRACK, find, list, str_at, text},
};

/// Home asks for more pages until it has this many shelves…
const MIN_SHELVES: usize = 10;
/// …but no more pages than this.
const MAX_PAGES: usize = 5;
/// The chip of podcasts, which cloofy can't play on YouTube Music. Titles are English, since
/// cloofy asks for English.
const PODCASTS: &str = "Podcasts";

/// Home, or the Home of the chip with this ID.
pub(crate) async fn home(api: &InnerTube, chip: Option<&str>) -> Result<Home> {
    let mut body = json!({ "browseId": "FEmusic_home" });
    if let Some(chip) = chip {
        body["params"] = chip.into();
    }
    let first = api.call("browse", body).await?;
    let sections = find(&first, "sectionListRenderer").context("Home: unexpected response")?;
    let chips = parse_chips(sections, chip);
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
    Ok(Home { greeting: String::new(), chips, shelves })
}

/// The chips above Home's shelves; `asked` is the ID of the chip Home was asked for.
fn parse_chips(sections: &Value, asked: Option<&str>) -> Vec<Chip> {
    let chips = sections.pointer("/header/chipCloudRenderer/chips").unwrap_or(&Value::Null);
    list(chips)
        .iter()
        .filter_map(|chip| {
            let chip = chip.get("chipCloudChipRenderer")?;
            let title = text(&chip["text"]);
            let id = str_at(chip, "/navigationEndpoint/browseEndpoint/params")?.to_owned();
            let selected = chip["isSelected"] == true || asked == Some(id.as_str());
            (!title.is_empty() && title != PODCASTS).then_some(Chip { title, id, selected })
        })
        .collect()
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
    let items: Vec<_> = list(&carousel["contents"]).iter().filter_map(parse::home_item).collect();
    // Quick picks and the like: songs in columns, rather than cards.
    let songs = carousel.get("numItemsPerColumn").is_some() && items.iter().all(|item| item.is_song());
    let play = header.pointer("/moreContentButton/buttonRenderer/navigationEndpoint/watchEndpoint");
    let play_all = play.and_then(|play| match str_at(play, "/playlistId") {
        Some(playlist) => Some(format!("{PLAYLIST}{playlist}")),
        // A list made up for the shelf: its songs, in order.
        None => {
            let videos: Vec<_> = items.iter().filter_map(|item| item.context_uri.strip_prefix(TRACK)).collect();
            (!videos.is_empty()).then(|| format!("{SONGS}{}", videos.join(",")))
        }
    });
    Some(Shelf {
        uri: format!("ytmusic:shelf:{title}"),
        title,
        strapline: text(&header["strapline"]),
        shortcuts: false,
        songs,
        play_all,
        items,
    })
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
        assert!(shelves[0].songs, "Quick picks are songs in columns");
        let videos = ["xukbqwRuN5w", "kOn-HdEg6AQ"];
        assert!(
            shelves[0].play_all.as_deref().is_some_and(|all| all.starts_with(&format!("{SONGS}{}", videos.join(",")))),
            "Play all plays the songs in order: {:?}",
            shelves[0].play_all
        );
        let song = &shelves[0].items[0];
        assert_eq!(song.kind, ItemKind::Playlist, "a song opens as its radio");
        assert_eq!(song.uri, "ytmusic:playlist:RDAMVMxukbqwRuN5w");
        assert_eq!(song.context_uri, "ytmusic:track:xukbqwRuN5w");
        assert_eq!(song.title, "Choosin' Texas");
        assert_eq!(song.subtitle, "Ella Langley • 346M plays • Dandelion");
        assert!(song.image_url.as_deref().is_some_and(|url| url.contains("=w300-h300")), "{:?}", song.image_url);

        let playlists = &shelves[1];
        assert_eq!(playlists.title, "100% Hits");
        assert_eq!(playlists.strapline, "DIE GRÖSSTEN HITS ALLER ZEITEN");
        assert!(!playlists.songs && playlists.play_all.is_none());
        let first = &playlists.items[0];
        assert_eq!(first.kind, ItemKind::Playlist);
        assert!(first.uri.starts_with("ytmusic:playlist:RDCLAK5uy_"), "{}", first.uri);
        assert_eq!(first.context_uri, first.uri);
        assert!(parse::continuation(sections).is_some());
    }

    #[test]
    fn chips_narrow_home_down_except_to_podcasts() {
        let page: Value = serde_json::from_str(include_str!("../tests/fixtures/home.json")).unwrap();
        let sections = find(&page, "sectionListRenderer").unwrap();
        let chips = parse_chips(sections, None);
        let titles: Vec<_> = chips.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Workout", "Relax"], "cloofy can't play podcasts on YouTube Music");
        assert!(chips.iter().all(|c| !c.selected && c.id.starts_with("ggN")));

        let relax = parse_chips(sections, Some(&chips[1].id));
        assert!(relax[1].selected && !relax[0].selected, "the chip asked for is the selected one");
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
