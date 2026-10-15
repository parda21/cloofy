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
}

impl Paths {
    pub fn resolve() -> Result<Self> {
        let base = etcetera::choose_base_strategy().context("Home directory not found")?;
        Ok(Self {
            config: base.config_dir().join(APP),
            data: base.data_dir().join(APP),
            cache: base.cache_dir().join(APP),
        })
    }

    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    pub fn webapi_token(&self) -> PathBuf {
        self.data.join("webapi-token.json")
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
