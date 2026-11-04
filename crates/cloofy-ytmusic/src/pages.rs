//! Playlist, album and artist pages, and the songs of playlists and albums page by page.

use anyhow::{Context, Result, bail};
use cloofy_core::{
    artist::{ArtistPage, TopTrack},
    catalog::{AlbumInfo, Page, PlaylistInfo, Track},
};
use futures::{Stream, stream};
use serde_json::Value;

use crate::{
    Source, YtMusic, home,
    innertube::InnerTube,
    parse::{self, ALBUM, ARTIST, Byline, IMAGE_SIZE, Link, PLAYLIST, find, list, runs, str_at, text},
    watch,
};

/// The playlist of the user's liked songs.
pub(crate) const LIKED: &str = "LM";

/// Radios only play: a song's (`RDAMVM…`), an artist's (`RDEM…`), an album's (`RDAMPL…`).
/// Other mixes (`RDTMAK…`, `RDCLAK…`) have a playlist page like any playlist.
pub(crate) fn is_mix(playlist: &str) -> bool {
    ["RDAMVM", "RDEM", "RDAMPL", "RDMM", "RDAO"].iter().any(|prefix| playlist.starts_with(prefix))
}

pub(crate) async fn playlist_info(api: &InnerTube, uri: &str) -> Result<PlaylistInfo> {
    let id = parse::id(uri, PLAYLIST)?;
    if !is_mix(id) {
        let page = browse_playlist(api, id).await?;
        match parse_playlist(&page, id) {
            Ok((info, _, _)) => return Ok(info),
            // Mixes of a kind cloofy doesn't know yet might only play.
            Err(_) if id.starts_with("RD") => {}
            Err(err) => return Err(err),
        }
    }
    let watch = mix(api, id).await?;
    Ok(mix_info(id, &watch))
}

async fn browse_playlist(api: &InnerTube, id: &str) -> Result<Value> {
    if id == LIKED && !api.signed_in() {
        bail!("Sign in to YouTube Music to see your liked songs (`cloofy login ytmusic`)");
    }
    api.browse(&format!("VL{id}")).await
}

/// The first page of a mix; a song's radio starts with the song.
async fn mix(api: &InnerTube, id: &str) -> Result<watch::Watch> {
    let video = id.strip_prefix("RDAMVM");
    watch::next(api, video, Some(id)).await
}

fn mix_info(id: &str, watch: &watch::Watch) -> PlaylistInfo {
    let name = match (&watch.title, id.strip_prefix("RDAMVM"), watch.tracks.first()) {
        (_, Some(_), Some(first)) => format!("{} Radio", first.name),
        (Some(title), _, _) => title.clone(),
        _ => "Mix".into(),
    };
    PlaylistInfo {
        id: id.to_owned(),
        uri: format!("{PLAYLIST}{id}"),
        name,
        owner_id: String::new(),
        owner_name: "YouTube Music".into(),
        collaborative: false,
        total: watch.tracks.len() as u32,
        image_url: watch.tracks.first().and_then(|t| t.image_url.clone()),
    }
}

/// A playlist page: what it is, its first songs, and the token for the rest.
fn parse_playlist(page: &Value, id: &str) -> Result<(PlaylistInfo, Vec<Track>, Option<String>)> {
    let shelf = find(page, "musicPlaylistShelfRenderer").or_else(|| find(page, "musicShelfRenderer"));
    let header = find(page, "musicResponsiveHeaderRenderer")
        .or_else(|| find(page, "musicDetailHeaderRenderer"))
        .context("Playlist not found")?;
    let tracks: Vec<Track> = shelf
        .map(|shelf| list(&shelf["contents"]))
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item.get("musicResponsiveListItemRenderer").and_then(parse::list_item_track))
        .collect();
    let token = shelf.and_then(parse::continuation);
    let strapline = Byline::parse([runs(&header["straplineTextOne"])]);
    let (owner_id, owner_name) = match strapline.artists.first() {
        Some((name, id)) => (id.clone().unwrap_or_default(), name.clone()),
        None => {
            let facepile = &header["facepile"]["avatarStackViewModel"];
            let name = str_at(facepile, "/text/content").unwrap_or_default();
            let id = facepile.pointer("/rendererContext/commandContext/onTap/innertubeCommand").and_then(Link::of);
            let id = match id {
                Some(Link::Artist(id)) => id,
                _ => String::new(),
            };
            (id, if name.is_empty() { text(&header["straplineTextOne"]) } else { name.to_owned() })
        }
    };
    let total = song_count(&text(&header["secondSubtitle"])).unwrap_or(tracks.len() as u32);
    let info = PlaylistInfo {
        id: id.to_owned(),
        uri: format!("{PLAYLIST}{id}"),
        name: if id == LIKED { "Liked Music".into() } else { text(&header["title"]) },
        owner_id,
        owner_name,
        collaborative: false,
        total,
        image_url: parse::image(&header["thumbnail"], IMAGE_SIZE),
    };
    Ok((info, tracks, token))
}

/// The number of songs in `1.6M views • 496 tracks • 31+ hours`.
fn song_count(subtitle: &str) -> Option<u32> {
    subtitle
        .split('•')
        .map(str::trim)
        .find(|part| ["song", "songs", "track", "tracks", "episode", "episodes"].iter().any(|w| part.ends_with(w)))
        .and_then(parse::leading_number)
}

/// The album, the ID of the playlist it plays as (which saving it needs), and its songs.
pub(crate) async fn album_info(api: &InnerTube, uri: &str) -> Result<(AlbumInfo, Option<String>)> {
    let page = api.browse(parse::id(uri, ALBUM)?).await?;
    let (album, playlist, _) = parse_album(&page, uri)?;
    Ok((album, playlist))
}

fn parse_album(page: &Value, uri: &str) -> Result<(AlbumInfo, Option<String>, Vec<Track>)> {
    let id = parse::id(uri, ALBUM)?;
    let header = find(page, "musicResponsiveHeaderRenderer")
        .or_else(|| find(page, "musicDetailHeaderRenderer"))
        .context("Album not found")?;
    let artists = Byline::parse([runs(&header["straplineTextOne"])]);
    let subtitle = Byline::parse([runs(&header["subtitle"])]);
    let image_url = parse::image(&header["thumbnail"], IMAGE_SIZE);
    let name = text(&header["title"]);
    let shelf = find(page, "musicShelfRenderer");
    let tracks: Vec<Track> = shelf
        .map(|shelf| list(&shelf["contents"]))
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item.get("musicResponsiveListItemRenderer").and_then(parse::list_item_track))
        .map(|track| Track {
            album: name.clone(),
            album_uri: uri.to_owned(),
            image_url: image_url.clone(),
            artists: if track.artists.is_empty() { artists.artist_names() } else { track.artists },
            artist_uris: if track.artist_uris.is_empty() { artists.artist_uris() } else { track.artist_uris },
            ..track
        })
        .collect();
    let mut ids = Vec::new();
    parse::find_all(page, "playlistId", &mut ids);
    let playlist = ids.iter().filter_map(|id| id.as_str()).find(|id| id.starts_with("OLAK5uy_")).map(str::to_owned);
    let album = AlbumInfo {
        id: id.to_owned(),
        uri: uri.to_owned(),
        name,
        artists: artists.artist_names(),
        year: subtitle.year.unwrap_or_default(),
        total: song_count(&text(&header["secondSubtitle"])).unwrap_or(tracks.len() as u32),
        image_url,
    };
    Ok((album, playlist, tracks))
}

pub(crate) async fn artist(api: &InnerTube, uri: &str) -> Result<ArtistPage> {
    let (mut artist, _) = artist_songs(api, uri).await?;
    // The artist page leaves out how long the top songs are.
    let missing: Vec<String> =
        artist.top_tracks.iter().filter(|t| t.track.duration_ms == 0).map(|t| t.track.uri.clone()).collect();
    if !missing.is_empty() {
        match watch::tracks_by_uri(api, &missing).await {
            Ok(details) => {
                for top in &mut artist.top_tracks {
                    if let Some(found) = details.iter().find(|d| d.uri == top.track.uri) {
                        top.track.duration_ms = found.duration_ms;
                    }
                }
            }
            Err(err) => tracing::debug!("lengths of the top songs: {err:#}"),
        }
    }
    Ok(artist)
}

/// The artist's page, and the playlist of their songs, which plays when the artist is played.
pub(crate) async fn artist_songs(api: &InnerTube, uri: &str) -> Result<(ArtistPage, Option<String>)> {
    let page = api.browse(parse::id(uri, ARTIST)?).await?;
    parse_artist(&page, uri)
}

/// The subscribe button's channel, which can differ from the artist page's.
pub(crate) async fn artist_channel(api: &InnerTube, uri: &str) -> Result<String> {
    let page = api.browse(parse::id(uri, ARTIST)?).await?;
    let button = find(&page, "subscribeButtonRenderer").context("This artist can't be followed")?;
    Ok(button["channelId"].as_str().context("This artist can't be followed")?.to_owned())
}

fn parse_artist(page: &Value, uri: &str) -> Result<(ArtistPage, Option<String>)> {
    // musicImmersiveHeaderRenderer, or musicVisualHeaderRenderer for artists without a banner.
    let header = page["header"].as_object().and_then(|h| h.values().next()).context("Artist not found")?;
    let sections = find(page, "sectionListRenderer").unwrap_or(&Value::Null);
    let songs = list(&sections["contents"]).iter().find_map(|s| s.get("musicShelfRenderer"));
    let songs_playlist =
        songs.and_then(|shelf| runs(&shelf["title"]).iter().find_map(Link::of_run)).and_then(|l| match l {
            Link::Playlist(id) => Some(id),
            _ => None,
        });
    let top_tracks = songs
        .map(|shelf| list(&shelf["contents"]))
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item.get("musicResponsiveListItemRenderer"))
        .filter_map(|row| {
            let track = parse::list_item_track(row)?;
            let counts: Vec<&[Value]> = list(&row["flexColumns"])
                .iter()
                .map(|c| runs(&c["musicResponsiveListItemFlexColumnRenderer"]["text"]))
                .collect();
            let plays = Byline::parse(counts.into_iter().skip(1))
                .counts
                .iter()
                .find_map(|c| c.ends_with("plays").then(|| parse::parse_count(c)).flatten());
            Some(TopTrack { track, playcount: plays })
        })
        .collect();
    let subscribe = find(header, "subscribeButtonRenderer");
    let followers = subscribe
        .map(|button| text(&button["subscriberCountText"]))
        .filter(|count| !count.is_empty())
        .and_then(|count| parse::parse_count(&count));
    let image_url =
        parse::image(&page["microformat"], IMAGE_SIZE).or_else(|| parse::image(&header["thumbnail"], IMAGE_SIZE));
    let artist = ArtistPage {
        uri: uri.to_owned(),
        name: text(&header["title"]),
        image_url,
        monthly_listeners: parse::parse_count(&text(&header["monthlyListenerCount"])),
        followers,
        saved: subscribe.is_some_and(|button| button["subscribed"] == true),
        top_tracks,
        shelves: home::parse_shelves(sections),
    };
    Ok((artist, songs_playlist))
}

enum State {
    Start(YtMusic, Source),
    More { client: YtMusic, kind: More, token: String, count: u32, total: u32 },
    Done,
}

#[derive(Clone, Copy)]
enum More {
    Playlist,
    Mix,
}

pub(crate) fn tracks(client: YtMusic, source: Source) -> impl Stream<Item = Result<Page<Track>>> + Send + 'static {
    stream::unfold(State::Start(client, source), |state| async move {
        let result = match state {
            State::Done => return None,
            State::Start(client, source) => first_page(client, source).await,
            State::More { client, kind, token, count, total } => more(client, kind, token, count, total).await,
        };
        Some(match result {
            Ok((page, state)) => (Ok(page), state),
            Err(err) => (Err(err), State::Done),
        })
    })
}

async fn first_page(client: YtMusic, source: Source) -> Result<(Page<Track>, State)> {
    let api = client.api();
    let playlist = match &source {
        Source::Liked => LIKED,
        Source::Playlist(uri) => parse::id(uri, PLAYLIST)?,
        Source::Album(uri) => {
            let page = api.browse(parse::id(uri, ALBUM)?).await?;
            let (_, _, tracks) = parse_album(&page, uri)?;
            let total = tracks.len() as u32;
            return Ok((Page { items: tracks, total, next_offset: None }, State::Done));
        }
    };
    if !is_mix(playlist) {
        let page = browse_playlist(api, playlist).await?;
        match parse_playlist(&page, playlist) {
            Ok((info, tracks, token)) => return Ok(self::page(client, More::Playlist, tracks, token, 0, info.total)),
            Err(_) if playlist.starts_with("RD") => {}
            Err(err) => return Err(err),
        }
    }
    let watch = mix(api, playlist).await?;
    Ok(page(client, More::Mix, watch.tracks, watch.continuation, 0, 0))
}

async fn more(client: YtMusic, kind: More, token: String, count: u32, total: u32) -> Result<(Page<Track>, State)> {
    let (tracks, token) = match kind {
        More::Playlist => {
            let page = client.api().continuation("browse", &token).await?;
            let items = continuation_items(&page);
            let tracks = items
                .iter()
                .filter_map(|item| item.get("musicResponsiveListItemRenderer").and_then(parse::list_item_track))
                .collect();
            (tracks, parse::continuation_in(items))
        }
        // Mixes go on forever; one more page is plenty.
        More::Mix => (watch::next_page(client.api(), &token).await?.tracks, None),
    };
    Ok(page(client, kind, tracks, token, count, total))
}

fn page(
    client: YtMusic,
    kind: More,
    items: Vec<Track>,
    token: Option<String>,
    before: u32,
    total: u32,
) -> (Page<Track>, State) {
    let count = before + items.len() as u32;
    match token.filter(|_| !items.is_empty()) {
        Some(token) => {
            let total = total.max(count);
            (Page { items, total, next_offset: Some(count) }, State::More { client, kind, token, count, total })
        }
        None => (Page { items, total: count, next_offset: None }, State::Done),
    }
}

/// The items of a continuation page, in the newer or the older shape.
fn continuation_items(page: &Value) -> &[Value] {
    if let Some(items) = list(&page["onResponseReceivedActions"])
        .iter()
        .find_map(|action| action.pointer("/appendContinuationItemsAction/continuationItems"))
    {
        return list(items);
    }
    page.pointer("/continuationContents/musicPlaylistShelfContinuation/contents").map_or(&[], |items| list(items))
}

#[cfg(test)]
mod tests {
    use cloofy_core::home::ItemKind;

    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn parses_a_playlist_page() {
        let (info, tracks, token) = parse_playlist(&fixture("playlist"), "PLciALUPf8sIJlvH350O8PWh4UbSbOffSJ").unwrap();
        assert_eq!(info.name, "Rolling Stone Magazine's 500 Greatest Songs Of All Time (2004)");
        assert_eq!(info.uri, "ytmusic:playlist:PLciALUPf8sIJlvH350O8PWh4UbSbOffSJ");
        assert_eq!(info.owner_name, "sweetchrisomine");
        assert_eq!(info.owner_id, "UCZzpKlkaLjrhI2rFnMjPdVg");
        assert!(info.total >= 490, "{}", info.total);
        assert!(info.image_url.is_some());
        let first = &tracks[0];
        assert_eq!(first.name, "Like a Rolling Stone");
        assert_eq!(first.artists, ["Bob Dylan"]);
        assert_eq!(first.album, "The Very Best Of");
        assert_eq!(first.duration_ms, 370_000);
        assert!(token.is_some(), "a long playlist goes on");
    }

    #[test]
    fn continuation_pages_have_more_songs() {
        let page = fixture("playlist_continuation");
        let items = continuation_items(&page);
        let tracks: Vec<_> = items
            .iter()
            .filter_map(|item| item.get("musicResponsiveListItemRenderer").and_then(parse::list_item_track))
            .collect();
        assert!(!tracks.is_empty());
        assert!(tracks.iter().all(|t| t.uri.starts_with("ytmusic:track:")));
    }

    #[test]
    fn parses_an_album_page() {
        let uri = "ytmusic:album:MPREb_K8qWMWVqXGi";
        let (album, playlist, tracks) = parse_album(&fixture("album"), uri).unwrap();
        assert_eq!(album.name, "Random Access Memories");
        assert_eq!(album.artists, ["Daft Punk"]);
        assert_eq!(album.year, "2013");
        assert_eq!(album.total, 13);
        assert!(playlist.as_deref().is_some_and(|id| id.starts_with("OLAK5uy_")));
        let first = &tracks[0];
        assert_eq!(first.name, "Give Life Back to Music");
        assert_eq!((first.album.as_str(), first.album_uri.as_str()), ("Random Access Memories", uri));
        assert_eq!(first.image_url, album.image_url, "album songs show the album's cover");
        assert_eq!(first.duration_ms, 276_000);
    }

    #[test]
    fn parses_an_artist_page() {
        let uri = "ytmusic:artist:UCRr1xG_2WIDs18a6cIiCxeA";
        let (page, songs) = parse_artist(&fixture("artist"), uri).unwrap();
        assert_eq!(page.name, "Daft Punk");
        assert!(page.monthly_listeners.is_some_and(|n| n > 1_000_000));
        assert!(page.followers.is_some_and(|n| n > 1_000_000));
        assert!(!page.saved);
        assert!(page.image_url.as_deref().is_some_and(|url| url.contains("w300-h300")), "{:?}", page.image_url);
        assert_eq!(page.top_tracks.len(), 3, "as many as the trimmed fixture has");
        assert!(page.top_tracks[0].playcount.is_some_and(|n| n > 1_000_000));
        assert!(songs.is_some_and(|id| id.starts_with("OLAK5uy_")));
        let albums = page.shelves.iter().find(|s| s.title == "Albums").unwrap();
        assert!(albums.items.iter().all(|item| item.kind == ItemKind::Album));
        assert!(page.shelves.iter().any(|s| s.items.iter().any(|i| i.kind == ItemKind::Artist)));
    }

    #[test]
    fn reads_the_song_count() {
        assert_eq!(song_count("1.6M views • 496 tracks • 31+ hours"), Some(496));
        assert_eq!(song_count("13 songs • 1 hour, 14 minutes"), Some(13));
        assert_eq!(song_count("1,234 songs"), Some(1_234));
        assert_eq!(song_count("2 hours"), None);
    }

    #[test]
    fn tells_mixes_from_playlists() {
        assert!(is_mix("RDAMVM52fNscjkST4"));
        assert!(is_mix("RDEMni3jl65KF37F9JYsJM8DGg"));
        assert!(!is_mix("RDCLAK5uy_k8d0XHQgAWWSZe7l7tUp0xLmEV_ncPxck"));
        assert!(!is_mix("PLciALUPf8sIJlvH350O8PWh4UbSbOffSJ"));
    }
}
