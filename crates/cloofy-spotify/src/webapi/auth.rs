use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngExt, distr::Alphanumeric};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use url::Url;

const AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
/// Registered in the Spotify app as `http://127.0.0.1:8974/callback`. Spotify lets loopback
/// redirects pick their port at request time, so a busy port falls back to a random one.
const REDIRECT_PORT: u16 = 8974;
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

pub const SCOPES: &[&str] = &[
    "user-read-private",
    "user-read-playback-state",
    "user-modify-playback-state",
    "user-read-currently-playing",
    "user-read-recently-played",
    "user-read-playback-position",
    "user-top-read",
    "user-library-read",
    "user-library-modify",
    "user-follow-read",
    "user-follow-modify",
    "playlist-read-private",
    "playlist-read-collaborative",
    "playlist-modify-private",
    "playlist-modify-public",
    "ugc-image-upload",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: u64,
    pub scopes: Vec<String>,
}

impl Token {
    pub fn is_fresh(&self) -> bool {
        unix_now() + 60 < self.expires_at
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    #[serde(default)]
    scope: String,
}

/// Authorization code flow with PKCE: opens the browser and waits for the loopback redirect.
pub(super) async fn login(http: &reqwest::Client, client_id: &str) -> Result<Token> {
    let listener = match TcpListener::bind(("127.0.0.1", REDIRECT_PORT)).await {
        Ok(listener) => listener,
        Err(_) => TcpListener::bind(("127.0.0.1", 0)).await.context("No free port for the login callback")?,
    };
    let redirect_uri = format!("http://127.0.0.1:{}/callback", listener.local_addr()?.port());

    let verifier = random_string(64);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_string(16);
    let scopes = SCOPES.join(" ");
    let url = Url::parse_with_params(
        AUTHORIZE_URL,
        [
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", &redirect_uri),
            ("scope", &scopes),
            ("code_challenge_method", "S256"),
            ("code_challenge", &challenge),
            ("state", &state),
        ],
    )?;

    eprintln!("  Opening your browser … if it doesn’t open, visit this URL:\n  {url}\n");
    let _ = open::that_detached(url.as_str());

    let code = tokio::time::timeout(LOGIN_TIMEOUT, wait_for_code(&listener, &state))
        .await
        .context("Login timed out (no response for 5 minutes)")??;

    let response = http
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect_uri),
            ("client_id", client_id),
            ("code_verifier", &verifier),
        ])
        .send()
        .await?;
    parse_token_response(response, None).await
}

pub(super) async fn refresh(http: &reqwest::Client, client_id: &str, refresh_token: &str) -> Result<Token> {
    let response = http
        .post(TOKEN_URL)
        .form(&[("grant_type", "refresh_token"), ("refresh_token", refresh_token), ("client_id", client_id)])
        .send()
        .await?;
    parse_token_response(response, Some(refresh_token)).await
}

async fn parse_token_response(response: reqwest::Response, previous_refresh: Option<&str>) -> Result<Token> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Token request failed ({status}): {body}");
    }
    let response: TokenResponse = response.json().await?;
    // Spotify may or may not rotate the refresh token; keep the old one if it doesn't.
    let refresh_token = response
        .refresh_token
        .or_else(|| previous_refresh.map(str::to_owned))
        .context("Spotify returned no refresh token")?;
    Ok(Token {
        access_token: response.access_token,
        refresh_token,
        expires_at: unix_now() + response.expires_in,
        scopes: response.scope.split_whitespace().map(str::to_owned).collect(),
    })
}

async fn wait_for_code(listener: &TcpListener, expected_state: &str) -> Result<String> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let Some(target) = read_request_target(&mut stream).await? else {
            continue;
        };
        let url = Url::parse("http://127.0.0.1")?.join(&target)?;
        if url.path() != "/callback" {
            respond(&mut stream, "404 Not Found", "").await;
            continue;
        }

        let param = |key: &str| url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned());
        let result = if param("state").as_deref() != Some(expected_state) {
            Err(anyhow!("Invalid state parameter in the login callback"))
        } else if let Some(code) = param("code") {
            Ok(code)
        } else {
            let error = param("error").unwrap_or_else(|| "unknown error".into());
            Err(anyhow!("Spotify rejected the login: {error}"))
        };

        respond(&mut stream, "200 OK", &callback_page(result.is_ok())).await;
        return result;
    }
}

/// Reads the request head and returns the target of a `GET` request.
async fn read_request_target(stream: &mut TcpStream) -> Result<Option<String>> {
    let mut head = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 8192 {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&head);
    let mut request_line = head.lines().next().unwrap_or_default().split_whitespace();
    Ok(match (request_line.next(), request_line.next()) {
        (Some("GET"), Some(target)) => Some(target.to_owned()),
        _ => None,
    })
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn callback_page(success: bool) -> String {
    let (class, mark, title, text) = if success {
        ("ok", "✓", "cloofy is connected", "You can close this window and return to the terminal.")
    } else {
        ("fail", "✕", "Login failed", "See the terminal for details.")
    };
    format!(
        r#"<!doctype html><html lang="en"><meta charset="utf-8"><title>cloofy</title>
<style>
:root{{color-scheme:dark}}
body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#0e0e10;color:#ededf0;font:16px/1.5 ui-sans-serif,system-ui,sans-serif}}
main{{text-align:center}}
.mark{{font-size:44px;line-height:1}}
.ok .mark{{color:#1ed760}}
.fail .mark{{color:#ff5c5c}}
h1{{font-size:20px;font-weight:600;margin:16px 0 4px}}
p{{margin:0;color:#8a8a93}}
</style>
<main class="{class}"><div class="mark">{mark}</div><h1>{title}</h1><p>{text}</p></main>"#
    )
}

fn random_string(len: usize) -> String {
    rand::rng().sample_iter(Alphanumeric).take(len).map(char::from).collect()
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
