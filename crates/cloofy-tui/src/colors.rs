//! Where the interface's colors come from: cloofy's own, Omarchy's theme or the terminal's, as
//! the Colors setting says, with `theme.toml` from the config folder over them. Follows
//! Omarchy's theme and the file as they change, without a restart.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use ratatui_image::picker::Capability;

use crate::{
    omarchy,
    preferences::ColorChoice,
    theme::{Palette, Rgb},
};

/// How often to look at `theme.toml` again.
const CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// What the terminal answered at startup when asked for its colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalColors {
    pub background: Rgb,
    /// Some terminals tell only their background.
    pub foreground: Option<Rgb>,
}

impl TerminalColors {
    /// The colors among what the terminal answered; `None` without its background.
    pub fn from_capabilities(capabilities: &[Capability]) -> Option<Self> {
        let background = capabilities.iter().find_map(|capability| match capability {
            Capability::Background(r, g, b) => Some(Rgb(*r, *g, *b)),
            _ => None,
        })?;
        let foreground = capabilities.iter().find_map(|capability| match capability {
            Capability::Foreground(r, g, b) => Some(Rgb(*r, *g, *b)),
            _ => None,
        });
        Some(Self { background, foreground })
    }

    /// The palette for these colors: shades mixed like an Omarchy theme's. Without the text
    /// color, cloofy's own text on a dark background and its background color on a light one.
    fn palette(self) -> Palette {
        let foreground = self.foreground.unwrap_or(if self.background.is_light() {
            Rgb::of(Palette::DEFAULT.bg)
        } else {
            Rgb::of(Palette::DEFAULT.text)
        });
        omarchy::terminal_palette(self.background, foreground)
    }
}

/// The palette to draw with, and when it changes.
pub struct Colors {
    choice: ColorChoice,
    /// The terminal's palette, when it told its colors.
    terminal: Option<Palette>,
    /// Omarchy's theme and its palette as last read; `None` without Omarchy.
    omarchy: Option<(omarchy::ThemeWatch, Option<Palette>)>,
    custom: CustomFile,
    /// Whether the palette may differ from the one [`Colors::poll`] returned last.
    stale: bool,
    shown: Option<Palette>,
}

impl Colors {
    /// `config` is the folder `theme.toml` is looked for in.
    pub fn new(choice: ColorChoice, terminal: Option<TerminalColors>, config: &Path) -> Self {
        let omarchy = if cfg!(target_os = "linux") { omarchy::ThemeWatch::new() } else { None };
        Self {
            choice,
            terminal: terminal.map(TerminalColors::palette),
            omarchy: omarchy.map(|watch| (watch, None)),
            custom: CustomFile::new(config.join("theme.toml")),
            stale: true,
            shown: None,
        }
    }

    pub fn set_choice(&mut self, choice: ColorChoice) {
        self.stale |= choice != self.choice;
        self.choice = choice;
    }

    /// Whether the terminal told its colors at startup.
    pub fn knows_terminal(&self) -> bool {
        self.terminal.is_some()
    }

    /// The palette the first time, then again whenever it changed: by the setting, Omarchy's
    /// theme or `theme.toml`. Looks at the files at most every second, so it may be called on
    /// every tick.
    pub fn poll(&mut self) -> Option<Palette> {
        if let Some((watch, palette)) = &mut self.omarchy
            && let Some(theme) = watch.poll()
        {
            *palette = Some(theme);
            self.stale = true;
        }
        self.stale |= self.custom.poll();
        if !std::mem::take(&mut self.stale) {
            return None;
        }
        let palette = self.palette();
        (self.shown != Some(palette)).then(|| {
            self.shown = Some(palette);
            palette
        })
    }

    fn palette(&self) -> Palette {
        let omarchy = self.omarchy.as_ref().and_then(|(_, palette)| *palette);
        let base = match self.choice {
            ColorChoice::Auto => omarchy,
            ColorChoice::Cloofy => None,
            ColorChoice::Terminal => self.terminal,
        };
        let base = base.unwrap_or(Palette::DEFAULT);
        match &self.custom.text {
            Some(text) => omarchy::apply(text, base),
            None => base,
        }
    }
}

/// `theme.toml`, read again whenever it changes.
struct CustomFile {
    path: PathBuf,
    checked: Option<Instant>,
    /// When the file last changed, as of the last look; `None` before the first look.
    seen: Option<Option<SystemTime>>,
    text: Option<String>,
}

impl CustomFile {
    fn new(path: PathBuf) -> Self {
        Self { path, checked: None, seen: None, text: None }
    }

    /// Whether the file changed since the last look, which is at most every [`CHECK_INTERVAL`].
    fn poll(&mut self) -> bool {
        if self.checked.is_some_and(|at| at.elapsed() < CHECK_INTERVAL) {
            return false;
        }
        self.checked = Some(Instant::now());
        let modified = fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if self.seen == Some(modified) {
            return false;
        }
        self.seen = Some(modified);
        let text = fs::read_to_string(&self.path).ok();
        let changed = text != self.text;
        self.text = text;
        changed
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;

    /// Ubuntu's terminal: aubergine with white text.
    const UBUNTU: TerminalColors =
        TerminalColors { background: Rgb(0x30, 0x0a, 0x24), foreground: Some(Rgb(255, 255, 255)) };

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cloofy-colors-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn colors(choice: ColorChoice, terminal: Option<TerminalColors>, config: &Path) -> Colors {
        let mut colors = Colors::new(choice, terminal, config);
        // Whatever theme the machine running the tests has.
        colors.omarchy = None;
        colors
    }

    #[test]
    fn reads_the_terminal_s_answer() {
        let answer = [Capability::Sixel, Capability::Foreground(1, 2, 3), Capability::Background(4, 5, 6)];
        assert_eq!(
            TerminalColors::from_capabilities(&answer),
            Some(TerminalColors { background: Rgb(4, 5, 6), foreground: Some(Rgb(1, 2, 3)) })
        );
        assert_eq!(TerminalColors::from_capabilities(&[Capability::Foreground(1, 2, 3)]), None);
    }

    #[test]
    fn the_terminal_s_colors_only_when_chosen() {
        let dir = folder("choice");
        let mut colors = colors(ColorChoice::Auto, Some(UBUNTU), &dir);
        assert_eq!(colors.poll(), Some(Palette::DEFAULT));
        assert_eq!(colors.poll(), None, "nothing changed");

        colors.set_choice(ColorChoice::Terminal);
        let palette = colors.poll().unwrap();
        assert_eq!(palette.bg, Color::Rgb(0x30, 0x0a, 0x24));
        assert_eq!(palette.text, Color::Rgb(255, 255, 255));
        assert_eq!(palette.accent, Palette::DEFAULT.accent, "cloofy's green");
        assert!(!palette.is_light());

        colors.set_choice(ColorChoice::Cloofy);
        assert_eq!(colors.poll(), Some(Palette::DEFAULT));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_terminal_that_tells_only_its_background() {
        let light = TerminalColors { background: Rgb(250, 250, 250), foreground: None };
        let palette = light.palette();
        assert!(palette.is_light());
        assert_eq!(palette.text, Palette::DEFAULT.bg, "dark text on a light background");
        let dark = TerminalColors { background: Rgb(0, 0, 0), foreground: None };
        assert_eq!(dark.palette().text, Palette::DEFAULT.text);
    }

    #[test]
    fn a_terminal_that_tells_nothing_keeps_cloofy_s_colors() {
        let dir = folder("silent");
        let mut colors = colors(ColorChoice::Terminal, None, &dir);
        assert!(!colors.knows_terminal());
        assert_eq!(colors.poll(), Some(Palette::DEFAULT));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn theme_toml_goes_over_any_of_them_and_is_followed() {
        let dir = folder("custom");
        let file = dir.join("theme.toml");
        fs::write(&file, "bg = \"#2b0a3d\"\n# text = \"#000000\"\n").unwrap();
        let mut colors = colors(ColorChoice::Terminal, Some(UBUNTU), &dir);
        let palette = colors.poll().unwrap();
        assert_eq!(palette.bg, Color::Rgb(0x2b, 0x0a, 0x3d));
        assert_eq!(palette.text, Color::Rgb(255, 255, 255), "the rest from the terminal");

        fs::write(&file, "bg = \"#101010\"\naccent = \"#ff8800\"\n").unwrap();
        // The modification time may be the same within the second; a new look reads it.
        colors.custom.checked = None;
        colors.custom.seen = None;
        let palette = colors.poll().unwrap();
        assert_eq!(palette.bg, Color::Rgb(0x10, 0x10, 0x10));
        assert_eq!(palette.accent, Rgb(0xff, 0x88, 0x00));

        fs::remove_file(&file).unwrap();
        colors.custom.checked = None;
        assert_eq!(colors.poll().map(|p| p.bg), Some(Color::Rgb(0x30, 0x0a, 0x24)), "removed: the terminal's again");
        fs::remove_dir_all(dir).unwrap();
    }
}
