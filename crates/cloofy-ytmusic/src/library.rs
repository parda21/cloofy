//! The user's library: playlists, albums, subscribed artists and liked songs. All of it needs
//! a sign-in; signed out, the library is empty and nothing is saved.

use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use cloofy_core::{
    catalog::{AlbumInfo, ArtistInfo, PlaylistInfo, User},
    home::{HomeItem, ItemKind},
};
use futures::{StreamExt, stream};
use serde_json::{Value, json};

use crate::{
    innertube::InnerTube,
    pages::{self, LIKED},
    parse::{self, ALBUM, ARTIST, PLAYLIST, TRACK, find, find_all, list, str_at, text},
};

/// Pages of a library list at most; YouTube sends 25 to 100 entries per page.
const MAX_PAGES: usize = 40;
/// How long the library's contents count as current for "is this saved?".
const SAVED_TTL: Duration = Duration::from_secs(60);
const QUEUE_BATCH: usize = 50;
const QUEUE_CONCURRENCY: usize = 4;

const PLAYLISTS: &str = "FEmusic_liked_playlists";
const ALBUMS: &str = "FEmusic_liked_albums";
const ARTISTS: &str = "FEmusic_library_corpus_artists";

/// What the library learned along the way.
#[derive(Default)]
pub(crate) struct Library {
    /// Album browse ID → the playlist ID the album plays as; saving an album saves that.
    album_playlists: Mutex<HashMap<String, String>>,
    /// The URIs in each library list, and when they were fetched.
    saved: Mutex<HashMap<&'static str, (Instant, HashSet<String>)>>,
}

impl Library {
    pub fn remember_album(&self, album_uri: &str, playlist: Option<String>) {
        if let Some(playlist) = playlist {
            self.album_playlists.lock().unwrap().insert(album_uri.to_owned(), playlist);
        }
    }

    fn remember_list(&self, list: &'static str, uris: impl IntoIterator<Item = String>) {
        self.saved.lock().unwrap().insert(list, (Instant::now(), uris.into_iter().collect()));
    }

    fn cached(&self, list: &'static str) -> Option<HashSet<String>> {
        let saved = self.saved.lock().unwrap();
        saved.get(list).filter(|(at, _)| at.elapsed() < SAVED_TTL).map(|(_, uris)| uris.clone())
    }

    fn forget(&self, list: &'static str) {
        self.saved.lock().unwrap().remove(list);
    }
}

fn require_sign_in(api: &InnerTube) -> Result<()> {
    if !api.signed_in() {
        bail!("Sign in to YouTube Music first (`cloofy login ytmusic`)");
    }
    Ok(())
}

pub(crate) async fn me(api: &InnerTube) -> Result<User> {
    let page = api.call("account/account_menu", json!({})).await?;
    parse_me(&page).context("YouTube doesn't accept the saved sign-in any more – run `cloofy login ytmusic` again")
}

fn parse_me(page: &Value) -> Option<User> {
    let header = find(page, "activeAccountHeaderRenderer")?;
    let name = text(&header["accountName"]);
    let mut ids = Vec::new();
    find_all(page, "browseId", &mut ids);
    let channel = ids.iter().filter_map(|id| id.as_str()).find(|id| id.starts_with("UC"));
    let handle = text(&header["channelHandle"]);
    let id = channel.map(str::to_owned).or_else(|| (!handle.is_empty()).then_some(handle)).unwrap_or(name.clone());
    (!name.is_empty()).then_some(User { id, name })
}

pub(crate) async fn playlists(api: &InnerTube, library: &Library) -> Result<Vec<PlaylistInfo>> {
    require_sign_in(api)?;
    let items = library_items(api, PLAYLISTS).await?;
    let playlists: Vec<PlaylistInfo> = items
        .into_iter()
        .filter(|item| item.kind == ItemKind::Playlist && item.uri != format!("{PLAYLIST}{LIKED}"))
        .map(|item| PlaylistInfo {
            id: item.uri.strip_prefix(PLAYLIST).unwrap_or_default().to_owned(),
            uri: item.uri,
            name: item.title,
            owner_id: item.owner_id,
            owner_name: item.owner_name,
            collaborative: false,
            total: count(&item.subtitle).unwrap_or(0),
            image_url: item.image_url,
        })
        .collect();
    library.remember_list(PLAYLISTS, playlists.iter().map(|p| p.uri.clone()));
    Ok(playlists)
}

pub(crate) async fn albums(api: &InnerTube, library: &Library) -> Result<Vec<AlbumInfo>> {
    require_sign_in(api)?;
    let items = library_items(api, ALBUMS).await?;
    let albums: Vec<AlbumInfo> = items
        .into_iter()
        .filter(|item| item.kind == ItemKind::Album)
        .map(|item| {
            // "Album • Daft Punk • 2013", with or without the kind up front.
            let parts: Vec<&str> = item.subtitle.split(" • ").collect();
            let year = parts.iter().find(|p| p.len() == 4 && p.chars().all(|c| c.is_ascii_digit()));
            let artists = parts
                .iter()
                .find(|p| !matches!(**p, "Album" | "Single" | "EP" | "Compilation") && Some(*p) != year)
                .map(|a| a.to_string());
            AlbumInfo {
                id: item.uri.strip_prefix(ALBUM).unwrap_or_default().to_owned(),
                uri: item.uri,
                name: item.title,
                artists: artists.into_iter().collect(),
                year: year.map(|y| y.to_string()).unwrap_or_default(),
                total: 0,
                image_url: item.image_url,
            }
        })
        .collect();
    library.remember_list(ALBUMS, albums.iter().map(|a| a.uri.clone()));
    Ok(albums)
}

pub(crate) async fn artists(api: &InnerTube, library: &Library) -> Result<Vec<ArtistInfo>> {
    require_sign_in(api)?;
    let items = library_items(api, ARTISTS).await?;
    let artists: Vec<ArtistInfo> = items
        .into_iter()
        .filter(|item| item.kind == ItemKind::Artist)
        .map(|item| ArtistInfo { uri: item.uri, name: item.title, image_url: item.image_url })
        .collect();
    library.remember_list(ARTISTS, artists.iter().map(|a| a.uri.clone()));
    Ok(artists)
}

fn count(subtitle: &str) -> Option<u32> {
    subtitle
        .split(" • ")
        .find(|part| part.ends_with("songs") || part.ends_with("tracks") || part.ends_with("song"))
        .and_then(parse::leading_number)
}

/// Every entry of a library list, following its pages.
async fn library_items(api: &InnerTube, browse_id: &str) -> Result<Vec<HomeItem>> {
    let page = api.browse(browse_id).await?;
    let (mut items, mut token) = library_page(&page);
    let mut pages = 1;
    while let Some(next) = token.take().filter(|_| pages < MAX_PAGES) {
        pages += 1;
        let page = api.continuation("browse", &next).await?;
        let (more, next) = library_page(&page);
        items.extend(more);
        token = next;
    }
    Ok(items)
}

/// The entries of one page of a library list (a grid of cards, or a list of rows) and the
/// token for the next page.
pub(crate) fn library_page(page: &Value) -> (Vec<HomeItem>, Option<String>) {
    let container = ["gridRenderer", "musicShelfRenderer", "gridContinuation", "musicShelfContinuation"]
        .iter()
        .find_map(|key| find(page, key));
    if let Some(container) = container {
        let entries = container.get("items").or_else(|| container.get("contents")).unwrap_or(&Value::Null);
        let items = list(entries).iter().filter_map(parse::home_item).collect();
        return (items, parse::continuation(container));
    }
    let entries = list(&page["onResponseReceivedActions"])
        .iter()
        .find_map(|action| action.pointer("/appendContinuationItemsAction/continuationItems"))
        .unwrap_or(&Value::Null);
    (list(entries).iter().filter_map(parse::home_item).collect(), parse::continuation(entries))
}

pub(crate) async fn liked_count(api: &InnerTube) -> Result<u32> {
    require_sign_in(api)?;
    let info = pages::playlist_info(api, &format!("{PLAYLIST}{LIKED}")).await?;
    Ok(info.total)
}

pub(crate) async fn check_saved(api: &InnerTube, library: &Library, uris: &[String]) -> Result<Vec<(String, bool)>> {
    if !api.signed_in() {
        return Ok(uris.iter().map(|uri| (uri.clone(), false)).collect());
    }
    let tracks: Vec<String> = uris.iter().filter(|uri| uri.starts_with(TRACK)).cloned().collect();
    let liked = liked_tracks(api, &tracks).await?;
    let mut lists: HashMap<&'static str, HashSet<String>> = HashMap::new();
    for uri in uris {
        if let Some(list) = list_of(uri)
            && !lists.contains_key(list)
        {
            let saved = match library.cached(list) {
                Some(saved) => saved,
                None => {
                    let uris = library_items(api, list).await?.into_iter().map(|item| item.uri);
                    library.remember_list(list, uris);
                    library.cached(list).unwrap_or_default()
                }
            };
            lists.insert(list, saved);
        }
    }
    Ok(uris
        .iter()
        .map(|uri| {
            let saved = if uri.starts_with(TRACK) {
                liked.contains(uri)
            } else if *uri == format!("{PLAYLIST}{LIKED}") {
                true
            } else {
                list_of(uri).and_then(|list| lists.get(list)).is_some_and(|saved| saved.contains(uri))
            };
            (uri.clone(), saved)
        })
        .collect())
}

/// The library list a URI would be in.
fn list_of(uri: &str) -> Option<&'static str> {
    if uri.starts_with(ALBUM) {
        Some(ALBUMS)
    } else if uri.starts_with(PLAYLIST) && *uri != format!("{PLAYLIST}{LIKED}") {
        Some(PLAYLISTS)
    } else if uri.starts_with(ARTIST) {
        Some(ARTISTS)
    } else {
        None
    }
}

/// Which of the songs are liked, from the like buttons in their queue entries.
async fn liked_tracks(api: &InnerTube, uris: &[String]) -> Result<HashSet<String>> {
    let ids: Vec<String> = uris.iter().filter_map(|uri| uri.strip_prefix(TRACK)).map(str::to_owned).collect();
    let mut liked = HashSet::new();
    let mut pages = stream::iter(ids.chunks(QUEUE_BATCH).map(<[_]>::to_vec))
        .map(|batch| api.call("music/get_queue", json!({ "videoIds": batch })))
        .buffered(QUEUE_CONCURRENCY);
    while let Some(page) = pages.next().await {
        liked.extend(like_states(&page?).into_iter().filter(|(_, liked)| *liked).map(|(id, _)| format!("{TRACK}{id}")));
    }
    Ok(liked)
}

/// Each song of a `music/get_queue` answer and whether it is liked, as far as the answer says.
pub(crate) fn like_states(page: &Value) -> Vec<(String, bool)> {
    list(&page["queueDatas"])
        .iter()
        .filter_map(|data| {
            let content = &data["content"];
            let video = content.get("playlistPanelVideoRenderer").or_else(|| {
                content.pointer("/playlistPanelVideoWrapperRenderer/primaryRenderer/playlistPanelVideoRenderer")
            })?;
            let id = video["videoId"].as_str()?.to_owned();
            Some((id, liked(video)?))
        })
        .collect()
}

/// A like button says so directly; a "Add to/Remove from liked songs" menu entry says what
/// it would do, which is the opposite of the state.
fn liked(video: &Value) -> Option<bool> {
    if let Some(status) = find(video, "likeButtonRenderer").and_then(|b| b["likeStatus"].as_str()) {
        return Some(status == "LIKE");
    }
    let mut toggles = Vec::new();
    find_all(video, "toggleMenuServiceItemRenderer", &mut toggles);
    toggles.into_iter().find_map(|toggle| {
        if let Some(status) = str_at(toggle, "/defaultServiceEndpoint/likeEndpoint/status") {
            return Some(status == "INDIFFERENT");
        }
        toggle.pointer("/toggledServiceEndpoint/likeEndpoint").and_then(|_| toggle["isToggled"].as_bool())
    })
}

pub(crate) async fn set_saved(api: &InnerTube, library: &Library, uri: &str, saved: bool) -> Result<()> {
    require_sign_in(api)?;
    let like = if saved { "like/like" } else { "like/removelike" };
    if let Some(video) = uri.strip_prefix(TRACK) {
        api.call(like, json!({ "target": { "videoId": video } })).await?;
    } else if uri.starts_with(ALBUM) {
        let known = library.album_playlists.lock().unwrap().get(uri).cloned();
        let playlist = match known {
            Some(playlist) => playlist,
            None => {
                let (_, playlist) = pages::album_info(api, uri).await?;
                let playlist = playlist.context("This album can't be saved")?;
                library.remember_album(uri, Some(playlist.clone()));
                playlist
            }
        };
        api.call(like, json!({ "target": { "playlistId": playlist } })).await?;
        library.forget(ALBUMS);
    } else if let Some(playlist) = uri.strip_prefix(PLAYLIST) {
        if playlist == LIKED {
            bail!("Liked Music is always in the library");
        }
        api.call(like, json!({ "target": { "playlistId": playlist } })).await?;
        library.forget(PLAYLISTS);
    } else if uri.starts_with(ARTIST) {
        let channel = pages::artist_channel(api, uri).await?;
        let endpoint = if saved { "subscription/subscribe" } else { "subscription/unsubscribe" };
        api.call(endpoint, json!({ "channelIds": [channel] })).await?;
        library.forget(ARTISTS);
    } else {
        bail!("Can't save {uri} to the YouTube Music library");
    }
    Ok(())
}

pub(crate) async fn add_to_playlist(api: &InnerTube, playlist_uri: &str, uris: &[String]) -> Result<()> {
    require_sign_in(api)?;
    let playlist = parse::id(playlist_uri, PLAYLIST)?;
    let actions: Vec<Value> = uris
        .iter()
        .filter_map(|uri| uri.strip_prefix(TRACK))
        .map(|video| json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": video, "dedupeOption": "DEDUPE_OPTION_SKIP" }))
        .collect();
    if actions.is_empty() {
        return Ok(());
    }
    let response = api.call("browse/edit_playlist", json!({ "playlistId": playlist, "actions": actions })).await?;
    match response["status"].as_str() {
        Some("STATUS_SUCCEEDED") | None => Ok(()),
        Some(status) => bail!("YouTube Music didn't add the songs ({status})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_like_states_from_the_queue() {
        let entry = |id: &str, default_status: &str| {
            json!({ "content": { "playlistPanelVideoRenderer": { "videoId": id, "menu": { "menuRenderer": { "items": [
                { "menuNavigationItemRenderer": {} },
                { "toggleMenuServiceItemRenderer": {
                    "defaultServiceEndpoint": { "likeEndpoint": { "status": default_status, "target": { "videoId": id } } },
                    "toggledServiceEndpoint": { "likeEndpoint": { "status": "LIKE", "target": { "videoId": id } } }
                } }
            ] } } } } })
        };
        let page = json!({ "queueDatas": [entry("liked", "INDIFFERENT"), entry("not", "LIKE")] });
        assert_eq!(like_states(&page), [("liked".to_owned(), true), ("not".to_owned(), false)]);
    }

    #[test]
    fn signed_out_queue_entries_say_nothing() {
        let page: Value = serde_json::from_str(include_str!("../tests/fixtures/queue.json")).unwrap();
        assert!(like_states(&page).is_empty(), "the menu only offers to sign in");
    }

    #[test]
    fn reads_grids_of_the_library() {
        let page = json!({ "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": { "content": {
            "sectionListRenderer": { "contents": [{ "gridRenderer": {
                "items": [
                    { "musicTwoRowItemRenderer": { "title": { "runs": [{ "text": "New playlist" }] },
                        "navigationEndpoint": { "createPlaylistEndpoint": {} } } },
                    { "musicTwoRowItemRenderer": {
                        "title": { "runs": [{ "text": "Road trip" }] },
                        "subtitle": { "runs": [{ "text": "Playlist" }, { "text": " • " }, { "text": "Sam" }, { "text": " • " }, { "text": "12 songs" }] },
                        "navigationEndpoint": { "browseEndpoint": { "browseId": "VLPL123",
                            "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": {
                                "pageType": "MUSIC_PAGE_TYPE_PLAYLIST" } } } } } }
                ],
                "continuations": [{ "nextContinuationData": { "continuation": "next-page" } }]
            } }] } } } }] } } });
        let (items, token) = library_page(&page);
        assert_eq!(items.len(), 1, "the New playlist button isn't an entry");
        assert_eq!(items[0].uri, "ytmusic:playlist:PL123");
        assert_eq!(count(&items[0].subtitle), Some(12));
        assert_eq!(token.as_deref(), Some("next-page"));
    }

    #[test]
    fn reads_who_is_signed_in() {
        let page = json!({ "actions": [{ "openPopupAction": { "popup": { "multiPageMenuRenderer": {
            "header": { "activeAccountHeaderRenderer": {
                "accountName": { "runs": [{ "text": "Sam Doe" }] },
                "channelHandle": { "runs": [{ "text": "@samdoe" }] } } },
            "sections": [{ "multiPageMenuSectionRenderer": { "items": [{ "compactLinkRenderer": {
                "navigationEndpoint": { "browseEndpoint": { "browseId": "UCsam" } } } }] } }]
        } } } }] });
        let user = parse_me(&page).unwrap();
        assert_eq!((user.id.as_str(), user.name.as_str()), ("UCsam", "Sam Doe"));
    }

    #[test]
    fn knows_which_list_holds_what() {
        assert_eq!(list_of("ytmusic:album:MPREb_1"), Some(ALBUMS));
        assert_eq!(list_of("ytmusic:playlist:PL1"), Some(PLAYLISTS));
        assert_eq!(list_of("ytmusic:playlist:LM"), None);
        assert_eq!(list_of("ytmusic:artist:UC1"), Some(ARTISTS));
        assert_eq!(list_of("ytmusic:track:x"), None);
    }
}
