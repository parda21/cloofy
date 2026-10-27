use std::time::{Duration, Instant};

use image::DynamicImage;
use ratatui::style::Color;

pub const BG: Color = Color::Rgb(13, 13, 16);
pub const SURFACE: Color = Color::Rgb(19, 19, 23);
pub const PLAYER: Color = Color::Rgb(17, 17, 21);
pub const RAISED: Color = Color::Rgb(30, 30, 37);
pub const TEXT: Color = Color::Rgb(236, 236, 241);
pub const MUTED: Color = Color::Rgb(140, 140, 152);
pub const FAINT: Color = Color::Rgb(78, 78, 88);
pub const DANGER: Color = Color::Rgb(255, 99, 99);

pub(crate) const DEFAULT_ACCENT: Rgb = Rgb(30, 215, 96);
const ACCENT_FADE: Duration = Duration::from_millis(600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(lerp(self.0, other.0), lerp(self.1, other.1), lerp(self.2, other.2))
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
        Self { from: DEFAULT_ACCENT, to: DEFAULT_ACCENT, since: Instant::now() - ACCENT_FADE }
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
        Rgb(30, 30, 37).mix(self.rgb(), 0.14).into()
    }

    pub fn is_fading(&self) -> bool {
        self.since.elapsed() < ACCENT_FADE
    }
}

/// Finds the dominant vivid hue of a cover and tunes it to read well on the dark background.
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
        return DEFAULT_ACCENT;
    }
    let (h, s, l) = rgb_to_hsl(r / weight, g / weight, b / weight);
    let (r, g, b) = hsl_to_rgb(h, s.clamp(0.55, 0.85), l.clamp(0.58, 0.70));
    Rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
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
}
