use std::{fs, io};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::paths::Paths;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Client ID of the user's own app from developer.spotify.com.
    pub client_id: Option<String>,
    /// Seconds consecutive tracks overlap (0–12); off when missing.
    pub crossfade: Option<u64>,
    /// `"nerd"` or `"text"`; detected from the terminal when missing.
    pub icons: Option<String>,
    /// `"keychain"` keeps sign-ins in the system's keychain; files when missing.
    pub credentials: Option<String>,
    /// Desktop notifications for new tracks while cloofy is in the background; on when missing.
    pub notifications: Option<bool>,
    /// Looks for a new release at start and offers to update; on when missing.
    pub update_check: Option<bool>,
}

impl Config {
    /// Reads `config.toml`; `CLOOFY_CLIENT_ID` overrides the file.
    pub fn load(paths: &Paths) -> Result<Self> {
        let path = paths.config_file();
        let mut config = match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("{} is not valid TOML", path.display()))?,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(err) => return Err(err).with_context(|| format!("{} is not readable", path.display())),
        };
        if let Ok(id) = std::env::var("CLOOFY_CLIENT_ID") {
            config.client_id = Some(id);
        }
        Ok(config)
    }

    /// Whether sign-ins go to the system's keychain rather than files.
    pub fn keychain(&self) -> Result<bool> {
        match self.credentials.as_deref() {
            None | Some("files") => Ok(false),
            Some("keychain") if cloofy_os::secrets::available() => Ok(true),
            Some("keychain") => bail!("credentials = \"keychain\" works on macOS only for now"),
            Some(other) => bail!("credentials = \"{other}\" is unknown: use \"keychain\" or \"files\""),
        }
    }

    pub fn client_id(&self, paths: &Paths) -> Result<&str> {
        match self.client_id.as_deref().map(str::trim) {
            Some(id) if !id.is_empty() => Ok(id),
            _ => bail!("No client ID configured. Add it to {}:\n\n  client_id = \"…\"", paths.config_file().display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credentials(value: Option<&str>) -> Config {
        Config { credentials: value.map(Into::into), ..Config::default() }
    }

    #[test]
    fn keeps_sign_ins_in_files_unless_asked() {
        assert!(!credentials(None).keychain().unwrap());
        assert!(!credentials(Some("files")).keychain().unwrap());
        assert_eq!(credentials(Some("keychain")).keychain().is_ok(), cfg!(target_os = "macos"));
        assert!(credentials(Some("vault")).keychain().is_err());
    }
}
