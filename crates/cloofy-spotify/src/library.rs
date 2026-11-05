//! The user's playlists as Spotify's apps list them, mixes included.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use cloofy_core::catalog::PlaylistInfo;
use futures::{StreamExt, TryStreamExt, stream};
use serde_json::{Value, json};

use crate::{catalog::PAGE_CONCURRENCY, parse, pathfinder::Pathfinder};

const PAGE: u32 = 500;

pub async fn playlists(pathfinder: &Pathfinder) -> Result<Vec<PlaylistInfo>> {
    let first = page(pathfinder, 0).await?;
    // Spotify may send fewer than asked for.
    let offsets: Vec<u32> = match first.entries {
        0 => Vec::new(),
        step => (step..first.total).step_by(step as usize).collect(),
    };
    let rest: Vec<Page> = stream::iter(offsets)
        .map(|offset| async move { page(pathfinder, offset).await })
        .buffered(PAGE_CONCURRENCY)
        .try_collect()
        .await?;
    Ok(std::iter::once(first).chain(rest).flat_map(|page| page.playlists).collect())
}

/// Adds the Web API's details, like song counts, to `found`. Also returns the Web API's
/// playlists that `found` lacks.
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
    /// Folders and unavailable playlists included.
    total: u32,
    entries: u32,
    playlists: Vec<PlaylistInfo>,
}

async fn page(pathfinder: &Pathfinder, offset: u32) -> Result<Page> {
    let variables = json!({
        "filters": ["Playlists"],
        "order": null,
        "textFilter": null,
        // Leaves out Liked Songs and Your Episodes.
        "features": [],
        "limit": PAGE,
        "offset": offset,
        "flatten": true,
        "includeFoldersWhenFlattening": true,
    });
    parse(&pathfinder.query("libraryV3", variables).await?)
}

fn parse(data: &Value) -> Result<Page> {
    let library = data.pointer("/me/libraryV3").context("Library: unexpected response")?;
    if library["__typename"] != "LibraryPage" {
        bail!("Spotify did not return the library ({})", library["__typename"]);
    }
    let entries = library["items"].as_array().context("Library: no entries")?;
    let total = library["totalCount"].as_u64().context("Library: no count")?;
    let playlists =
        entries.iter().filter_map(|entry| entry.pointer("/item/data")).filter_map(parse::playlist).collect();
    Ok(Page { total: total as u32, entries: entries.len() as u32, playlists })
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
        assert_eq!((page.total, page.entries), (113, 4));
        let names: Vec<_> = page.playlists.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Chill Mix", "electronic"]);
        assert_eq!(page.playlists[0].owner_id, "spotify");
        assert_eq!(page.playlists[1].image_url.as_deref(), Some("cover of electronic"));
    }

    #[test]
    fn says_when_spotify_answers_with_something_else() {
        let data = json!({ "me": { "libraryV3": { "__typename": "LibraryInvalidFilterIdError" } } });
        assert!(parse(&data).is_err());
        let without_count = json!({ "me": { "libraryV3": { "__typename": "LibraryPage", "items": [] } } });
        assert!(parse(&without_count).is_err());
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
