//! YouTube Music as it answers today, signed out. These go online, so ordinary runs leave them
//! out; the nightly workflow runs every ignored test of this crate, to notice when InnerTube
//! changes before people do.

use cloofy_core::{lyrics::LyricsQuery, paths::Paths};
use cloofy_ytmusic::{Source, YtMusic};
use futures::StreamExt;

fn client(name: &str) -> YtMusic {
    let root = std::env::temp_dir().join(format!("cloofy-ytm-live-{name}-{}", std::process::id()));
    let paths =
        Paths { config: root.join("config"), data: root.join("data"), cache: root.join("cache"), keychain: false };
    YtMusic::new(&paths).unwrap()
}

#[tokio::test]
#[ignore = "online"]
async fn search_finds_songs_albums_and_artists() {
    let results = client("search").search("daft punk").await.unwrap();
    let song = results.tracks.first().expect("songs");
    assert!(song.uri.starts_with("ytmusic:track:"), "{song:?}");
    assert!(!song.name.is_empty() && !song.artists.is_empty() && song.duration_ms > 0, "{song:?}");
    assert!(results.albums.iter().any(|a| a.uri.starts_with("ytmusic:album:")), "{:?}", results.albums);
    assert!(results.artists.iter().any(|a| a.uri.starts_with("ytmusic:artist:")), "{:?}", results.artists);
}

#[tokio::test]
#[ignore = "online"]
async fn an_album_lists_its_songs() {
    let client = client("album");
    let results = client.search("daft punk discovery").await.unwrap();
    let album = results.albums.first().expect("albums");
    let info = client.album_info(&album.uri).await.unwrap();
    assert!(!info.name.is_empty(), "{info:?}");
    let page = client.tracks(Source::Album(album.uri.clone())).boxed().next().await.expect("a page").unwrap();
    assert!(!page.items.is_empty(), "{album:?}");
    assert!(page.items.iter().all(|t| t.uri.starts_with("ytmusic:track:") && t.duration_ms > 0), "{:?}", page.items);
}

#[tokio::test]
#[ignore = "online"]
async fn an_artist_has_top_songs() {
    let client = client("artist");
    let results = client.search("daft punk").await.unwrap();
    let artist = results.artists.first().expect("artists");
    let page = client.artist(&artist.uri).await.unwrap();
    assert!(!page.name.is_empty() && !page.top_tracks.is_empty(), "{page:?}");
}

#[tokio::test]
#[ignore = "online"]
async fn a_song_has_a_radio_and_lyrics() {
    let client = client("radio");
    let results = client.search("around the world daft punk").await.unwrap();
    let song = results.tracks.first().expect("songs");
    let radio = client.radio(&song.uri).await.unwrap();
    assert!(radio.len() > 5, "{radio:?}");
    let found = client.tracks_by_uri(std::slice::from_ref(&song.uri)).await.unwrap();
    assert_eq!(found.first().map(|t| &t.uri), Some(&song.uri));
    let query = LyricsQuery {
        uri: &song.uri,
        title: &song.name,
        artist: &song.artists[0],
        album: &song.album,
        duration_ms: song.duration_ms,
    };
    let lyrics = client.lyrics(&query).await.unwrap().expect("lyrics for Around the World");
    assert!(!lyrics.lines.is_empty());
}

#[tokio::test]
#[ignore = "online"]
async fn home_and_suggestions_answer_signed_out() {
    let client = client("home");
    let home = client.home(None).await.unwrap();
    assert!(home.shelves.iter().any(|shelf| !shelf.items.is_empty()), "{home:?}");
    let suggestions = client.suggestions("daft p").await.unwrap();
    assert!(!suggestions.is_empty());
}
