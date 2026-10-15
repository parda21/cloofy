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
    /// Desktop notifications for new tracks while cloofy is in the background; on when missing.
    pub notifications: Option<bool>,
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

    pub fn client_id(&self, paths: &Paths) -> Result<&str> {
        match self.client_id.as_deref().map(str::trim) {
            Some(id) if !id.is_empty() => Ok(id),
            _ => bail!("No client ID configured. Add it to {}:\n\n  client_id = \"…\"", paths.config_file().display()),
        }
    }
}
