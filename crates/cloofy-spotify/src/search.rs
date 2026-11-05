//! Playlist search through the internal API, which also finds the user's mixes.

use anyhow::{Context, Result};
use cloofy_core::catalog::PlaylistInfo;
use serde_json::{Value, json};

use crate::{parse, pathfinder::Pathfinder};

const LIMIT: u32 = 10;

pub async fn playlists(pathfinder: &Pathfinder, query: &str) -> Result<Vec<PlaylistInfo>> {
    let variables = json!({
        "searchTerm": query,
        "offset": 0,
        "limit": LIMIT,
        "numberOfTopResults": 5,
        "includeAudiobooks": false,
        "includePreReleases": false,
        "includeAuthors": false,
        "includeEpisodeContentRatingsV2": false,
    });
    parse(&pathfinder.query("searchPlaylists", variables).await?)
}

fn parse(data: &Value) -> Result<Vec<PlaylistInfo>> {
    let items = data.pointer("/searchV2/playlists/items").context("Search: unexpected response")?;
    Ok(items.as_array().into_iter().flatten().filter_map(|item| item.get("data")).filter_map(parse::playlist).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_users_mixes() {
        let data = json!({ "searchV2": { "playlists": { "totalCount": 201, "items": [
            { "__typename": "PlaylistResponseWrapper", "data": {
                "__typename": "Playlist", "uri": "spotify:playlist:37i9dQZF1EIhdusJHpCNoA", "name": "Synthwave Mix",
                "ownerV2": { "data": { "name": "Spotify", "uri": "spotify:user:spotify" } } } },
            { "__typename": "PlaylistResponseWrapper", "data": { "__typename": "NotFound" } },
            { "__typename": "PlaylistResponseWrapper", "data": {
                "__typename": "Playlist", "uri": "spotify:playlist:7gh5NAVKOqUZ8fPrM5NvXh", "name": "Synthwave Night Drive",
                "ownerV2": { "data": { "name": "Kurate Music", "uri": "spotify:user:kurate" } } } }
        ] } } });
        let names: Vec<_> = parse(&data).unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Synthwave Mix", "Synthwave Night Drive"]);
        assert!(parse(&json!({ "searchV2": {} })).is_err());
    }
}
