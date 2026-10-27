//! Probes which Spotify features are reachable, and through which path.

use anyhow::Result;
use cloofy_core::doctor::timed;
pub use cloofy_core::doctor::{Check, Expect, Outcome, Section};
use librespot::{
    core::{SpotifyId, SpotifyUri, session::Session},
    metadata::{Artist, Lyrics, Metadata, Playlist, Track},
    protocol::playlist4_external::SelectedListContent,
};
use protobuf::Message;
use serde_json::Value;

use crate::webapi::WebApi;

const ARTIST: &str = "4tZwfgrHOc3mvqYlEYSvVi"; // Daft Punk
const ALBUM: &str = "2noRn2Aes5aoNVsU6iWThc"; // Discovery
const TRACK: &str = "0DiWol3AO6WpXZgp0goxAV"; // One More Time
const EDITORIAL_PLAYLIST: &str = "37i9dQZF1DXcBWIGoYBM5M"; // Today's Top Hits

/// Turns a successful response body into the detail shown next to a check.
type Detail = fn(&Value) -> String;

pub async fn web_api(api: &WebApi) -> Section {
    use Expect::{Available, Removed};

    let mut checks = Vec::new();
    let me = api.get("/me").await;
    let my_id = me.as_ref().ok().and_then(|r| r.body["id"].as_str()).unwrap_or_default().to_owned();
    checks.push(check_response("Profile", Available, me, |b| {
        format!("{} ({})", text(b, "/display_name"), text(b, "/id"))
    }));

    let probes: [(&str, String, Expect, Detail); 23] = [
        ("Your playlists", "/me/playlists?limit=1".into(), Available, total),
        ("Liked Songs", "/me/tracks?limit=1".into(), Available, total),
        ("Saved albums", "/me/albums?limit=1".into(), Available, total),
        ("Followed artists", "/me/following?type=artist&limit=1".into(), Available, |b| {
            format!("{} artists", number(b, "/artists/total"))
        }),
        ("Podcasts", "/me/shows?limit=1".into(), Available, total),
        ("Audiobooks", "/me/audiobooks?limit=1".into(), Available, total),
        ("Top songs", "/me/top/tracks?limit=1".into(), Available, |b| format!("#1 {}", text(b, "/items/0/name"))),
        ("Recently played", "/me/player/recently-played?limit=1".into(), Available, |b| text(b, "/items/0/track/name")),
        ("Devices", "/me/player/devices".into(), Available, |b| {
            let names: Vec<_> =
                b["devices"].as_array().into_iter().flatten().filter_map(|d| d["name"].as_str()).collect();
            if names.is_empty() { "none active".into() } else { names.join(", ") }
        }),
        ("Playback state", "/me/player".into(), Available, |b| {
            if b.is_null() { "nothing active".into() } else { text(b, "/item/name") }
        }),
        ("Queue", "/me/player/queue".into(), Available, |b| {
            format!("{} items", b["queue"].as_array().map_or(0, Vec::len))
        }),
        ("Search (10 results)", search_path(10), Available, |b| {
            ["tracks", "artists", "albums", "playlists", "shows"]
                .iter()
                .map(|kind| format!("{} {kind}", b[kind]["items"].as_array().map_or(0, Vec::len)))
                .collect::<Vec<_>>()
                .join(" · ")
        }),
        ("Search (50 results)", search_path(50), Removed, |_| "50 allowed".into()),
        ("Library check (/me/library)", format!("/me/library/contains?uris=spotify:track:{TRACK}"), Available, |b| {
            format!("saved: {}", b[0])
        }),
        ("Artist", format!("/artists/{ARTIST}"), Available, |b| text(b, "/name")),
        ("Artist discography", format!("/artists/{ARTIST}/albums?limit=5"), Available, total),
        ("Album", format!("/albums/{ALBUM}"), Available, |b| {
            format!("{} · {} songs", text(b, "/name"), number(b, "/total_tracks"))
        }),
        ("Artist top tracks", format!("/artists/{ARTIST}/top-tracks"), Removed, |_| String::new()),
        ("Several tracks at once", format!("/tracks?ids={TRACK}"), Removed, |_| String::new()),
        ("New releases", "/browse/new-releases?limit=1".into(), Removed, |_| String::new()),
        ("Categories", "/browse/categories?limit=1".into(), Removed, |_| String::new()),
        ("Recommendations", format!("/recommendations?seed_artists={ARTIST}&limit=1"), Removed, |_| String::new()),
        ("Related artists", format!("/artists/{ARTIST}/related-artists"), Removed, |_| String::new()),
    ];
    for (name, path, expect, detail) in probes {
        checks.push(check_response(name, expect, api.get(&path).await, detail));
    }
    checks.push(check_response("Audio features", Removed, api.get(&format!("/audio-features/{TRACK}")).await, |_| {
        String::new()
    }));

    let (own, foreign) = pick_playlists(api, &my_id).await;
    if let Some(id) = own {
        let response = api.get(&format!("/playlists/{id}/items?limit=1")).await;
        checks.push(check_response("Your playlist: songs", Available, response, total));
    }
    if let Some(id) = foreign {
        checks.push(playlist_contents(api, "Followed playlist: songs", &id).await);
    }
    checks.push(playlist_contents(api, "Spotify playlist: songs", EDITORIAL_PLAYLIST).await);

    Section { title: "WEB API · your app", checks }
}

pub async fn spclient(session: &Session) -> Section {
    let mut checks = vec![Check {
        name: "Session".into(),
        expect: Expect::Available,
        outcome: Outcome::Ok(format!("{} · {}", session.username(), session.country())),
    }];

    checks.push(
        timed("Your playlists (rootlist)", async {
            let bytes = session.spclient().get_rootlist(0, Some(500)).await?;
            let list = SelectedListContent::parse_from_bytes(&bytes)?;
            Ok(format!("{} Playlists", list.length()))
        })
        .await,
    );

    checks.push(
        timed("Spotify playlist: songs", async {
            let uri = SpotifyUri::from_uri(&format!("spotify:playlist:{EDITORIAL_PLAYLIST}"))?;
            let playlist = Playlist::get(session, &uri).await?;
            Ok(format!("{} · {} songs", playlist.name(), playlist.tracks().len()))
        })
        .await,
    );

    checks.push(
        timed("Artist top tracks", async {
            let uri = SpotifyUri::from_uri(&format!("spotify:artist:{ARTIST}"))?;
            let artist = Artist::get(session, &uri).await?;
            let country = session.country();
            let top = artist.top_tracks.iter().find(|t| t.country == country).or_else(|| artist.top_tracks.first());
            Ok(format!("{} · {} top tracks", artist.name, top.map_or(0, |t| t.tracks.len())))
        })
        .await,
    );

    checks.push(
        timed("Track metadata", async {
            let uri = SpotifyUri::from_uri(&format!("spotify:track:{TRACK}"))?;
            let track = Track::get(session, &uri).await?;
            let artist = track.artists.first().map(|a| a.name.as_str()).unwrap_or("?");
            Ok(format!("{} – {artist}", track.name))
        })
        .await,
    );

    checks.push(
        timed("Song radio", async {
            let uri = SpotifyUri::from_uri(&format!("spotify:track:{TRACK}"))?;
            let bytes = session.spclient().get_radio_for_track(&uri).await?;
            let json: Value = serde_json::from_slice(&bytes)?;
            Ok(text(&json, "/mediaItems/0/uri"))
        })
        .await,
    );

    checks.push(
        timed("Resolve context", async {
            let context = session.spclient().get_context(&format!("spotify:playlist:{EDITORIAL_PLAYLIST}")).await?;
            let tracks: usize = context.pages.iter().map(|p| p.tracks.len()).sum();
            Ok(format!("{} pages · {tracks} songs", context.pages.len()))
        })
        .await,
    );

    checks.push(
        timed("Lyrics (Spotify)", async {
            let lyrics = Lyrics::get(session, &SpotifyId::from_base62(TRACK)?).await?;
            Ok(format!(
                "{} lines · {:?} · {}",
                lyrics.lyrics.lines.len(),
                lyrics.lyrics.sync_type,
                lyrics.lyrics.provider_display_name
            ))
        })
        .await,
    );

    Section { title: "LIBRESPOT · Spotify internal", checks }
}

fn check_response(name: &str, expect: Expect, response: Result<crate::webapi::ApiResponse>, detail: Detail) -> Check {
    let outcome = match response {
        Ok(r) if r.status.is_success() => Outcome::Ok(detail(&r.body)),
        Ok(r) => Outcome::Failed(r.error_text()),
        Err(err) => Outcome::Failed(format!("{err:#}")),
    };
    Check { name: name.into(), expect, outcome }
}

/// Development Mode apps only get playlist contents for playlists the user owns or collaborates on.
async fn playlist_contents(api: &WebApi, name: &str, id: &str) -> Check {
    let outcome = match api.get(&format!("/playlists/{id}")).await {
        Ok(r) if r.status.is_success() => {
            let items = r.body.get("items").or_else(|| r.body.get("tracks"));
            match items.and_then(|i| i["total"].as_u64()) {
                Some(total) => Outcome::Ok(format!("{} · {total} songs", text(&r.body, "/name"))),
                None => Outcome::Failed(format!("{}: metadata only, no songs", text(&r.body, "/name"))),
            }
        }
        Ok(r) => Outcome::Failed(r.error_text()),
        Err(err) => Outcome::Failed(format!("{err:#}")),
    };
    Check { name: name.into(), expect: Expect::Removed, outcome }
}

/// The first playlist the user owns, and the first one they only follow.
async fn pick_playlists(api: &WebApi, my_id: &str) -> (Option<String>, Option<String>) {
    let Ok(response) = api.get("/me/playlists?limit=50").await else {
        return (None, None);
    };
    let playlists = response.body["items"].as_array().cloned().unwrap_or_default();
    let find = |owned: bool| {
        playlists
            .iter()
            .find(|p| (p["owner"]["id"].as_str() == Some(my_id)) == owned)
            .and_then(|p| p["id"].as_str())
            .map(str::to_owned)
    };
    (find(true), find(false))
}

fn search_path(limit: u8) -> String {
    format!("/search?q=daft%20punk&type=track,artist,album,playlist,show&limit={limit}")
}

fn total(body: &Value) -> String {
    format!("{} items", number(body, "/total"))
}

fn number(body: &Value, pointer: &str) -> String {
    body.pointer(pointer).and_then(Value::as_u64).map_or_else(|| "?".into(), |n| n.to_string())
}

fn text(body: &Value, pointer: &str) -> String {
    body.pointer(pointer).and_then(Value::as_str).unwrap_or("?").to_owned()
}
