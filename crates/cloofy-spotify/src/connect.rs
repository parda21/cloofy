//! Spotify Connect through the Web API: list devices, see what plays where, and control
//! playback on other devices (phone, speakers, web player).

use anyhow::Result;
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::webapi::WebApi;

#[derive(Debug, Clone)]
pub struct Device {
    pub id: String,
    pub name: String,
    /// Spotify's device type, e.g. "Computer", "Smartphone", "Speaker".
    pub kind: String,
    pub active: bool,
    pub volume_percent: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Playback {
    pub device: Device,
    pub playing: bool,
    pub progress_ms: u32,
    pub shuffle: bool,
    /// "off", "context" or "track".
    pub repeat: String,
    pub track: Option<PlayingTrack>,
    /// The playlist, album or artist being played, if any.
    pub context: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PlayingTrack {
    pub uri: String,
    pub name: String,
    pub artists: Vec<String>,
    pub artist_uris: Vec<String>,
    pub album: String,
    pub album_uri: Option<String>,
    pub duration_ms: u32,
    pub cover_url: Option<String>,
    pub cover_url_large: Option<String>,
}

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
