//! Signing in to YouTube Music with a browser's cookies. Google's sign-in for TV apps no longer
//! works for YouTube Music, so cloofy borrows the session of a browser that is signed in:
//! yt-dlp reads its cookies (it knows every browser's cookie store and keyring), and cloofy
//! keeps only those of youtube.com. They sign in every Google account signed in to the
//! browser, so the sign-in also keeps which one it acts as.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use cloofy_core::{catalog::User, paths::Paths};
use tokio::process::Command;

use crate::{
    YtMusic,
    account::{self, Account, Origin},
    cookies::Jar,
    innertube::{self, InnerTube},
    library,
};

/// A browser yt-dlp can read cookies from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Browser {
    /// The name people know, e.g. "Brave".
    pub name: String,
    /// What `--cookies-from-browser` takes, e.g. `brave`, or `firefox:<profiles>` for forks.
    pub spec: String,
}

/// A browser cloofy knows.
struct Known {
    /// How the system names it: desktop entries on Linux, bundle ids on macOS.
    ids: &'static [&'static str],
    name: &'static str,
    /// yt-dlp's name for it, or for the browser it is a fork of.
    reader: &'static str,
    /// Where it keeps its profiles, relative to the home directory.
    dir: &'static str,
    /// Whether yt-dlp needs the folder: it doesn't know forks.
    fork: bool,
}

const fn known(ids: &'static [&'static str], name: &'static str, reader: &'static str, dir: &'static str) -> Known {
    Known { ids, name, reader, dir, fork: false }
}

const fn fork(ids: &'static [&'static str], name: &'static str, reader: &'static str, dir: &'static str) -> Known {
    Known { ids, name, reader, dir, fork: true }
}

#[cfg(target_os = "macos")]
const KNOWN: &[Known] = &[
    known(&["com.google.chrome"], "Google Chrome", "chrome", "Library/Application Support/Google/Chrome"),
    known(&["com.brave.browser"], "Brave", "brave", "Library/Application Support/BraveSoftware/Brave-Browser"),
    known(&["org.chromium.chromium"], "Chromium", "chromium", "Library/Application Support/Chromium"),
    known(&["com.microsoft.edgemac"], "Microsoft Edge", "edge", "Library/Application Support/Microsoft Edge"),
    known(&["com.vivaldi.vivaldi"], "Vivaldi", "vivaldi", "Library/Application Support/Vivaldi"),
    known(&["com.operasoftware.opera"], "Opera", "opera", "Library/Application Support/com.operasoftware.Opera"),
    known(&["org.mozilla.firefox"], "Firefox", "firefox", "Library/Application Support/Firefox"),
    known(&["com.apple.safari"], "Safari", "safari", "Library/Containers/com.apple.Safari"),
    fork(&["app.zen-browser.zen"], "Zen", "firefox", "Library/Application Support/zen"),
    fork(
        &["org.mozilla.librewolf", "io.gitlab.librewolf-community"],
        "LibreWolf",
        "firefox",
        "Library/Application Support/librewolf",
    ),
];
#[cfg(not(target_os = "macos"))]
const KNOWN: &[Known] = &[
    known(&["chromium.desktop", "chromium-browser.desktop"], "Chromium", "chromium", ".config/chromium"),
    known(&["brave-browser.desktop", "brave.desktop"], "Brave", "brave", ".config/BraveSoftware/Brave-Browser"),
    // Omarchy's Brave, which keeps its profiles apart from Brave's.
    fork(&["brave-origin.desktop"], "Brave Origin", "brave", ".config/BraveSoftware/Brave-Origin"),
    known(&["google-chrome.desktop"], "Google Chrome", "chrome", ".config/google-chrome"),
    known(&["microsoft-edge.desktop"], "Microsoft Edge", "edge", ".config/microsoft-edge"),
    known(&["vivaldi-stable.desktop", "vivaldi.desktop"], "Vivaldi", "vivaldi", ".config/vivaldi"),
    known(&["opera.desktop"], "Opera", "opera", ".config/opera"),
    known(&["firefox.desktop"], "Firefox", "firefox", ".mozilla/firefox"),
    known(&["firefox.desktop"], "Firefox", "firefox", ".config/mozilla/firefox"),
    fork(&["zen.desktop", "zen-browser.desktop"], "Zen", "firefox", ".zen"),
    fork(&["librewolf.desktop"], "LibreWolf", "firefox", ".librewolf"),
    fork(&["floorp.desktop"], "Floorp", "firefox", ".floorp"),
];

impl Known {
    /// The browser, when it has a profile in its folder: a folder alone may be left over, or
    /// made by another program.
    fn found(&self, home: &Path) -> Option<Browser> {
        let dir = home.join(self.dir);
        let profile = match self.reader {
            "firefox" => dir.join("profiles.ini").is_file(),
            "safari" => dir.is_dir(),
            _ => dir.join("Local State").is_file(),
        };
        let spec = if self.fork { format!("{}:{}", self.reader, dir.display()) } else { self.reader.into() };
        profile.then(|| Browser { name: self.name.into(), spec })
    }
}

/// YouTube's sign-in, which leads to YouTube Music afterwards.
const SIGN_IN_URL: &str =
    "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F";
/// Google's sign-in for one more account, as YouTube's "Add account" opens it: through
/// YouTube's own sign-in, which gives youtube.com the new account's cookies, to YouTube Music.
const ADD_ACCOUNT_URL: &str = "https://accounts.google.com/AddSession?service=youtube&passive=false&continue=\
     https%3A%2F%2Fwww.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26app%3Ddesktop%26next%3D\
     https%253A%252F%252Fmusic.youtube.com%252F";

/// yt-dlp exports the cookies of every site; this many seconds is plenty for that.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(120);

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The browsers on this computer that have a profile yt-dlp could read cookies from.
pub fn browsers() -> Vec<Browser> {
    let Some(home) = home() else { return Vec::new() };
    let mut found: Vec<Browser> = Vec::new();
    for browser in KNOWN.iter().filter_map(|known| known.found(&home)) {
        if !found.iter().any(|b| b.name == browser.name) {
            found.push(browser);
        }
    }
    found
}

/// The browser that opens links, when cloofy knows it and it has a profile.
pub fn default_browser() -> Option<Browser> {
    let id = default_browser_id()?;
    let home = home()?;
    KNOWN.iter().filter(|known| known.ids.contains(&id.as_str())).find_map(|known| known.found(&home))
}

/// The desktop entry of the browser that opens links.
#[cfg(not(target_os = "macos"))]
fn default_browser_id() -> Option<String> {
    // Omarchy's own entry only hands links on to the browser it names.
    let asks: [(&str, &[&str]); 3] = [
        ("omarchy-cmd-default-browser", &[]),
        ("xdg-settings", &["get", "default-web-browser"]),
        ("xdg-mime", &["query", "default", "x-scheme-handler/https"]),
    ];
    asks.into_iter().find_map(|(program, args)| {
        let output = std::process::Command::new(program).args(args).stderr(Stdio::null()).output().ok()?;
        let id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        (output.status.success() && !id.is_empty() && id != "omarchy-browser.desktop").then_some(id)
    })
}

/// The bundle id of the app that opens https links; Safari unless another was picked.
#[cfg(target_os = "macos")]
fn default_browser_id() -> Option<String> {
    let plist = home()?.join("Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist");
    let handlers = std::process::Command::new("plutil")
        .args(["-extract", "LSHandlers", "json", "-o", "-"])
        .arg(plist)
        .stderr(Stdio::null())
        .output()
        .ok()
        .and_then(|output| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok());
    let id = handlers
        .as_ref()
        .and_then(|handlers| handlers.as_array())
        .and_then(|handlers| handlers.iter().find(|handler| handler["LSHandlerURLScheme"] == "https"))
        .and_then(|handler| handler["LSHandlerRoleAll"].as_str())
        .unwrap_or("com.apple.safari");
    Some(id.to_lowercase())
}

/// Opens YouTube's sign-in in the browser that opens links; whether it could.
pub fn open_sign_in() -> bool {
    open::that_detached(SIGN_IN_URL).is_ok()
}

/// Opens Google's sign-in for one more account in the browser that opens links; whether it
/// could.
pub fn open_add_account() -> bool {
    open::that_detached(ADD_ACCOUNT_URL).is_ok()
}

/// A sign-in read from a browser or pasted, not kept yet: it replaces the one before only once
/// YouTube Music took it, so one that doesn't work leaves a working one in place. It acts as the
/// browser's first account until `choose` picks another.
pub struct SignIn {
    client: YtMusic,
}

impl SignIn {
    /// `browser` is where the cookies were read, as yt-dlp names it; `None` for pasted ones.
    fn new(jar: Jar, browser: Option<String>) -> Result<Self> {
        let client = YtMusic::unsaved(jar)?;
        client.api().set_origin(Origin { browser, user: None });
        Ok(Self { client })
    }

    /// The accounts it can act as: each Google account signed in to the browser, in Google's
    /// order, with its brand accounts. Fails when YouTube turns the cookies down.
    pub async fn accounts(&self) -> Result<Vec<Account>> {
        account::list(self.client.api()).await
    }

    /// Acts as `account` from now on, and once kept.
    pub fn choose(&self, account: &Account) {
        self.client.api().set_identity(account.identity.clone());
    }

    /// Asks YouTube Music whose sign-in it is; fails when it turns the cookies down. The
    /// account is kept with the sign-in, for when cloofy reads the browser's cookies again.
    pub async fn check(&self) -> Result<User> {
        let api = self.client.api();
        let user = library::me(api).await?;
        api.set_origin(Origin { user: Some(user.id.clone()), ..api.origin().unwrap_or_default() });
        Ok(user)
    }

    /// Keeps it in place of the sign-in before, with the account chosen and the cookies
    /// YouTube refreshed meanwhile.
    pub fn save(self, paths: &Paths) -> Result<()> {
        let text = self.client.api().sign_in_text().context("No cookies to keep")?;
        private_dir(paths)?;
        paths.ytmusic_cookies().write(text.as_bytes())
    }
}

/// Reads the sign-in of `browser`; `None` while it isn't signed in to YouTube. The browser's
/// cookie store may be locked while it runs (Chromium-based browsers on Windows); yt-dlp says
/// so. Where yt-dlp guesses the keyring wrong, on desktops it doesn't know such as Hyprland,
/// `browser` keeps the keyring that worked, for the next time.
pub async fn from_browser(paths: &Paths, browser: &mut Browser) -> Result<Option<SignIn>> {
    let (mut jar, mut stderr) = export(paths, &browser.spec).await?;
    let signed_in = |jar: &Jar| innertube::check_jar(jar).is_ok();
    if cfg!(target_os = "linux") && !signed_in(&jar) && undecrypted(&stderr) && !browser.spec.contains('+') {
        let spec = with_keyring(&browser.spec, "gnomekeyring");
        (jar, stderr) = export(paths, &spec).await?;
        if stderr.contains("secretstorage not available") {
            bail!(
                "yt-dlp needs Python's secretstorage to read {}'s cookies from the keyring: install \
                 python-secretstorage (Arch) or python3-secretstorage (Debian, Ubuntu, Fedora)",
                browser.name
            );
        }
        browser.spec = spec;
    }
    if signed_in(&jar) {
        return SignIn::new(jar, Some(browser.spec.clone())).map(Some);
    }
    // Cookies it couldn't decrypt may be the sign-in.
    if undecrypted(&stderr) {
        bail!("yt-dlp can't decrypt {}'s cookies{}", browser.name, reason(&stderr));
    }
    if jar.is_empty() {
        bail!("No YouTube cookies found in {}{}", browser.name, reason(&stderr));
    }
    Ok(None)
}

/// Reads the cookies again from the browser the sign-in of `api` came from, for when YouTube
/// stopped taking the ones kept: a browser that goes on using its sign-in gets new cookies for
/// it, and the copy cloofy took goes stale within days. They replace the kept ones only when
/// YouTube takes them and they sign in the same account as before. Fails otherwise, and for a
/// sign-in that didn't come from a browser.
pub(crate) async fn renew(api: &InnerTube, paths: &Paths) -> Result<User> {
    let origin = api.origin().context("Not signed in")?;
    let spec = origin.browser.as_deref().context("The sign-in didn't come from a browser")?;
    let (jar, stderr) = export(paths, spec).await?;
    innertube::check_jar(&jar).with_context(|| format!("The browser isn't signed in to YouTube{}", reason(&stderr)))?;
    let trial = InnerTube::unsaved(jar)?;
    trial.set_identity(api.identity().unwrap_or_default());
    let user = library::me(&trial).await?;
    if !same_account(&origin, &user) {
        bail!("The browser is signed in to another account now");
    }
    api.renew(trial.jar().context("Not signed in")?);
    Ok(user)
}

/// Whether `user` is the account the sign-in was made for. One from before cloofy kept the
/// account passes: the browser's account at the same place is the best there is to go by.
fn same_account(origin: &Origin, user: &User) -> bool {
    origin.user.as_ref().is_none_or(|id| *id == user.id)
}

/// The cookies of youtube.com in the browser of `spec`, and what yt-dlp printed to stderr.
async fn export(paths: &Paths, spec: &str) -> Result<(Jar, String)> {
    let dir = private_dir(paths)?;
    // yt-dlp writes every site's cookies; they stay in this private folder only until cloofy
    // picked out YouTube's. yt-dlp makes the file: it turns down one that has no cookies yet.
    let file = dir.join("browser-export.txt");
    let _ = fs::remove_file(&file);
    let result = export_cookies(spec, &file).await;
    let text = fs::read_to_string(&file).unwrap_or_default();
    let _ = fs::remove_file(&file);
    Ok((Jar::parse(&text).youtube_only(), result?))
}

/// Whether yt-dlp found cookies it couldn't decrypt, for want of the keyring's key.
fn undecrypted(stderr: &str) -> bool {
    stderr.contains("cannot decrypt") || stderr.contains("failed to decrypt")
}

/// `spec` reading the key from `keyring`: `brave:<profiles>` becomes `brave+gnomekeyring:<profiles>`.
fn with_keyring(spec: &str, keyring: &str) -> String {
    match spec.split_once(':') {
        Some((browser, profile)) => format!("{browser}+{keyring}:{profile}"),
        None => format!("{spec}+{keyring}"),
    }
}

/// Runs yt-dlp; returns what it printed to stderr. Its warnings stay on: they tell when it
/// couldn't decrypt cookies.
async fn export_cookies(spec: &str, file: &Path) -> Result<String> {
    let mut command = Command::new("yt-dlp");
    command
        .args(["--no-config", "--skip-download", "--cookies-from-browser", spec, "--cookies"])
        .arg(file)
        // Any youtube.com page will do; yt-dlp saves the cookies even though it can't use it.
        .arg("https://music.youtube.com/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .kill_on_drop(true);
    let output = match tokio::time::timeout(EXPORT_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) if err.kind() == std::io::ErrorKind::NotFound => {
            bail!("yt-dlp is not installed; it reads the browser's cookies")
        }
        Ok(Err(err)) => return Err(err).context("Can't run yt-dlp"),
        Err(_) => bail!("yt-dlp took too long to read the browser's cookies"),
    };
    Ok(String::from_utf8_lossy(&output.stderr).into_owned())
}

/// yt-dlp's last error, unless it's only that the page isn't a video.
fn reason(stderr: &str) -> String {
    stderr
        .lines()
        .rfind(|line| line.starts_with("ERROR:") && !line.contains("Unsupported URL"))
        .map(|line| format!(": {}", line.trim_start_matches("ERROR:").trim()))
        .unwrap_or_default()
}

/// Reads a `Cookie` header copied from the browser's developer tools, for when yt-dlp can't
/// read the browser.
pub fn from_cookie_header(header: &str) -> Result<SignIn> {
    let jar = Jar::from_header(header);
    innertube::check_jar(&jar)?;
    SignIn::new(jar, None)
}

/// Forgets the cookies; YouTube Music is signed out on the next start.
pub fn sign_out(paths: &Paths) -> Result<()> {
    paths.ytmusic_cookies().delete()
}

/// The folder of the sign-in, readable by the user only.
fn private_dir(paths: &Paths) -> Result<PathBuf> {
    let cookies = paths.ytmusic_cookies();
    let dir = cookies.file().parent().map(Path::to_path_buf).context("No folder for the cookies")?;
    fs::create_dir_all(&dir).with_context(|| format!("{} can't be created", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Identity;

    fn paths(name: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("cloofy-ytm-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        Paths { config: root.join("config"), data: root.join("data"), cache: root.join("cache"), keychain: false }
    }

    #[test]
    fn a_pasted_header_signs_in_and_out() {
        let paths = paths("header");
        assert!(from_cookie_header("PREF=x").is_err(), "not signed in without LOGIN_INFO");
        let sign_in = from_cookie_header("Cookie: SAPISID=abc; LOGIN_INFO=def; SID=ghi").unwrap();
        let file = paths.ytmusic_cookies().file().to_owned();
        assert!(!file.exists(), "kept only once it is saved");
        sign_in.save(&paths).unwrap();
        let saved = Jar::parse(&fs::read_to_string(&file).unwrap());
        assert_eq!(saved.get("SAPISID"), Some("abc"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(fs::metadata(file.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        }
        let client = crate::YtMusic::new(&paths).unwrap();
        assert!(client.signed_in());
        assert_eq!(client.api().identity(), Some(Identity::default()), "the first account unless one was picked");
        // One that YouTube Music turned down is never saved, which leaves this one.
        drop(from_cookie_header("SAPISID=other; LOGIN_INFO=other").unwrap());
        assert_eq!(Jar::parse(&fs::read_to_string(&file).unwrap()).get("SAPISID"), Some("abc"));
        sign_out(&paths).unwrap();
        assert!(!file.exists());
        assert!(!crate::YtMusic::new(&paths).unwrap().signed_in());
        sign_out(&paths).unwrap();
        fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_account_picked_is_kept_with_the_sign_in() {
        let paths = paths("account");
        let sign_in = from_cookie_header("SAPISID=abc; LOGIN_INFO=def").unwrap();
        let brand = Identity { index: 1, page_id: Some("100000000000000000099".into()) };
        sign_in.client.api().set_identity(brand.clone());
        sign_in.save(&paths).unwrap();
        let client = crate::YtMusic::new(&paths).unwrap();
        assert_eq!(client.api().identity(), Some(brand));
        assert_eq!(client.api().jar().unwrap().get("SAPISID"), Some("abc"));
        fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }

    /// Issue #26: a browser that goes on using its sign-in makes the cookies cloofy copied go
    /// stale. The sign-in keeps where it came from and whose it is, to read it there again.
    #[test]
    fn where_the_sign_in_came_from_is_kept_with_it() {
        let paths = paths("origin");
        let jar = Jar::from_header("SAPISID=abc; LOGIN_INFO=def");
        let sign_in = SignIn::new(jar, Some("firefox:/home/x/.zen".into())).unwrap();
        // As `check` does once YouTube named the account.
        let api = sign_in.client.api();
        api.set_origin(Origin { user: Some("UCabc".into()), ..api.origin().unwrap() });
        sign_in.save(&paths).unwrap();

        let client = crate::YtMusic::new(&paths).unwrap();
        let origin = Origin { browser: Some("firefox:/home/x/.zen".into()), user: Some("UCabc".into()) };
        assert_eq!(client.api().origin(), Some(origin));
        assert_eq!(client.api().jar().unwrap().get("SAPISID"), Some("abc"), "the notes aren't cookies");

        // New cookies from the browser go on as the same account, from the same browser.
        let brand = Identity { index: 1, page_id: None };
        client.api().set_identity(brand.clone());
        client.api().renew(Jar::from_header("SAPISID=new; LOGIN_INFO=def"));
        let again = crate::YtMusic::new(&paths).unwrap();
        assert_eq!(again.api().jar().unwrap().get("SAPISID"), Some("new"), "kept right away");
        assert_eq!(again.api().identity(), Some(brand));
        assert_eq!(again.api().origin(), client.api().origin());
        fs::remove_dir_all(paths.data.parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn a_pasted_sign_in_has_no_browser_to_read_again() {
        let paths = paths("pasted");
        let sign_in = from_cookie_header("SAPISID=abc; LOGIN_INFO=def").unwrap();
        assert_eq!(sign_in.client.api().origin(), Some(Origin::default()));
        // Fails before yt-dlp or YouTube are asked anything.
        let err = renew(sign_in.client.api(), &paths).await.unwrap_err();
        assert!(format!("{err:#}").contains("didn't come from a browser"), "{err:#}");
    }

    #[test]
    fn cookies_read_again_must_be_the_same_account_s() {
        let user = |id: &str| User { id: id.into(), name: "Someone".into() };
        let kept = Origin { browser: Some("firefox".into()), user: Some("UCabc".into()) };
        assert!(same_account(&kept, &user("UCabc")));
        assert!(!same_account(&kept, &user("UCother")), "the browser signed in to another account");
        assert!(same_account(&Origin::default(), &user("UCother")), "nothing kept to compare with");
    }

    #[test]
    fn adding_an_account_comes_back_to_youtube_music() {
        let url = url::Url::parse(ADD_ACCOUNT_URL).unwrap();
        assert_eq!(url.host_str(), Some("accounts.google.com"));
        let query: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(query["service"], "youtube");
        let next = url::Url::parse(&query["continue"]).unwrap();
        assert_eq!(next.host_str(), Some("www.youtube.com"));
        let next: std::collections::HashMap<_, _> = next.query_pairs().collect();
        assert_eq!(next["next"], "https://music.youtube.com/");
    }

    #[test]
    fn a_cookies_file_from_yt_dlp_keeps_only_youtube() {
        let text = "# Netscape HTTP Cookie File\n\
            .youtube.com\tTRUE\t/\tTRUE\t1893456000\tSAPISID\tabc\n\
            .youtube.com\tTRUE\t/\tTRUE\t1893456000\tLOGIN_INFO\tdef\n\
            .bank.example\tTRUE\t/\tTRUE\t1893456000\tsession\tsecret\n\
            accounts.google.com\tFALSE\t/\tTRUE\t1893456000\tLSID\tx\n";
        let jar = Jar::parse(text).youtube_only();
        assert!(jar.to_text().contains("SAPISID"));
        assert!(!jar.to_text().contains("bank.example"));
        assert!(!jar.to_text().contains("LSID"));
    }

    #[test]
    fn a_browser_counts_once_it_has_a_profile() {
        let home = std::env::temp_dir().join(format!("cloofy-ytm-browsers-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let chromium = known(&[], "Chromium", "chromium", "chromium");
        let origin = fork(&[], "Brave Origin", "brave", "Brave-Origin");
        let firefox = known(&[], "Firefox", "firefox", "firefox");
        // Folders another program left, without a profile.
        fs::create_dir_all(home.join("chromium/NativeMessagingHosts")).unwrap();
        fs::create_dir_all(home.join("firefox")).unwrap();
        assert_eq!(chromium.found(&home), None);
        assert_eq!(firefox.found(&home), None);
        fs::write(home.join("chromium/Local State"), "{}").unwrap();
        fs::write(home.join("firefox/profiles.ini"), "").unwrap();
        assert_eq!(chromium.found(&home).unwrap().spec, "chromium");
        assert_eq!(firefox.found(&home).unwrap().spec, "firefox");
        // yt-dlp doesn't know forks, so it gets their folder.
        fs::create_dir_all(home.join("Brave-Origin")).unwrap();
        fs::write(home.join("Brave-Origin/Local State"), "{}").unwrap();
        assert_eq!(origin.found(&home).unwrap().spec, format!("brave:{}", home.join("Brave-Origin").display()));
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn names_the_keyring_before_the_profiles() {
        assert_eq!(with_keyring("chromium", "gnomekeyring"), "chromium+gnomekeyring");
        assert_eq!(
            with_keyring("brave:/home/x/Brave-Origin", "gnomekeyring"),
            "brave+gnomekeyring:/home/x/Brave-Origin"
        );
    }

    #[test]
    fn notices_cookies_it_could_not_decrypt() {
        assert!(undecrypted("WARNING: cannot decrypt v11 cookies: no key found\n"));
        assert!(undecrypted("WARNING: failed to decrypt cookie (AES-CBC) because UTF-8 decoding failed.\n"));
        assert!(!undecrypted("WARNING: [generic] Falling back on generic information extractor\n"));
    }

    #[test]
    fn explains_why_the_export_failed() {
        let stderr = "ERROR: could not find chromium cookies database in \"/home/x/.config/chromium\"\n";
        assert_eq!(reason(stderr), ": could not find chromium cookies database in \"/home/x/.config/chromium\"");
        assert_eq!(reason("ERROR: Unsupported URL: https://music.youtube.com/\n"), "");
    }
}
