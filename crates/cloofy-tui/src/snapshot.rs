//! Dev tool: renders the current frame to a PNG, so the UI can be reviewed without a screen.
//!
//! Start cloofy with `CLOOFY_SNAPSHOT_DIR=<dir>`; F12 then writes `<dir>/latest.png`. Fonts come
//! from `CLOOFY_SNAPSHOT_FONTS` (default `~/.cache/cloofy/dev-fonts`): JetBrains Mono and the
//! Nerd Font symbols, which is what Ghostty uses out of the box.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use fontdue::{Font, FontSettings};
use image::{DynamicImage, Rgba, RgbaImage, imageops::FilterType};
use ratatui::{
    buffer::{Buffer, Cell},
    layout::{Rect, Size},
    style::{Color, Modifier},
};

/// Font size in pixels; cells come out about 12×26, close to a typical terminal.
const PX: f32 = 20.0;
const DEFAULT_BG: [u8; 3] = [13, 13, 16];
const DEFAULT_FG: [u8; 3] = [236, 236, 241];
const KITTY_PLACEHOLDER: char = '\u{10EEEE}';

/// A rasterized glyph, or `None` if no font has it.
type Glyph = Option<(fontdue::Metrics, Vec<u8>)>;

/// An image the app drew, to paste over its cells in full resolution.
#[derive(Clone)]
pub struct Placement {
    /// The visible cells.
    pub area: Rect,
    /// The whole image in cells, of which the top `skip` rows are scrolled out of view.
    pub size: Size,
    pub skip: u16,
    /// Kept only while snapshots are on.
    pub source: Option<Arc<DynamicImage>>,
    /// The image's cells as drawn, row by row; cells that differ at the end of the frame were
    /// drawn over. Empty while snapshots are off.
    pub cells: Vec<Cell>,
    /// The first time a Kitty image is drawn: the upload of its pixels, written after the frame.
    pub upload: Option<String>,
}

pub struct Snapshotter {
    dir: PathBuf,
    regular: Vec<Font>,
    bold: Vec<Font>,
    cell_w: u32,
    cell_h: u32,
    ascent: f32,
}

impl Snapshotter {
    pub fn from_env() -> Option<Self> {
        let dir = PathBuf::from(std::env::var_os("CLOOFY_SNAPSHOT_DIR")?);
        match Self::new(dir) {
            Ok(snapshotter) => Some(snapshotter),
            Err(err) => {
                tracing::error!("snapshots disabled: {err:#}");
                None
            }
        }
    }

    fn new(dir: PathBuf) -> Result<Self> {
        let fonts = std::env::var_os("CLOOFY_SNAPSHOT_FONTS")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache/cloofy/dev-fonts")))
            .context("no font directory")?;
        let load = |name: &str| -> Option<Font> {
            let bytes = std::fs::read(fonts.join(name)).ok()?;
            Font::from_bytes(bytes, FontSettings::default()).ok()
        };
        let symbols = load("SymbolsNerdFontMono-Regular.ttf");
        let regular: Vec<Font> = [load("JetBrainsMono-Regular.ttf"), symbols.clone()].into_iter().flatten().collect();
        let mut bold: Vec<Font> = [load("JetBrainsMono-Bold.ttf")].into_iter().flatten().collect();
        bold.extend(regular.iter().cloned());
        let primary = regular.first().context("JetBrainsMono-Regular.ttf missing")?;
        let lines = primary.horizontal_line_metrics(PX).context("font without line metrics")?;
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            cell_w: primary.metrics('M', PX).advance_width.ceil() as u32,
            cell_h: (lines.ascent - lines.descent + lines.line_gap).ceil() as u32,
            ascent: lines.ascent,
            regular,
            bold,
        })
    }

    /// Renders and writes `latest.png`; returns its path.
    pub fn save(&self, buffer: &Buffer, placements: &[Placement]) -> Result<PathBuf> {
        let image = self.render(buffer, placements);
        let path = self.dir.join("latest.png");
        image.save(&path)?;
        Ok(path)
    }

    fn render(&self, buffer: &Buffer, placements: &[Placement]) -> RgbaImage {
        let area = buffer.area;
        let mut image = RgbaImage::new(area.width as u32 * self.cell_w, area.height as u32 * self.cell_h);
        let mut glyphs: HashMap<(char, bool), Glyph> = HashMap::new();
        let origin = |x: u16, y: u16| ((x - area.x) as u32 * self.cell_w, (y - area.y) as u32 * self.cell_h);

        // Backgrounds first: glyphs such as Nerd Font icons may overhang into the next cell.
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let (px, py) = origin(x, y);
                fill(&mut image, px, py, self.cell_w, self.cell_h, colors(buffer, x, y).1, 1.0);
            }
        }
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                self.draw_glyph(&mut image, &mut glyphs, buffer, x, y);
            }
        }
        for placement in placements {
            let Some(source) = &placement.source else { continue };
            let (w, h) = (placement.size.width as u32 * self.cell_w, placement.size.height as u32 * self.cell_h);
            // Fit the image into its cells, centred, as kitty does; the images of the other
            // protocols already have about the shape of their cells.
            let fitted = source.resize(w, h, FilterType::Triangle).to_rgba8();
            let mut canvas = RgbaImage::new(w, h);
            let (dx, dy) = ((w - fitted.width()) / 2, (h - fitted.height()) / 2);
            image::imageops::overlay(&mut canvas, &fitted, dx.into(), dy.into());
            let scaled = DynamicImage::from(canvas);
            let visible =
                scaled.crop_imm(0, placement.skip as u32 * self.cell_h, w, placement.area.height as u32 * self.cell_h);
            let x = (placement.area.x - area.x) as i64 * self.cell_w as i64;
            let y = (placement.area.y - area.y) as i64 * self.cell_h as i64;
            image::imageops::overlay(&mut image, &visible.to_rgba8(), x, y);
            // Text drawn over the picture, such as a play button, replaces those image cells.
            let cells = placement.area.intersection(area);
            for y in cells.top()..cells.bottom() {
                for x in cells.left()..cells.right() {
                    let index = (y - placement.area.y) as usize * placement.area.width as usize
                        + (x - placement.area.x) as usize;
                    if placement.cells.get(index).is_some_and(|drawn| *drawn != buffer[(x, y)]) {
                        let (px, py) = origin(x, y);
                        fill(&mut image, px, py, self.cell_w, self.cell_h, colors(buffer, x, y).1, 1.0);
                        self.draw_glyph(&mut image, &mut glyphs, buffer, x, y);
                    }
                }
            }
        }
        image
    }

    fn draw_glyph(
        &self,
        image: &mut RgbaImage,
        glyphs: &mut HashMap<(char, bool), Glyph>,
        buffer: &Buffer,
        x: u16,
        y: u16,
    ) {
        let cell = &buffer[(x, y)];
        let area = buffer.area;
        let (px, py) = ((x - area.x) as u32 * self.cell_w, (y - area.y) as u32 * self.cell_h);
        let (fg, _) = colors(buffer, x, y);
        let Some(c) = cell.symbol().chars().next() else { return };
        if c != ' ' && c != KITTY_PLACEHOLDER && !self.draw_block(image, c, px, py, fg) {
            let bold = cell.modifier.contains(Modifier::BOLD);
            let glyph = glyphs.entry((c, bold)).or_insert_with(|| self.rasterize(c, bold));
            if let Some((metrics, coverage)) = glyph {
                let gx = px as i32 + metrics.xmin;
                let gy = py as i32 + self.ascent.round() as i32 - metrics.height as i32 - metrics.ymin;
                blit(image, gx, gy, metrics.width, metrics.height, coverage, fg);
            }
        }
        if cell.modifier.contains(Modifier::UNDERLINED) {
            let color = rgb(cell.underline_color, fg);
            fill(image, px, py + self.cell_h - 3, self.cell_w, 2, color, 1.0);
        }
    }

    fn rasterize(&self, c: char, bold: bool) -> Glyph {
        let fonts = if bold { &self.bold } else { &self.regular };
        let font = fonts.iter().find(|f| f.lookup_glyph_index(c) != 0)?;
        Some(font.rasterize(c, PX))
    }

    /// Block elements and lines are drawn exactly, like terminals do, so they tile seamlessly.
    fn draw_block(&self, image: &mut RgbaImage, c: char, x: u32, y: u32, fg: [u8; 3]) -> bool {
        let (w, h) = (self.cell_w, self.cell_h);
        let eighth_h = |n: u32| (h * n) / 8;
        let eighth_w = |n: u32| (w * n) / 8;
        let line = (h / 12).max(2);
        let (rx, ry, rw, rh) = match c {
            '█' => (0, 0, w, h),
            '▀' => (0, 0, w, h / 2),
            '▄' => (0, h / 2, w, h - h / 2),
            '▌' => (0, 0, w / 2, h),
            '▐' => (w / 2, 0, w - w / 2, h),
            '▁'..='▇' => {
                let n = c as u32 - '▁' as u32 + 1;
                (0, h - eighth_h(n), w, eighth_h(n))
            }
            '▉'..='▏' => {
                let n = 8 - (c as u32 - '▉' as u32 + 1);
                (0, 0, eighth_w(n), h)
            }
            '▔' => (0, 0, w, eighth_h(1)),
            '▕' => (w - eighth_w(1), 0, eighth_w(1), h),
            '━' => (0, h / 2 - line / 2 - 1, w, line + 2),
            '─' => (0, h / 2 - line / 2, w, line / 2 + 1),
            '│' => (w / 2 - 1, 0, 2, h),
            '┃' => (w / 2 - 2, 0, 4, h),
            _ => return false,
        };
        fill(image, x + rx, y + ry, rw, rh, fg, 1.0);
        true
    }
}

/// Foreground and background of a cell as they end up on screen.
fn colors(buffer: &Buffer, x: u16, y: u16) -> ([u8; 3], [u8; 3]) {
    let cell = &buffer[(x, y)];
    let (mut fg, mut bg) = (rgb(cell.fg, DEFAULT_FG), rgb(cell.bg, DEFAULT_BG));
    if cell.modifier.contains(Modifier::REVERSED) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if cell.modifier.contains(Modifier::DIM) {
        fg = fg.map(|c| (c as f32 * 0.6) as u8);
    }
    (fg, bg)
}

fn rgb(color: Color, default: [u8; 3]) -> [u8; 3] {
    match color {
        Color::Rgb(r, g, b) => [r, g, b],
        Color::Reset => default,
        Color::Black => [0, 0, 0],
        Color::White | Color::Gray => [229, 229, 229],
        Color::DarkGray => [102, 102, 102],
        Color::Red | Color::LightRed => [229, 80, 80],
        Color::Green | Color::LightGreen => [80, 200, 120],
        Color::Yellow | Color::LightYellow => [229, 200, 80],
        Color::Blue | Color::LightBlue => [90, 140, 229],
        Color::Magenta | Color::LightMagenta => [200, 90, 200],
        Color::Cyan | Color::LightCyan => [80, 200, 210],
        Color::Indexed(_) => default,
    }
}

fn fill(image: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, color: [u8; 3], alpha: f32) {
    for py in y..(y + h).min(image.height()) {
        for px in x..(x + w).min(image.width()) {
            blend(image.get_pixel_mut(px, py), color, alpha);
        }
    }
}

fn blit(image: &mut RgbaImage, x: i32, y: i32, w: usize, h: usize, coverage: &[u8], color: [u8; 3]) {
    for gy in 0..h {
        for gx in 0..w {
            let (px, py) = (x + gx as i32, y + gy as i32);
            if px < 0 || py < 0 || px as u32 >= image.width() || py as u32 >= image.height() {
                continue;
            }
            let alpha = coverage[gy * w + gx] as f32 / 255.0;
            if alpha > 0.0 {
                blend(image.get_pixel_mut(px as u32, py as u32), color, alpha);
            }
        }
    }
}

fn blend(pixel: &mut Rgba<u8>, color: [u8; 3], alpha: f32) {
    for (channel, target) in pixel.0.iter_mut().zip(color) {
        *channel = (*channel as f32 * (1.0 - alpha) + target as f32 * alpha) as u8;
    }
    pixel.0[3] = 255;
}
