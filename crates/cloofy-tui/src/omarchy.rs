//! Omarchy's theme. cloofy takes its colors from the theme Omarchy has set and follows it when
//! it changes, the way Omarchy's own apps do.
//!
//! A theme's colors come from `cloofy.toml` in the theme's directory, which Omarchy renders
//! from the template `cloofy omarchy install` puts in place, and which a theme may also ship.
//! Without one, cloofy renders the same template itself from the theme's `colors.toml`.

use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use ratatui::style::Color;

use crate::theme::{Palette, Rgb};

/// The template that turns an Omarchy theme into cloofy's colors.
pub const TEMPLATE: &str = include_str!("../../../integrations/omarchy/cloofy.toml.tpl");
/// How often to look for a new theme; switching takes Omarchy about as long.
const CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// Omarchy's state, `~/.local/state/omarchy`; `CLOOFY_OMARCHY_STATE` points elsewhere, for
/// trying a theme without switching to it. `None` without Omarchy.
pub fn state_dir() -> Option<PathBuf> {
    let dir = match env::var_os("CLOOFY_OMARCHY_STATE") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env::var_os("HOME")?).join(".local/state/omarchy"),
    };
    dir.join("current/theme").is_dir().then_some(dir)
}

/// Follows the theme Omarchy has set.
pub struct ThemeWatch {
    current: PathBuf,
    checked: Option<Instant>,
    /// What the theme's files looked like when they were last read.
    seen: Option<Stamp>,
}

/// When the files that make up the theme last changed. A switch replaces the theme's
/// directory, then rewrites `theme.name`.
#[derive(Debug, PartialEq)]
struct Stamp([Option<SystemTime>; 3]);

impl ThemeWatch {
    /// `None` without Omarchy.
    pub fn new() -> Option<Self> {
        Some(Self { current: state_dir()?.join("current"), checked: None, seen: None })
    }

    /// The theme's palette the first time, then again whenever the theme changed. Looks at the
    /// files at most every [`CHECK_INTERVAL`], so it may be called on every tick.
    pub fn poll(&mut self) -> Option<Palette> {
        if self.checked.is_some_and(|at| at.elapsed() < CHECK_INTERVAL) {
            return None;
        }
        self.checked = Some(Instant::now());
        let theme = self.current.join("theme");
        let modified = |path: &Path| fs::metadata(path).and_then(|m| m.modified()).ok();
        let stamp = Stamp([
            modified(&self.current.join("theme.name")),
            modified(&theme.join("colors.toml")),
            modified(&theme.join("cloofy.toml")),
        ]);
        if self.seen.as_ref() == Some(&stamp) {
            return None;
        }
        // Read in the middle of a switch, the directory may be gone for a moment; the next
        // look tries again.
        let colors = fs::read_to_string(theme.join("colors.toml")).ok()?;
        let cloofy = fs::read_to_string(theme.join("cloofy.toml")).ok();
        self.seen = Some(stamp);
        Some(palette(cloofy.as_deref(), &colors))
    }
}

/// The palette for a theme: what its `cloofy.toml` says, and for whatever that leaves out, the
/// template rendered from its `colors.toml`.
pub fn palette(cloofy: Option<&str>, colors: &str) -> Palette {
    let derived = read_palette(&render(TEMPLATE, &variables(colors)), Palette::DEFAULT);
    match cloofy {
        Some(cloofy) => read_palette(cloofy, derived),
        None => derived,
    }
}

/// `colors.toml` as the variables templates use, with the fallbacks Omarchy applies for the
/// ones templates here need, so older themes that name only ANSI colors work too.
pub fn variables(colors: &str) -> HashMap<String, String> {
    let mut variables = entries(colors);
    let fallbacks = [
        ("background", ["bg", "color0"]),
        ("foreground", ["fg", "color7"]),
        ("red", ["color1", "bright_red"]),
        ("accent", ["blue", "color4"]),
        ("bright_foreground", ["bright_fg", "color15"]),
    ];
    for (key, others) in fallbacks {
        if !variables.contains_key(key)
            && let Some(value) = others.iter().find_map(|other| variables.get(*other))
        {
            variables.insert(key.to_owned(), value.clone());
        }
    }
    if let Some(foreground) = variables.get("foreground").cloned() {
        variables.entry("bright_foreground".to_owned()).or_insert(foreground);
    }
    variables
}

/// The `key = "value"` lines of a flat TOML file such as `colors.toml`.
fn entries(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let key = key.trim().trim_matches(['"', '\'']);
            let value = value.trim().strip_prefix(['"', '\''])?;
            let value = &value[..value.find(['"', '\''])?];
            let plain = !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            plain.then(|| (key.to_owned(), value.to_owned()))
        })
        .collect()
}

/// Fills in a template the way Omarchy does: `{{ key }}`, `{{ key_strip }}` (without `#`),
/// `{{ key_rgb }}` (`r,g,b`) and `{{ mix a b 30% }}`, which mixes 30 % of `b` into `a`.
/// Anything it can't fill in stays as it was, like in Omarchy.
pub fn render(template: &str, variables: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let Some(end) = rest[start..].find("}}").map(|end| start + end + 2) else { break };
        let token = &rest[start..end];
        out.push_str(&rest[..start]);
        out.push_str(&fill(token, variables).unwrap_or_else(|| token.to_owned()));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn fill(token: &str, variables: &HashMap<String, String>) -> Option<String> {
    let inner = &token[2..token.len() - 2];
    let words: Vec<&str> = inner.split_whitespace().collect();
    if let [function, a, b, amount] = words[..] {
        let mixed = mix(hex(variables.get(a)?)?, hex(variables.get(b)?)?, percent(amount)?);
        return match function {
            "mix" => Some(mixed.hex()),
            "mix_strip" => Some(mixed.hex()[1..].to_owned()),
            "mix_rgb" => Some(mixed.rgb()),
            _ => None,
        };
    }
    // Omarchy replaces these by exact text, with one space each side.
    let name = inner.strip_prefix(' ')?.strip_suffix(' ')?;
    if let Some(value) = variables.get(name) {
        return Some(value.clone());
    }
    if let Some(value) = name.strip_suffix("_strip").and_then(|key| variables.get(key)) {
        return Some(value.strip_prefix('#').unwrap_or(value).to_owned());
    }
    name.strip_suffix("_rgb").and_then(|key| variables.get(key)).and_then(|value| hex(value)).map(|color| color.rgb())
}

/// An amount such as `30%`, `0.3` or `30`, as a fraction.
fn percent(amount: &str) -> Option<f64> {
    let (number, percent) = match amount.strip_suffix('%') {
        Some(number) => (number, true),
        None => (amount, false),
    };
    if !number.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let value: f64 = number.parse().ok()?;
    Some(if percent || value > 1.0 { value / 100.0 } else { value }.clamp(0.0, 1.0))
}

/// Mixes like Omarchy's templates, rounding halves up, so cloofy's own rendering matches theirs
/// to the last digit.
fn mix(a: Rgb, b: Rgb, amount: f64) -> Rgb {
    let channel = |a: u8, b: u8| (f64::from(a) * (1.0 - amount) + f64::from(b) * amount + 0.5) as u8;
    Rgb(channel(a.0, b.0), channel(a.1, b.1), channel(a.2, b.2))
}

fn hex(value: &str) -> Option<Rgb> {
    let digits = value.strip_prefix('#')?;
    if digits.len() != 6 || !digits.is_ascii() {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    Some(Rgb(channel(0)?, channel(2)?, channel(4)?))
}

impl Rgb {
    fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    fn rgb(self) -> String {
        format!("{},{},{}", self.0, self.1, self.2)
    }
}

/// `base` with the colors `text` names in `key = "#rrggbb"` lines.
fn read_palette(text: &str, base: Palette) -> Palette {
    let entries = entries(text);
    let rgb = |key: &str| entries.get(key).and_then(|value| hex(value));
    let color = |key: &str, fallback: Color| rgb(key).map_or(fallback, Color::from);
    Palette {
        bg: color("bg", base.bg),
        surface: color("surface", base.surface),
        player: color("player", base.player),
        raised: color("raised", base.raised),
        text: color("text", base.text),
        muted: color("muted", base.muted),
        faint: color("faint", base.faint),
        danger: color("danger", base.danger),
        unfocused: color("unfocused", base.unfocused),
        disabled: color("disabled", base.disabled),
        placeholder: color("placeholder", base.placeholder),
        thumb: color("thumb", base.thumb),
        lyric_near: color("lyric_near", base.lyric_near),
        lyric_far: color("lyric_far", base.lyric_far),
        highlight: rgb("highlight").unwrap_or(base.highlight),
        accent: rgb("accent").unwrap_or(base.accent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKYO_NIGHT: &str = r##"
mode = "dark"
accent = "#7aa2f7"
background = "#1a1b26"
foreground = "#a9b1d6"
bright_foreground = "#c0caf5"
red = "#f7768e"
"##;

    const CATPPUCCIN_LATTE: &str = r##"
mode = "light"
accent = "#1e66f5"
background = "#eff1f5"
foreground = "#4c4f69"
bright_foreground = "#4c4f69"
red = "#d20f39"
"##;

    #[test]
    fn takes_the_theme_s_colors() {
        let palette = palette(None, TOKYO_NIGHT);
        assert_eq!(palette.bg, Color::Rgb(0x1a, 0x1b, 0x26));
        assert_eq!(palette.text, Color::Rgb(0xa9, 0xb1, 0xd6));
        assert_eq!(palette.accent, Rgb(0x7a, 0xa2, 0xf7));
        assert_eq!(palette.danger, Color::Rgb(0xf7, 0x76, 0x8e));
        assert_eq!(palette.highlight, Rgb(0xc0, 0xca, 0xf5));
        // 7 % of the way from the background to the text.
        assert_eq!(palette.raised, Color::Rgb(36, 38, 50));
        assert!(!palette.is_light());
    }

    #[test]
    fn light_themes_get_light_shades() {
        let palette = palette(None, CATPPUCCIN_LATTE);
        assert!(palette.is_light());
        let (bg, raised, muted) = (Rgb::of(palette.bg), Rgb::of(palette.raised), Rgb::of(palette.muted));
        assert!(raised.0 < bg.0 && muted.0 < raised.0, "shades darken toward the text");
        assert_eq!(palette.highlight, Rgb(0x4c, 0x4f, 0x69), "hovering darkens");
    }

    #[test]
    fn the_theme_s_own_file_wins_and_gaps_are_filled() {
        let cloofy = "bg = \"#000000\"\naccent = \"{{ accent }}\"\n# muted = \"#ffffff\"\n";
        let palette = palette(Some(cloofy), TOKYO_NIGHT);
        assert_eq!(palette.bg, Color::Rgb(0, 0, 0));
        assert_eq!(palette.accent, Rgb(0x7a, 0xa2, 0xf7), "left unrendered, so derived");
        assert_eq!(palette.muted, super::palette(None, TOKYO_NIGHT).muted);
    }

    #[test]
    fn older_themes_with_only_ansi_colors_work() {
        let palette = palette(None, "color0 = \"#101010\"\ncolor7 = \"#e0e0e0\"\ncolor4 = \"#3366ff\"\n");
        assert_eq!(palette.bg, Color::Rgb(0x10, 0x10, 0x10));
        assert_eq!(palette.text, Color::Rgb(0xe0, 0xe0, 0xe0));
        assert_eq!(palette.accent, Rgb(0x33, 0x66, 0xff));
        assert_eq!(palette.highlight, Rgb(0xe0, 0xe0, 0xe0));
        assert_eq!(palette.danger, Palette::DEFAULT.danger, "no red, so cloofy's own");
    }

    #[test]
    fn renders_templates_like_omarchy() {
        let variables = variables(TOKYO_NIGHT);
        let rendered = render(
            "a = {{ accent }} {{ accent_strip }} {{ accent_rgb }}\nb = {{ mix background foreground 50% }} {{mix_strip red red 0.3}}\nc = {{ nope }} {{accent}} {{ mix background nope 3% }}",
            &variables,
        );
        assert_eq!(
            rendered,
            "a = #7aa2f7 7aa2f7 122,162,247\nb = #62667e f7768e\nc = {{ nope }} {{accent}} {{ mix background nope 3% }}"
        );
    }

    #[test]
    fn the_template_fills_every_color() {
        for colors in [TOKYO_NIGHT, CATPPUCCIN_LATTE] {
            let rendered = render(TEMPLATE, &variables(colors));
            assert!(!rendered.contains("{{"), "{rendered}");
            let keys = entries(&rendered);
            assert_eq!(keys.len(), 16);
            assert!(keys.values().all(|value| hex(value).is_some()), "{keys:?}");
        }
    }

    #[test]
    fn follows_theme_switches() {
        let state = env::temp_dir().join(format!("cloofy-omarchy-{}", std::process::id()));
        let theme = state.join("current/theme");
        fs::create_dir_all(&theme).unwrap();
        fs::write(theme.join("colors.toml"), TOKYO_NIGHT).unwrap();
        let mut watch = ThemeWatch { current: state.join("current"), checked: None, seen: None };
        assert_eq!(watch.poll().map(|p| p.bg), Some(Color::Rgb(0x1a, 0x1b, 0x26)));
        watch.checked = None;
        assert_eq!(watch.poll(), None, "nothing changed");

        // A switch replaces the directory and then names the theme.
        fs::remove_dir_all(&theme).unwrap();
        fs::create_dir_all(&theme).unwrap();
        fs::write(theme.join("colors.toml"), CATPPUCCIN_LATTE).unwrap();
        fs::write(state.join("current/theme.name"), "catppuccin-latte\n").unwrap();
        watch.checked = None;
        assert_eq!(watch.poll().map(|p| p.bg), Some(Color::Rgb(0xef, 0xf1, 0xf5)));
        assert_eq!(watch.poll(), None, "not looked at again right away");
        fs::remove_dir_all(state).unwrap();
    }
}
