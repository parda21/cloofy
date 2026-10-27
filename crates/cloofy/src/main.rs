// Without a service, the commands for one have nothing to hand their arguments to, and stop
// at the first service they would go through.
#![cfg_attr(not(feature = "spotify"), allow(unused_variables, clippy::never_loop))]

mod doctor;
#[cfg(feature = "spotify")]
mod media;
#[cfg(feature = "spotify")]
mod spotify;
mod style;
mod update;

use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, builder::PossibleValuesParser};
use cloofy_core::{config::Config, paths::Paths, service::Service};
use cloofy_tui::SERVICES;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "cloofy", version = version(), about = about())]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Sign in to your music services, or to the one named
    Login {
        /// The service; every one in use when left out
        #[arg(value_parser = PossibleValuesParser::new(Service::ALL.map(Service::key)))]
        service: Option<String>,
        /// Sign in again, even where already signed in
        #[arg(long)]
        force: bool,
    },
    /// Check what each of your music services can reach
    Doctor,
    /// Update cloofy to the latest release
    #[cfg_attr(not(any(feature = "spotify", feature = "ytmusic")), command(hide = true))]
    Update,
    /// Play a song, album, playlist or artist directly in cloofy
    Play {
        /// URI or link, e.g. https://open.spotify.com/album/2noRn2Aes5aoNVsU6iWThc
        uri: String,
        /// Shuffle
        #[arg(short, long)]
        shuffle: bool,
    },
    /// Development: run a pathfinder query and print the JSON
    #[cfg(feature = "spotify")]
    #[command(hide = true)]
    Query {
        operation: String,
        /// Variables as JSON
        #[arg(default_value = "{}")]
        variables: String,
    },
    /// Development: save the DJ narration for the next songs as MP3, with metadata
    #[cfg(feature = "spotify")]
    #[command(hide = true)]
    DjSample {
        /// Target directory
        dir: std::path::PathBuf,
    },
    /// Try the interface with a built-in sample library, no account needed (nothing plays)
    Demo,
    /// Remote mode: wait until a Spotify app picks cloofy as its device
    #[cfg(feature = "spotify")]
    Connect {
        /// Name the device shows up as in Spotify
        #[arg(long, default_value = "cloofy")]
        name: String,
    },
}

/// The version and the music services built in, like `0.4.0 (spotify)`. The installer reads
/// them from `cloofy --version`.
fn version() -> &'static str {
    static VERSION: LazyLock<String> = LazyLock::new(|| {
        let services: Vec<&str> = SERVICES.iter().map(|service| service.key()).collect();
        let services = if services.is_empty() { "demo only".to_owned() } else { services.join(", ") };
        format!("{} ({services})", env!("CARGO_PKG_VERSION"))
    });
    &VERSION
}

/// Like "Spotify in your terminal", naming the services built in.
fn about() -> &'static str {
    static ABOUT: LazyLock<String> = LazyLock::new(|| {
        let names: Vec<&str> = SERVICES.iter().map(|service| service.name()).collect();
        let names = if names.is_empty() { "Music".to_owned() } else { names.join(" and ") };
        format!("{names} in your terminal")
    });
    &ABOUT
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
    // The main thread belongs to the OS integration; cloofy itself runs on another one.
    cloofy_os::run_main(|| tokio::runtime::Runtime::new()?.block_on(run()))
}

/// glibc gives each thread that allocates its own heap arena and holds on to memory freed in
/// it; across tokio's worker pool that adds up to tens of megabytes. Two arenas are plenty.
///
/// glibc also raises its mmap threshold to the largest block freed so far, up to 32 MB. After
/// the first big cover, every image buffer and response body was carved from the arenas, which
/// only shrink when trimmed. A fixed threshold keeps large blocks in their own mappings, which
/// go back to the system as soon as they are freed.
fn limit_malloc_arenas() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: called before any other thread exists.
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 2);
        libc::mallopt(libc::M_MMAP_THRESHOLD, 256 * 1024);
    }
}

async fn run() -> Result<()> {
    // reqwest and librespot pull in both rustls backends, so rustls can't pick one by itself.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let cli = Cli::parse();
    let mut paths = Paths::resolve()?;
    paths.keychain = Config::load(&paths)?.keychain()?;
    init_logging(&paths, matches!(cli.command, None | Some(Command::Demo)))?;
    match cli.command {
        Some(Command::Login { service, force }) => login(&paths, service.as_deref(), force).await,
        Some(Command::Doctor) => doctor::run(&paths, &services(&paths, None)?).await,
        Some(Command::Update) => update::run().await,
        Some(Command::Play { uri, shuffle }) => play(&paths, &uri, shuffle).await,
        Some(Command::Demo) => update::restart_if_asked(cloofy_tui::demo(&paths).await?),
        #[cfg(feature = "spotify")]
        Some(Command::Connect { name }) => spotify::connect(&paths, name).await,
        #[cfg(feature = "spotify")]
        Some(Command::Query { operation, variables }) => spotify::query(&paths, &operation, &variables).await,
        #[cfg(feature = "spotify")]
        Some(Command::DjSample { dir }) => spotify::dj_sample(&paths, &dir).await,
        None => update::restart_if_asked(cloofy_tui::run(&paths).await?),
    }
}

/// The services a command is about: the one `named`, which must be built in, or else every
/// one in use.
fn services(paths: &Paths, named: Option<&str>) -> Result<Vec<Service>> {
    let Some(key) = named else {
        let enabled = Config::load(paths)?.enabled_services(SERVICES);
        if enabled.is_empty() {
            bail!("{}", cloofy_tui::no_service(paths));
        }
        return Ok(enabled);
    };
    let service = Service::from_key(key).with_context(|| format!("“{key}” is not a music service cloofy knows"))?;
    if !SERVICES.contains(&service) {
        bail!("This cloofy was built without {}.", service.name());
    }
    Ok(vec![service])
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

async fn login(paths: &Paths, named: Option<&str>, force: bool) -> Result<()> {
    let config = Config::load(paths)?;
    let services = services(paths, named)?;
    for &service in &services {
        if services.len() > 1 {
            println!("\n  {}", style::bold(service.name()));
        }
        match service {
            #[cfg(feature = "spotify")]
            Service::Spotify => spotify::login(paths, &config, force).await?,
            #[allow(unreachable_patterns)]
            other => bail!("{} isn't built into this cloofy", other.name()),
        }
    }
    println!("  Done. Test everything with {}\n", style::bold("cloofy doctor"));
    Ok(())
}

/// Plays a URI or link of a service without the interface.
async fn play(paths: &Paths, input: &str, shuffle: bool) -> Result<()> {
    // Something that isn't any service's goes to the one built in, to say what's wrong with it.
    match link_service(input).or(SERVICES.first().copied()) {
        #[cfg(feature = "spotify")]
        Some(Service::Spotify) => spotify::play(paths, input, shuffle).await,
        #[allow(unreachable_patterns)]
        Some(other) => bail!("{} isn't built into this cloofy", other.name()),
        None => bail!("{}", cloofy_tui::no_service(paths)),
    }
}

/// The service of a URI, or of a link to its website.
fn link_service(input: &str) -> Option<Service> {
    if input.starts_with("https://open.spotify.com/") {
        Some(Service::Spotify)
    } else if input.starts_with("https://music.youtube.com/") {
        Some(Service::YouTubeMusic)
    } else {
        Service::of(input)
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn links_go_to_their_service() {
        assert_eq!(link_service("https://open.spotify.com/album/2noRn2Aes5aoNVsU6iWThc"), Some(Service::Spotify));
        assert_eq!(link_service("spotify:track:1"), Some(Service::Spotify));
        assert_eq!(link_service("https://music.youtube.com/watch?v=dQw4w9WgXcQ"), Some(Service::YouTubeMusic));
        assert_eq!(link_service("ytmusic:album:MPREb_1"), Some(Service::YouTubeMusic));
        assert_eq!(link_service("https://example.com/"), None);
    }

    #[test]
    fn says_which_services_are_built_in() {
        assert!(version().starts_with(env!("CARGO_PKG_VERSION")));
        assert_eq!(version().contains("spotify"), cfg!(feature = "spotify"));
        Cli::command().debug_assert();
    }
}
