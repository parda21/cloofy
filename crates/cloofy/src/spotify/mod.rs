//! The commands for Spotify: signing in, its checks for `cloofy doctor`, playing without the
//! interface, and a few for development.

mod playback;

use anyhow::Result;
use cloofy_core::{
    config::Config,
    doctor::{Check, Expect, Outcome, Section},
    paths::Paths,
};
use cloofy_spotify::{doctor, session, webapi::WebApi};

pub use self::playback::{connect, play};
use crate::style;

/// Signs in to the Web API with the user's own app, then to librespot for streaming.
pub async fn login(paths: &Paths, config: &Config, force: bool) -> Result<()> {
    let api = WebApi::new(config.client_id(paths)?, paths.webapi_token())?;

    println!("\n  {} Web API (your app)", style::dim("1/2"));
    if force || !api.is_logged_in().await {
        api.login().await?;
        println!("  {} connected\n", style::green("✓"));
    } else {
        println!("  {} already connected\n", style::green("✓"));
    }

    println!("  {} Streaming (librespot)", style::dim("2/2"));
    if force || !session::is_logged_in(paths)? {
        let session = session::login(paths).await?;
        println!("  {} connected as {}\n", style::green("✓"), session.username());
        session.shutdown();
    } else {
        println!("  {} already connected\n", style::green("✓"));
    }
    Ok(())
}

/// What `cloofy doctor` finds through the Web API and through librespot, each section handed
/// to `report` as soon as it is done.
pub async fn doctor(paths: &Paths, config: &Config, mut report: impl FnMut(Section)) -> Result<()> {
    let api = WebApi::new(config.client_id(paths)?, paths.webapi_token())?;
    report(if api.is_logged_in().await { doctor::web_api(&api).await } else { not_logged_in("WEB API · your app") });

    let internal = match session::prepare(paths) {
        Ok(pending) => match pending.connect().await {
            Ok(session) => {
                let section = doctor::spclient(&session).await;
                session.shutdown();
                section
            }
            Err(err) => failed_section("LIBRESPOT · Spotify internal", "Session", format!("{err:#}")),
        },
        Err(_) => not_logged_in("LIBRESPOT · Spotify internal"),
    };
    report(internal);
    Ok(())
}

fn not_logged_in(title: &'static str) -> Section {
    failed_section(title, "Login", "not logged in – run `cloofy login`".into())
}

fn failed_section(title: &'static str, name: &str, reason: String) -> Section {
    Section {
        title,
        checks: vec![Check { name: name.into(), expect: Expect::Available, outcome: Outcome::Failed(reason) }],
    }
}

/// Fetches the DJ's lines for the first tracks of a fresh DJ session, without playing anything.
pub async fn dj_sample(paths: &Paths, dir: &std::path::Path) -> Result<()> {
    use anyhow::Context as _;
    let session = session::prepare(paths)?.connect().await?;
    let spclient = session.spclient();
    let dj = spclient.get_context("spotify:playlist:37i9dQZF1EYkqdzj48dyYq").await?;
    println!("context metadata: {:#?}", dj.metadata);
    let lexicon = dj.metadata.get("lexicon_context_url").context("no lexicon_context_url")?;
    let context = spclient.get_context_url(lexicon).await?;
    std::fs::create_dir_all(dir)?;
    let mut count = 0;
    for track in context.pages.iter().flat_map(|page| &page.tracks) {
        let mut narration: Vec<_> = track.metadata.iter().filter(|(key, _)| key.starts_with("narration.")).collect();
        if narration.is_empty() {
            continue;
        }
        narration.sort();
        println!("\n{}", track.uri());
        for (key, value) in &narration {
            println!("  {key} = {value}");
        }
        let field = |key: String| track.metadata.get(&key).map(String::as_str).unwrap_or_default();
        for kind in ["intro", "outro"] {
            let ssml = field(format!("narration.{kind}.ssml"));
            if ssml.is_empty() {
                continue;
            }
            let voice = field(format!("narration.{kind}.voice"));
            let provider = field(format!("narration.{kind}.tts_provider"));
            let mp3 = spclient.get_narration(ssml, voice, provider).await?;
            let file = dir.join(format!("{count}-{kind}.mp3"));
            std::fs::write(&file, &mp3)?;
            println!("  -> {} ({} bytes)", file.display(), mp3.len());
        }
        count += 1;
        if count == 3 {
            break;
        }
    }
    session.shutdown();
    Ok(())
}

/// Runs a pathfinder query and prints the JSON.
pub async fn query(paths: &Paths, operation: &str, variables: &str) -> Result<()> {
    let session = session::prepare(paths)?.connect().await?;
    let pathfinder = cloofy_spotify::pathfinder::Pathfinder::new(session.clone(), paths.pathfinder_hashes());
    let data = pathfinder.query(operation, serde_json::from_str(variables)?).await?;
    println!("{}", serde_json::to_string_pretty(&data)?);
    session.shutdown();
    Ok(())
}
