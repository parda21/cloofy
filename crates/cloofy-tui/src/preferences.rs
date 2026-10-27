//! What people set on the Settings page. Every change goes to `config.toml` right away, so
//! nobody has to edit it by hand, and what's in the file shows up here.

use cloofy_core::config::{Config, Value};
use cloofy_os::settings::Settings;

use crate::{graphics::ImageProtocol, theme::Rgb};

const CROSSFADE_STEPS: [u64; 5] = [0, 3, 6, 9, 12];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preferences {
    /// Seconds consecutive tracks overlap.
    pub crossfade: u64,
    pub covers: bool,
    pub images: ImageProtocol,
    pub accent: AccentChoice,
    pub icons: IconChoice,
    pub power_saving: PowerSaving,
    pub notifications: bool,
    pub update_check: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self::from_config(&Config::default())
    }
}

impl Preferences {
    pub fn from_config(config: &Config) -> Self {
        Self {
            crossfade: config.crossfade.unwrap_or(0).min(12),
            covers: config.covers != Some(false),
            images: config.images.as_deref().and_then(ImageProtocol::parse).unwrap_or_default(),
            accent: AccentChoice::from_config(config.accent.as_deref()),
            icons: match config.icons.as_deref() {
                Some("nerd") => IconChoice::Nerd,
                Some("text") => IconChoice::Text,
                _ => IconChoice::Auto,
            },
            power_saving: match config.power_saving.as_deref() {
                Some("on") => PowerSaving::On,
                Some("off") => PowerSaving::Off,
                _ => PowerSaving::Auto,
            },
            notifications: config.notifications != Some(false),
            update_check: config.update_check != Some(false),
        }
    }

    /// The system's settings with the user's choice for power saving applied.
    pub fn apply(&self, system: Settings) -> Settings {
        let low_power = match self.power_saving {
            PowerSaving::Auto => system.low_power,
            PowerSaving::On => true,
            PowerSaving::Off => false,
        };
        Settings { low_power, ..system }
    }

    /// The setting as shown on the page.
    pub fn value(&self, setting: Setting) -> String {
        let on_off = |on: bool| if on { "On" } else { "Off" }.to_owned();
        match setting {
            Setting::Crossfade if self.crossfade == 0 => "Off".to_owned(),
            Setting::Crossfade => format!("{} s", self.crossfade),
            Setting::Covers => on_off(self.covers),
            Setting::Images => self.images.label().to_owned(),
            Setting::Accent => self.accent.label().to_owned(),
            Setting::Icons => self.icons.label().to_owned(),
            Setting::PowerSaving => self.power_saving.label().to_owned(),
            Setting::Notifications => on_off(self.notifications),
            Setting::UpdateCheck => on_off(self.update_check),
        }
    }

    /// For settings that are switched rather than chosen: whether it's on.
    pub fn switch(&self, setting: Setting) -> Option<bool> {
        match setting {
            Setting::Covers => Some(self.covers),
            Setting::Notifications => Some(self.notifications),
            Setting::UpdateCheck => Some(self.update_check),
            Setting::Crossfade | Setting::Images | Setting::Accent | Setting::Icons | Setting::PowerSaving => None,
        }
    }

    /// Moves the setting to its next or previous choice, wrapping around; switches flip.
    pub fn step(&mut self, setting: Setting, forward: bool) {
        match setting {
            Setting::Crossfade => self.crossfade = cycle(&CROSSFADE_STEPS, &self.crossfade, forward),
            Setting::Covers => self.covers = !self.covers,
            Setting::Images => self.images = cycle(&ImageProtocol::ALL, &self.images, forward),
            Setting::Accent => self.accent = cycle(&AccentChoice::ALL, &self.accent, forward),
            Setting::Icons => self.icons = cycle(&IconChoice::ALL, &self.icons, forward),
            Setting::PowerSaving => self.power_saving = cycle(&PowerSaving::ALL, &self.power_saving, forward),
            Setting::Notifications => self.notifications = !self.notifications,
            Setting::UpdateCheck => self.update_check = !self.update_check,
        }
    }

    /// The setting's key in `config.toml` and its value there; `None` for the default, which
    /// leaves the file without the key.
    pub fn entry(&self, setting: Setting) -> (&'static str, Option<Value>) {
        let off = |on: bool| (!on).then(|| Value::from(false));
        match setting {
            Setting::Crossfade => ("crossfade", (self.crossfade > 0).then(|| Value::from(self.crossfade as i64))),
            Setting::Covers => ("covers", off(self.covers)),
            Setting::Images => (
                "images",
                (self.images != ImageProtocol::Auto).then(|| Value::from(self.images.label().to_lowercase())),
            ),
            Setting::Accent => ("accent", self.accent.config().map(Value::from)),
            Setting::Icons => ("icons", self.icons.config().map(Value::from)),
            Setting::PowerSaving => ("power_saving", self.power_saving.config().map(Value::from)),
            Setting::Notifications => ("notifications", off(self.notifications)),
            Setting::UpdateCheck => ("update_check", off(self.update_check)),
        }
    }
}

/// The item after (or before) `current` in `items`, wrapping around.
fn cycle<T: Clone + PartialEq>(items: &[T], current: &T, forward: bool) -> T {
    let index = items.iter().position(|item| item == current).unwrap_or(0);
    let next = if forward { (index + 1) % items.len() } else { (index + items.len() - 1) % items.len() };
    items[next].clone()
}

/// One line on the Settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Crossfade,
    Covers,
    Images,
    Accent,
    Icons,
    PowerSaving,
    Notifications,
    UpdateCheck,
}

/// The Settings page: section titles with their settings, in order.
pub const SECTIONS: [(&str, &[Setting]); 4] = [
    ("Playback", &[Setting::Crossfade]),
    ("Display", &[Setting::Covers, Setting::Images, Setting::Accent, Setting::Icons]),
    ("Power", &[Setting::PowerSaving]),
    ("Notifications and updates", &[Setting::Notifications, Setting::UpdateCheck]),
];

impl Setting {
    /// Every setting in page order.
    pub fn all() -> impl Iterator<Item = Setting> {
        SECTIONS.iter().flat_map(|(_, settings)| settings.iter().copied())
    }

    pub fn label(self) -> &'static str {
        match self {
            Setting::Crossfade => "Crossfade",
            Setting::Covers => "Cover art",
            Setting::Images => "Cover images",
            Setting::Accent => "Accent color",
            Setting::Icons => "Icons",
            Setting::PowerSaving => "Power saving",
            Setting::Notifications => "Song notifications",
            Setting::UpdateCheck => "Look for updates",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Setting::Crossfade => "Let songs fade into each other.",
            Setting::Covers => "Turn off to save memory and processor time on slower computers.",
            Setting::Images => "How covers reach the terminal. Blocks work in every terminal.",
            Setting::Accent => "Follows the cover of the playing song, or stays one color.",
            Setting::Icons => "Text icons work with every font; Nerd Font icons need one installed.",
            Setting::PowerSaving => "Stops animations and redraws less often. Auto follows the system.",
            Setting::Notifications => "Shows each new song while cloofy is in the background.",
            Setting::UpdateCheck => "Offers new versions of cloofy at start.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccentChoice {
    /// The dominant color of the playing cover.
    Cover,
    Fixed(NamedColor),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedColor {
    Green,
    Violet,
    Pink,
    Blue,
    Orange,
}

impl AccentChoice {
    const ALL: [AccentChoice; 6] = [
        AccentChoice::Cover,
        AccentChoice::Fixed(NamedColor::Green),
        AccentChoice::Fixed(NamedColor::Violet),
        AccentChoice::Fixed(NamedColor::Pink),
        AccentChoice::Fixed(NamedColor::Blue),
        AccentChoice::Fixed(NamedColor::Orange),
    ];

    fn from_config(value: Option<&str>) -> Self {
        Self::ALL.into_iter().find(|choice| choice.config() == value).unwrap_or(AccentChoice::Cover)
    }

    fn config(self) -> Option<&'static str> {
        match self {
            AccentChoice::Cover => None,
            AccentChoice::Fixed(color) => Some(color.name()),
        }
    }

    fn label(self) -> &'static str {
        match self {
            AccentChoice::Cover => "From cover",
            AccentChoice::Fixed(color) => color.label(),
        }
    }

    /// The color to show, if it doesn't depend on the cover.
    pub fn fixed(self) -> Option<Rgb> {
        match self {
            AccentChoice::Cover => None,
            AccentChoice::Fixed(color) => Some(color.rgb()),
        }
    }
}

impl NamedColor {
    fn name(self) -> &'static str {
        match self {
            NamedColor::Green => "green",
            NamedColor::Violet => "violet",
            NamedColor::Pink => "pink",
            NamedColor::Blue => "blue",
            NamedColor::Orange => "orange",
        }
    }

    fn label(self) -> &'static str {
        match self {
            NamedColor::Green => "Green",
            NamedColor::Violet => "Violet",
            NamedColor::Pink => "Pink",
            NamedColor::Blue => "Blue",
            NamedColor::Orange => "Orange",
        }
    }

    /// Bright enough to read on the dark background, like the colors taken from covers.
    fn rgb(self) -> Rgb {
        match self {
            NamedColor::Green => crate::theme::DEFAULT_ACCENT,
            NamedColor::Violet => Rgb(167, 139, 250),
            NamedColor::Pink => Rgb(255, 143, 177),
            NamedColor::Blue => Rgb(96, 165, 250),
            NamedColor::Orange => Rgb(251, 146, 60),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconChoice {
    /// Nerd Font icons in terminals that bundle the font, text icons elsewhere.
    Auto,
    Nerd,
    Text,
}

impl IconChoice {
    const ALL: [IconChoice; 3] = [IconChoice::Auto, IconChoice::Nerd, IconChoice::Text];

    /// The value [`crate::icons::init`] takes, which is also the one in the config file.
    pub fn config(self) -> Option<&'static str> {
        match self {
            IconChoice::Auto => None,
            IconChoice::Nerd => Some("nerd"),
            IconChoice::Text => Some("text"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            IconChoice::Auto => "Auto",
            IconChoice::Nerd => "Nerd Font",
            IconChoice::Text => "Text",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerSaving {
    /// Follows the system: Low Power Mode on macOS, the power-saver profile on Linux.
    Auto,
    On,
    Off,
}

impl PowerSaving {
    const ALL: [PowerSaving; 3] = [PowerSaving::Auto, PowerSaving::On, PowerSaving::Off];

    fn config(self) -> Option<&'static str> {
        match self {
            PowerSaving::Auto => None,
            PowerSaving::On => Some("on"),
            PowerSaving::Off => Some("off"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            PowerSaving::Auto => "Auto",
            PowerSaving::On => "On",
            PowerSaving::Off => "Off",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the file says after storing every setting, read back the way cloofy starts.
    fn round_trip(preferences: &Preferences) -> Preferences {
        let mut document = toml_edit::DocumentMut::new();
        for setting in Setting::all() {
            if let (key, Some(value)) = preferences.entry(setting) {
                document[key] = toml_edit::value(value);
            }
        }
        Preferences::from_config(&toml::from_str(&document.to_string()).unwrap())
    }

    #[test]
    fn defaults_leave_the_file_empty() {
        let preferences = Preferences::default();
        assert!(Setting::all().all(|setting| preferences.entry(setting).1.is_none()));
        assert!(preferences.covers && preferences.notifications && preferences.update_check);
        assert_eq!(preferences.crossfade, 0);
    }

    #[test]
    fn every_choice_survives_a_restart() {
        let mut preferences = Preferences::default();
        for setting in Setting::all() {
            // Two steps from the default reach a non-default value for every kind of setting,
            // except switches, which are back where they started.
            for steps in 1..=2 {
                preferences.step(setting, true);
                assert_eq!(round_trip(&preferences), preferences, "{setting:?} after {steps} steps");
            }
        }
    }

    #[test]
    fn steps_wrap_around_both_ways() {
        let mut preferences = Preferences::default();
        preferences.step(Setting::Crossfade, false);
        assert_eq!(preferences.crossfade, 12);
        preferences.step(Setting::Crossfade, true);
        assert_eq!(preferences.crossfade, 0);
        preferences.step(Setting::Images, false);
        assert_eq!(preferences.images, ImageProtocol::Blocks);
        assert_eq!(
            preferences.entry(Setting::Images).1.and_then(|v| v.as_str().map(str::to_owned)).as_deref(),
            Some("blocks")
        );
        preferences.step(Setting::Accent, false);
        assert_eq!(preferences.accent, AccentChoice::Fixed(NamedColor::Orange));
        assert_eq!(preferences.value(Setting::Accent), "Orange");
    }

    #[test]
    fn power_saving_overrides_the_system() {
        let system = Settings { reduce_motion: true, low_power: true };
        let mut preferences = Preferences::default();
        assert!(preferences.apply(system).low_power);
        preferences.power_saving = PowerSaving::Off;
        assert_eq!(preferences.apply(system), Settings { reduce_motion: true, low_power: false });
        preferences.power_saving = PowerSaving::On;
        assert!(preferences.apply(Settings::default()).low_power);
    }

    #[test]
    fn unknown_values_fall_back_to_the_default() {
        let config: Config = toml::from_str("accent = \"chartreuse\"\npower_saving = \"max\"").unwrap();
        let preferences = Preferences::from_config(&config);
        assert_eq!(preferences.accent, AccentChoice::Cover);
        assert_eq!(preferences.power_saving, PowerSaving::Auto);
    }
}
