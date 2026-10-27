//! `cloofy doctor`'s findings, the same for every service: each probes what it can reach and
//! reports it in these, and the command prints them all alike.

use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    Available,
    /// Spotify removed this for Development Mode apps; a failure is the expected result.
    Removed,
}

#[derive(Debug)]
pub enum Outcome {
    Ok(String),
    Failed(String),
}

#[derive(Debug)]
pub struct Check {
    pub name: String,
    pub expect: Expect,
    pub outcome: Outcome,
}

#[derive(Debug)]
pub struct Section {
    pub title: &'static str,
    pub checks: Vec<Check>,
}

/// Runs a probe that should succeed, giving up after a while; its answer is the detail shown.
pub async fn timed(name: &str, probe: impl Future<Output = Result<String>>) -> Check {
    let outcome = match tokio::time::timeout(TIMEOUT, probe).await {
        Ok(Ok(detail)) => Outcome::Ok(detail),
        Ok(Err(err)) => Outcome::Failed(format!("{err:#}")),
        Err(_) => Outcome::Failed("Timed out".into()),
    };
    Check { name: name.into(), expect: Expect::Available, outcome }
}

/// What every service relies on besides itself: LRCLIB, where lyrics come from when the
/// service has none.
pub async fn external() -> Section {
    let check = timed("Lyrics (LRCLIB)", async {
        let results: Value = reqwest::Client::builder()
            .user_agent(concat!("cloofy/", env!("CARGO_PKG_VERSION")))
            .build()?
            .get("https://lrclib.net/api/search")
            .query(&[("track_name", "One More Time"), ("artist_name", "Daft Punk")])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let synced = results.as_array().into_iter().flatten().filter(|r| r["syncedLyrics"].is_string()).count();
        Ok(format!("{synced} time-synced versions"))
    })
    .await;
    Section { title: "EXTERNAL", checks: vec![check] }
}
