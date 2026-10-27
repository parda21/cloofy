use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use etcetera::BaseStrategy;

const APP: &str = "cloofy";

/// Where cloofy keeps its files: the XDG layout, on macOS as well.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    /// Sign-ins go to the system's keychain instead of files (`credentials = "keychain"`).
    pub keychain: bool,
}

impl Paths {
    pub fn resolve() -> Result<Self> {
        let base = etcetera::choose_base_strategy().context("Home directory not found")?;
        Ok(Self {
            config: base.config_dir().join(APP),
            data: base.data_dir().join(APP),
            cache: base.cache_dir().join(APP),
            keychain: false,
        })
    }

    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    pub fn webapi_token(&self) -> Secret {
        self.secret("webapi-token", self.data.join("webapi-token.json"))
    }

    /// librespot's reusable streaming credentials. In files, librespot keeps them itself.
    pub fn streaming_credentials(&self) -> Secret {
        self.secret("streaming", self.session_dir().join("credentials.json"))
    }

    /// The YouTube Music sign-in: youtube.com's cookies, borrowed from a browser, as a
    /// `cookies.txt`.
    pub fn ytmusic_cookies(&self) -> Secret {
        self.secret("ytmusic-cookies", self.data.join("ytmusic").join("cookies.txt"))
    }

    fn secret(&self, name: &'static str, file: PathBuf) -> Secret {
        if self.keychain { Secret::Keychain { name, file } } else { Secret::File(file) }
    }

    pub fn session_dir(&self) -> PathBuf {
        self.data.join("session")
    }

    pub fn device_id(&self) -> PathBuf {
        self.data.join("device-id")
    }

    pub fn audio_cache(&self) -> PathBuf {
        self.cache.join("audio")
    }

    /// Tracks being streamed; librespot copies each to the audio cache once it is complete.
    pub fn audio_tmp(&self) -> PathBuf {
        self.cache.join("tmp")
    }

    pub fn pathfinder_hashes(&self) -> PathBuf {
        self.cache.join("pathfinder-hashes.json")
    }

    /// Open view, selection and last track, restored on the next start.
    pub fn state_file(&self) -> PathBuf {
        self.data.join("state.json")
    }

    pub fn log_file(&self) -> PathBuf {
        self.cache.join("cloofy.log")
    }

    /// When cloofy last asked GitHub for a new release, and the answer.
    pub fn update_check(&self) -> PathBuf {
        self.cache.join("update-check.json")
    }
}

/// Where a sign-in is kept.
#[derive(Debug, Clone)]
pub enum Secret {
    /// A file only the user can read.
    File(PathBuf),
    /// The system's keychain, under `name`. A `file` from before moves over when read.
    Keychain { name: &'static str, file: PathBuf },
}

impl Secret {
    pub fn read(&self) -> Result<Option<Vec<u8>>> {
        match self {
            Secret::File(path) => read_file(path),
            Secret::Keychain { name, file } => {
                // A file wins: it was written by a login, or before the switch to the keychain.
                if let Some(bytes) = read_file(file)? {
                    cloofy_os::secrets::set(name, &bytes).context("Saving to the keychain failed")?;
                    fs::remove_file(file).with_context(|| format!("{} can't be removed", file.display()))?;
                    return Ok(Some(bytes));
                }
                cloofy_os::secrets::get(name).context("Reading from the keychain failed")
            }
        }
    }

    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        match self {
            Secret::File(path) => {
                write_private(path, bytes).with_context(|| format!("{} is not writable", path.display()))
            }
            Secret::Keychain { name, .. } => {
                cloofy_os::secrets::set(name, bytes).context("Saving to the keychain failed")
            }
        }
    }

    /// Forgets the secret wherever it is kept, a file from before the keychain included; fine
    /// when there is none.
    pub fn delete(&self) -> Result<()> {
        let file = self.file();
        match fs::remove_file(file) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).with_context(|| format!("{} can't be removed", file.display())),
        }
        if let Secret::Keychain { name, .. } = self {
            cloofy_os::secrets::delete(name).context("Removing it from the keychain failed")?;
        }
        Ok(())
    }

    /// The file the secret is kept in, or was before it moved to the keychain.
    pub fn file(&self) -> &Path {
        match self {
            Secret::File(path) | Secret::Keychain { file: path, .. } => path,
        }
    }
}

fn read_file(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("{} is not readable", path.display())),
    }
}

/// Writes a file only the current user can read, creating parent directories as needed.
pub fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cloofy-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn keeps_a_secret_in_a_private_file() {
        let dir = scratch("secret");
        let secret = Secret::File(dir.join("token.json"));
        assert_eq!(secret.read().unwrap(), None);
        secret.write(b"{}").unwrap();
        assert_eq!(secret.read().unwrap().as_deref(), Some(&b"{}"[..]));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join("token.json")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        secret.delete().unwrap();
        assert_eq!(secret.read().unwrap(), None);
        secret.delete().unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    /// Writes to the login keychain: run it by hand with
    /// `cargo test -p cloofy-core -- --ignored`.
    #[test]
    #[ignore = "uses the login keychain"]
    fn moves_a_file_into_the_keychain() {
        let dir = scratch("keychain");
        let name: &'static str = format!("test-{}", std::process::id()).leak();
        let file = dir.join("token.json");
        write_private(&file, b"from a file").unwrap();
        let secret = Secret::Keychain { name, file: file.clone() };
        assert_eq!(secret.read().unwrap().as_deref(), Some(&b"from a file"[..]));
        assert!(!file.exists(), "the file is gone once in the keychain");
        assert_eq!(cloofy_os::secrets::get(name).unwrap().as_deref(), Some(&b"from a file"[..]));
        secret.write(b"updated").unwrap();
        assert_eq!(secret.read().unwrap().as_deref(), Some(&b"updated"[..]));
        cloofy_os::secrets::delete(name).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}
