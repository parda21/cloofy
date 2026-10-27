use std::{
    sync::{PoisonError, RwLock},
    time::{Duration, Instant},
};

use image::DynamicImage;
use ratatui::style::Color;

const ACCENT_FADE: Duration = Duration::from_millis(600);

/// The colors the interface is drawn with. They can change while cloofy runs, so drawing code
/// reads them with [`palette`] as it draws rather than keeping them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Behind everything.
    pub bg: Color,
    /// The sidebar.
    pub surface: Color,
    /// The player bar.
    pub player: Color,
    /// Cards, menus and anything else that sits on top.
    pub raised: Color,
    pub text: Color,
    /// Secondary text, such as artists and counts.
    pub muted: Color,
    /// Hints and separators.
    pub faint: Color,
    /// Errors.
    pub danger: Color,
    /// The selected row of a list that doesn't have focus; with focus it takes the accent.
    pub unfocused: Color,
    /// Buttons that do nothing right now.
    pub disabled: Color,
    /// Where a cover is still loading, on something raised.
    pub placeholder: Color,
    /// The part of a scrollbar that shows where the view is.
    pub thumb: Color,
    /// Lyrics next to the line being sung, which fade toward `lyric_far` with distance.
    pub lyric_near: Color,
    pub lyric_far: Color,
    /// What hovered and highlighted things move toward: white on a dark background.
    pub highlight: Rgb,
    /// The accent while no cover or setting picks another.
    pub accent: Rgb,
}

impl Palette {
    /// cloofy's own dark colors, with Spotify's green.
    pub const DEFAULT: Palette = Palette {
        bg: Color::Rgb(13, 13, 16),
        surface: Color::Rgb(19, 19, 23),
        player: Color::Rgb(17, 17, 21),
        raised: Color::Rgb(30, 30, 37),
        text: Color::Rgb(236, 236, 241),
        muted: Color::Rgb(140, 140, 152),
        faint: Color::Rgb(78, 78, 88),
        danger: Color::Rgb(255, 99, 99),
        unfocused: Color::Rgb(25, 25, 31),
        disabled: Color::Rgb(58, 58, 68),
        placeholder: Color::Rgb(40, 40, 48),
        thumb: Color::Rgb(96, 96, 108),
        lyric_near: Color::Rgb(200, 200, 210),
        lyric_far: Color::Rgb(70, 70, 80),
        highlight: Rgb(255, 255, 255),
        accent: Rgb(30, 215, 96),
    };

    /// Whether this is a light theme, with dark text on a light background.
    pub fn is_light(&self) -> bool {
        let Rgb(r, g, b) = Rgb::of(self.bg);
        // The way Omarchy tells, by the sum of the channels.
        u16::from(r) + u16::from(g) + u16::from(b) > 382
    }
}

/// A lock rather than constants, so the colors can follow the system's theme. Drawing only
/// reads it, which costs next to nothing while nobody writes.
static PALETTE: RwLock<Palette> = RwLock::new(Palette::DEFAULT);

/// The colors to draw with now.
pub fn palette() -> Palette {
    // A palette is plain data, so one left by a panicking writer is still whole.
    *PALETTE.read().unwrap_or_else(PoisonError::into_inner)
}

/// Draws with `palette` from the next frame on.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Only Omarchy's theme feeds it so far.
pub fn set_palette(palette: Palette) {
    *PALETTE.write().unwrap_or_else(PoisonError::into_inner) = palette;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(lerp(self.0, other.0), lerp(self.1, other.1), lerp(self.2, other.2))
    }

    /// The parts of a palette color, which is always RGB; anything else counts as black.
    pub fn of(color: Color) -> Rgb {
        match color {
            Color::Rgb(r, g, b) => Rgb(r, g, b),
            _ => Rgb(0, 0, 0),
        }
    }
}

impl From<Rgb> for Color {
    fn from(c: Rgb) -> Self {
        Color::Rgb(c.0, c.1, c.2)
    }
}

/// The app's accent color. It follows the cover art and fades between covers.
pub struct Accent {
    from: Rgb,
    to: Rgb,
    since: Instant,
}

impl Accent {
    pub fn new() -> Self {
        let accent = palette().accent;
        Self { from: accent, to: accent, since: Instant::now() - ACCENT_FADE }
    }

    pub fn set(&mut self, target: Rgb) {
        self.from = self.rgb();
        self.to = target;
        self.since = Instant::now();
    }

    /// Switches without fading.
    pub fn jump(&mut self, target: Rgb) {
        self.from = target;
        self.to = target;
        self.since = Instant::now() - ACCENT_FADE;
    }

    pub fn rgb(&self) -> Rgb {
        let t = (self.since.elapsed().as_secs_f32() / ACCENT_FADE.as_secs_f32()).min(1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        self.from.mix(self.to, eased)
    }

    pub fn color(&self) -> Color {
        self.rgb().into()
    }

    /// The selection background: a raised surface tinted with the accent.
    pub fn selection(&self) -> Color {
        Rgb::of(palette().raised).mix(self.rgb(), 0.14).into()
    }

    pub fn is_fading(&self) -> bool {
        self.since.elapsed() < ACCENT_FADE
    }
}

/// Finds the dominant vivid hue of a cover and tunes it to read well on the background.
pub fn accent_from_image(image: &DynamicImage) -> Rgb {
    const BUCKETS: usize = 24;
    let mut buckets = [[0f32; 4]; BUCKETS];
    for pixel in image.thumbnail(40, 40).to_rgb8().pixels() {
        let [r, g, b] = pixel.0.map(|c| c as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        if max < 0.15 {
            continue;
        }
        let saturation = (max - min) / max;
        let weight = saturation * saturation * max;
        let (hue, _, _) = rgb_to_hsl(r, g, b);
        let bucket = &mut buckets[((hue / 360.0) * BUCKETS as f32) as usize % BUCKETS];
        bucket[0] += r * weight;
        bucket[1] += g * weight;
        bucket[2] += b * weight;
        bucket[3] += weight;
    }
    let [r, g, b, weight] = buckets.into_iter().max_by(|a, b| a[3].total_cmp(&b[3])).unwrap_or_default();
    if weight < 1.0 {
        return palette().accent;
    }
    let (h, s, l) = rgb_to_hsl(r / weight, g / weight, b / weight);
    let lightness = if palette().is_light() { LIGHT_LIGHTNESS } else { DARK_LIGHTNESS };
    let (r, g, b) = hsl_to_rgb(h, s.clamp(0.55, 0.85), l.clamp(lightness.0, lightness.1));
    Rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// How light an accent may be to read well on a dark background, and on a light one.
const DARK_LIGHTNESS: (f32, f32) = (0.58, 0.70);
const LIGHT_LIGHTNESS: (f32, f32) = (0.30, 0.42);

/// `color` darkened enough to read on a light background, and as it is on a dark one, which
/// cloofy's own colors are made for.
pub fn readable(color: Rgb) -> Rgb {
    if !palette().is_light() {
        return color;
    }
    let Rgb(r, g, b) = color;
    let (h, s, l) = rgb_to_hsl(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0);
    let (r, g, b) = hsl_to_rgb(h, s, l.min(LIGHT_LIGHTNESS.1));
    Rgb((r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8)
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 {
        0..60 => (c, x, 0.0),
        60..120 => (x, c, 0.0),
        120..180 => (0.0, c, x),
        180..240 => (0.0, x, c),
        240..300 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (r + m, g + m, b + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jumping_skips_the_fade() {
        let mut accent = Accent::new();
        let target = Rgb(200, 40, 90);
        accent.set(target);
        assert!(accent.is_fading());
        accent.jump(target);
        assert!(!accent.is_fading());
        assert_eq!(accent.rgb(), target);
    }

    #[test]
    fn tells_light_themes_from_dark_ones() {
        assert!(!Palette::DEFAULT.is_light());
        assert!(Palette { bg: Color::Rgb(239, 241, 245), ..Palette::DEFAULT }.is_light());
    }
}
