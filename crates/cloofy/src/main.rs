mod doctor;
mod playback;
mod style;

use anyhow::Result;
use clap::{Parser, Subcommand};
use cloofy_spotify::{config::Config, paths::Paths, session, webapi::WebApi};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "cloofy", version, about = "Spotify in your terminal")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Log in to Spotify (Web API and streaming)
    Login {
        /// Log in again, even where already connected
        #[arg(long)]
        force: bool,
    },
    /// Check which Spotify features are reachable
    Doctor,
    /// Play a song, album, playlist or artist directly in cloofy
    Play {
        /// Spotify URI or link, e.g. https://open.spotify.com/album/2noRn2Aes5aoNVsU6iWThc
        uri: String,
        /// Shuffle
        #[arg(short, long)]
        shuffle: bool,
    },
    /// Development: run a pathfinder query and print the JSON
    #[command(hide = true)]
    Query {
        operation: String,
        /// Variables as JSON
        #[arg(default_value = "{}")]
        variables: String,
    },
    /// Development: save the DJ narration for the next songs as MP3, with metadata
    #[command(hide = true)]
    DjSample {
        /// Target directory
        dir: std::path::PathBuf,
    },
    /// Remote mode: wait until a Spotify app picks cloofy as its device
    Connect {
        /// Name the device shows up as in Spotify
        #[arg(long, default_value = "cloofy")]
        name: String,
    },
}

fn main() -> Result<()> {
    // Names cloofy's stream in the system volume mixer (read by librespot's PulseAudio output).
    // SAFETY: no other threads exist yet, so nothing can read the environment concurrently.
    unsafe {
        std::env::set_var("PULSE_PROP_application.name", "cloofy");
        std::env::set_var("PULSE_PROP_application.icon_name", "audio-x-generic");
        std::env::set_var("PULSE_PROP_media.role", "music");
    }
    limit_malloc_arenas();
    tokio::runtime::Runtime::new()?.block_on(run())
}

/// glibc gives each thread that allocates its own heap arena and holds on to memory freed in
/// it; across tokio's worker pool that adds up to tens of megabytes. Two arenas are plenty.
fn limit_malloc_arenas() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: called before any other thread exists.
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
}

async fn run() -> Result<()> {
    // reqwest and librespot pull in both rustls backends, so rustls can't pick one by itself.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let cli = Cli::parse();
    let paths = Paths::resolve()?;
    init_logging(&paths, cli.command.is_none())?;
    match cli.command {
        Some(Command::Login { force }) => login(&paths, force).await,
        Some(Command::Doctor) => doctor::run(&paths).await,
        Some(Command::Play { uri, shuffle }) => playback::play(&paths, &uri, shuffle).await,
        Some(Command::Connect { name }) => playback::connect(&paths, name).await,
        Some(Command::Query { operation, variables }) => query(&paths, &operation, &variables).await,
        Some(Command::DjSample { dir }) => dj_sample(&paths, &dir).await,
        None => cloofy_tui::run(&paths).await,
    }
}

/// Logs go to stderr for CLI commands; the TUI owns the screen, so it logs to a file.
fn init_logging(paths: &Paths, tui: bool) -> Result<()> {
    let filter = EnvFilter::try_from_env("CLOOFY_LOG").unwrap_or_else(|_| EnvFilter::new("error"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if tui {
        std::fs::create_dir_all(&paths.cache)?;
        std::fs::File::create(paths.log_file())?;
        // Append mode: the TUI also redirects stderr into this file.
        let file = std::fs::OpenOptions::new().append(true).open(paths.log_file())?;
        builder.with_ansi(false).with_writer(std::sync::Mutex::new(file)).init();
    } else {
        builder.with_writer(std::io::stderr).init();
    }
    Ok(())
}

async fn login(paths: &Paths, force: bool) -> Result<()> {
    let config = Config::load(paths)?;
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

    println!("  Done. Test everything with {}\n", style::bold("cloofy doctor"));
    Ok(())
}

/// Fetches the DJ's lines for the first tracks of a fresh DJ session, without playing anything.
async fn dj_sample(paths: &Paths, dir: &std::path::Path) -> Result<()> {
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

async fn query(paths: &Paths, operation: &str, variables: &str) -> Result<()> {
    let session = session::prepare(paths)?.connect().await?;
    let pathfinder = cloofy_spotify::pathfinder::Pathfinder::new(session.clone(), paths.pathfinder_hashes());
    let data = pathfinder.query(operation, serde_json::from_str(variables)?).await?;
    println!("{}", serde_json::to_string_pretty(&data)?);
    session.shutdown();
    Ok(())
}
