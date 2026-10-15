use std::{fs, path::Path, time::Duration};

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
/// Downloading a track takes seconds, and every write refreshes the file's modified time.
const STALE_TMP_AGE: Duration = Duration::from_secs(60 * 60);

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
    // librespot buffers each streamed track in a temp file that stays open until the track
    // ends. /tmp is often tmpfs, where that file would hold 10–15 MB of RAM per track.
    let tmp = paths.audio_tmp();
    fs::create_dir_all(&tmp)?;
    remove_stale_tmp_files(&tmp);
    config.tmp_dir = tmp;
    Ok(config)
}

/// A killed cloofy leaves its half-streamed tracks behind, which tmpfs used to drop at boot.
/// Only old files go: another cloofy process may have just created one and not yet reopened
/// it by name. Files that are open keep working after removal, so nothing else is at risk.
fn remove_stale_tmp_files(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age > STALE_TMP_AGE));
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;

    #[test]
    fn removes_only_stale_tmp_files() {
        let dir = std::env::temp_dir().join(format!("cloofy-tmp-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (old, fresh) = (dir.join("old"), dir.join("fresh"));
        fs::File::create(&old).unwrap().set_modified(SystemTime::now() - 2 * STALE_TMP_AGE).unwrap();
        fs::File::create(&fresh).unwrap();
        remove_stale_tmp_files(&dir);
        let left = (old.exists(), fresh.exists());
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(left, (false, true));
    }
}
