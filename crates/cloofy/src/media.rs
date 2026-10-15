//! Media keys and the system's Now Playing for `cloofy play` and `cloofy connect`.

use std::{sync::Arc, time::Duration};

use anyhow::Result;
use cloofy_os::media::{Command, Controls, State, Track};
use librespot::{
    connect::Spirc,
    metadata::audio::{AudioItem, UniqueFields},
    playback::player::PlayerEvent,
};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub enum Event {
    Command(Command),
    Artwork { url: String, bytes: Arc<[u8]> },
}

/// Follows the player and shows it in the system's media controls, where there are any.
pub struct Media {
    controls: Option<Controls>,
    state: State,
    artwork_url: Option<String>,
    tx: UnboundedSender<Event>,
    http: reqwest::Client,
}

impl Media {
    /// The receiver yields requests from the system and loaded covers, for [`Media::on_event`].
    pub fn new() -> (Self, UnboundedReceiver<Event>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let controls = Controls::new({
            let tx = tx.clone();
            move |command| {
                let _ = tx.send(Event::Command(command));
            }
        });
        let media = Self { controls, state: State::default(), artwork_url: None, tx, http: reqwest::Client::new() };
        (media, rx)
    }

    pub fn on_player(&mut self, event: &PlayerEvent) {
        if self.controls.is_none() {
            return;
        }
        match event {
            PlayerEvent::TrackChanged { audio_item } => {
                self.state.track = Some(track(audio_item));
                self.state.position = Duration::ZERO;
                self.load_artwork(audio_item);
            }
            PlayerEvent::Playing { position_ms, .. } => {
                self.state.playing = true;
                self.state.position = Duration::from_millis((*position_ms).into());
            }
            PlayerEvent::Paused { position_ms, .. } => {
                self.state.playing = false;
                self.state.position = Duration::from_millis((*position_ms).into());
            }
            PlayerEvent::Seeked { position_ms, .. } | PlayerEvent::PositionCorrection { position_ms, .. } => {
                self.state.position = Duration::from_millis((*position_ms).into());
            }
            PlayerEvent::Stopped { .. } => self.state = State::default(),
            _ => return,
        }
        if let Some(controls) = &self.controls {
            controls.update(self.state.clone());
        }
    }

    /// Shows a loaded cover; requests from the system go to the player.
    pub fn on_event(&mut self, event: Event, spirc: &Spirc) {
        match event {
            Event::Command(command) => {
                if let Err(err) = control(spirc, command) {
                    tracing::warn!("media control {command:?}: {err:#}");
                }
            }
            Event::Artwork { url, bytes } => {
                if let (Some(controls), Some(track)) = (&self.controls, &mut self.state.track)
                    && self.artwork_url.as_deref() == Some(url.as_str())
                {
                    track.artwork = Some(bytes);
                    controls.update(self.state.clone());
                }
            }
        }
    }

    fn load_artwork(&mut self, item: &AudioItem) {
        let Some(url) = item.covers.iter().max_by_key(|c| c.width).map(|c| c.url.clone()) else { return };
        self.artwork_url = Some(url.clone());
        let (http, tx) = (self.http.clone(), self.tx.clone());
        tokio::spawn(async move {
            let result = async { http.get(&url).send().await?.error_for_status()?.bytes().await }.await;
            match result {
                Ok(bytes) => {
                    let _ = tx.send(Event::Artwork { url, bytes: Arc::from(bytes.as_ref()) });
                }
                Err(err) => tracing::warn!("artwork {url}: {err}"),
            }
        });
    }
}

fn control(spirc: &Spirc, command: Command) -> Result<()> {
    match command {
        Command::Play => spirc.play()?,
        Command::Pause => spirc.pause()?,
        Command::Toggle => spirc.play_pause()?,
        Command::Next => spirc.next()?,
        Command::Previous => spirc.prev()?,
        Command::Seek(position) => spirc.set_position_ms(position.as_millis().try_into().unwrap_or(u32::MAX))?,
    }
    Ok(())
}

fn track(item: &AudioItem) -> Track {
    let (artist, album) = match &item.unique_fields {
        UniqueFields::Track { artists, album, .. } => {
            (artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", "), album.clone())
        }
        UniqueFields::Episode { show_name, .. } => (show_name.clone(), String::new()),
        UniqueFields::Local { artists, album, .. } => {
            (artists.clone().unwrap_or_default(), album.clone().unwrap_or_default())
        }
    };
    Track {
        title: item.name.clone(),
        artist,
        album,
        duration: Duration::from_millis(item.duration_ms.into()),
        artwork: None,
    }
}
