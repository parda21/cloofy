//! `cloofy update`, and starting the new binary after the interface installed an update.

use std::{
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use cloofy_core::update;

use crate::style;

const BAR_WIDTH: usize = 30;

pub async fn run() -> Result<()> {
    let current = update::current_version();
    let release = update::latest().await.context("Couldn't ask GitHub for the latest release")?;
    println!();
    if !update::newer(release.version(), &current) {
        println!("  {} cloofy {current} is the latest version\n", style::green("✓"));
        return Ok(());
    }
    println!("  {} cloofy {} is out, this is {current}", style::cyan("↑"), style::bold(release.version()));
    if let Some(manager) = update::PackageManager::of_running() {
        println!("    {}\n", manager.how_to_update());
        return Ok(());
    }
    let mut progress = Progress::default();
    let installed = update::install(&release, |done, total| progress.draw(done, total)).await;
    progress.clear();
    let binary = installed?;
    println!("  {} Updated {}\n", style::green("✓"), style::dim(&binary.display().to_string()));
    Ok(())
}

/// The download's progress line, redrawn when what it shows changes. Nothing when piped.
#[derive(Default)]
struct Progress {
    shown: Option<String>,
}

impl Progress {
    fn draw(&mut self, done: u64, total: Option<u64>) {
        if !std::io::stdout().is_terminal() {
            return;
        }
        let megabytes = |bytes: u64| format!("{:.1} MB", bytes as f64 / 1_000_000.0);
        let line = match total {
            Some(total) if total > 0 => {
                let filled = ((done as f64 / total as f64 * BAR_WIDTH as f64).round() as usize).min(BAR_WIDTH);
                format!(
                    "{}{}  {}",
                    style::green(&"━".repeat(filled)),
                    style::dim(&"─".repeat(BAR_WIDTH - filled)),
                    style::dim(&format!("{} / {}", megabytes(done), megabytes(total)))
                )
            }
            _ => style::dim(&megabytes(done)),
        };
        if self.shown.as_ref() != Some(&line) {
            print!("\r\x1b[2K  Downloading  {line}");
            let _ = std::io::stdout().flush();
            self.shown = Some(line);
        }
    }

    /// Takes the line away, before anything else is printed.
    fn clear(&self) {
        if self.shown.is_some() {
            print!("\r\x1b[2K");
            let _ = std::io::stdout().flush();
        }
    }
}

/// Starts the binary the interface closed for, after the user installed an update and asked
/// to restart.
pub fn restart_if_asked(binary: Option<PathBuf>) -> Result<()> {
    match binary {
        Some(binary) => restart(&binary),
        None => Ok(()),
    }
}

/// Starts `binary` in place of this process, with the same arguments.
#[cfg(unix)]
fn restart(binary: &Path) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new(binary).args(std::env::args_os().skip(1)).exec();
    Err(err).with_context(|| format!("Couldn't start {}", binary.display()))
}

#[cfg(not(unix))]
fn restart(binary: &Path) -> Result<()> {
    let status = std::process::Command::new(binary).args(std::env::args_os().skip(1)).status()?;
    std::process::exit(status.code().unwrap_or(1))
}
