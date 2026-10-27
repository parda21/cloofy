//! Song lyrics: Spotify's own (via librespot), with LRCLIB as fallback.

use anyhow::Result;
use cloofy_core::lyrics::{self, LyricLine, Lyrics, LyricsQuery};
use librespot::{
    core::{SpotifyUri, session::Session},
    metadata::{Lyrics as SpotifyLyrics, lyrics::SyncType},
};

/// `None` when neither source has lyrics for the track.
pub async fn fetch(
    session: Option<&Session>,
    http: &reqwest::Client,
    query: LyricsQuery<'_>,
) -> Result<Option<Lyrics>> {
    if let Some(session) = session
        && let Ok(Some(lyrics)) = spotify(session, query.uri).await
    {
        return Ok(Some(lyrics));
    }
    lyrics::lrclib(http, &query).await
}

async fn spotify(session: &Session, uri: &str) -> Result<Option<Lyrics>> {
    let SpotifyUri::Track { id } = SpotifyUri::from_uri(uri)? else {
        return Ok(None);
    };
    let lyrics = SpotifyLyrics::get(session, &id).await?.lyrics;
    if lyrics.lines.is_empty() {
        return Ok(None);
    }
    Ok(Some(Lyrics {
        synced: lyrics.sync_type == SyncType::LineSynced,
        lines: lyrics
            .lines
            .into_iter()
            .map(|line| LyricLine { start_ms: line.start_time_ms.parse().unwrap_or(0), text: line.words })
            .collect(),
        provider: lyrics.provider_display_name,
    }))
}
