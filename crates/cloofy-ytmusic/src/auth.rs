//! Signing in to YouTube Music with a browser's cookies. Google's sign-in for TV apps no longer
//! works for YouTube Music, so cloofy borrows the session of a browser that is signed in:
//! yt-dlp reads its cookies (it knows every browser's cookie store and keyring), and cloofy
//! keeps only those of youtube.com.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use cloofy_core::{
    catalog::User,
    paths::{Paths, write_private},
};
use tokio::process::Command;

use crate::{YtMusic, cookies::Jar, innertube, library};

/// A browser yt-dlp can read cookies from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Browser {
    /// The name people know, e.g. "Brave".
    pub name: String,
    /// What `--cookies-from-browser` takes, e.g. `brave`, or `firefox:<profiles>` for forks.
    pub spec: String,
}

/// yt-dlp's name for each browser, its display name, and where it keeps profiles, relative to
/// the home directory.
#[cfg(target_os = "macos")]
const KNOWN: &[(&str, &str, &str)] = &[
    ("chrome", "Google Chrome", "Library/Application Support/Google/Chrome"),
    ("brave", "Brave", "Library/Application Support/BraveSoftware/Brave-Browser"),
    ("chromium", "Chromium", "Library/Application Support/Chromium"),
    ("edge", "Microsoft Edge", "Library/Application Support/Microsoft Edge"),
    ("vivaldi", "Vivaldi", "Library/Application Support/Vivaldi"),
    ("opera", "Opera", "Library/Application Support/com.operasoftware.Opera"),
    ("firefox", "Firefox", "Library/Application Support/Firefox"),
    ("safari", "Safari", "Library/Containers/com.apple.Safari"),
];
#[cfg(not(target_os = "macos"))]
const KNOWN: &[(&str, &str, &str)] = &[
    ("chromium", "Chromium", ".config/chromium"),
    ("brave", "Brave", ".config/BraveSoftware/Brave-Browser"),
    ("chrome", "Google Chrome", ".config/google-chrome"),
    ("edge", "Microsoft Edge", ".config/microsoft-edge"),
    ("vivaldi", "Vivaldi", ".config/vivaldi"),
    ("opera", "Opera", ".config/opera"),
    ("firefox", "Firefox", ".mozilla/firefox"),
    ("firefox", "Firefox", ".config/mozilla/firefox"),
];
/// Firefox's forks, which yt-dlp reads as Firefox given their profiles folder.
#[cfg(not(target_os = "macos"))]
const FIREFOX_FORKS: &[(&str, &str)] = &[("Zen", ".zen"), ("LibreWolf", ".librewolf"), ("Floorp", ".floorp")];
#[cfg(target_os = "macos")]
const FIREFOX_FORKS: &[(&str, &str)] =
    &[("Zen", "Library/Application Support/zen"), ("LibreWolf", "Library/Application Support/librewolf")];

/// yt-dlp exports the cookies of every site; this many seconds is plenty for that.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(120);

/// The browsers on this computer that have a profile yt-dlp could read cookies from.
pub fn browsers() -> Vec<Browser> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
    let mut found: Vec<Browser> = Vec::new();
    for (spec, name, dir) in KNOWN {
        if home.join(dir).is_dir() && !found.iter().any(|b| b.spec == *spec) {
            found.push(Browser { name: (*name).into(), spec: (*spec).into() });
        }
    }
    for (name, dir) in FIREFOX_FORKS {
        let profiles = home.join(dir);
        if profiles.is_dir() {
            found.push(Browser { name: (*name).into(), spec: format!("firefox:{}", profiles.display()) });
        }
    }
    found
}

/// A sign-in read from a browser or pasted, not kept yet: it replaces the one before only once
/// YouTube Music took it, so one that doesn't work leaves a working one in place.
pub struct SignIn {
    client: YtMusic,
}

impl SignIn {
    fn new(jar: Jar) -> Result<Self> {
        Ok(Self { client: YtMusic::unsaved(jar)? })
    }

    /// Asks YouTube Music whose sign-in it is; fails when it turns the cookies down.
    pub async fn check(&self) -> Result<User> {
        library::me(self.client.api()).await
    }

    /// Keeps it in place of the sign-in before, with the cookies YouTube refreshed meanwhile.
    pub fn save(self, paths: &Paths) -> Result<()> {
        let jar = self.client.api().jar().context("No cookies to keep")?;
        private_dir(paths)?;
        save(paths, &jar)
    }
}

/// Reads the cookies of `browser`, which must be signed in to YouTube. The browser's cookie
/// store may be locked while it runs (Chromium-based browsers on Windows); yt-dlp says so.
pub async fn from_browser(paths: &Paths, browser: &Browser) -> Result<SignIn> {
    let dir = private_dir(paths)?;
    // yt-dlp writes every site's cookies; they stay in this private folder only until cloofy
    // picked out YouTube's.
    let export = dir.join("browser-export.txt");
    write_private(&export, b"")?;
    let result = export_cookies(&browser.spec, &export).await;
    let text = fs::read_to_string(&export).unwrap_or_default();
    let _ = fs::remove_file(&export);
    let stderr = result?;
    let jar = Jar::parse(&text).youtube_only();
    if jar.is_empty() {
        bail!("No YouTube cookies found in {}{}", browser.name, reason(&stderr));
    }
    innertube::check_jar(&jar)
        .with_context(|| format!("{} isn't signed in to YouTube – sign in at music.youtube.com first", browser.name))?;
    SignIn::new(jar)
}

/// Runs yt-dlp; returns what it printed to stderr.
async fn export_cookies(spec: &str, file: &Path) -> Result<String> {
    let mut command = Command::new("yt-dlp");
    command
        .args(["--no-config", "--skip-download", "--no-warnings", "--cookies-from-browser", spec, "--cookies"])
        .arg(file)
        // Any youtube.com page will do; yt-dlp saves the cookies even though it can't use it.
        .arg("https://music.youtube.com/")
        .stdin(Stdio::null())
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
    SignIn::new(jar)
}

/// Forgets the cookies; YouTube Music is signed out on the next start.
pub fn sign_out(paths: &Paths) -> Result<()> {
    paths.ytmusic_cookies().delete()
}

fn save(paths: &Paths, jar: &Jar) -> Result<()> {
    paths.ytmusic_cookies().write(jar.to_text().as_bytes())
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
    fn explains_why_the_export_failed() {
        let stderr = "ERROR: could not find chromium cookies database in \"/home/x/.config/chromium\"\n";
        assert_eq!(reason(stderr), ": could not find chromium cookies database in \"/home/x/.config/chromium\"");
        assert_eq!(reason("ERROR: Unsupported URL: https://music.youtube.com/\n"), "");
    }
}
