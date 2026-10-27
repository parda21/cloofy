//! Lyrics: YouTube Music's time-synced ones (only its Android app gets those), its plain ones,
//! then LRCLIB's.

use anyhow::Result;
use cloofy_core::lyrics::{self, LyricLine, Lyrics, LyricsQuery};
use serde_json::{Value, json};

use crate::{
    innertube::{ClientKind, InnerTube},
    parse::{TRACK, find, list, text},
    watch,
};

pub(crate) async fn lyrics(api: &InnerTube, http: &reqwest::Client, query: &LyricsQuery<'_>) -> Result<Option<Lyrics>> {
    if let Some(video) = query.uri.strip_prefix(TRACK) {
        match youtube(api, video).await {
            Ok(Some(lyrics)) => return Ok(Some(lyrics)),
            Ok(None) => {}
            Err(err) => tracing::debug!("YouTube Music lyrics: {err:#}"),
        }
    }
    match lyrics::lrclib(http, query).await {
        Ok(lyrics) => Ok(lyrics),
        // LRCLIB answers "not found" with 503 at times.
        Err(err)
            if err.downcast_ref::<reqwest::Error>().and_then(reqwest::Error::status)
                == Some(reqwest::StatusCode::SERVICE_UNAVAILABLE) =>
        {
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

async fn youtube(api: &InnerTube, video: &str) -> Result<Option<Lyrics>> {
    let watch = watch::next(api, Some(video), None).await?;
    let Some(browse_id) = watch.lyrics else { return Ok(None) };
    let body = json!({ "browseId": browse_id });
    match api.call_as(ClientKind::Android, "browse", body.clone()).await {
        Ok(page) => {
            if let Some(lyrics) = parse_timed(&page) {
                return Ok(Some(lyrics));
            }
        }
        Err(err) => tracing::debug!("time-synced lyrics: {err:#}"),
    }
    Ok(parse_plain(&api.call("browse", body).await?))
}

fn parse_timed(page: &Value) -> Option<Lyrics> {
    let data = find(page, "timedLyricsModel")?.get("lyricsData")?;
    let lines: Vec<LyricLine> = list(&data["timedLyricsData"])
        .iter()
        .filter_map(|line| {
            let start = &line["cueRange"]["startTimeMilliseconds"];
            let start_ms = start.as_str().and_then(|s| s.parse().ok()).or(start.as_u64().map(|n| n as u32))?;
            Some(LyricLine { start_ms, text: line["lyricLine"].as_str()?.to_owned() })
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(Lyrics { synced: true, lines, provider: provider(data["sourceMessage"].as_str().unwrap_or_default()) })
}

fn parse_plain(page: &Value) -> Option<Lyrics> {
    let shelf = find(page, "musicDescriptionShelfRenderer")?;
    let body = text(&shelf["description"]);
    if body.trim().is_empty() {
        return None;
    }
    let lines =
        body.lines().map(|line| LyricLine { start_ms: 0, text: line.trim_end_matches('\r').to_owned() }).collect();
    Some(Lyrics { synced: false, lines, provider: provider(&text(&shelf["footer"])) })
}

/// `Source: LyricFind` → `LyricFind`.
fn provider(source: &str) -> String {
    let name = source.strip_prefix("Source:").unwrap_or(source).trim();
    if name.is_empty() { "YouTube Music".into() } else { name.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn parses_time_synced_lyrics() {
        let lyrics = parse_timed(&fixture("lyrics_timed")).unwrap();
        assert!(lyrics.synced);
        assert_eq!(lyrics.provider, "LyricFind");
        assert_eq!(lyrics.lines[1].text, "Ayy");
        assert_eq!(lyrics.lines[1].start_ms, 15_200);
        assert!(lyrics.lines.windows(2).all(|w| w[0].start_ms <= w[1].start_ms));
    }

    #[test]
    fn parses_plain_lyrics() {
        let lyrics = parse_plain(&fixture("lyrics_plain")).unwrap();
        assert!(!lyrics.synced);
        assert_eq!(lyrics.provider, "LyricFind");
        assert_eq!(lyrics.lines[0].text, "Ayy");
        assert!(lyrics.lines.iter().all(|line| !line.text.ends_with('\r')));
    }

    #[test]
    fn no_lyrics_is_none() {
        assert!(parse_plain(&fixture("lyrics_none")).is_none());
        assert!(parse_timed(&fixture("lyrics_none")).is_none());
    }
}
