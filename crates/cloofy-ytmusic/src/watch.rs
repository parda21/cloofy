//! The player's side of InnerTube: `next`, which lists what plays after a song (its radio, or
//! the rest of a mix), and `music/get_queue`, which has the details of any songs by ID.

use std::collections::HashMap;

use anyhow::Result;
use cloofy_core::catalog::Track;
use futures::{StreamExt, TryStreamExt, stream};
use serde_json::{Value, json};

use crate::{
    innertube::{self, InnerTube},
    parse::{self, TRACK, find, list, str_at, text},
};

/// Songs per `music/get_queue` request.
const QUEUE_BATCH: usize = 50;
const QUEUE_CONCURRENCY: usize = 4;

/// One page of `next`: songs, and what else the watch page links to.
#[derive(Debug, Default)]
pub(crate) struct Watch {
    pub tracks: Vec<Track>,
    /// The queue's title, e.g. "Daft Punk Mix".
    pub title: Option<String>,
    pub continuation: Option<String>,
    /// The browse ID of the first song's lyrics, when it has any.
    pub lyrics: Option<String>,
}

/// What plays with `video`, or the songs of the mix `playlist`.
pub(crate) async fn next(api: &InnerTube, video: Option<&str>, playlist: Option<&str>) -> Result<Watch> {
    let mut body = json!({
        "enablePersistentPlaylistPanel": true,
        "isAudioOnly": true,
        "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
    });
    if let Some(video) = video {
        body["videoId"] = video.into();
    }
    if let Some(playlist) = playlist {
        body["playlistId"] = playlist.into();
        body["params"] = "wAEB".into();
    }
    Ok(parse_next(&api.call("next", body).await?))
}

pub(crate) async fn next_page(api: &InnerTube, token: &str) -> Result<Watch> {
    let page = api.continuation("next", token).await?;
    let panel = page.pointer("/continuationContents/playlistPanelContinuation").unwrap_or(&Value::Null);
    Ok(Watch {
        tracks: list(&panel["contents"]).iter().filter_map(parse::panel_track).collect(),
        continuation: parse::continuation(panel),
        ..Watch::default()
    })
}

pub(crate) fn parse_next(page: &Value) -> Watch {
    let tabs = find(page, "watchNextTabbedResultsRenderer").map_or(&Value::Null, |r| &r["tabs"]);
    let queue = find(tabs, "musicQueueRenderer").unwrap_or(&Value::Null);
    let panel = find(queue, "playlistPanelRenderer").unwrap_or(&Value::Null);
    let lyrics = list(tabs).iter().find_map(|tab| {
        let tab = &tab["tabRenderer"];
        let id = str_at(tab, "/endpoint/browseEndpoint/browseId")?;
        (id.starts_with("MPLYt") && tab["unselectable"] != true).then(|| id.to_owned())
    });
    let title = find(queue, "musicQueueHeaderRenderer").map(|h| text(&h["subtitle"])).filter(|t| !t.is_empty());
    Watch {
        tracks: list(&panel["contents"]).iter().filter_map(parse::panel_track).collect(),
        title,
        continuation: parse::continuation(panel),
        lyrics,
    }
}

/// A song's radio: the song first, then about a hundred more.
pub(crate) async fn radio(api: &InnerTube, track_uri: &str) -> Result<Vec<Track>> {
    let video = parse::id(track_uri, TRACK)?;
    let first = next(api, Some(video), Some(&parse::radio_playlist(video))).await?;
    let mut tracks = first.tracks;
    if let Some(token) = first.continuation {
        match next_page(api, &token).await {
            Ok(more) => tracks.extend(more.tracks),
            Err(err) => tracing::warn!("more of the radio: {err:#}"),
        }
    }
    dedup(&mut tracks);
    Ok(tracks)
}

/// Leaves out songs listed before, which radios repeat now and then.
pub(crate) fn dedup(tracks: &mut Vec<Track>) {
    let mut seen = std::collections::HashSet::new();
    tracks.retain(|track| seen.insert(track.uri.clone()));
}

pub(crate) async fn tracks_by_uri(api: &InnerTube, uris: &[String]) -> Result<Vec<Track>> {
    let ids: Vec<&str> = uris.iter().filter_map(|uri| uri.strip_prefix(TRACK)).collect();
    let batches: Vec<Vec<String>> =
        ids.chunks(QUEUE_BATCH).map(|batch| batch.iter().map(|id| (*id).to_owned()).collect()).collect();
    let pages: Vec<Vec<Value>> =
        stream::iter(batches).map(|batch| queue(api, batch)).buffered(QUEUE_CONCURRENCY).try_collect().await?;
    let mut found: HashMap<String, Track> =
        pages.iter().flatten().flat_map(parse_queue).map(|track| (track.uri.clone(), track)).collect();
    Ok(uris.iter().filter_map(|uri| found.remove(uri)).collect())
}

/// The queue entries of a batch of songs. YouTube refuses a whole batch for one song it
/// doesn't know (a removed video, say); then the songs are asked for one by one.
async fn queue(api: &InnerTube, batch: Vec<String>) -> Result<Vec<Value>> {
    match api.call("music/get_queue", json!({ "videoIds": batch })).await {
        Ok(page) => Ok(vec![page]),
        Err(err) if batch.len() > 1 && innertube::status(&err) == Some(reqwest::StatusCode::NOT_FOUND) => {
            let single =
                |id: String| async move { api.call("music/get_queue", json!({ "videoIds": [id] })).await.ok() };
            Ok(stream::iter(batch)
                .map(single)
                .buffered(QUEUE_CONCURRENCY)
                .filter_map(|page| async { page })
                .collect()
                .await)
        }
        Err(err) if innertube::status(&err) == Some(reqwest::StatusCode::NOT_FOUND) => Ok(Vec::new()),
        Err(err) => Err(err),
    }
}

pub(crate) fn parse_queue(page: &Value) -> Vec<Track> {
    list(&page["queueDatas"]).iter().filter_map(|data| parse::panel_track(&data["content"])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn parses_a_radio() {
        let watch = parse_next(&fixture("next"));
        assert!(watch.tracks.len() >= 3);
        let first = &watch.tracks[0];
        assert_eq!(first.uri, "ytmusic:track:52fNscjkST4");
        assert_eq!(first.name, "Aerodynamic");
        assert_eq!(first.artists, ["Daft Punk"]);
        assert_eq!(first.album, "Discovery");
        assert_eq!(first.album_uri, "ytmusic:album:MPREb_7ltM34kr0mH");
        assert_eq!(first.duration_ms, 213_000);
        assert!(watch.continuation.is_some());
        assert!(watch.lyrics.as_deref().is_some_and(|id| id.starts_with("MPLYt_")));
    }

    #[test]
    fn a_mix_has_a_title() {
        let watch = parse_next(&fixture("next_mix"));
        assert_eq!(watch.title.as_deref(), Some("Daft Punk Mix"));
        assert!(!watch.tracks.is_empty());
    }

    #[test]
    fn parses_queue_details() {
        let tracks = parse_queue(&fixture("queue"));
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].uri, "ytmusic:track:plnfIj7dkJE");
        assert_eq!(tracks[0].artists, ["The Weeknd"]);
        assert!(tracks[0].image_url.as_deref().is_some_and(|url| url.contains("w300-h300")));
    }

    #[test]
    fn leaves_out_repeats() {
        let track = |uri: &str| Track {
            uri: uri.into(),
            name: String::new(),
            artists: Vec::new(),
            artist_uris: Vec::new(),
            album: String::new(),
            album_uri: String::new(),
            duration_ms: 0,
            image_url: None,
        };
        let mut tracks = vec![track("a"), track("b"), track("a")];
        dedup(&mut tracks);
        assert_eq!(tracks.iter().map(|t| t.uri.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    }
}
