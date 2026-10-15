use std::fs;

use anyhow::{Context, Result};
use librespot::{
    core::{authentication::Credentials, cache::Cache, config::SessionConfig, session::Session},
    oauth::OAuthClientBuilder,
};

use crate::paths::{Paths, write_private};

/// librespot only streams with Spotify's own client ID, so this login is separate from the
/// Web API one. The port is free to change: Spotify accepts any port on loopback redirects.
const OAUTH_REDIRECT: &str = "http://127.0.0.1:8975/login";
const AUDIO_CACHE_LIMIT: u64 = 1 << 30;

/// The scopes Spotify's desktop client requests (the same list librespot uses).
const OAUTH_SCOPES: &[&str] = &[
    "app-remote-control",
    "playlist-modify",
    "playlist-modify-private",
    "playlist-modify-public",
    "playlist-read",
    "playlist-read-collaborative",
    "playlist-read-private",
    "streaming",
    "ugc-image-upload",
    "user-follow-modify",
    "user-follow-read",
    "user-library-modify",
    "user-library-read",
    "user-modify",
    "user-modify-playback-state",
    "user-modify-private",
    "user-personalized",
    "user-read-birthdate",
    "user-read-currently-playing",
    "user-read-email",
    "user-read-play-history",
    "user-read-playback-position",
    "user-read-playback-state",
    "user-read-private",
    "user-read-recently-played",
    "user-top-read",
];

/// A session with cached credentials that is not connected yet. Spirc connects it on its
/// own; everything else calls [`PendingSession::connect`].
pub struct PendingSession {
    pub session: Session,
    pub credentials: Credentials,
}

impl PendingSession {
    pub async fn connect(self) -> Result<Session> {
        self.session.connect(self.credentials, true).await.context("Connecting to Spotify failed")?;
        Ok(self.session)
    }
}

pub fn prepare(paths: &Paths) -> Result<PendingSession> {
    let cache = cache(paths)?;
    let credentials = cache.credentials().context("Streaming not logged in – run `cloofy login`")?;
    Ok(PendingSession { session: Session::new(config(paths)?, Some(cache)), credentials })
}

pub fn is_logged_in(paths: &Paths) -> Result<bool> {
    Ok(cache(paths)?.credentials().is_some())
}

/// Interactive OAuth login. Connecting once stores reusable credentials in the cache.
pub async fn login(paths: &Paths) -> Result<Session> {
    let config = config(paths)?;
    let token = OAuthClientBuilder::new(&config.client_id, OAUTH_REDIRECT, OAUTH_SCOPES.to_vec())
        .open_in_browser()
        .with_custom_message("cloofy: Streaming is connected. You can close this window.")
        .build()?
        .get_access_token_async()
        .await?;
    PendingSession {
        session: Session::new(config, Some(cache(paths)?)),
        credentials: Credentials::with_access_token(token.access_token),
    }
    .connect()
    .await
}

fn cache(paths: &Paths) -> Result<Cache> {
    let session_dir = paths.session_dir();
    let audio_dir = paths.audio_cache();
    Ok(Cache::new(Some(&session_dir), Some(&session_dir), Some(&audio_dir), Some(AUDIO_CACHE_LIMIT))?)
}

/// Keeps the device ID stable so Spotify shows one cloofy device, not a new one per start.
fn config(paths: &Paths) -> Result<SessionConfig> {
    let mut config = SessionConfig::default();
    let path = paths.device_id();
    match fs::read_to_string(&path) {
        Ok(id) if !id.trim().is_empty() => config.device_id = id.trim().to_owned(),
        _ => write_private(&path, config.device_id.as_bytes())?,
    }
    Ok(config)
}
