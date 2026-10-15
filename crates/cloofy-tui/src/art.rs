//! Artwork cloofy draws itself.

use image::{DynamicImage, Rgba, RgbaImage};

/// Pseudo-URL for the Liked Songs cover; the loader generates it instead of downloading.
pub const LIKED_COVER: &str = "cloofy:liked";
/// Pseudo-URL for items without a cover, such as new playlists.
pub const PLACEHOLDER_COVER: &str = "cloofy:placeholder";

const SIZE: u32 = 256;
/// Supersampling per axis, for smooth heart edges.
const SAMPLES: u32 = 3;

/// Spotify's Liked Songs look: a violet-to-mint gradient with a white heart.
pub fn liked_cover() -> DynamicImage {
    let from = [69.0, 10.0, 245.0];
    let to = [196.0, 239.0, 217.0];
    let image = RgbaImage::from_fn(SIZE, SIZE, |x, y| {
        let t = (x + y) as f32 / (2 * SIZE) as f32;
        let gradient = [0, 1, 2].map(|i| from[i] + (to[i] - from[i]) * t);
        let coverage = heart_coverage(x, y);
        let [r, g, b] = gradient.map(|c| (c + (255.0 - c) * coverage) as u8);
        Rgba([r, g, b, 255])
    });
    image.into()
}

/// Fraction of the pixel inside the heart curve (x² + y² − 1)³ − x²y³ ≤ 0.
fn heart_coverage(x: u32, y: u32) -> f32 {
    let mut inside = 0;
    for sx in 0..SAMPLES {
        for sy in 0..SAMPLES {
            let px = (x as f32 + (sx as f32 + 0.5) / SAMPLES as f32) / SIZE as f32;
            let py = (y as f32 + (sy as f32 + 0.5) / SAMPLES as f32) / SIZE as f32;
            // About a third of the cover, slightly above the center.
            let hx = (px - 0.5) * 6.0;
            let hy = (0.47 - py) * 6.0;
            if (hx * hx + hy * hy - 1.0).powi(3) - hx * hx * hy.powi(3) <= 0.0 {
                inside += 1;
            }
        }
    }
    inside as f32 / (SAMPLES * SAMPLES) as f32
}

/// Spotify's look for playlists without a cover: a dark square with a muted music note.
pub fn placeholder_cover() -> DynamicImage {
    let image = RgbaImage::from_fn(SIZE, SIZE, |x, y| {
        let t = (x + y) as f32 / (2 * SIZE) as f32;
        let background = 40.0 - 12.0 * t;
        let coverage = supersample(x, y, note);
        let c = (background + (130.0 - background) * coverage) as u8;
        Rgba([c, c, c.saturating_add(6), 255])
    });
    image.into()
}

/// An eighth note: an oval head, a stem and a short flag, in unit coordinates.
fn note(px: f32, py: f32) -> bool {
    let (hx, hy) = (px - 0.44, py - 0.64);
    let (rx, ry) = (hx * 0.94 + hy * 0.34, -hx * 0.34 + hy * 0.94);
    let head = (rx / 0.105).powi(2) + (ry / 0.075).powi(2) <= 1.0;
    let stem = (0.515..=0.545).contains(&px) && (0.30..=0.64).contains(&py);
    let flag = (0.53..=0.66).contains(&px) && {
        let curve = 0.30 + (px - 0.53) * 1.1;
        py >= curve && py <= curve + 0.05
    };
    head || stem || flag
}

fn supersample(x: u32, y: u32, shape: fn(f32, f32) -> bool) -> f32 {
    let mut inside = 0;
    for sx in 0..SAMPLES {
        for sy in 0..SAMPLES {
            let px = (x as f32 + (sx as f32 + 0.5) / SAMPLES as f32) / SIZE as f32;
            let py = (y as f32 + (sy as f32 + 0.5) / SAMPLES as f32) / SIZE as f32;
            if shape(px, py) {
                inside += 1;
            }
        }
    }
    inside as f32 / (SAMPLES * SAMPLES) as f32
}
