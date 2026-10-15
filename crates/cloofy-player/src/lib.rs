#[cfg(target_os = "macos")]
mod macos;

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use cloofy_spotify::session::PendingSession;
use librespot::{
    connect::{ConnectConfig, Spirc},
    core::{config::DeviceType, session::Session},
    playback::{
        audio_backend::{self, Sink, SinkBuilder},
        config::{AudioFormat, Bitrate, PlayerConfig},
        mixer::{self, Mixer, MixerConfig},
        player::Player,
    },
};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

/// Preferred audio outputs, best first; librespot's default (rodio) is the fallback.
#[cfg(target_os = "linux")]
const PREFERRED_BACKENDS: &[&str] = &["pulseaudio"];
#[cfg(not(target_os = "linux"))]
const PREFERRED_BACKENDS: &[&str] = &[];

/// Local audio playback through librespot.
pub struct Engine {
    pub player: Arc<Player>,
    mixer: Arc<dyn Mixer>,
}

impl Engine {
    /// `crossfade` overlaps consecutive tracks by that long; zero plays them back to back.
    ///
    /// The receiver gets the name of each audio output that goes away during playback, such as
    /// headphones being unplugged; playback should pause then.
    pub fn new(session: &Session, crossfade: Duration) -> Result<(Self, UnboundedReceiver<String>)> {
        let backend = PREFERRED_BACKENDS
            .iter()
            .find_map(|name| audio_backend::find(Some((*name).to_owned())))
            .or_else(|| audio_backend::find(None))
            .context("No audio backend available")?;
        let mixer = (mixer::find(None).context("No mixer available")?)(MixerConfig::default())?;
        let config = PlayerConfig { bitrate: Bitrate::Bitrate320, crossfade, ..PlayerConfig::default() };
        let (lost_tx, lost_rx) = mpsc::unbounded_channel();
        let player =
            Player::new(config, session.clone(), mixer.get_soft_volume(), move || open_output(backend, lost_tx));
        Ok((Self { player, mixer }, lost_rx))
    }

    /// The volume playback has now.
    pub fn volume(&self) -> u16 {
        self.mixer.volume()
    }

    /// Registers the player as a Spotify Connect device. Spirc connects the session itself,
    /// which is why it takes a pending session. It sets the volume to `volume`, or to
    /// librespot's default without one.
    pub async fn connect_device(
        &self,
        pending: PendingSession,
        name: String,
        volume: Option<u16>,
    ) -> Result<(Spirc, impl Future<Output = ()> + use<>)> {
        let mut config = ConnectConfig { name, device_type: DeviceType::Computer, ..ConnectConfig::default() };
        if let Some(volume) = volume {
            config.initial_volume = volume;
        }
        Ok(Spirc::new(config, pending.session, pending.credentials, self.player.clone(), self.mixer.clone()).await?)
    }
}

/// On macOS, an output that follows the system's default output.
#[cfg(target_os = "macos")]
fn open_output(backend: SinkBuilder, lost: UnboundedSender<String>) -> Box<dyn Sink> {
    match macos::FollowingSink::new(lost) {
        Some(sink) => Box::new(sink),
        None => backend(None, AudioFormat::default()),
    }
}

/// PulseAudio follows the default output by itself.
#[cfg(not(target_os = "macos"))]
fn open_output(backend: SinkBuilder, _lost: UnboundedSender<String>) -> Box<dyn Sink> {
    backend(None, AudioFormat::default())
}
