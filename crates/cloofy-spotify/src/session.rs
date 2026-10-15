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
const CORRUPTED: &str = "Stored streaming credentials corrupted – run `cloofy login`";

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
    let cache = cache(paths, !paths.keychain)?;
    let credentials = stored_credentials(paths, &cache)?.context("Streaming not logged in – run `cloofy login`")?;
    Ok(PendingSession { session: Session::new(config(paths)?, Some(cache)), credentials })
}

pub fn is_logged_in(paths: &Paths) -> Result<bool> {
    Ok(stored_credentials(paths, &cache(paths, !paths.keychain)?)?.is_some())
}

/// Updates the credentials in the keychain when Spotify handed out new ones while connecting;
/// for files, librespot does that itself.
pub fn remember(paths: &Paths, session: &Session) -> Result<()> {
    if !paths.keychain {
        return Ok(());
    }
    let secret = paths.streaming_credentials();
    let Some(bytes) = secret.read()? else { return Ok(()) };
    let mut stored: Credentials = serde_json::from_slice(&bytes).context(CORRUPTED)?;
    let auth_data = session.auth_data();
    if auth_data.is_empty() || auth_data == stored.auth_data {
        return Ok(());
    }
    stored.username = Some(session.username());
    stored.auth_data = auth_data;
    secret.write(&serde_json::to_vec(&stored)?)
}

fn stored_credentials(paths: &Paths, cache: &Cache) -> Result<Option<Credentials>> {
    if !paths.keychain {
        return Ok(cache.credentials());
    }
    match paths.streaming_credentials().read()? {
        Some(bytes) => Ok(Some(serde_json::from_slice(&bytes).context(CORRUPTED)?)),
        None => Ok(None),
    }
}

/// Interactive OAuth login. Connecting once stores reusable credentials: librespot writes
/// them to a file, which moves into the keychain right away in keychain mode.
pub async fn login(paths: &Paths) -> Result<Session> {
    let config = config(paths)?;
    let token = OAuthClientBuilder::new(&config.client_id, OAUTH_REDIRECT, OAUTH_SCOPES.to_vec())
        .open_in_browser()
        .with_custom_message("cloofy: Streaming is connected. You can close this window.")
        .build()?
        .get_access_token_async()
        .await?;
    let session = PendingSession {
        session: Session::new(config, Some(cache(paths, true)?)),
        credentials: Credentials::with_access_token(token.access_token),
    }
    .connect()
    .await?;
    if paths.keychain {
        paths.streaming_credentials().read()?;
    }
    Ok(session)
}

/// librespot keeps the credentials in a file only with `credentials_file`.
fn cache(paths: &Paths, credentials_file: bool) -> Result<Cache> {
    let session_dir = paths.session_dir();
    let audio_dir = paths.audio_cache();
    let credentials = credentials_file.then_some(&session_dir);
    Ok(Cache::new(credentials, Some(&session_dir), Some(&audio_dir), Some(AUDIO_CACHE_LIMIT))?)
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
