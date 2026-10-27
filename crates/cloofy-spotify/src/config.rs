use std::{fs, io, path::Path};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
/// A value to [`store`] in the config file.
pub use toml_edit::Value;

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
    /// Cover art on every page and in the player; on when missing.
    pub covers: Option<bool>,
    /// `"on"` or `"off"`: fewer redraws and no animations; follows the system when missing.
    pub power_saving: Option<String>,
    /// A color name such as `"violet"`; follows the playing cover when missing.
    pub accent: Option<String>,
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

/// Sets `key` in the config file, or removes it with `None` so the default applies. The rest
/// of the file stays as it was, comments included.
pub fn store(paths: &Paths, key: &str, value: Option<Value>) -> Result<()> {
    let path = paths.config_file();
    store_at(&path, key, value).with_context(|| format!("Couldn't save the setting to {}", path.display()))
}

fn store_at(path: &Path, key: &str, value: Option<Value>) -> Result<()> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err.into()),
    };
    let mut document: toml_edit::DocumentMut = text.parse()?;
    match value {
        Some(value) => document[key] = toml_edit::value(value),
        None => {
            document.remove(key);
        }
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // Write and rename, so a crash mid-write never leaves a broken file behind.
    let temp = path.with_extension("toml.tmp");
    fs::write(&temp, document.to_string())?;
    fs::rename(temp, path)?;
    Ok(())
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

    #[test]
    fn stores_settings_and_keeps_the_rest() {
        let dir = std::env::temp_dir().join(format!("cloofy-config-{}", std::process::id()));
        let path = dir.join("config.toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, "# my app\nclient_id = \"abc\" # from the dashboard\ncrossfade = 6\n").unwrap();

        store_at(&path, "covers", Some(false.into())).unwrap();
        store_at(&path, "crossfade", None).unwrap();
        store_at(&path, "accent", Some("violet".into())).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# my app\nclient_id = \"abc\" # from the dashboard\n"), "{text}");
        let config: Config = toml::from_str(&text).unwrap();
        assert_eq!(config.covers, Some(false));
        assert_eq!(config.crossfade, None);
        assert_eq!(config.accent.as_deref(), Some("violet"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn creates_the_file_for_the_first_setting() {
        let dir = std::env::temp_dir().join(format!("cloofy-config-new-{}", std::process::id()));
        let path = dir.join("config.toml");
        store_at(&path, "covers", Some(false.into())).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "covers = false\n");
        fs::remove_dir_all(dir).unwrap();
    }
}
