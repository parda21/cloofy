//! The commands for YouTube Music: signing in with a browser's sign-in, as one of the accounts
//! signed in there, and out again, its checks for `cloofy doctor`, and playing without the
//! interface.

mod playback;

use std::time::Duration;

use anyhow::{Result, bail};
use cloofy_core::{doctor::Section, paths::Paths};
use cloofy_ytmusic::{Account, YtMusic, auth, doctor};

pub use self::playback::play;
use crate::{prompt, style};

/// How long `cloofy login ytmusic` waits for a sign-in, or another account, in the browser.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// How often it looks whether the browser is signed in by now.
const SIGN_IN_POLL: Duration = Duration::from_secs(5);

/// Signs in with the sign-in of a browser, or with a pasted Cookie header, as the account
/// picked among those signed in there. `optional` asks first, for when it comes along with
/// signing in to another service. Returns whether it is signed in now.
pub async fn login(paths: &Paths, force: bool, optional: bool) -> Result<bool> {
    let client = YtMusic::new(paths)?;
    if client.signed_in() && !force {
        match who(&client).await {
            Ok(Some(who)) => {
                println!("  {} already signed in as {who}", style::green("✓"));
                println!("  {}\n", style::dim("`cloofy login ytmusic --force` signs in with another account"));
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
    let found = match auth::default_browser() {
        Some(browser) => match from_default_browser(paths, browser).await {
            Ok(found) => found,
            Err(err) => {
                println!("  {} {}\n", style::red("✗"), style::red(&format!("{err:#}")));
                pick_browser(paths).await?
            }
        },
        None => pick_browser(paths).await?,
    };
    let Some((sign_in, browser)) = found else {
        println!();
        return Ok(false);
    };
    let Some((sign_in, account)) = choose_account(paths, sign_in, browser).await? else {
        println!();
        return Ok(false);
    };
    // Cookies YouTube turns down aren't kept, and the sign-in before stays.
    if let Err(err) = sign_in.check().await {
        bail!("YouTube Music doesn't take this sign-in, so cloofy didn't keep it: {err:#}");
    }
    sign_in.save(paths)?;
    println!("  {} signed in as {}\n", style::green("✓"), account.label());
    Ok(true)
}

/// Who the sign-in acts as: the account with its address, else the name YouTube Music shows.
async fn who(client: &YtMusic) -> Result<Option<String>> {
    if let Some(account) = client.account().await? {
        return Ok(Some(account.label()));
    }
    Ok(client.me().await?.map(|user| user.name))
}

/// Asks which of the accounts signed in to the browser to sign in as; `None` when cancelled. A
/// browser's cookies sign in every Google account signed in to it, so cloofy never just takes
/// the first: that may be a work account. With cookies from a browser, another account can
/// sign in there right away.
async fn choose_account(
    paths: &Paths,
    mut sign_in: auth::SignIn,
    mut browser: Option<auth::Browser>,
) -> Result<Option<(auth::SignIn, Account)>> {
    let mut accounts = match sign_in.accounts().await {
        Ok(accounts) if !accounts.is_empty() => accounts,
        Ok(_) => bail!("YouTube Music doesn't take this sign-in, so cloofy didn't keep it: it names no account"),
        Err(err) => bail!("YouTube Music doesn't take this sign-in, so cloofy didn't keep it: {err:#}"),
    };
    // A pasted Cookie header of one account leaves nothing to pick.
    if browser.is_none() && accounts.len() == 1 {
        let account = accounts.remove(0);
        sign_in.choose(&account);
        return Ok(Some((sign_in, account)));
    }
    let mut selected = 0;
    loop {
        let mut items: Vec<String> = accounts.iter().map(Account::label).collect();
        if let Some(browser) = &browser {
            items.push(format!("Another account: sign in to it in {}", browser.name));
        }
        let Some(choice) = prompt::pick_from("Sign in to YouTube Music as", &items, selected)? else {
            return Ok(None);
        };
        if let Some(account) = accounts.get(choice) {
            sign_in.choose(account);
            return Ok(Some((sign_in, account.clone())));
        }
        let Some(browser) = browser.as_mut() else { return Ok(None) };
        if let Some((more, listed, added)) = add_account(paths, browser, &accounts).await? {
            (sign_in, accounts, selected) = (more, listed, added);
        }
    }
}

/// Opens Google's sign-in for another account in `browser`, and waits until its cookies show
/// an account that wasn't among `known`. Returns the new sign-in, its accounts and where the
/// new one is among them; `None` when none came in time.
async fn add_account(
    paths: &Paths,
    browser: &mut auth::Browser,
    known: &[Account],
) -> Result<Option<(auth::SignIn, Vec<Account>, usize)>> {
    if auth::open_add_account() {
        println!("  Sign in to the other account in the {} window that opened.", browser.name);
    } else {
        println!("  Open accounts.google.com in {} and sign in to the other account there.", browser.name);
    }
    println!("  {}", style::dim("Waiting for it… ctrl+c cancels"));
    let waited = tokio::time::timeout(SIGN_IN_TIMEOUT, async {
        loop {
            // Browsers write new cookies to disk a few seconds after they get them.
            tokio::time::sleep(SIGN_IN_POLL).await;
            let Some(sign_in) = auth::from_browser(paths, browser).await? else { continue };
            // Signing in to another account may leave the cookies unusable for a moment.
            let Ok(accounts) = sign_in.accounts().await else { continue };
            if let Some(added) = accounts.iter().position(|account| !known.iter().any(|k| k.same_as(account))) {
                return anyhow::Ok((sign_in, accounts, added));
            }
        }
    })
    .await;
    match waited {
        Ok(found) => found.map(Some),
        Err(_) => {
            println!("  {}\n", style::dim(&format!("– no other account in {} within 5 minutes", browser.name)));
            Ok(None)
        }
    }
}

/// Takes the sign-in of the browser that opens links, and the browser, to read again for
/// another account. One that isn't signed in to YouTube gets YouTube's sign-in, and cloofy
/// waits until its cookies show it signed in. `None` when that doesn't happen in time.
async fn from_default_browser(
    paths: &Paths,
    mut browser: auth::Browser,
) -> Result<Option<(auth::SignIn, Option<auth::Browser>)>> {
    println!("  {}", style::dim(&format!("Reading {}'s cookies, which may ask for your keyring…", browser.name)));
    if let Some(sign_in) = auth::from_browser(paths, &mut browser).await? {
        return Ok(Some((sign_in, Some(browser))));
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
        Ok(sign_in) => sign_in.map(|sign_in| Some((sign_in, Some(browser)))),
        Err(_) => {
            println!("  {}", style::dim(&format!("– no sign-in in {} within 5 minutes", browser.name)));
            Ok(None)
        }
    }
}

/// Takes the sign-in of a browser picked from a list, with the browser, or a pasted Cookie
/// header. `None` when cancelled.
async fn pick_browser(paths: &Paths) -> Result<Option<(auth::SignIn, Option<auth::Browser>)>> {
    let mut browsers = auth::browsers();
    let mut items: Vec<String> = browsers.iter().map(|browser| browser.name.clone()).collect();
    items.push("Another way: paste a Cookie header".into());
    let Some(choice) = prompt::pick("Sign in with", &items)? else { return Ok(None) };
    if choice < browsers.len() {
        let mut browser = browsers.swap_remove(choice);
        println!("  {}", style::dim(&format!("Reading {}'s cookies, which may ask for your keyring…", browser.name)));
        return match auth::from_browser(paths, &mut browser).await? {
            Some(sign_in) => Ok(Some((sign_in, Some(browser)))),
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
    auth::from_cookie_header(&header).map(|sign_in| Some((sign_in, None)))
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
