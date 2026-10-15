//! Spotify's internal GraphQL API ("pathfinder"), which the official clients use for Home and
//! other views the Web API no longer offers.
//!
//! Queries are persisted: Spotify only accepts known query hashes, and those rotate every few
//! weeks. The current ones are read from the web player's JavaScript and cached on disk.

use std::{collections::HashMap, fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use librespot::core::session::Session;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::paths::write_private;

const ENDPOINT: &str = "https://api-partner.spotify.com/pathfinder/v2/query";
const WEB_PLAYER: &str = "https://open.spotify.com/";
/// Used until the web player has been read once.
const KNOWN_HASHES: &[(&str, &str)] = &[("home", "5fb7da7a03ee2776c0670856609cb85a189a3cecc36cc160b91cde79063b9cf1")];
/// Greetings and shelf titles come back in this language, matching the English UI.
const ACCEPT_LANGUAGE: &str = "en";
/// The web player's JavaScript bundles are large; reading them may take a while.
const BUNDLE_TIMEOUT: Duration = Duration::from_secs(60);
const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Safari/537.36";

pub struct Pathfinder {
    http: reqwest::Client,
    session: Session,
    cache_path: PathBuf,
    hashes: Mutex<HashMap<String, String>>,
}

impl Pathfinder {
    pub fn new(session: Session, cache_path: PathBuf) -> Self {
        let mut hashes: HashMap<String, String> =
            KNOWN_HASHES.iter().map(|(op, hash)| (op.to_string(), hash.to_string())).collect();
        if let Ok(cached) = fs::read(&cache_path)
            && let Ok(cached) = serde_json::from_slice::<HashMap<String, String>>(&cached)
        {
            hashes.extend(cached);
        }
        Self {
            http: crate::http::client(USER_AGENT).build().unwrap_or_default(),
            session,
            cache_path,
            hashes: Mutex::new(hashes),
        }
    }

    /// Runs a persisted query and returns its `data`. Refreshes the hash once if it is stale.
    pub async fn query(&self, operation: &str, variables: Value) -> Result<Value> {
        let hash = self.hashes.lock().await.get(operation).cloned();
        if let Some(hash) = hash {
            match self.post(operation, &hash, &variables).await {
                Err(err) if is_stale(&err) => tracing::info!("pathfinder {operation}: hash outdated"),
                result => return result,
            }
        }
        self.refresh_hashes().await?;
        let hash = self
            .hashes
            .lock()
            .await
            .get(operation)
            .cloned()
            .with_context(|| format!("Spotify operation “{operation}” not found"))?;
        self.post(operation, &hash, &variables).await
    }

    async fn post(&self, operation: &str, hash: &str, variables: &Value) -> Result<Value> {
        let token = self.session.login5().auth_token().await?.access_token;
        let client_token = self.session.spclient().client_token().await?;
        let body = json!({
            "operationName": operation,
            "variables": variables,
            "extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } },
        });
        let response = self
            .http
            .post(ENDPOINT)
            .bearer_auth(token)
            .header("client-token", client_token)
            .header("app-platform", "WebPlayer")
            .header("accept-language", ACCEPT_LANGUAGE)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if let Some(message) = body.pointer("/errors/0/message").and_then(Value::as_str) {
            bail!("pathfinder {operation}: {message}");
        }
        if !status.is_success() {
            bail!("pathfinder {operation}: HTTP {status}");
        }
        body.get("data").cloned().context("pathfinder: response without data")
    }

    /// Reads all persisted-query hashes from the web player bundle and caches them.
    async fn refresh_hashes(&self) -> Result<()> {
        let html = self.http.get(WEB_PLAYER).send().await?.text().await?;
        let mut found = HashMap::new();
        for script in script_urls(&html).filter(|url| url.contains("web-player")) {
            // The bundles are several megabytes, more than the default timeout allows for.
            let source = self.http.get(&script).timeout(BUNDLE_TIMEOUT).send().await?.text().await?;
            found.extend(persisted_hashes(&source));
        }
        if found.is_empty() {
            bail!("Could not read Spotify’s web player");
        }
        let _ = write_private(&self.cache_path, &serde_json::to_vec(&found)?);
        self.hashes.lock().await.extend(found);
        Ok(())
    }
}

fn is_stale(err: &anyhow::Error) -> bool {
    let text = err.to_string();
    text.contains("PersistedQueryNotFound") || text.contains("HTTP 412")
}

/// `<script src="…">` URLs of a page.
fn script_urls(html: &str) -> impl Iterator<Item = String> + '_ {
    html.split("<script").skip(1).filter_map(|tag| {
        let src = tag.split("src=\"").nth(1)?.split('"').next()?;
        src.ends_with(".js").then(|| src.to_owned())
    })
}

/// Finds `"<operation>","query","<sha256>"` triples in minified JavaScript.
fn persisted_hashes(source: &str) -> HashMap<String, String> {
    const MARKER: &str = "\",\"query\",\"";
    let mut hashes = HashMap::new();
    let mut rest = source;
    while let Some(at) = rest.find(MARKER) {
        let before = &rest[..at];
        let after = &rest[at + MARKER.len()..];
        let name = before.rsplit('"').next().unwrap_or_default();
        let hash: String = after.chars().take_while(char::is_ascii_hexdigit).collect();
        if hash.len() == 64 && !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric()) {
            hashes.insert(name.to_owned(), hash);
        }
        rest = after;
    }
    hashes
}

/// The system time zone, which Home needs for its greeting and time-of-day shelves.
pub fn time_zone() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_persisted_hashes() {
        let hash = "a".repeat(64);
        let js = format!(r#"x=new o("home","query","{hash}",null),y=new o("bad name","query","{hash}")"#);
        let hashes = persisted_hashes(&js);
        assert_eq!(hashes.get("home"), Some(&hash));
        assert_eq!(hashes.len(), 1);
    }

    #[test]
    fn finds_script_urls() {
        let html =
            r#"<script src="https://x/web-player.1.js"></script><script>inline</script><script src="/a.css"></script>"#;
        assert_eq!(script_urls(html).collect::<Vec<_>>(), ["https://x/web-player.1.js"]);
    }
}
