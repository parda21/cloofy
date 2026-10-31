//! A command line for trying the YouTube Music backend without the interface:
//!
//! ```text
//! cargo run -p cloofy-ytmusic --example ytm -- home
//! cargo run -p cloofy-ytmusic --example ytm -- search daft punk
//! cargo run -p cloofy-ytmusic --example ytm -- playlist|album|artist|radio|lyrics <uri>
//! cargo run -p cloofy-ytmusic --example ytm -- suggest|tracks|saved|me|browsers|doctor …
//! cargo run -p cloofy-ytmusic --example ytm -- play <uri> [--demo] [--ao=null …mpv options]
//! cargo run -p cloofy-ytmusic --example ytm -- raw <endpoint> '<json body>' > response.json
//! cargo run -p cloofy-ytmusic --example ytm -- capture <dir>
//! ```
//!
//! `play` prints the player's events and reads commands from stdin: `n` next, `p` previous,
//! `t` play/pause, `s <seconds>` seek, `v <percent>` volume, `sh` shuffle, `r` repeat,
//! `q <uri>` queue, `j <uri>` skip to, `w <seconds>` wait (for a file of commands), `x` quit.
//! `--demo` runs through pause, seek, a gapless transition, next and previous by itself.
//! `RUST_LOG=cloofy_ytmusic=debug` shows what the player does, and the gap between songs.
//!
//! `capture` saves the raw responses the parser tests use; `tests/fixtures/` has them trimmed.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use cloofy_core::{catalog::Track, lyrics::LyricsQuery, paths::Paths, player::PlayerEvent};
use cloofy_ytmusic::{Player, Source, YtMusic, auth, doctor};
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::mpsc,
};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = args.split_first() else { bail!("usage: ytm <command> [args]") };
    let paths = Paths::resolve()?;
    let client = YtMusic::new(&paths)?;
    let arg = || rest.first().cloned().context("missing argument");
    let started = Instant::now();
    match command.as_str() {
        "home" => {
            let home = client.home().await?;
            for shelf in &home.shelves {
                println!("\n## {} ({} items)", shelf.title, shelf.items.len());
                for item in &shelf.items {
                    println!(
                        "  {:?} {} — {} [{}] play={}",
                        item.kind, item.title, item.subtitle, item.uri, item.context_uri
                    );
                }
            }
            println!("\n{} shelves", home.shelves.len());
        }
        "search" => {
            let results = client.search(&rest.join(" ")).await?;
            println!("## Songs");
            results.tracks.iter().for_each(print_track);
            println!("## Albums");
            for album in &results.albums {
                println!("  {} — {} ({}) [{}]", album.name, album.artists.join(", "), album.year, album.uri);
            }
            println!("## Artists");
            for artist in &results.artists {
                println!("  {} [{}]", artist.name, artist.uri);
            }
            println!("## Playlists");
            for playlist in &results.playlists {
                println!("  {} — {} · {} songs [{}]", playlist.name, playlist.owner_name, playlist.total, playlist.uri);
            }
        }
        "suggest" => println!("{:#?}", client.suggestions(&rest.join(" ")).await?),
        "playlist" | "album" | "liked" => {
            let source = match command.as_str() {
                "playlist" => {
                    let info = client.playlist_info(&arg()?).await?;
                    println!(
                        "{} by {} ({}) · {} songs · {:?}",
                        info.name, info.owner_name, info.owner_id, info.total, info.image_url
                    );
                    Source::Playlist(arg()?)
                }
                "album" => {
                    let info = client.album_info(&arg()?).await?;
                    println!(
                        "{} by {} ({}) · {} songs · {:?}",
                        info.name,
                        info.artists.join(", "),
                        info.year,
                        info.total,
                        info.image_url
                    );
                    Source::Album(arg()?)
                }
                _ => Source::Liked,
            };
            let mut pages = std::pin::pin!(client.tracks(source));
            while let Some(page) = pages.next().await {
                let page = page?;
                println!(
                    "-- page: {} songs, total {}, next_offset {:?} ({} ms)",
                    page.items.len(),
                    page.total,
                    page.next_offset,
                    started.elapsed().as_millis()
                );
                page.items.iter().take(5).for_each(print_track);
            }
        }
        "artist" => {
            let artist = client.artist(&arg()?).await?;
            println!(
                "{} · {:?} monthly · {:?} followers · followed {} · {:?}",
                artist.name, artist.monthly_listeners, artist.followers, artist.saved, artist.image_url
            );
            for top in &artist.top_tracks {
                print!("  {:?} plays ", top.playcount);
                print_track(&top.track);
            }
            for shelf in &artist.shelves {
                println!("## {} ({})", shelf.title, shelf.items.len());
                for item in shelf.items.iter().take(4) {
                    println!("  {:?} {} — {} [{}]", item.kind, item.title, item.subtitle, item.uri);
                }
            }
        }
        "radio" => client.radio(&arg()?).await?.iter().for_each(print_track),
        "tracks" => client.tracks_by_uri(rest).await?.iter().for_each(print_track),
        "lyrics" => {
            let track = client.tracks_by_uri(&[arg()?]).await?.into_iter().next().context("song not found")?;
            let query = LyricsQuery {
                uri: &track.uri,
                title: &track.name,
                artist: track.artists.first().map(String::as_str).unwrap_or_default(),
                album: &track.album,
                duration_ms: track.duration_ms,
            };
            match client.lyrics(&query).await? {
                Some(lyrics) => {
                    println!("{} lines, synced {}, from {}", lyrics.lines.len(), lyrics.synced, lyrics.provider);
                    for line in lyrics.lines.iter().take(12) {
                        println!("  [{:>6}] {}", line.start_ms, line.text);
                    }
                }
                None => println!("no lyrics"),
            }
        }
        "saved" => println!("{:?}", client.check_saved(rest).await?),
        "me" => println!("{:?}", client.me().await?),
        "browsers" => println!("{:#?}\ndefault: {:?}", auth::browsers(), auth::default_browser()),
        // Reads the default browser's sign-in without keeping it.
        "browser-sign-in" => {
            let mut browser = auth::default_browser().ok_or_else(|| anyhow::anyhow!("no default browser known"))?;
            match auth::from_browser(&paths, &mut browser).await? {
                Some(sign_in) => println!("{} signed in as {:?}", browser.spec, sign_in.check().await?.name),
                None => println!("{} isn't signed in to YouTube", browser.spec),
            }
        }
        "doctor" => {
            for section in doctor::all(&client).await {
                println!("\n{}", section.title);
                for check in section.checks {
                    println!("  {:<24} {:?}", check.name, check.outcome);
                }
            }
        }
        "raw" => {
            let body = rest.get(1).map(|b| serde_json::from_str(b)).transpose()?.unwrap_or_default();
            let value = client.raw(&arg()?, body).await?;
            println!("{}", serde_json::to_string(&value)?);
        }
        "play" => play(client, &paths, rest).await?,
        "capture" => capture(&client, &PathBuf::from(arg()?)).await?,
        other => bail!("unknown command {other}"),
    }
    eprintln!("({} ms)", started.elapsed().as_millis());
    Ok(())
}

/// Saves the raw responses the parser tests read, in one session so continuations work.
/// `tests/fixtures/` holds them trimmed down.
async fn capture(client: &YtMusic, dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    let save = |name: &str, value: &Value| -> Result<()> {
        fs::write(dir.join(format!("{name}.json")), serde_json::to_string(value)?)?;
        eprintln!("saved {name}");
        Ok(())
    };
    let token = |value: &Value| -> Option<String> {
        let mut found = None;
        walk(value, &mut |key, v| {
            if found.is_none() && (key == "continuation" || key == "token") && v.is_string() {
                found = v.as_str().map(str::to_owned);
            }
        });
        found
    };
    let home = client.raw("browse", json!({ "browseId": "FEmusic_home" })).await?;
    save("home", &home)?;
    let more = token(&home).context("Home has no second page")?;
    save("home_continuation", &client.raw("browse", json!({ "continuation": more })).await?)?;
    for (name, params) in [
        ("search_songs", "EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D"),
        ("search_albums", "EgWKAQIYAWoKEAkQBRAKEAMQBA%3D%3D"),
        ("search_artists", "EgWKAQIgAWoKEAkQBRAKEAMQBA%3D%3D"),
        ("search_featured", "EgeKAQQoADgBagwQDhAKEAMQBRAJEAQ%3D"),
        ("search_community", "EgeKAQQoAEABagoQAxAEEAoQCRAF"),
    ] {
        save(name, &client.raw("search", json!({ "query": "daft punk", "params": params })).await?)?;
    }
    save("suggestions", &client.raw("music/get_search_suggestions", json!({ "input": "daft pu" })).await?)?;
    let watch = |video: Option<&str>, playlist: &str| {
        let mut body = json!({ "playlistId": playlist, "params": "wAEB", "isAudioOnly": true,
            "enablePersistentPlaylistPanel": true });
        if let Some(video) = video {
            body["videoId"] = video.into();
        }
        body
    };
    save("next", &client.raw("next", watch(Some("52fNscjkST4"), "RDAMVM52fNscjkST4")).await?)?;
    save("next_mix", &client.raw("next", watch(None, "RDEMni3jl65KF37F9JYsJM8DGg")).await?)?;
    let queue = json!({ "videoIds": ["plnfIj7dkJE", "IluRBvnYMoY"] });
    save("queue", &client.raw("music/get_queue", queue).await?)?;
    let playlist = client.raw("browse", json!({ "browseId": "VLPLciALUPf8sIJlvH350O8PWh4UbSbOffSJ" })).await?;
    save("playlist", &playlist)?;
    let more = token(&playlist).context("the playlist has no second page")?;
    save("playlist_continuation", &client.raw("browse", json!({ "continuation": more })).await?)?;
    save("album", &client.raw("browse", json!({ "browseId": "MPREb_K8qWMWVqXGi" })).await?)?;
    save("artist", &client.raw("browse", json!({ "browseId": "UCRr1xG_2WIDs18a6cIiCxeA" })).await?)?;
    for (video, timed, plain) in [("plnfIj7dkJE", "lyrics_timed", "lyrics_plain"), ("52fNscjkST4", "", "lyrics_none")] {
        let next = client.raw("next", json!({ "videoId": video })).await?;
        let mut lyrics = None;
        walk(&next, &mut |key, v| {
            if key == "browseId" && v.as_str().is_some_and(|id| id.starts_with("MPLYt")) {
                lyrics = v.as_str().map(str::to_owned);
            }
        });
        let browse = json!({ "browseId": lyrics.context("no lyrics tab")? });
        if !timed.is_empty() {
            save(timed, &client.raw_android("browse", browse.clone()).await?)?;
        }
        save(plain, &client.raw("browse", browse).await?)?;
    }
    Ok(())
}

/// Calls `visit` with every key and value, depth first.
fn walk(value: &Value, visit: &mut impl FnMut(&str, &Value)) {
    match value {
        Value::Object(map) => map.iter().for_each(|(k, v)| {
            visit(k, v);
            walk(v, visit);
        }),
        Value::Array(items) => items.iter().for_each(|v| walk(v, visit)),
        _ => {}
    }
}

fn print_track(track: &Track) {
    println!(
        "  {} — {} · {} · {}:{:02} [{}] {:?}",
        track.name,
        track.artists.join(", "),
        track.album,
        track.duration_ms / 60_000,
        track.duration_ms / 1000 % 60,
        track.uri,
        track.artist_uris.first()
    );
}

async fn play(client: YtMusic, paths: &Paths, args: &[String]) -> Result<()> {
    let uri = args.first().context("play <uri>")?.clone();
    let demo = args.iter().any(|a| a == "--demo");
    let mpv_args: Vec<String> = args[1..].iter().filter(|a| a.starts_with("--") && *a != "--demo").cloned().collect();
    let (tx, mut events) = mpsc::unbounded_channel();
    let started = Instant::now();
    let player = Player::spawn_with_args(client, paths, u16::MAX / 2, tx, mpv_args).await?;
    eprintln!("[{:>6} ms] mpv ready", started.elapsed().as_millis());
    player.play(uri, None)?;
    let stamp = move || started.elapsed().as_millis();
    if demo {
        return run_demo(&player, &mut events, stamp).await;
    }
    let mut stdin = BufReader::new(tokio::io::stdin()).lines();
    let (mut shuffle, mut repeat, mut done) = (false, 0, false);
    // `w <seconds>` holds off the next command, so a file of commands can drive the player.
    let mut wait = Box::pin(tokio::time::sleep(Duration::ZERO));
    let mut waiting = false;
    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                print_event(stamp(), &event);
                if event == PlayerEvent::Deactivated {
                    break;
                }
            }
            _ = &mut wait, if waiting => waiting = false,
            line = stdin.next_line(), if !waiting && !done => {
                let Some(line) = line? else { done = true; continue };
                println!("\n[{:>6} ms] > {line}", stamp());
                let mut words = line.split_whitespace();
                let result = match (words.next().unwrap_or_default(), words.next()) {
                    ("w", Some(s)) => {
                        wait.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs_f64(s.parse()?));
                        waiting = true;
                        Ok(())
                    }
                    ("n", _) => player.next(),
                    ("p", _) => player.previous(),
                    ("t", _) => player.toggle(true),
                    ("s", Some(s)) => player.seek((s.parse::<f64>()? * 1000.0) as u32),
                    ("v", Some(v)) => player.set_volume((v.parse::<u32>()? * u16::MAX as u32 / 100) as u16),
                    ("sh", _) => { shuffle = !shuffle; player.shuffle(shuffle) }
                    ("r", _) => { repeat = (repeat + 1) % 3; player.repeat(repeat > 0, repeat == 2) }
                    ("q", Some(uri)) => player.add_to_queue(uri),
                    ("j", Some(uri)) => player.skip_to(uri),
                    ("x", _) => { player.shutdown(); Ok(()) }
                    _ => { eprintln!("n p t s<sec> v<%> sh r q<uri> j<uri> x"); Ok(()) }
                };
                result?;
            }
        }
    }
    Ok(())
}

fn print_event(ms: u128, event: &PlayerEvent) {
    match event {
        PlayerEvent::PositionChanged { position_ms } => {
            eprint!("\r[{ms:>6} ms] position {:>5.1} s ", *position_ms as f64 / 1000.0);
            let _ = std::io::stderr().flush();
        }
        PlayerEvent::TrackChanged { track } => {
            LAST_DURATION.store(track.duration_ms, Ordering::Relaxed);
            println!(
                "\n[{ms:>6} ms] TrackChanged {} — {} · {} · {} s · {:?}",
                track.name,
                track.artists.join(", "),
                track.album,
                track.duration_ms / 1000,
                track.cover_url
            )
        }
        PlayerEvent::QueueChanged { context_uri, current, next } => println!(
            "\n[{ms:>6} ms] QueueChanged {context_uri} current={current:?} next={} [{}]",
            next.len(),
            next.iter().take(3).map(|t| format!("{} ({})", t.uri, t.provider)).collect::<Vec<_>>().join(", ")
        ),
        other => println!("\n[{ms:>6} ms] {other:?}"),
    }
}

/// Waits for an event that matches, printing everything on the way.
async fn wait_for(
    events: &mut mpsc::UnboundedReceiver<PlayerEvent>,
    stamp: impl Fn() -> u128,
    what: &str,
    matches: impl Fn(&PlayerEvent) -> bool,
) -> Result<(u128, PlayerEvent)> {
    let deadline = tokio::time::sleep(Duration::from_secs(60));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            event = events.recv() => {
                let event = event.context("player stopped")?;
                let at = stamp();
                print_event(at, &event);
                if matches(&event) {
                    return Ok((at, event));
                }
            }
            _ = &mut deadline => bail!("timed out waiting for {what}"),
        }
    }
}

async fn pause_for(events: &mut mpsc::UnboundedReceiver<PlayerEvent>, stamp: impl Fn() -> u128, time: Duration) {
    let end = tokio::time::sleep(time);
    tokio::pin!(end);
    loop {
        tokio::select! {
            Some(event) = events.recv() => print_event(stamp(), &event),
            _ = &mut end => return,
        }
    }
}

async fn run_demo(
    player: &Player,
    events: &mut mpsc::UnboundedReceiver<PlayerEvent>,
    stamp: impl Fn() -> u128 + Copy,
) -> Result<()> {
    let is_playing = |e: &PlayerEvent| matches!(e, PlayerEvent::Playing { .. });
    let (first_audio, _) = wait_for(events, stamp, "the first song", is_playing).await?;
    println!("\n>>> first song audible after {first_audio} ms");
    pause_for(events, stamp, Duration::from_secs(3)).await;

    let at = stamp();
    player.pause()?;
    let (paused, _) = wait_for(events, stamp, "pause", |e| matches!(e, PlayerEvent::Paused { .. })).await?;
    println!("\n>>> pause took {} ms", paused - at);
    pause_for(events, stamp, Duration::from_secs(1)).await;
    player.toggle(true)?;
    wait_for(events, stamp, "resume", is_playing).await?;

    // Near the end of the song, to see mpv move on to the one loaded ahead.
    let duration = LAST_DURATION.load(Ordering::Relaxed);
    let at = stamp();
    player.seek(duration.saturating_sub(6_000))?;
    let (seeked, _) = wait_for(events, stamp, "seek", |e| matches!(e, PlayerEvent::Seeked { .. })).await?;
    println!("\n>>> seek took {} ms", seeked - at);
    let (changed, _) =
        wait_for(events, stamp, "the next song", |e| matches!(e, PlayerEvent::TrackChanged { .. })).await?;
    let (playing, _) = wait_for(events, stamp, "the next song playing", is_playing).await?;
    println!(
        "\n>>> moved on by itself: TrackChanged → Playing in {} ms (see the debug log for mpv's gap)",
        playing - changed
    );
    pause_for(events, stamp, Duration::from_secs(3)).await;

    let at = stamp();
    player.next()?;
    let (playing, _) = wait_for(events, stamp, "next", is_playing).await?;
    println!("\n>>> next: audible after {} ms", playing - at);
    pause_for(events, stamp, Duration::from_secs(1)).await;

    let at = stamp();
    player.previous()?;
    let (playing, _) = wait_for(events, stamp, "previous", is_playing).await?;
    println!("\n>>> previous (within 3 s) went back: audible after {} ms", playing - at);
    pause_for(events, stamp, Duration::from_secs(4)).await;
    player.previous()?;
    wait_for(events, stamp, "restart", |e| matches!(e, PlayerEvent::Seeked { position_ms } if *position_ms == 0))
        .await?;
    println!("\n>>> previous after 3 s restarted the song");

    player.shutdown();
    wait_for(events, stamp, "shutdown", |e| *e == PlayerEvent::Deactivated).await?;
    println!("\n>>> shut down");
    Ok(())
}

/// The length of the last song in a TrackChanged, for the demo's seek to its end.
static LAST_DURATION: AtomicU32 = AtomicU32::new(0);
