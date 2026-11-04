//! Song lyrics, and LRCLIB, which has them for most songs of any service.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const LRCLIB: &str = "https://lrclib.net/api/get";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lyrics {
    /// Whether lines carry real timestamps.
    pub synced: bool,
    pub lines: Vec<LyricLine>,
    /// Who provides the text, e.g. "Musixmatch" or "LRCLIB".
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricLine {
    pub start_ms: u32,
    pub text: String,
}

impl Lyrics {
    /// Index of the line being sung at `position_ms`.
    pub fn current_line(&self, position_ms: u32) -> Option<usize> {
        if !self.synced {
            return None;
        }
        self.lines.iter().rposition(|line| line.start_ms <= position_ms)
    }
}

pub struct LyricsQuery<'a> {
    pub uri: &'a str,
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    pub duration_ms: u32,
}

/// Lyrics from LRCLIB, found by title, artist, album and length; `None` when it has none.
pub async fn lrclib(http: &reqwest::Client, query: &LyricsQuery<'_>) -> Result<Option<Lyrics>> {
    let duration = (query.duration_ms / 1000).to_string();
    let response = http
        .get(LRCLIB)
        .query(&[
            ("track_name", query.title),
            ("artist_name", query.artist),
            ("album_name", query.album),
            ("duration", &duration),
        ])
        .send()
        .await?;
    // LRCLIB answers "not found" with 503 at times.
    if matches!(response.status(), reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::SERVICE_UNAVAILABLE) {
        return Ok(None);
    }
    let body: Value = response.error_for_status()?.json().await?;
    let provider = "LRCLIB".to_owned();
    if let Some(lrc) = body["syncedLyrics"].as_str() {
        return Ok(Some(Lyrics { synced: true, lines: parse_lrc(lrc), provider }));
    }
    Ok(body["plainLyrics"].as_str().map(|text| Lyrics {
        synced: false,
        lines: text.lines().map(|line| LyricLine { start_ms: 0, text: line.to_owned() }).collect(),
        provider,
    }))
}

/// Parses `[mm:ss.xx] text` lines; a line may carry several timestamps.
fn parse_lrc(lrc: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for raw in lrc.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(tag) = rest.strip_prefix('[') {
            let Some((stamp, after)) = tag.split_once(']') else { break };
            if let Some(ms) = parse_timestamp(stamp) {
                stamps.push(ms);
            }
            rest = after;
        }
        lines.extend(stamps.into_iter().map(|start_ms| LyricLine { start_ms, text: rest.trim().to_owned() }));
    }
    lines.sort_by_key(|line| line.start_ms);
    lines
}

fn parse_timestamp(stamp: &str) -> Option<u32> {
    let (minutes, seconds) = stamp.split_once(':')?;
    let minutes: u32 = minutes.parse().ok()?;
    let seconds: f64 = seconds.parse().ok()?;
    Some(minutes * 60_000 + (seconds * 1000.0) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lrc_with_multiple_stamps() {
        let lines = parse_lrc("[ti:Song]\n[00:12.50]Hello\n[01:02.00][00:30.25]Again\n");
        let parsed: Vec<_> = lines.iter().map(|l| (l.start_ms, l.text.as_str())).collect();
        assert_eq!(parsed, [(12_500, "Hello"), (30_250, "Again"), (62_000, "Again")]);
    }

    #[test]
    fn finds_current_line() {
        let lyrics =
            Lyrics { synced: true, lines: parse_lrc("[00:01.00]a\n[00:05.00]b\n[00:09.00]c"), provider: String::new() };
        assert_eq!(lyrics.current_line(0), None);
        assert_eq!(lyrics.current_line(5_000), Some(1));
        assert_eq!(lyrics.current_line(60_000), Some(2));
    }
}
