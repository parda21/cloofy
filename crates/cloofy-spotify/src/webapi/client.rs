use std::time::Duration;

use anyhow::{Context, Result, bail};
use cloofy_core::paths::Secret;
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::Mutex;

use super::auth::{self, Token};

const API_BASE: &str = "https://api.spotify.com/v1";
/// Tries per read request, including the first.
const MAX_ATTEMPTS: u32 = 3;
/// Pause before repeating a read that failed on the way (timeout, 502, 503).
const RETRY_DELAY: Duration = Duration::from_millis(300);
/// Longest rate-limit wait worth sitting out; longer ones are reported instead.
const MAX_RETRY_AFTER: u64 = 10;

/// Web API client for the user's own Spotify app; refreshes its token on demand.
pub struct WebApi {
    http: reqwest::Client,
    client_id: String,
    store: Secret,
    token: Mutex<Option<Token>>,
}

#[derive(Debug)]
pub struct ApiResponse {
    pub status: StatusCode,
    /// `Value::Null` when the response has no JSON body.
    pub body: Value,
}

impl ApiResponse {
    /// Status plus Spotify's error message, e.g. `403 Forbidden`.
    pub fn error_text(&self) -> String {
        let message = self.body.pointer("/error/message").and_then(Value::as_str).unwrap_or_default();
        format!("{} {message}", self.status.as_u16()).trim_end().to_owned()
    }
}

impl WebApi {
    pub fn new(client_id: impl Into<String>, store: Secret) -> Result<Self> {
        let http = cloofy_core::http::client(concat!("cloofy/", env!("CARGO_PKG_VERSION"))).build()?;
        let token = match store.read()? {
            Some(bytes) => Some(serde_json::from_slice(&bytes).context("Stored token corrupted – run `cloofy login`")?),
            None => None,
        };
        Ok(Self { http, client_id: client_id.into(), store, token: Mutex::new(token) })
    }

    pub async fn is_logged_in(&self) -> bool {
        self.token.lock().await.is_some()
    }

    pub async fn login(&self) -> Result<()> {
        let token = auth::login(&self.http, &self.client_id).await?;
        self.persist(&token)?;
        *self.token.lock().await = Some(token);
        Ok(())
    }

    pub async fn get(&self, path: &str) -> Result<ApiResponse> {
        self.request(Method::GET, path, None).await
    }

    /// GET that fails on non-2xx and deserializes the body.
    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self.get(path).await?;
        if !response.status.is_success() {
            bail!("Spotify: {}", response.error_text());
        }
        serde_json::from_value(response.body).with_context(|| format!("Unexpected response from {path}"))
    }

    /// A write request that only has to succeed.
    pub async fn call(&self, method: Method, path: &str, body: Option<&Value>) -> Result<()> {
        let response = self.request(method, path, body).await?;
        if !response.status.is_success() {
            bail!("Spotify: {}", response.error_text());
        }
        Ok(())
    }

    /// Reads are retried after a rate limit or a passing network failure. Writes never are:
    /// a play or save must not happen twice, or seconds after the user asked for it.
    pub async fn request(&self, method: Method, path: &str, body: Option<&Value>) -> Result<ApiResponse> {
        let token = self.access_token().await?;
        let url = format!("{API_BASE}{path}");
        let retries = method == Method::GET;
        let mut attempt = 1;
        loop {
            let mut request = self.http.request(method.clone(), &url).bearer_auth(&token);
            request = match body {
                Some(body) => request.json(body),
                // Spotify rejects body-less writes without a Content-Length (411).
                None if method != Method::GET => request.body(Vec::new()),
                None => request,
            };
            let delay = match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    let delay = match status {
                        StatusCode::TOO_MANY_REQUESTS => {
                            tracing::warn!("Web API rate limit on {path}");
                            retry_after(&response)
                        }
                        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE if attempt == 1 => Some(RETRY_DELAY),
                        _ => None,
                    };
                    match delay {
                        Some(delay) if retries && attempt < MAX_ATTEMPTS => delay,
                        _ => {
                            let body = serde_json::from_slice(&response.bytes().await?).unwrap_or(Value::Null);
                            return Ok(ApiResponse { status, body });
                        }
                    }
                }
                Err(err) if retries && attempt == 1 && (err.is_timeout() || err.is_connect()) => RETRY_DELAY,
                Err(err) => return Err(err.into()),
            };
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }

    async fn access_token(&self) -> Result<String> {
        let mut guard = self.token.lock().await;
        let token = guard.as_ref().context("Not logged in to the Web API – run `cloofy login`")?;
        if token.is_fresh() {
            return Ok(token.access_token.clone());
        }
        let refreshed = auth::refresh(&self.http, &self.client_id, &token.refresh_token)
            .await
            .context("Token refresh failed – run `cloofy login` again")?;
        self.persist(&refreshed)?;
        let access_token = refreshed.access_token.clone();
        *guard = Some(refreshed);
        Ok(access_token)
    }

    fn persist(&self, token: &Token) -> Result<()> {
        self.store.write(&serde_json::to_vec_pretty(token)?)
    }
}

/// How long Spotify asks to wait after a rate limit; `None` when that is too long to sit out.
fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    let seconds = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(1);
    (seconds <= MAX_RETRY_AFTER).then(|| Duration::from_secs(seconds))
}
