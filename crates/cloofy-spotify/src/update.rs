//! Updates from cloofy's releases on GitHub: a look for a new one now and then, and putting the
//! build for this machine in place of the running binary.

use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::paths::Paths;

const REPO: &str = "parda21/cloofy";
const USER_AGENT: &str = concat!("cloofy/", env!("CARGO_PKG_VERSION"));
/// GitHub answers 60 requests an hour without an account; a look every few hours is plenty.
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// Bounds the whole download, which the usual request timeout would cut short.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// What to run where cloofy can't replace itself.
pub const INSTALLER: &str = "curl -fsSL https://raw.githubusercontent.com/parda21/cloofy/main/install.sh | bash";

/// A package manager that installed cloofy. It owns the binary and updates it, so cloofy only
/// says how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Homebrew,
    /// The system's, such as pacman with cloofy-bin from the AUR.
    System,
    Nix,
}

impl PackageManager {
    /// The one that installed the running cloofy, if any.
    pub fn of_running() -> Option<Self> {
        // The real file, where a symlink such as Homebrew's in `bin` points to it.
        Self::of(&std::env::current_exe().ok()?.canonicalize().ok()?)
    }

    /// The one that owns the binary at `path`, by where each puts its files.
    fn of(path: &Path) -> Option<Self> {
        if path.starts_with("/nix/store") {
            Some(Self::Nix)
        } else if path.components().any(|part| part.as_os_str() == "Cellar") {
            Some(Self::Homebrew)
        } else if path.starts_with("/usr/bin") {
            Some(Self::System)
        } else {
            None
        }
    }

    /// How to update cloofy, as a sentence.
    pub fn how_to_update(self) -> &'static str {
        match self {
            Self::Homebrew => "Update with brew upgrade cloofy",
            Self::System => "Update with your package manager",
            Self::Nix => "Update with Nix",
        }
    }
}

/// A published release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// The tag, such as `v0.3.0`.
    pub tag: String,
}

impl Release {
    /// The version without the tag's `v`, such as `0.3.0`.
    pub fn version(&self) -> &str {
        self.tag.trim_start_matches('v')
    }
}

/// The version running. `CLOOFY_PRETEND_VERSION` stands in for it, to try updating.
pub fn current_version() -> String {
    std::env::var("CLOOFY_PRETEND_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_owned())
}

/// The latest release if it is newer than this cloofy. Asks GitHub at most every few hours and
/// remembers the answer in between; `now` asks regardless.
pub async fn check(paths: &Paths, now: bool) -> Result<Option<Release>> {
    let file = paths.update_check();
    let remembered = fs::read(&file)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Checked>(&bytes).ok())
        .filter(|checked| !now && seconds_since_epoch().saturating_sub(checked.at) < CHECK_INTERVAL.as_secs());
    let latest = match remembered {
        Some(checked) => checked.latest,
        None => {
            let latest = latest().await?;
            let checked = Checked { at: seconds_since_epoch(), latest: latest.clone() };
            if let Err(err) = remember(paths, &checked) {
                tracing::warn!("Couldn't remember the update check: {err}");
            }
            latest
        }
    };
    Ok(newer(latest.version(), &current_version()).then_some(latest))
}

fn remember(paths: &Paths, checked: &Checked) -> io::Result<()> {
    fs::create_dir_all(&paths.cache)?;
    fs::write(paths.update_check(), serde_json::to_vec(checked)?)
}

#[derive(Serialize, Deserialize)]
struct Checked {
    /// Seconds since the epoch.
    at: u64,
    latest: Release,
}

fn seconds_since_epoch() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The latest release, asked of GitHub.
pub async fn latest() -> Result<Release> {
    #[derive(Deserialize)]
    struct Latest {
        tag_name: String,
    }
    let latest: Latest = crate::http::client(USER_AGENT)
        .build()?
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(Release { tag: latest.tag_name })
}

/// Whether `version` comes after `current`, by major, minor and patch.
pub fn newer(version: &str, current: &str) -> bool {
    fn parts(version: &str) -> Option<(u64, u64, u64)> {
        let core = version.trim_start_matches('v').split(['-', '+']).next()?;
        let mut numbers = core.split('.').map(str::parse);
        Some((numbers.next()?.ok()?, numbers.next()?.ok()?, numbers.next().unwrap_or(Ok(0)).ok()?))
    }
    matches!((parts(version), parts(current)), (Some(version), Some(current)) if version > current)
}

/// The release workflow's name for a build that runs on this machine.
fn target() -> Option<&'static str> {
    Some(match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => "x86_64-unknown-linux-gnu",
        ("aarch64", "linux") => "aarch64-unknown-linux-gnu",
        ("x86_64", "macos") => "x86_64-apple-darwin",
        ("aarch64", "macos") => "aarch64-apple-darwin",
        _ => return None,
    })
}

/// Downloads `release` for this machine, checks it against its published checksum and puts it
/// in place of the running binary, which carries on until restarted. `progress` hears the bytes
/// that arrived so far and the total, when known. Returns where the new binary is.
pub async fn install(release: &Release, mut progress: impl FnMut(u64, Option<u64>)) -> Result<PathBuf> {
    if let Some(manager) = PackageManager::of_running() {
        bail!("A package manager installed cloofy, so it doesn't replace itself. {}", manager.how_to_update())
    }
    let Some(target) = target() else {
        bail!("There is no prebuilt cloofy for this system; update with `cargo install` instead")
    };
    // The real file, where a symlink points to it.
    let binary = std::env::current_exe()?.canonicalize()?;
    let dir = binary.parent().context("The cloofy binary has no folder")?;
    // Before downloading anything, make sure the binary can be replaced at all.
    let mut staged = Staged::create(dir)?;

    let archive_name = format!("cloofy-{target}.tar.gz");
    let base = format!("https://github.com/{REPO}/releases/download/{}", release.tag);
    let http = crate::http::client(USER_AGENT).timeout(DOWNLOAD_TIMEOUT).build()?;
    let mut response = http.get(format!("{base}/{archive_name}")).send().await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("cloofy {} has no build for {target}", release.version());
    }
    response = response.error_for_status()?;
    let total = response.content_length();
    let mut archive = Vec::with_capacity(total.unwrap_or_default() as usize);
    while let Some(chunk) = response.chunk().await? {
        archive.extend_from_slice(&chunk);
        progress(archive.len() as u64, total);
    }
    let checksums = http.get(format!("{base}/{archive_name}.sha256")).send().await?.error_for_status()?.text().await?;
    let expected = checksums.split_whitespace().next().context("The release has an empty checksum")?;
    if !expected.eq_ignore_ascii_case(&hex(&Sha256::digest(&archive))) {
        bail!("The download doesn't match its checksum, so nothing was changed");
    }

    staged.write(&unpack(&archive)?, &binary)?;
    // A build that can't start here, say for want of a system library, must not replace one that can.
    if !reports_version(&staged.path, release.version()).await {
        bail!("cloofy {} doesn't start on this system, so nothing was changed", release.version());
    }
    staged.replace(&binary)?;
    Ok(binary)
}

/// Whether `binary` starts and says it is `version`.
async fn reports_version(binary: &Path, version: &str) -> bool {
    let output = tokio::process::Command::new(binary).arg("--version").output().await;
    output.is_ok_and(|output| output.status.success() && String::from_utf8_lossy(&output.stdout).contains(version))
}

/// The `cloofy` binary from a release archive.
fn unpack(archive: &[u8]) -> Result<Vec<u8>> {
    let mut entries = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in entries.entries()? {
        let mut entry = entry?;
        if entry.header().entry_type().is_file() && entry.path()?.as_ref() == Path::new("cloofy") {
            let mut binary = Vec::new();
            entry.read_to_end(&mut binary)?;
            return Ok(binary);
        }
    }
    bail!("The release archive holds no cloofy binary")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The new binary, next to the old one so it can take its place in one step. Removed again
/// unless it does.
struct Staged {
    path: PathBuf,
    file: Option<fs::File>,
}

impl Staged {
    fn create(dir: &Path) -> Result<Self> {
        let path = dir.join(format!(".cloofy-update-{}", std::process::id()));
        let file = fs::OpenOptions::new().write(true).create(true).truncate(true).open(&path);
        match file {
            Ok(file) => Ok(Self { path, file: Some(file) }),
            Err(err) if matches!(err.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem) => {
                bail!("cloofy can't write to {}. Update with the installer instead:\n  {INSTALLER}", dir.display())
            }
            Err(err) => Err(err).with_context(|| format!("Couldn't write to {}", dir.display())),
        }
    }

    /// Writes `binary` with the permissions of the one it replaces, and closes the file: Linux
    /// doesn't start a program that is still open for writing.
    fn write(&mut self, binary: &[u8], replaces: &Path) -> Result<()> {
        let mut file = self.file.take().context("Already written")?;
        file.write_all(binary)?;
        file.sync_all()?;
        drop(file);
        fs::set_permissions(&self.path, fs::metadata(replaces)?.permissions())?;
        Ok(())
    }

    /// Takes the place of `binary`. Running processes keep the old file until they exit.
    fn replace(self, binary: &Path) -> Result<()> {
        fs::rename(&self.path, binary).with_context(|| format!("Couldn't replace {}", binary.display()))
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_by_their_numbers() {
        assert!(newer("v0.3.0", "0.2.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0.0", "0.99.0"));
        assert!(!newer("v0.2.0", "0.2.0"));
        assert!(!newer("0.1.9", "0.2.0"));
        // A prerelease counts as its version; a tag that isn't one never wins.
        assert!(!newer("0.2.0-beta", "0.2.0"));
        assert!(!newer("nightly", "0.2.0"));
    }

    #[test]
    fn knows_package_managers_by_where_the_binary_is() {
        let of = |path: &str| PackageManager::of(Path::new(path));
        assert_eq!(of("/opt/homebrew/Cellar/cloofy/0.3.0/bin/cloofy"), Some(PackageManager::Homebrew));
        assert_eq!(of("/usr/local/Cellar/cloofy/0.3.0/bin/cloofy"), Some(PackageManager::Homebrew));
        assert_eq!(of("/home/linuxbrew/.linuxbrew/Cellar/cloofy/0.3.0/bin/cloofy"), Some(PackageManager::Homebrew));
        assert_eq!(of("/usr/bin/cloofy"), Some(PackageManager::System));
        assert_eq!(
            of("/nix/store/0ac6p7kvsyhwlnh7z7mfqmzjy3zcvqqg-cloofy-0.3.0/bin/cloofy"),
            Some(PackageManager::Nix)
        );
        // Where the installer, cargo or a checkout put it, cloofy replaces itself.
        assert_eq!(of("/home/me/.local/bin/cloofy"), None);
        assert_eq!(of("/usr/local/bin/cloofy"), None);
        assert_eq!(of("/home/me/.cargo/bin/cloofy"), None);
        assert_eq!(of("/home/me/cloofy/target/release/cloofy"), None);
        assert_eq!(of("/usr/bin-old/cloofy"), None);
    }

    #[test]
    fn unpacks_the_binary_from_a_release_archive() {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        for (name, content) in [("LICENSE", &b"MIT"[..]), ("cloofy", &b"\x7fELF binary"[..])] {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, name, content).unwrap();
        }
        let archive = tar.into_inner().unwrap().finish().unwrap();
        assert_eq!(unpack(&archive).unwrap(), b"\x7fELF binary");
    }
}
