//! Reading InnerTube's renderers. YouTube reshuffles them now and then, so everything here is
//! lenient: a field that isn't where it should be leaves its value empty, and an item that
//! can't be made sense of is skipped rather than failing the whole page.

use cloofy_core::{
    catalog::Track,
    home::{HomeItem, ItemKind},
};
use serde_json::Value;

/// Cover size for lists and cards, in pixels.
pub(crate) const IMAGE_SIZE: u32 = 300;

pub(crate) const TRACK: &str = "ytmusic:track:";
pub(crate) const ALBUM: &str = "ytmusic:album:";
pub(crate) const PLAYLIST: &str = "ytmusic:playlist:";
pub(crate) const ARTIST: &str = "ytmusic:artist:";

/// The text of a `runs` or `simpleText` object.
pub(crate) fn text(value: &Value) -> String {
    if let Some(simple) = value["simpleText"].as_str() {
        return simple.to_owned();
    }
    runs(value).iter().filter_map(|run| run["text"].as_str()).collect()
}

pub(crate) fn runs(value: &Value) -> &[Value] {
    value["runs"].as_array().map_or(&[], Vec::as_slice)
}

pub(crate) fn list(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

/// The first value under `key`, searched depth first.
pub(crate) fn find<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map.get(key).or_else(|| map.values().find_map(|v| find(v, key))),
        Value::Array(items) => items.iter().find_map(|v| find(v, key)),
        _ => None,
    }
}

/// Every value under `key`, outermost first; matches aren't searched further.
pub(crate) fn find_all<'a>(value: &'a Value, key: &str, found: &mut Vec<&'a Value>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if k == key {
                    found.push(v);
                } else {
                    find_all(v, key, found);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| find_all(v, key, found)),
        _ => {}
    }
}

pub(crate) fn str_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Where a link goes: a page (browse ID and page type) or a song.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Link {
    Album(String),
    Artist(String),
    Playlist(String),
    /// A song or video, maybe within a playlist.
    Watch {
        video: String,
        playlist: Option<String>,
    },
    Other,
}

impl Link {
    pub fn of(endpoint: &Value) -> Option<Link> {
        if let Some(browse) = endpoint.get("browseEndpoint") {
            let id = browse["browseId"].as_str()?.to_owned();
            let page =
                str_at(browse, "/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
                    .unwrap_or_default();
            return Some(match page {
                "MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK" => Link::Album(id),
                "MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL" | "MUSIC_PAGE_TYPE_LIBRARY_ARTIST" => {
                    Link::Artist(id)
                }
                "MUSIC_PAGE_TYPE_PLAYLIST" => Link::Playlist(id.strip_prefix("VL").unwrap_or(&id).to_owned()),
                _ if id.starts_with("MPREb_") => Link::Album(id),
                _ if id.starts_with("UC") => Link::Artist(id),
                _ if id.starts_with("VL") => Link::Playlist(id[2..].to_owned()),
                _ => Link::Other,
            });
        }
        let watch = endpoint.get("watchEndpoint")?;
        Some(Link::Watch {
            video: watch["videoId"].as_str()?.to_owned(),
            playlist: watch["playlistId"].as_str().map(str::to_owned),
        })
    }

    /// The link of a run of text.
    pub fn of_run(run: &Value) -> Option<Link> {
        Link::of(run.get("navigationEndpoint")?)
    }
}

/// A line of text such as `Song • Daft Punk & Pharrell • Random Access Memories • 4:08`,
/// sorted into what each part says.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Byline {
    /// Song, Video, Album, Single, EP, Playlist, Artist…
    pub label: Option<String>,
    /// Names, and the channel ID of those that link to one.
    pub artists: Vec<(String, Option<String>)>,
    pub album: Option<(String, String)>,
    pub duration_ms: Option<u32>,
    pub year: Option<String>,
    /// "1.2B plays", "7.2M subscribers", "50 songs"…
    pub counts: Vec<String>,
    /// Parts that are none of the above, such as a playlist's owner without a link.
    pub other: Vec<String>,
}

const LABELS: [&str; 13] = [
    "Song",
    "Video",
    "Album",
    "Single",
    "EP",
    "Playlist",
    "Artist",
    "Episode",
    "Podcast",
    "Profile",
    "Station",
    "Audiobook",
    "Mix",
];
const SEPARATORS: [&str; 6] = [", ", " & ", " and ", " x ", " + ", ","];

impl Byline {
    /// Sorts the runs of one or more columns. Parts are separated by ` • ` runs.
    pub fn parse<'a>(columns: impl IntoIterator<Item = &'a [Value]>) -> Byline {
        let mut byline = Byline::default();
        for runs in columns {
            for part in runs.split(|run| run["text"].as_str().is_some_and(|t| t.trim() == "•")) {
                byline.add(part);
            }
        }
        byline
    }

    fn add(&mut self, part: &[Value]) {
        let text: String = part.iter().filter_map(|run| run["text"].as_str()).collect();
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let links: Vec<_> = part.iter().map(Link::of_run).collect();
        if let Some(i) = links.iter().position(|l| matches!(l, Some(Link::Album(_)))) {
            if let Some(Link::Album(id)) = &links[i] {
                self.album = Some((part[i]["text"].as_str().unwrap_or_default().to_owned(), id.clone()));
            }
            return;
        }
        if links.iter().any(|l| matches!(l, Some(Link::Artist(_)))) && self.artists.is_empty() {
            for (run, link) in part.iter().zip(&links) {
                let name = run["text"].as_str().unwrap_or_default();
                if name.trim().is_empty() || SEPARATORS.contains(&name) {
                    continue;
                }
                let id = match link {
                    Some(Link::Artist(id)) => Some(id.clone()),
                    _ => None,
                };
                self.artists.push((name.trim().to_owned(), id));
            }
            return;
        }
        if let Some(ms) = parse_duration(text) {
            self.duration_ms = Some(ms);
        } else if self.label.is_none() && self.artists.is_empty() && LABELS.contains(&text) {
            self.label = Some(text.to_owned());
        } else if text.len() == 4 && text.chars().all(|c| c.is_ascii_digit()) {
            self.year = Some(text.to_owned());
        } else if text.chars().next().is_some_and(|c| c.is_ascii_digit()) && text.contains(' ') {
            self.counts.push(text.to_owned());
        } else {
            self.other.push(text.to_owned());
        }
    }

    /// The artists, or else the first unlinked part, which is where the artist stands.
    pub fn artist_names(&self) -> Vec<String> {
        if !self.artists.is_empty() {
            return self.artists.iter().map(|(name, _)| name.clone()).collect();
        }
        self.other.first().map(|name| vec![name.clone()]).unwrap_or_default()
    }

    /// Artist URIs for the leading artists that link to a channel; cloofy pairs them with the
    /// names from the front.
    pub fn artist_uris(&self) -> Vec<String> {
        self.artists.iter().map_while(|(_, id)| id.as_ref().map(|id| format!("{ARTIST}{id}"))).collect()
    }
}

/// `3:45` or `1:02:03` in milliseconds.
pub(crate) fn parse_duration(text: &str) -> Option<u32> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if !(2..=3).contains(&parts.len()) || parts.iter().any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let seconds = parts.iter().try_fold(0u32, |total, part| Some(total * 60 + part.parse::<u32>().ok()?))?;
    Some(seconds * 1000)
}

/// `1.2B plays`, `68.3M monthly audience`, `7,215 subscribers` as a number.
pub(crate) fn parse_count(text: &str) -> Option<u64> {
    let number = text.split_whitespace().next()?;
    let (digits, factor) = match number.chars().last()? {
        'K' | 'k' => (&number[..number.len() - 1], 1e3),
        'M' => (&number[..number.len() - 1], 1e6),
        'B' => (&number[..number.len() - 1], 1e9),
        _ => (number, 1.0),
    };
    let value: f64 = digits.replace(',', "").parse().ok()?;
    Some((value * factor).round() as u64)
}

/// The first number in a text like `1,234 songs` or `496 tracks`.
pub(crate) fn leading_number(text: &str) -> Option<u32> {
    let digits: String =
        text.chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit() || *c == ',').collect();
    digits.replace(',', "").parse().ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Thumbnail {
    pub url: String,
    pub width: u32,
    pub height: u32,
}

/// The thumbnails anywhere within `value`, smallest first as YouTube lists them.
pub(crate) fn thumbnails(value: &Value) -> Vec<Thumbnail> {
    let list = find(value, "thumbnails").or_else(|| find(value, "sources"));
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            Some(Thumbnail {
                url: t["url"].as_str()?.to_owned(),
                width: t["width"].as_u64().unwrap_or(0) as u32,
                height: t["height"].as_u64().unwrap_or(0) as u32,
            })
        })
        .collect()
}

/// A cover of about `size` pixels. Square covers on Google's image servers are made in that
/// size; others (video stills) are picked from what YouTube offers.
pub(crate) fn image(value: &Value, size: u32) -> Option<String> {
    pick(&thumbnails(value), size)
}

pub(crate) fn pick(thumbnails: &[Thumbnail], size: u32) -> Option<String> {
    let largest = thumbnails.iter().max_by_key(|t| t.width)?;
    if largest.width == largest.height
        && let Some(url) = resize(&largest.url, size)
    {
        return Some(url);
    }
    thumbnails.iter().filter(|t| t.width >= size).min_by_key(|t| t.width).or(Some(largest)).map(|t| t.url.clone())
}

/// The URL of the same image in another size, where the server makes any size
/// (`…=w120-h120-l90-rj` → `…=w300-h300-l90-rj`).
pub(crate) fn resize(url: &str, size: u32) -> Option<String> {
    let host = url.strip_prefix("https://")?.split('/').next()?;
    if !(host.ends_with(".googleusercontent.com") || host.ends_with(".ggpht.com")) {
        return None;
    }
    let (base, options) = url.rsplit_once('=')?;
    if options.starts_with('s') && options[1..].chars().take_while(char::is_ascii_digit).count() > 0 {
        let rest = options.trim_start_matches(|c: char| c == 's' || c.is_ascii_digit());
        return Some(format!("{base}=s{size}{rest}"));
    }
    if !options.starts_with('w') {
        return None;
    }
    let rest: Vec<&str> =
        options.split('-').filter(|o| !(o.starts_with('w') || o.starts_with('h') || o.starts_with('s'))).collect();
    let rest = if rest.is_empty() { String::new() } else { format!("-{}", rest.join("-")) };
    Some(format!("{base}=w{size}-h{size}{rest}"))
}

/// The ID in a URI of the given prefix, e.g. the video ID in `ytmusic:track:…`.
pub(crate) fn id<'a>(uri: &'a str, prefix: &str) -> anyhow::Result<&'a str> {
    uri.strip_prefix(prefix).filter(|id| !id.is_empty()).ok_or_else(|| anyhow::anyhow!("Not a {prefix}… URI: {uri}"))
}

fn flex_columns(item: &Value) -> Vec<&[Value]> {
    list(&item["flexColumns"]).iter().map(|c| runs(&c["musicResponsiveListItemFlexColumnRenderer"]["text"])).collect()
}

fn fixed_columns(item: &Value) -> Vec<&[Value]> {
    list(&item["fixedColumns"]).iter().map(|c| runs(&c["musicResponsiveListItemFixedColumnRenderer"]["text"])).collect()
}

/// The song in a row of a list (`musicResponsiveListItemRenderer`): search results,
/// playlists, albums, an artist's top songs. Rows that can't play are left out.
pub(crate) fn list_item_track(item: &Value) -> Option<Track> {
    if item["musicItemRendererDisplayPolicy"] == "MUSIC_ITEM_RENDERER_DISPLAY_POLICY_GREY_OUT" {
        return None;
    }
    let columns = flex_columns(item);
    let title = columns.first()?;
    let video = str_at(item, "/playlistItemData/videoId")
        .or_else(|| title.iter().find_map(|run| str_at(run, "/navigationEndpoint/watchEndpoint/videoId")))
        .or_else(|| str_at(item, "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId"))?;
    let name: String = title.iter().filter_map(|run| run["text"].as_str()).collect();
    let byline = Byline::parse(columns.iter().skip(1).copied().chain(fixed_columns(item)));
    let (album, album_uri) = byline.album.clone().map(|(name, id)| (name, format!("{ALBUM}{id}"))).unwrap_or_default();
    Some(Track {
        uri: format!("{TRACK}{video}"),
        name,
        artists: byline.artist_names(),
        artist_uris: byline.artist_uris(),
        album,
        album_uri,
        duration_ms: byline.duration_ms.unwrap_or(0),
        image_url: image(&item["thumbnail"], IMAGE_SIZE),
    })
}

/// A song of a queue or radio (`playlistPanelVideoRenderer`, maybe in a wrapper that pairs
/// the song with its music video).
pub(crate) fn panel_track(item: &Value) -> Option<Track> {
    let item = item
        .get("playlistPanelVideoRenderer")
        .or_else(|| item.pointer("/playlistPanelVideoWrapperRenderer/primaryRenderer/playlistPanelVideoRenderer"))?;
    if item.get("unplayableText").is_some() {
        return None;
    }
    let video = item["videoId"].as_str().or_else(|| str_at(item, "/navigationEndpoint/watchEndpoint/videoId"))?;
    let byline = Byline::parse([runs(&item["longBylineText"])]);
    let (album, album_uri) = byline.album.clone().map(|(name, id)| (name, format!("{ALBUM}{id}"))).unwrap_or_default();
    let mut artists = byline.artist_names();
    if artists.is_empty() {
        artists = vec![text(&item["shortBylineText"])].into_iter().filter(|a| !a.is_empty()).collect();
    }
    Some(Track {
        uri: format!("{TRACK}{video}"),
        name: text(&item["title"]),
        artists,
        artist_uris: byline.artist_uris(),
        album,
        album_uri,
        duration_ms: parse_duration(&text(&item["lengthText"])).or(byline.duration_ms).unwrap_or(0),
        image_url: image(&item["thumbnail"], IMAGE_SIZE),
    })
}

/// A card of a shelf (`musicTwoRowItemRenderer`) or a row standing for an album, playlist or
/// artist. Songs become their radio: opening one lists the song and what YouTube Music would
/// play after it, playing one plays the song and then the radio.
pub(crate) fn home_item(item: &Value) -> Option<HomeItem> {
    if let Some(card) = item.get("musicTwoRowItemRenderer") {
        let title_run = runs(&card["title"]).first();
        let link = Link::of(&card["navigationEndpoint"]).or_else(|| title_run.and_then(Link::of_run))?;
        let byline = Byline::parse([runs(&card["subtitle"])]);
        return make_item(link, text(&card["title"]), &byline, text(&card["subtitle"]), &card["thumbnailRenderer"]);
    }
    let row = item.get("musicResponsiveListItemRenderer")?;
    if let Some(track) = list_item_track(row)
        && row.get("navigationEndpoint").is_none()
    {
        return Some(song_item(&track));
    }
    let link = Link::of(row.get("navigationEndpoint")?)?;
    let columns = flex_columns(row);
    let title: String = columns.first()?.iter().filter_map(|r| r["text"].as_str()).collect();
    let byline = Byline::parse(columns.iter().skip(1).copied());
    let subtitle =
        columns.iter().skip(1).map(|runs| runs.iter().filter_map(|r| r["text"].as_str()).collect::<String>());
    let subtitle = subtitle.filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" • ");
    make_item(link, title, &byline, subtitle, &row["thumbnail"])
}

fn make_item(link: Link, title: String, byline: &Byline, subtitle: String, thumbnail: &Value) -> Option<HomeItem> {
    if title.is_empty() {
        return None;
    }
    let (kind, uri) = match link {
        Link::Album(id) => (ItemKind::Album, format!("{ALBUM}{id}")),
        Link::Artist(id) => (ItemKind::Artist, format!("{ARTIST}{id}")),
        Link::Playlist(id) => (ItemKind::Playlist, format!("{PLAYLIST}{id}")),
        Link::Watch { video, .. } => {
            let mut item = song_item(&Track {
                uri: format!("{TRACK}{video}"),
                name: title,
                artists: byline.artist_names(),
                artist_uris: Vec::new(),
                album: String::new(),
                album_uri: String::new(),
                duration_ms: 0,
                image_url: image(thumbnail, IMAGE_SIZE),
            });
            item.subtitle = subtitle;
            return Some(item);
        }
        Link::Other => return None,
    };
    // Only "Playlist • <owner> • …" names the owner; other subtitles list the artists in it.
    let labeled = kind == ItemKind::Playlist && byline.label.as_deref() == Some("Playlist");
    let (owner_id, owner_name) = match byline.artists.first() {
        Some((name, id)) if labeled => (id.clone().unwrap_or_default(), name.clone()),
        None if labeled => (String::new(), byline.other.first().cloned().unwrap_or_default()),
        _ => (String::new(), String::new()),
    };
    Some(HomeItem {
        context_uri: uri.clone(),
        uri,
        kind,
        title,
        subtitle,
        image_url: image(thumbnail, IMAGE_SIZE),
        owner_id,
        owner_name,
    })
}

/// A song as a card: its radio to open, the song itself to play.
pub(crate) fn song_item(track: &Track) -> HomeItem {
    let video = track.uri.strip_prefix(TRACK).unwrap_or_default();
    let mut subtitle = vec!["Song".to_owned()];
    subtitle.extend(track.artists.first().cloned());
    HomeItem {
        uri: format!("{PLAYLIST}{}", radio_playlist(video)),
        kind: ItemKind::Playlist,
        title: track.name.clone(),
        subtitle: subtitle.join(" • "),
        image_url: track.image_url.clone(),
        owner_id: String::new(),
        owner_name: "YouTube Music".into(),
        context_uri: track.uri.clone(),
    }
}

/// The ID of a song's radio, which `next` understands as a playlist.
pub(crate) fn radio_playlist(video: &str) -> String {
    format!("RDAMVM{video}")
}

/// The token of the next page, in either of the shapes YouTube uses.
pub(crate) fn continuation(value: &Value) -> Option<String> {
    if let Some(token) = value
        .pointer("/continuations/0/nextContinuationData/continuation")
        .or_else(|| value.pointer("/continuations/0/nextRadioContinuationData/continuation"))
        .and_then(Value::as_str)
    {
        return Some(token.to_owned());
    }
    let items = value.get("contents").or_else(|| value.get("items")).or_else(|| value.get("continuationItems"));
    continuation_in(list(items.unwrap_or(value)))
}

/// The token in the item that ends a list, in the newer shape.
pub(crate) fn continuation_in(items: &[Value]) -> Option<String> {
    items.iter().rev().find_map(|item| {
        str_at(item, "/continuationItemRenderer/continuationEndpoint/continuationCommand/token").map(str::to_owned)
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn run(text: &str) -> Value {
        json!({ "text": text })
    }

    fn linked(text: &str, id: &str, page: &str) -> Value {
        json!({ "text": text, "navigationEndpoint": { "browseEndpoint": { "browseId": id,
            "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": {
                "pageType": format!("MUSIC_PAGE_TYPE_{page}") } } } } })
    }

    #[test]
    fn sorts_a_byline_into_its_parts() {
        let runs = [
            run("Song"),
            run(" • "),
            linked("Daft Punk", "UC1", "ARTIST"),
            run(" & "),
            run("Pharrell"),
            run(" • "),
            linked("Random Access Memories", "MPREb_1", "ALBUM"),
            run(" • "),
            run("4:08"),
            run(" • "),
            run("1.2B plays"),
        ];
        let byline = Byline::parse([&runs[..]]);
        assert_eq!(byline.label.as_deref(), Some("Song"));
        assert_eq!(byline.artist_names(), ["Daft Punk", "Pharrell"]);
        assert_eq!(byline.artist_uris(), ["ytmusic:artist:UC1"], "only the leading linked artists");
        assert_eq!(byline.album, Some(("Random Access Memories".into(), "MPREb_1".into())));
        assert_eq!(byline.duration_ms, Some(248_000));
        assert_eq!(byline.counts, ["1.2B plays"]);
    }

    #[test]
    fn an_unlinked_artist_is_the_first_plain_part() {
        let runs = [run("Various Artists"), run(" • "), run("2001")];
        let byline = Byline::parse([&runs[..]]);
        assert_eq!(byline.artist_names(), ["Various Artists"]);
        assert!(byline.artist_uris().is_empty());
        assert_eq!(byline.year.as_deref(), Some("2001"));
    }

    #[test]
    fn reads_durations_and_counts() {
        assert_eq!(parse_duration("3:45"), Some(225_000));
        assert_eq!(parse_duration("1:02:03"), Some(3_723_000));
        assert_eq!(parse_duration("2001"), None);
        assert_eq!(parse_duration("a:b"), None);
        assert_eq!(parse_count("1.2B plays"), Some(1_200_000_000));
        assert_eq!(parse_count("68.3M monthly audience"), Some(68_300_000));
        assert_eq!(parse_count("7,215 subscribers"), Some(7_215));
        assert_eq!(parse_count("812K"), Some(812_000));
        assert_eq!(leading_number("1.6M views • 1,496 tracks"), Some(1));
        assert_eq!(leading_number("1,496 tracks"), Some(1_496));
    }

    #[test]
    fn resizes_covers_on_googles_image_servers() {
        let url = "https://lh3.googleusercontent.com/abc=w120-h120-l90-rj";
        assert_eq!(resize(url, 300).as_deref(), Some("https://lh3.googleusercontent.com/abc=w300-h300-l90-rj"));
        let url = "https://yt3.googleusercontent.com/abc=w60-h60-p-l90-rj";
        assert_eq!(resize(url, 800).as_deref(), Some("https://yt3.googleusercontent.com/abc=w800-h800-p-l90-rj"));
        let url = "https://yt3.ggpht.com/abc=s192";
        assert_eq!(resize(url, 300).as_deref(), Some("https://yt3.ggpht.com/abc=s300"));
        assert_eq!(resize("https://i.ytimg.com/vi/x/hqdefault.jpg?sqp=a=b", 300), None);
    }

    #[test]
    fn picks_a_still_for_video_thumbnails() {
        let stills = [
            Thumbnail { url: "small".into(), width: 120, height: 90 },
            Thumbnail { url: "medium".into(), width: 400, height: 225 },
            Thumbnail { url: "large".into(), width: 1280, height: 720 },
        ];
        assert_eq!(pick(&stills, 300).as_deref(), Some("medium"));
        assert_eq!(pick(&stills, 2000).as_deref(), Some("large"));
        assert_eq!(pick(&[], 300), None);
    }

    #[test]
    fn tells_links_apart() {
        let artist = linked("x", "UC1", "ARTIST");
        assert_eq!(Link::of_run(&artist), Some(Link::Artist("UC1".into())));
        let playlist = linked("x", "VLPL1", "PLAYLIST");
        assert_eq!(Link::of_run(&playlist), Some(Link::Playlist("PL1".into())));
        let watch = json!({ "navigationEndpoint": { "watchEndpoint": { "videoId": "v", "playlistId": "p" } } });
        assert_eq!(Link::of_run(&watch), Some(Link::Watch { video: "v".into(), playlist: Some("p".into()) }));
    }
}
