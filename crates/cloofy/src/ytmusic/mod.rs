//! The commands for YouTube Music: signing in with a browser's sign-in and out again, its
//! checks for `cloofy doctor`, and playing without the interface.

mod playback;

use std::time::Duration;

use anyhow::{Result, bail};
use cloofy_core::{doctor::Section, paths::Paths};
use cloofy_ytmusic::{YtMusic, auth, doctor};

pub use self::playback::play;
use crate::{prompt, style};

/// How long `cloofy login ytmusic` waits for a sign-in in the browser.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// How often it looks whether the browser is signed in by now.
const SIGN_IN_POLL: Duration = Duration::from_secs(5);

/// Signs in with the sign-in of a browser, or with a pasted Cookie header. `optional` asks
/// first, for when it comes along with signing in to another service. Returns whether it is
/// signed in now.
pub async fn login(paths: &Paths, force: bool, optional: bool) -> Result<bool> {
    let client = YtMusic::new(paths)?;
    if client.signed_in() && !force {
        match client.me().await {
            Ok(Some(user)) => {
                println!("  {} already signed in as {}\n", style::green("✓"), user.name);
                return Ok(true);
            }
            Ok(None) => {}
            Err(err) => println!("  {} {}\n", style::red("✗"), style::red(&format!("{err:#}"))),
        }
    }
    println!("  Browsing and playing work without an account; signing in adds your library and likes.");
    if !prompt::interactive() {
        if optional {
            println!("  {}\n", style::dim("– skipped: `cloofy login ytmusic` in a terminal signs in"));
            return Ok(false);
        }
        bail!("Signing in to YouTube Music may need you in the browser: run `cloofy login ytmusic` in a terminal.");
    }
    if optional && !prompt::confirm("Sign in to YouTube Music?", false)? {
        println!("  {}\n", style::dim("– skipped: `cloofy login ytmusic` signs in later"));
        return Ok(false);
    }
    println!("  cloofy borrows your browser's sign-in to YouTube.\n");
    let sign_in = match auth::default_browser() {
        Some(browser) => match from_default_browser(paths, browser).await {
            Ok(sign_in) => sign_in,
            Err(err) => {
                println!("  {} {}\n", style::red("✗"), style::red(&format!("{err:#}")));
                pick_browser(paths).await?
            }
        },
        None => pick_browser(paths).await?,
    };
    let Some(sign_in) = sign_in else {
        println!();
        return Ok(false);
    };
    // Cookies YouTube turns down aren't kept, and the sign-in before stays.
    let user = match sign_in.check().await {
        Ok(user) => user,
        Err(err) => bail!("YouTube Music doesn't take this sign-in, so cloofy didn't keep it: {err:#}"),
    };
    sign_in.save(paths)?;
    println!("  {} signed in as {}\n", style::green("✓"), user.name);
    Ok(true)
}

/// Takes the sign-in of the browser that opens links. One that isn't signed in to YouTube gets
/// YouTube's sign-in, and cloofy waits until its cookies show it signed in. `None` when that
/// doesn't happen in time.
async fn from_default_browser(paths: &Paths, mut browser: auth::Browser) -> Result<Option<auth::SignIn>> {
    println!("  {}", style::dim(&format!("Reading {}'s cookies, which may ask for your keyring…", browser.name)));
    if let Some(sign_in) = auth::from_browser(paths, &mut browser).await? {
        return Ok(Some(sign_in));
    }
    if !auth::open_sign_in() {
        bail!(
            "{} isn't signed in to YouTube, and cloofy couldn't open it – sign in at music.youtube.com",
            browser.name
        );
    }
    println!("  {} isn't signed in to YouTube yet: sign in in the window that opened.", browser.name);
    println!("  {}", style::dim("Waiting for the sign-in… ctrl+c cancels"));
    let waited = tokio::time::timeout(SIGN_IN_TIMEOUT, async {
        loop {
            // Browsers write new cookies to disk a few seconds after they get them.
            tokio::time::sleep(SIGN_IN_POLL).await;
            if let Some(sign_in) = auth::from_browser(paths, &mut browser).await? {
                return anyhow::Ok(sign_in);
            }
        }
    })
    .await;
    match waited {
        Ok(sign_in) => sign_in.map(Some),
        Err(_) => {
            println!("  {}", style::dim(&format!("– no sign-in in {} within 5 minutes", browser.name)));
            Ok(None)
        }
    }
}

/// Takes the sign-in of a browser picked from a list, or a pasted Cookie header. `None` when
/// cancelled.
async fn pick_browser(paths: &Paths) -> Result<Option<auth::SignIn>> {
    let mut browsers = auth::browsers();
    let mut items: Vec<String> = browsers.iter().map(|browser| browser.name.clone()).collect();
    items.push("Another way: paste a Cookie header".into());
    let Some(choice) = prompt::pick("Sign in with", &items)? else { return Ok(None) };
    if let Some(browser) = browsers.get_mut(choice) {
        println!("  {}", style::dim(&format!("Reading {}'s cookies, which may ask for your keyring…", browser.name)));
        return match auth::from_browser(paths, browser).await? {
            Some(sign_in) => Ok(Some(sign_in)),
            None => bail!("{} isn't signed in to YouTube – sign in at music.youtube.com first", browser.name),
        };
    }
    println!();
    println!("  In a browser signed in to music.youtube.com, open the developer tools, then");
    println!("  Network, reload, pick a request to music.youtube.com and copy its Cookie header.");
    let Some(header) = prompt::secret("Paste it here and press enter:")? else {
        println!("  {}", style::dim("cancelled"));
        return Ok(None);
    };
    auth::from_cookie_header(&header).map(Some)
}

/// Whether a sign-in is saved.
pub fn signed_in(paths: &Paths) -> Result<bool> {
    Ok(YtMusic::new(paths)?.signed_in())
}

/// Forgets the sign-in; YouTube Music browses and plays signed out from then on.
pub fn logout(paths: &Paths) -> Result<()> {
    let signed_in = YtMusic::new(paths)?.signed_in();
    auth::sign_out(paths)?;
    let text = if signed_in { "Signed out of YouTube Music" } else { "Not signed in to YouTube Music" };
    println!("\n  {} {text}\n", style::green("✓"));
    Ok(())
}

/// What `cloofy doctor` finds for YouTube Music, each section handed to `report` as soon as
/// it is done.
pub async fn doctor(paths: &Paths, mut report: impl FnMut(Section)) -> Result<()> {
    let client = YtMusic::new(paths)?;
    report(doctor::programs().await);
    report(doctor::innertube(&client).await);
    report(doctor::playback().await);
    Ok(())
}
