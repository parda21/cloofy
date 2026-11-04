//! Stream URLs from yt-dlp. YouTube signs its audio URLs with challenges only a JavaScript
//! runtime can solve, which yt-dlp does (with deno). A URL is good for about six hours.

use std::{
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use tokio::process::Command;

/// yt-dlp takes about two seconds; one that takes this long has hung.
const TIMEOUT: Duration = Duration::from_secs(40);
/// URLs count as stale this long before YouTube's expiry…
const MARGIN: Duration = Duration::from_secs(30 * 60);
/// …or, where the URL doesn't say, this long after they were made.
const LIFETIME: Duration = Duration::from_secs(5 * 60 * 60);

/// The page yt-dlp resolves for a song.
pub(crate) fn watch_url(video: &str) -> String {
    format!("https://music.youtube.com/watch?v={video}")
}

/// A stream URL and until when to use it.
#[derive(Debug, Clone)]
pub(crate) struct Stream {
    pub url: String,
    pub until: Instant,
}

impl Stream {
    pub fn fresh(&self) -> bool {
        Instant::now() < self.until
    }
}

/// Resolves the best audio stream of a song, without cookies: signed-out streams are the same.
pub(crate) async fn resolve(video: &str, cache_dir: Option<PathBuf>) -> Result<Stream> {
    let mut command = Command::new("yt-dlp");
    command.args(["--no-config", "--no-playlist", "--no-warnings", "--quiet", "-f", "bestaudio/best", "-g"]);
    if let Some(dir) = cache_dir {
        command.arg("--cache-dir").arg(dir);
    }
    command.arg(watch_url(video)).stdin(Stdio::null()).kill_on_drop(true);
    let output = match tokio::time::timeout(TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) if err.kind() == std::io::ErrorKind::NotFound => {
            bail!("yt-dlp is not installed; YouTube Music streams come from it")
        }
        Ok(Err(err)) => return Err(err).context("Can't run yt-dlp"),
        Err(_) => bail!("yt-dlp took too long"),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let url = stdout.lines().find(|line| line.starts_with("http"));
    match url {
        Some(url) if output.status.success() => Ok(Stream { url: url.to_owned(), until: valid_until(url) }),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = stderr.lines().rfind(|line| !line.trim().is_empty()).unwrap_or("no URL").trim();
            bail!("{}", reason.strip_prefix("ERROR: ").unwrap_or(reason))
        }
    }
}

/// When to stop using a URL, from its `expire` parameter.
fn valid_until(url: &str) -> Instant {
    let expires = url::Url::parse(url)
        .ok()
        .and_then(|url| url.query_pairs().find(|(key, _)| key == "expire").and_then(|(_, v)| v.parse::<u64>().ok()));
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let lifetime = match expires {
        Some(expires) => Duration::from_secs(expires.saturating_sub(now)).saturating_sub(MARGIN).min(LIFETIME * 2),
        None => LIFETIME,
    };
    Instant::now() + lifetime
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_good_until_shortly_before_they_expire() {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let url = format!("https://rr3.googlevideo.com/videoplayback?expire={}&itag=251", now + 6 * 3600);
        let left = valid_until(&url) - Instant::now();
        assert!(
            left > Duration::from_secs(5 * 3600 + 1700) && left <= Duration::from_secs(5 * 3600 + 1800),
            "{left:?}"
        );
        let expired = format!("https://rr3.googlevideo.com/videoplayback?expire={}", now - 10);
        assert!(valid_until(&expired) <= Instant::now() + Duration::from_secs(1));
        assert!(valid_until("https://example.com/a.webm") > Instant::now() + Duration::from_secs(4 * 3600));
    }
}
