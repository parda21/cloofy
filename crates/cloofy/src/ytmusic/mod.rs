//! The commands for YouTube Music: signing in with a browser's sign-in and out again, its
//! checks for `cloofy doctor`, and playing without the interface.

mod playback;

use anyhow::{Result, bail};
use cloofy_core::{doctor::Section, paths::Paths};
use cloofy_ytmusic::{YtMusic, auth, doctor};

pub use self::playback::play;
use crate::{prompt, style};

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
        bail!(
            "Signing in to YouTube Music asks which browser to take the sign-in from: run `cloofy login ytmusic` in a terminal."
        );
    }
    if optional && !prompt::confirm("Sign in to YouTube Music?", false)? {
        println!("  {}\n", style::dim("– skipped: `cloofy login ytmusic` signs in later"));
        return Ok(false);
    }
    println!("  cloofy borrows the sign-in of a browser where you're signed in to music.youtube.com.\n");
    let browsers = auth::browsers();
    let mut items: Vec<String> = browsers.iter().map(|browser| browser.name.clone()).collect();
    items.push("Another way: paste a Cookie header".into());
    let Some(choice) = prompt::pick("Sign in with", &items)? else {
        println!();
        return Ok(false);
    };
    match browsers.get(choice) {
        Some(browser) => {
            println!(
                "  {}",
                style::dim(&format!("Reading {}'s cookies, which may ask for your keyring…", browser.name))
            );
            auth::import_from_browser(paths, browser).await?;
        }
        None => {
            println!();
            println!("  In a browser signed in to music.youtube.com, open the developer tools, then");
            println!("  Network, reload, pick a request to music.youtube.com and copy its Cookie header.");
            let Some(header) = prompt::secret("Paste it here and press enter:")? else {
                println!("  {}\n", style::dim("cancelled"));
                return Ok(false);
            };
            auth::import_cookie_header(paths, &header)?;
        }
    }
    // Cookies YouTube turns down aren't kept.
    let user = match YtMusic::new(paths)?.me().await {
        Ok(user) => user,
        Err(err) => {
            auth::sign_out(paths)?;
            bail!("YouTube Music doesn't take this sign-in, so cloofy didn't keep it: {err:#}");
        }
    };
    let name = user.map(|user| user.name).unwrap_or_default();
    println!("  {} signed in as {name}\n", style::green("✓"));
    Ok(true)
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
