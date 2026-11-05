//! The user's playlists as Spotify's own apps list them. Unlike the Web API's list, this one
//! includes the playlists Spotify made, like mixes, Daily Mixes and Discover Weekly, and keeps
//! the order picked in Spotify.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use cloofy_core::catalog::PlaylistInfo;
use futures::{StreamExt, TryStreamExt, stream};
use serde_json::{Value, json};

use crate::{catalog::PAGE_CONCURRENCY, parse, pathfinder::Pathfinder};

/// Library entries per request.
const PAGE: u32 = 50;

pub async fn playlists(pathfinder: &Pathfinder) -> Result<Vec<PlaylistInfo>> {
    let first = page(pathfinder, 0).await?;
    let offsets: Vec<u32> = (PAGE..first.total).step_by(PAGE as usize).collect();
    let rest: Vec<Page> = stream::iter(offsets)
        .map(|offset| async move { page(pathfinder, offset).await })
        .buffered(PAGE_CONCURRENCY)
        .try_collect()
        .await?;
    Ok(std::iter::once(first).chain(rest).flat_map(|page| page.playlists).collect())
}

/// `found` in its order, each with what the Web API tells about the same playlist, like how
/// many songs it has. Also returns the Web API's playlists that `found` lacks.
pub fn with_web_details(found: Vec<PlaylistInfo>, web: Vec<PlaylistInfo>) -> (Vec<PlaylistInfo>, Vec<PlaylistInfo>) {
    let positions: HashMap<String, usize> = web.iter().enumerate().map(|(at, p)| (p.uri.clone(), at)).collect();
    let mut web: Vec<Option<PlaylistInfo>> = web.into_iter().map(Some).collect();
    let merged = found
        .into_iter()
        .map(|playlist| match positions.get(&playlist.uri).and_then(|&at| web[at].take()) {
            Some(known) => PlaylistInfo { image_url: known.image_url.or(playlist.image_url), ..known },
            None => playlist,
        })
        .collect();
    (merged, web.into_iter().flatten().collect())
}

struct Page {
    /// Entries in the whole library, folders and unavailable playlists included.
    total: u32,
    playlists: Vec<PlaylistInfo>,
}

async fn page(pathfinder: &Pathfinder, offset: u32) -> Result<Page> {
    let variables = json!({
        "filters": ["Playlists"],
        // The order picked in Spotify.
        "order": null,
        "textFilter": null,
        // Liked Songs and Your Episodes would come first otherwise.
        "features": [],
        "limit": PAGE,
        "offset": offset,
        // Lists the playlists in folders as well.
        "flatten": true,
        "includeFoldersWhenFlattening": true,
    });
    parse(&pathfinder.query("libraryV3", variables).await?)
}

/// Folders and playlists that are no longer available are left out.
fn parse(data: &Value) -> Result<Page> {
    let library = data.pointer("/me/libraryV3").context("Library: unexpected response")?;
    if library["__typename"] != "LibraryPage" {
        bail!("Spotify did not return the library ({})", library["__typename"]);
    }
    let playlists = library["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.pointer("/item/data"))
        .filter_map(parse::playlist)
        .collect();
    Ok(Page { total: library["totalCount"].as_u64().unwrap_or_default() as u32, playlists })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(data: Value) -> Value {
        json!({ "depth": 0, "item": { "__typename": "PlaylistResponseWrapper", "data": data } })
    }

    fn playlist(uri: &str, name: &str, owner: &str) -> Value {
        json!({
            "__typename": "Playlist", "uri": uri, "name": name,
            "images": { "items": [{ "sources": [{ "url": format!("cover of {name}"), "width": null }] }] },
            "ownerV2": { "data": { "__typename": "User", "name": owner, "uri": format!("spotify:user:{}", owner.to_lowercase()) } }
        })
    }

    fn info(uri: &str, total: Option<u32>, image_url: Option<&str>) -> PlaylistInfo {
        PlaylistInfo {
            id: uri.rsplit(':').next().unwrap().into(),
            uri: uri.into(),
            name: uri.into(),
            owner_id: "me".into(),
            owner_name: "Me".into(),
            collaborative: false,
            total,
            image_url: image_url.map(Into::into),
        }
    }

    #[test]
    fn lists_mixes_and_skips_folders_and_lost_playlists() {
        let data = json!({ "me": { "libraryV3": {
            "__typename": "LibraryPage",
            "totalCount": 113,
            "items": [
                entry(playlist("spotify:playlist:37i9dQZF1EVHGWrwldPRtj", "Chill Mix", "Spotify")),
                { "depth": 0, "item": { "__typename": "LibraryFolderResponseWrapper", "data": {
                    "__typename": "Folder", "name": "Music", "uri": "spotify:user:me:folder:1" } } },
                entry(json!({ "__typename": "NotFound" })),
                entry(playlist("spotify:playlist:5nBb020NZX9LV8RQRNKGNw", "electronic", "Me")),
            ]
        } } });
        let page = parse(&data).unwrap();
        assert_eq!(page.total, 113);
        let names: Vec<_> = page.playlists.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Chill Mix", "electronic"]);
        assert_eq!(page.playlists[0].owner_id, "spotify");
        assert_eq!(page.playlists[1].image_url.as_deref(), Some("cover of electronic"));
    }

    #[test]
    fn says_when_spotify_answers_with_something_else() {
        let data = json!({ "me": { "libraryV3": { "__typename": "LibraryInvalidFilterIdError" } } });
        assert!(parse(&data).is_err());
    }

    #[test]
    fn takes_song_counts_from_the_web_api_and_keeps_the_order() {
        let found = vec![
            info("spotify:playlist:mix", None, Some("mix cover")),
            info("spotify:playlist:own", None, Some("own")),
        ];
        let web = vec![info("spotify:playlist:gone", Some(1), None), info("spotify:playlist:own", Some(9), None)];
        let (merged, rest) = with_web_details(found, web);
        let summary: Vec<_> = merged.iter().map(|p| (p.uri.as_str(), p.total, p.image_url.as_deref())).collect();
        assert_eq!(
            summary,
            [("spotify:playlist:mix", None, Some("mix cover")), ("spotify:playlist:own", Some(9), Some("own"))]
        );
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].uri, "spotify:playlist:gone");
    }
}
