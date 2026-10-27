//! What a link plays: music.youtube.com's and youtube.com's own, or a `ytmusic:` URI.

use anyhow::{Result, bail};
use url::Url;

use crate::parse::{ALBUM, ARTIST, PLAYLIST, TRACK};

/// What to play: a context, and the song in it to start at, if any. A song by itself is its
/// own context and plays with its radio after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playable {
    pub context: String,
    pub track: Option<String>,
}

/// What `input` plays: a `ytmusic:` URI, or a link such as
/// `https://music.youtube.com/watch?v=…` (with `&list=…`, the song in that playlist),
/// `/playlist?list=…`, `/browse/MPREb_…`, `/channel/UC…`, or a youtube.com or youtu.be link.
pub fn playable(input: &str) -> Result<Playable> {
    let input = input.trim();
    if [TRACK, ALBUM, PLAYLIST, ARTIST].iter().any(|prefix| input.strip_prefix(prefix).is_some_and(|id| !id.is_empty()))
    {
        return Ok(Playable { context: input.to_owned(), track: None });
    }
    let Some(url) = Url::parse(input).ok().filter(|url| matches!(url.scheme(), "https" | "http")) else {
        bail!("“{input}” is neither a ytmusic: URI nor a YouTube Music link");
    };
    let host = url.host_str().unwrap_or_default().trim_start_matches("www.").trim_start_matches("m.");
    let query = |name: &str| url.query_pairs().find(|(key, _)| key == name).map(|(_, value)| value.into_owned());
    let path: Vec<&str> = url.path_segments().into_iter().flatten().filter(|part| !part.is_empty()).collect();
    let video = match (host, path.as_slice()) {
        ("youtu.be", [video]) => Some((*video).to_owned()),
        ("music.youtube.com" | "youtube.com", ["watch"]) => query("v"),
        _ => None,
    };
    let playlist = query("list").filter(|list| !list.is_empty());
    let playable = match (video, playlist, path.as_slice()) {
        (Some(video), Some(list), _) => {
            Playable { context: format!("{PLAYLIST}{list}"), track: Some(format!("{TRACK}{video}")) }
        }
        (Some(video), None, _) => Playable { context: format!("{TRACK}{video}"), track: None },
        (None, Some(list), ["playlist"]) => Playable { context: format!("{PLAYLIST}{list}"), track: None },
        (None, _, ["browse", id]) if id.starts_with("MPREb_") => {
            Playable { context: format!("{ALBUM}{id}"), track: None }
        }
        (None, _, ["browse", id]) if id.starts_with("VL") => {
            Playable { context: format!("{PLAYLIST}{}", &id[2..]), track: None }
        }
        (None, _, ["channel" | "browse", id]) if id.starts_with("UC") => {
            Playable { context: format!("{ARTIST}{id}"), track: None }
        }
        _ => bail!("cloofy doesn't know what to play for {input}"),
    };
    Ok(playable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(input: &str) -> String {
        playable(input).unwrap().context
    }

    #[test]
    fn plays_songs_with_their_radio_and_pages_as_they_are() {
        let song = playable("https://music.youtube.com/watch?v=d7woYjGWs6k&si=abc").unwrap();
        assert_eq!(song, Playable { context: "ytmusic:track:d7woYjGWs6k".into(), track: None });
        assert_eq!(context("https://www.youtube.com/watch?v=dQw4w9WgXcQ"), "ytmusic:track:dQw4w9WgXcQ");
        assert_eq!(context("https://youtu.be/dQw4w9WgXcQ?t=10"), "ytmusic:track:dQw4w9WgXcQ");
        assert_eq!(context("https://music.youtube.com/playlist?list=PL123"), "ytmusic:playlist:PL123");
        assert_eq!(context("https://music.youtube.com/browse/MPREb_K8qWMWVqXGi"), "ytmusic:album:MPREb_K8qWMWVqXGi");
        assert_eq!(context("https://music.youtube.com/browse/VLPL123"), "ytmusic:playlist:PL123");
        assert_eq!(
            context("https://music.youtube.com/channel/UCRr1xG_2WIDs18a6cIiCxeA"),
            "ytmusic:artist:UCRr1xG_2WIDs18a6cIiCxeA"
        );
        assert_eq!(context("ytmusic:album:MPREb_1"), "ytmusic:album:MPREb_1");
    }

    #[test]
    fn a_song_in_a_playlist_plays_there() {
        let song = playable("https://music.youtube.com/watch?v=abc&list=OLAK5uy_x").unwrap();
        assert_eq!(song.context, "ytmusic:playlist:OLAK5uy_x");
        assert_eq!(song.track.as_deref(), Some("ytmusic:track:abc"));
    }

    #[test]
    fn says_what_it_cant_play() {
        assert!(playable("https://music.youtube.com/explore").is_err());
        assert!(playable("https://example.com/watch?v=x").is_err());
        assert!(playable("ytmusic:track:").is_err());
        assert!(playable("spotify:track:1").is_err());
    }
}
