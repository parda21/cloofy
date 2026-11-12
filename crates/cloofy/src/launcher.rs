//! cloofy in the desktop's app menu on Linux: a desktop entry and an icon in the user's own
//! folders, where GNOME, KDE Plasma, rofi, Walker and the like find them. The packages put the
//! same two files in `/usr/share`; on Omarchy, `cloofy omarchy install` writes an entry of its
//! own, which opens cloofy the way Omarchy opens its terminal apps.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::Result;
use clap::Subcommand;

use crate::{
    omarchy::{self, remove_file, write_if_changed},
    style,
};

/// The entry, which opens cloofy in the desktop's own terminal.
const ENTRY: &str = include_str!("../../../integrations/linux/cloofy.desktop");
pub const ICON: &str = include_str!("../../../integrations/omarchy/cloofy.svg");
/// Marks the desktop entry as the one `install` wrote.
const MARK: &str = "X-cloofy-launcher=true";

#[derive(Subcommand)]
pub enum Action {
    /// Add cloofy and its icon to the app menu
    Install,
    /// Take them out again
    Remove,
}

pub fn run(action: Action) -> Result<()> {
    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        anyhow::bail!("HOME is not set, so there is no app menu to add cloofy to");
    };
    let launcher = Launcher { home };
    let done = match action {
        Action::Install => launcher.install()?,
        Action::Remove => launcher.remove()?,
    };
    println!("  {}  {}", style::green("✓"), style::dim(done));
    Ok(())
}

/// Whether the desktop entry at `path` is one cloofy wrote here.
pub fn is_ours(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|entry| entry.lines().any(|line| line.trim() == MARK))
}

struct Launcher {
    home: PathBuf,
}

impl Launcher {
    fn entry(&self) -> PathBuf {
        self.home.join(".local/share/applications/cloofy.desktop")
    }

    fn icon(&self) -> PathBuf {
        self.home.join(".local/share/icons/hicolor/scalable/apps/cloofy.svg")
    }

    fn install(&self) -> Result<&'static str> {
        let entry = self.entry();
        if omarchy::is_ours(&entry) {
            return Ok("Omarchy's launcher for cloofy is there, so it stays");
        }
        if entry.exists() && !is_ours(&entry) {
            return Ok("cloofy.desktop is your own, so it stays");
        }
        // An icon of the user's own stays, and the entry shows that one.
        let icon = self.icon();
        if fs::symlink_metadata(&icon).is_err() {
            write_if_changed(&icon, ICON)?;
        }
        write_if_changed(&entry, ENTRY)?;
        self.refresh();
        Ok("cloofy is in the app menu")
    }

    fn remove(&self) -> Result<&'static str> {
        if !is_ours(&self.entry()) {
            return Ok("cloofy's entry in the app menu wasn't there");
        }
        remove_file(&self.entry())?;
        if fs::read_to_string(self.icon()).is_ok_and(|icon| icon == ICON) {
            remove_file(&self.icon())?;
            // The folders `install` may have made for it, where nothing else is in them.
            for dir in self.icon().ancestors().skip(1).take(2) {
                let _ = fs::remove_dir(dir);
            }
        }
        self.refresh();
        Ok("cloofy is out of the app menu")
    }

    /// Tells the desktop about the change, where it has the tools for that; it finds out by
    /// itself otherwise, at the latest when the user signs in again.
    fn refresh(&self) {
        let share = self.home.join(".local/share");
        let _ = Command::new("update-desktop-database").arg(share.join("applications")).output();
        let _ = Command::new("gtk-update-icon-cache").arg(share.join("icons/hicolor")).output();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> Launcher {
        let home = env::temp_dir().join(format!("cloofy-launcher-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        Launcher { home }
    }

    /// What desktops read from an entry: the keys of its `[Desktop Entry]` group.
    fn key<'a>(entry: &'a str, key: &str) -> Option<&'a str> {
        entry.lines().find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
    }

    #[test]
    fn the_entry_opens_cloofy_in_a_terminal() {
        assert_eq!(ENTRY.lines().next(), Some("[Desktop Entry]"));
        assert_eq!(key(ENTRY, "Exec"), Some("cloofy"));
        assert_eq!(key(ENTRY, "Terminal"), Some("true"));
        assert_eq!(key(ENTRY, "Icon"), Some("cloofy"));
        assert_eq!(key(ENTRY, "Categories"), Some("AudioVideo;Audio;Player;"));
        assert!(ENTRY.lines().any(|line| line == MARK));
    }

    #[test]
    fn installs_and_removes_the_entry_and_the_icon() {
        let launcher = scratch("both");
        launcher.install().unwrap();
        assert!(is_ours(&launcher.entry()));
        assert_eq!(fs::read_to_string(launcher.icon()).unwrap(), ICON);
        launcher.remove().unwrap();
        assert!(!launcher.entry().exists() && !launcher.icon().exists());
        assert!(!launcher.home.join(".local/share/icons/hicolor/scalable").exists(), "the folders it made go too");
        fs::remove_dir_all(&launcher.home).unwrap();
    }

    #[test]
    fn leaves_an_entry_and_an_icon_of_the_user_s_own() {
        let launcher = scratch("theirs");
        let theirs = "[Desktop Entry]\nName=cloofy\nExec=kitty -e cloofy\n";
        write_if_changed(&launcher.entry(), theirs).unwrap();
        launcher.install().unwrap();
        assert_eq!(fs::read_to_string(launcher.entry()).unwrap(), theirs);
        assert!(!launcher.icon().exists(), "nothing to show an icon for");
        launcher.remove().unwrap();
        assert_eq!(fs::read_to_string(launcher.entry()).unwrap(), theirs);

        fs::remove_file(launcher.entry()).unwrap();
        let icon = "<svg><!-- mine --></svg>\n";
        write_if_changed(&launcher.icon(), icon).unwrap();
        launcher.install().unwrap();
        launcher.remove().unwrap();
        assert_eq!(fs::read_to_string(launcher.icon()).unwrap(), icon);
        fs::remove_dir_all(&launcher.home).unwrap();
    }

    #[test]
    fn leaves_omarchy_s_launcher() {
        let launcher = scratch("omarchy");
        let omarchy = "[Desktop Entry]\nName=cloofy\nExec=omarchy-launch-or-focus-tui cloofy\nX-cloofy-omarchy=true\n";
        write_if_changed(&launcher.entry(), omarchy).unwrap();
        launcher.install().unwrap();
        launcher.remove().unwrap();
        assert_eq!(fs::read_to_string(launcher.entry()).unwrap(), omarchy);
        fs::remove_dir_all(&launcher.home).unwrap();
    }
}
