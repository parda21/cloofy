use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use cloofy_spotify::session::PendingSession;
use librespot::{
    connect::{ConnectConfig, Spirc},
    core::{config::DeviceType, session::Session},
    playback::{
        audio_backend,
        config::{AudioFormat, Bitrate, PlayerConfig},
        mixer::{self, Mixer, MixerConfig},
        player::Player,
    },
};

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
    pub fn new(session: &Session, crossfade: Duration) -> Result<Self> {
        let sink = PREFERRED_BACKENDS
            .iter()
            .find_map(|name| audio_backend::find(Some((*name).to_owned())))
            .or_else(|| audio_backend::find(None))
            .context("No audio backend available")?;
        let mixer = (mixer::find(None).context("No mixer available")?)(MixerConfig::default())?;
        let config = PlayerConfig { bitrate: Bitrate::Bitrate320, crossfade, ..PlayerConfig::default() };
        let player =
            Player::new(config, session.clone(), mixer.get_soft_volume(), move || sink(None, AudioFormat::default()));
        Ok(Self { player, mixer })
    }

    /// Registers the player as a Spotify Connect device. Spirc connects the session itself,
    /// which is why it takes a pending session.
    pub async fn connect_device(
        &self,
        pending: PendingSession,
        name: String,
    ) -> Result<(Spirc, impl Future<Output = ()> + use<>)> {
        let config = ConnectConfig { name, device_type: DeviceType::Computer, ..ConnectConfig::default() };
        Ok(Spirc::new(config, pending.session, pending.credentials, self.player.clone(), self.mixer.clone()).await?)
    }
}
