//! Spotify Connect through the Web API: list devices, see what plays where, and control
//! playback on other devices (phone, speakers, web player). What the Web API can't do with
//! the queue goes to the device through Spotify Connect itself, sent as cloofy's own device.

use anyhow::Result;
use cloofy_core::connect::{Device, Playback, PlayingTrack};
use librespot::{core::session::Session, protocol::player::ProvidedTrack};
use protobuf_json_mapping::PrintOptions;
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::webapi::WebApi;

#[derive(Debug, Clone)]
pub enum Command {
    Play,
    Pause,
    Next,
    Previous,
    Seek(u32),
    Volume(u8),
    Shuffle(bool),
    /// "off", "context" or "track".
    Repeat(&'static str),
    /// Starts a context on the active device, optionally at a given track or shuffled.
    PlayContext {
        context: String,
        track: Option<String>,
        shuffle: bool,
    },
}

pub async fn devices(api: &WebApi) -> Result<Vec<Device>> {
    let raw: RawDevices = api.get_json("/me/player/devices").await?;
    Ok(raw.devices.into_iter().map(Device::from).collect())
}

/// What plays where; `None` when no device is active.
pub async fn playback(api: &WebApi) -> Result<Option<Playback>> {
    let response = api.get("/me/player").await?;
    if response.body.is_null() {
        return Ok(None);
    }
    if !response.status.is_success() {
        anyhow::bail!("Spotify: {}", response.error_text());
    }
    let raw: RawPlayback = serde_json::from_value(response.body)?;
    Ok(Some(Playback {
        device: raw.device.into(),
        playing: raw.is_playing,
        progress_ms: raw.progress_ms.unwrap_or(0),
        shuffle: raw.shuffle_state,
        repeat: raw.repeat_state,
        track: raw.item.and_then(playing_track),
        context: raw.context.and_then(|c| c["uri"].as_str().map(str::to_owned)),
    }))
}

/// Moves playback to `device_id` and keeps it playing.
pub async fn transfer(api: &WebApi, device_id: &str) -> Result<()> {
    let body = json!({ "device_ids": [device_id], "play": true });
    api.call(Method::PUT, "/me/player", Some(&body)).await
}

pub async fn send(api: &WebApi, command: Command) -> Result<()> {
    let (method, path, body) = match command {
        Command::Play => (Method::PUT, "/me/player/play".to_owned(), None),
        Command::Pause => (Method::PUT, "/me/player/pause".to_owned(), None),
        Command::Next => (Method::POST, "/me/player/next".to_owned(), None),
        Command::Previous => (Method::POST, "/me/player/previous".to_owned(), None),
        Command::Seek(ms) => (Method::PUT, format!("/me/player/seek?position_ms={ms}"), None),
        Command::Volume(percent) => (Method::PUT, format!("/me/player/volume?volume_percent={percent}"), None),
        Command::Shuffle(on) => (Method::PUT, format!("/me/player/shuffle?state={on}"), None),
        Command::Repeat(mode) => (Method::PUT, format!("/me/player/repeat?state={mode}"), None),
        Command::PlayContext { context, track, shuffle } => {
            if shuffle {
                api.call(Method::PUT, "/me/player/shuffle?state=true", None).await?;
            }
            let mut body = json!({ "context_uri": context });
            if let Some(track) = track {
                body["offset"] = json!({ "uri": track });
            }
            (Method::PUT, "/me/player/play".to_owned(), Some(body))
        }
    };
    api.call(method, &path, body.as_ref()).await
}

/// Skips ahead to a track that plays later on `device`, as a click in Spotify's queue does.
/// `uid` tells apart entries of the same track; empty where unknown.
pub async fn skip_to(session: &Session, device: &str, uri: &str, uid: &str, provider: &str) -> Result<()> {
    let track = json!({ "uri": uri, "uid": uid, "provider": provider });
    command(session, device, json!({ "endpoint": "skip_next", "track": track, "logging_params": {} })).await
}

/// Replaces what plays next on `device`, as removing a track from Spotify's queue does. The
/// tracks go back as the device listed them, under the revision it listed them in.
pub async fn set_queue(
    session: &Session,
    device: &str,
    next: &[ProvidedTrack],
    previous: &[ProvidedTrack],
    revision: &str,
) -> Result<()> {
    let tracks = |tracks: &[ProvidedTrack]| tracks.iter().map(provided_track_json).collect::<Result<Vec<_>>>();
    let command = json!({
        "endpoint": "set_queue",
        "next_tracks": tracks(next)?,
        "prev_tracks": tracks(previous)?,
        "queue_revision": revision,
        "logging_params": {},
    });
    self::command(session, device, command).await
}

async fn command(session: &Session, device: &str, command: Value) -> Result<()> {
    let endpoint = format!("/connect-state/v1/player/command/from/{}/to/{device}", session.device_id());
    let body = json!({ "command": command }).to_string();
    session.spclient().request(&Method::POST, &endpoint, None, Some(body.as_bytes())).await?;
    Ok(())
}

/// A track in Spotify's own JSON form, field names as in its protocol.
fn provided_track_json(track: &ProvidedTrack) -> Result<Value> {
    let options = PrintOptions { proto_field_name: true, ..PrintOptions::default() };
    Ok(serde_json::from_str(&protobuf_json_mapping::print_to_string_with_options(track, &options)?)?)
}

fn playing_track(item: Value) -> Option<PlayingTrack> {
    let text = |pointer: &str| item.pointer(pointer).and_then(Value::as_str).unwrap_or_default().to_owned();
    let images: Vec<(u64, String)> = item
        .pointer("/album/images")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|i| Some((i["width"].as_u64().unwrap_or(300), i["url"].as_str()?.to_owned())))
        .collect();
    let cover =
        |preferred: u64| images.iter().min_by_key(|(width, _)| width.abs_diff(preferred)).map(|(_, url)| url.clone());
    let uri = text("/uri");
    (!uri.is_empty()).then(|| PlayingTrack {
        name: text("/name"),
        artists: item["artists"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| a["name"].as_str().map(str::to_owned))
            .collect(),
        artist_uris: item["artists"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| a["uri"].as_str().map(str::to_owned))
            .collect(),
        album: text("/album/name"),
        album_uri: Some(text("/album/uri")).filter(|uri| !uri.is_empty()),
        duration_ms: item["duration_ms"].as_u64().unwrap_or(0) as u32,
        cover_url: cover(300),
        cover_url_large: cover(640),
        uri,
    })
}

impl From<RawDevice> for Device {
    fn from(raw: RawDevice) -> Self {
        Self {
            id: raw.id.unwrap_or_default(),
            name: raw.name,
            kind: raw.kind,
            active: raw.is_active,
            volume_percent: raw.volume_percent,
        }
    }
}

#[derive(Deserialize)]
struct RawDevices {
    devices: Vec<RawDevice>,
}

#[derive(Deserialize)]
struct RawDevice {
    id: Option<String>,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    is_active: bool,
    volume_percent: Option<u32>,
}

#[derive(Deserialize)]
struct RawPlayback {
    device: RawDevice,
    #[serde(default)]
    is_playing: bool,
    progress_ms: Option<u32>,
    #[serde(default)]
    shuffle_state: bool,
    #[serde(default)]
    repeat_state: String,
    item: Option<Value>,
    context: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_go_back_with_their_metadata_and_field_names() {
        let mut track = ProvidedTrack {
            uri: "spotify:track:1".into(),
            uid: "q0".into(),
            provider: "queue".into(),
            album_uri: "spotify:album:1".into(),
            ..ProvidedTrack::default()
        };
        track.metadata.insert("is_queued".into(), "true".into());
        let json = provided_track_json(&track).unwrap();
        assert_eq!(json["uid"], "q0");
        assert_eq!(json["provider"], "queue");
        assert_eq!(json["album_uri"], "spotify:album:1");
        assert_eq!(json["metadata"]["is_queued"], "true");
    }
}
