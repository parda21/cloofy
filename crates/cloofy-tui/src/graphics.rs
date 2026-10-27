//! How covers reach the terminal: the protocol choice, and images for terminals that draw a
//! picture from one escape sequence (Sixel, iTerm2).
//!
//! Kitty images and half blocks are cells that ratatui draws like text. A Sixel or iTerm2 image
//! is not: ratatui would have to send the whole sequence again whenever any of its cells
//! changes, and leaves what was drawn over the image on screen once it is gone. So cloofy keeps
//! ratatui away from those cells and writes the images itself after each frame, only those that
//! are new, moved, or came out from under something.

use std::{
    io::{self, Write},
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use crossterm::{cursor::MoveTo, queue};
use image::{DynamicImage, RgbaImage, imageops::FilterType};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    buffer::{Buffer, Cell, CellDiffOption},
    layout::{Rect, Size},
};
use ratatui_image::{
    FontSize,
    picker::{Capability, Picker, ProtocolType},
};

use crate::snapshot::Placement;

/// What the cells of a Sixel or iTerm2 image hold while the frame is drawn. Anything drawn over
/// the image replaces it, which is how cloofy tells which cells still show the picture.
pub const INLINE_CELL: &str = "\u{10EEED}";

/// How covers are drawn. The Settings page offers these; `Auto` uses what the terminal answered
/// at startup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageProtocol {
    #[default]
    Auto,
    Kitty,
    Sixel,
    Iterm2,
    /// Two colored pixels per cell, in any terminal.
    Blocks,
}

impl ImageProtocol {
    pub const ALL: [Self; 5] = [Self::Auto, Self::Kitty, Self::Sixel, Self::Iterm2, Self::Blocks];

    /// The name to show for it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Kitty => "Kitty",
            Self::Sixel => "Sixel",
            Self::Iterm2 => "iTerm2",
            Self::Blocks => "Blocks",
        }
    }

    /// `CLOOFY_IMAGES` set to `auto`, `kitty`, `sixel`, `iterm2` or `blocks`.
    pub fn from_env() -> Option<Self> {
        Self::parse(&std::env::var("CLOOFY_IMAGES").ok()?)
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|protocol| protocol.label().eq_ignore_ascii_case(name))
    }

    fn of(kind: ProtocolType) -> Self {
        match kind {
            ProtocolType::Kitty => Self::Kitty,
            ProtocolType::Sixel => Self::Sixel,
            ProtocolType::Iterm2 => Self::Iterm2,
            ProtocolType::Halfblocks => Self::Blocks,
        }
    }
}

/// What the terminal said about images at startup, kept so that the protocol can change later
/// without asking again.
#[derive(Clone)]
pub struct ImageTerminal {
    picker: Picker,
    /// What `Auto` draws with.
    auto: ProtocolType,
}

impl ImageTerminal {
    /// Asks the terminal how it shows images and how big its cells are. Must run after entering
    /// the alternate screen and before reading input events.
    ///
    /// When `CLOOFY_IMAGES` forces a protocol, nothing is asked and cells are taken as 10×22 px:
    /// that is for measuring a protocol where the terminal cannot answer, like tmux in tests.
    pub fn detect() -> Self {
        if ImageProtocol::from_env().is_some_and(|forced| forced != ImageProtocol::Auto) {
            // Deprecated because a guessed font size draws images at the wrong size; fine here.
            #[allow(deprecated)]
            let picker = Picker::from_fontsize(FontSize { width: 10, height: 22 });
            return Self { picker, auto: ProtocolType::Halfblocks };
        }
        let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
        let cell_size = picker.capabilities().iter().any(|cap| matches!(cap, Capability::CellSize(Some(_))));
        let set = |name| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        let host = Host {
            tmux: picker.tmux_detected(),
            konsole: set("KONSOLE_VERSION") && cell_size,
            vscode: std::env::var("TERM_PROGRAM").is_ok_and(|program| program == "vscode"),
        };
        let auto = auto_protocol(picker.protocol_type(), host);
        let font = picker.font_size();
        tracing::info!(
            "images: terminal offers {:?}, auto picks {auto:?}, cells of {}x{} px{}",
            picker.protocol_type(),
            font.width,
            font.height,
            if picker.tmux_detected() { ", in tmux" } else { "" },
        );
        Self { picker, auto }
    }

    /// What `Auto` draws with in this terminal.
    pub fn auto(&self) -> ImageProtocol {
        ImageProtocol::of(self.auto)
    }

    /// A picker that draws with `protocol`.
    pub fn picker(&self, protocol: ImageProtocol) -> Picker {
        let mut picker = self.picker.clone();
        picker.set_protocol_type(match protocol {
            ImageProtocol::Auto => self.auto,
            ImageProtocol::Kitty => ProtocolType::Kitty,
            ImageProtocol::Sixel => ProtocolType::Sixel,
            ImageProtocol::Iterm2 => ProtocolType::Iterm2,
            ImageProtocol::Blocks => ProtocolType::Halfblocks,
        });
        picker
    }
}

/// Where cloofy runs, as far as images go.
#[derive(Clone, Copy, Default)]
struct Host {
    tmux: bool,
    /// Konsole, which told its cell size.
    konsole: bool,
    vscode: bool,
}

/// What `Auto` draws with, given the protocol the terminal answered with.
fn auto_protocol(answered: ProtocolType, host: Host) -> ProtocolType {
    match answered {
        // tmux keeps ten Sixel images at most and drops one as soon as text touches it, and it
        // cannot place images it passes through: covers would come and go. Kitty images are
        // cells, which tmux keeps like text.
        ProtocolType::Sixel | ProtocolType::Iterm2 if host.tmux => ProtocolType::Halfblocks,
        // VS Code answers with Sixel when images are on in its settings; iTerm2 is only
        // ratatui-image's guess from its name, and shows nothing while they are off.
        ProtocolType::Iterm2 if host.vscode => ProtocolType::Halfblocks,
        // ratatui-image passes over Konsole's Sixel and Kitty, which leave traces, but has no
        // way to tell its iTerm2 images apart.
        ProtocolType::Halfblocks if host.konsole && !host.tmux => ProtocolType::Iterm2,
        other => other,
    }
}

/// Numbers every inline image, so that one is never mistaken for another drawn at the same place.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A Sixel or iTerm2 image, encoded once, that can be drawn from any of its rows to any other.
pub struct Inline {
    id: u64,
    /// In cells.
    size: Size,
    font_height: u16,
    data: InlineData,
}

enum InlineData {
    /// One image whose bands of six pixel rows can be left out at either end.
    Sixel {
        /// The introducer and raster attributes up to the height, which depends on the bands drawn.
        start: String,
        palette: String,
        bands: String,
        /// Where each band ends in `bands`, including its `-`.
        ends: Vec<usize>,
    },
    /// A PNG per row of cells; an iTerm2 image cannot be cut.
    Iterm2 { rows: Vec<String> },
}

impl Inline {
    /// Encodes `image` to fit `size` cells, in proportion, at the top left.
    pub fn new(kind: ProtocolType, image: &DynamicImage, size: Size, font: FontSize, tmux: bool) -> Result<Self> {
        let (width, height) = (u32::from(font.width), u32::from(font.height));
        // Triangle (bilinear) is several times faster than Lanczos and looks the same when shrinking.
        let fitted = image.resize(u32::from(size.width) * width, u32::from(size.height) * height, FilterType::Triangle);
        let size = Size::new(fitted.width().div_ceil(width) as u16, fitted.height().div_ceil(height) as u16);
        // Whole cells, so that rows are as tall as cells; the rest stays transparent.
        let mut canvas = RgbaImage::new(u32::from(size.width) * width, u32::from(size.height) * height);
        image::imageops::overlay(&mut canvas, &fitted.to_rgba8(), 0, 0);
        let data = match kind {
            ProtocolType::Sixel => sixel(&canvas)?,
            ProtocolType::Iterm2 => InlineData::Iterm2 { rows: iterm2_rows(&canvas, size.width, height, tmux)? },
            other => bail!("{other:?} images are cells"),
        };
        Ok(Self { id: NEXT_ID.fetch_add(1, Ordering::Relaxed), size, font_height: font.height, data })
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn kind(&self) -> ProtocolType {
        match self.data {
            InlineData::Sixel { .. } => ProtocolType::Sixel,
            InlineData::Iterm2 { .. } => ProtocolType::Iterm2,
        }
    }

    /// Memory the encoded image takes.
    #[cfg(test)]
    pub fn bytes(&self) -> usize {
        match &self.data {
            InlineData::Sixel { start, palette, bands, ends } => {
                start.len() + palette.len() + bands.len() + ends.len() * size_of::<usize>()
            }
            InlineData::Iterm2 { rows } => rows.iter().map(String::len).sum(),
        }
    }

    /// Writes what draws `rows` of the image with the first of them at `(x, y)`.
    fn draw(&self, out: &mut Vec<u8>, (x, y): (u16, u16), rows: Range<u16>) -> io::Result<()> {
        match &self.data {
            InlineData::Sixel { start, palette, bands, ends } => {
                // Bands are six pixels, rows as tall as the font: start at the first band that
                // begins in the top row and stop before one that would reach past the bottom
                // row. A few pixels of the image may go missing; none is drawn outside its rows.
                let height = usize::from(self.font_height);
                let first = (usize::from(rows.start) * height).div_ceil(6);
                let last = (usize::from(rows.end) * height / 6).min(ends.len());
                if first >= last {
                    return Ok(());
                }
                let from = if first == 0 { 0 } else { ends[first - 1] };
                queue!(out, MoveTo(x, y))?;
                write!(out, "{start}{}{palette}", (last - first) * 6)?;
                out.extend_from_slice(&bands.as_bytes()[from..ends[last - 1]]);
                out.extend_from_slice(b"\x1b\\");
            }
            InlineData::Iterm2 { rows: images } => {
                let rows = usize::from(rows.start)..usize::from(rows.end).min(images.len());
                for (row, image) in images.get(rows).unwrap_or_default().iter().enumerate() {
                    queue!(out, MoveTo(x, y + row as u16))?;
                    out.extend_from_slice(image.as_bytes());
                }
            }
        }
        Ok(())
    }
}

/// Encodes a Sixel image and splits it into its bands.
fn sixel(image: &RgbaImage) -> Result<InlineData> {
    let (width, height) = image.dimensions();
    let options = icy_sixel::EncodeOptions::default();
    let data = icy_sixel::sixel_encode(image.as_raw(), width as usize, height as usize, &options)
        .map_err(|err| anyhow::anyhow!("sixel: {err}"))?;
    // `ESC P … q "pan;pad;width;height`, then the palette (`#n;2;r;g;b` each), then the bands,
    // each ending in `-`, then `ESC \`.
    let Some(body) = data.strip_suffix("\x1b\\") else { bail!("sixel: no string terminator") };
    let Some(raster) = body.find('"') else { bail!("sixel: no raster attributes") };
    let Some(palette_at) = body[raster..].find('#').map(|at| raster + at) else { bail!("sixel: no colors") };
    let Some(height_at) = body[..palette_at].rfind(';').map(|at| at + 1) else { bail!("sixel: no height") };
    let bands_at = palette_end(body, palette_at);
    let bands = &body[bands_at..];
    let ends = bands.match_indices('-').map(|(at, _)| at + 1).collect();
    Ok(InlineData::Sixel {
        start: body[..height_at].to_owned(),
        palette: body[palette_at..bands_at].to_owned(),
        bands: bands.to_owned(),
        ends,
    })
}

/// Where the color definitions starting at `at` end: at the first `#n` that selects a color
/// rather than defining one.
fn palette_end(body: &str, mut at: usize) -> usize {
    let bytes = body.as_bytes();
    while bytes.get(at) == Some(&b'#') {
        let digits = bytes[at + 1..].iter().take_while(|b| b.is_ascii_digit()).count();
        if bytes.get(at + 1 + digits) != Some(&b';') {
            break;
        }
        let rest = &bytes[at + 1..];
        at += 1 + rest.iter().position(|&b| b == b'#').unwrap_or(rest.len());
    }
    at
}

/// One iTerm2 image per row of cells, each stretched over exactly its cells, so that a terminal
/// whose cells are not the size cloofy was told still shows rows that line up.
fn iterm2_rows(image: &RgbaImage, columns: u16, row_height: u32, tmux: bool) -> Result<Vec<String>> {
    let (width, height) = image.dimensions();
    let mut rows = Vec::new();
    for top in (0..height).step_by(row_height as usize) {
        let row = image::imageops::crop_imm(image, 0, top, width, row_height).to_image();
        let mut png = Vec::new();
        DynamicImage::from(row).write_to(&mut io::Cursor::new(&mut png), image::ImageFormat::Png)?;
        let sequence = format!(
            "\x1b]1337;File=inline=1;size={};width={columns};height=1;preserveAspectRatio=0;doNotMoveCursor=1:{}\x07",
            png.len(),
            STANDARD.encode(&png),
        );
        // tmux has no iTerm2 images of its own; they pass through to the terminal around it.
        rows.push(if tmux { format!("\x1bPtmux;{}\x1b\\", sequence.replace('\x1b', "\x1b\x1b")) } else { sequence });
    }
    Ok(rows)
}

/// Writes the Sixel and iTerm2 images of each frame, after ratatui has written the text.
#[derive(Default)]
pub struct InlineLayer {
    /// The images of the last frame.
    shown: Vec<Shown>,
    screen: Rect,
    pending: Vec<Pending>,
}

struct Shown {
    id: u64,
    area: Rect,
    skip: u16,
    /// For each cell of `area`, row by row, whether it showed the image.
    cells: Vec<bool>,
}

/// An image to write after the frame, and the cells drawn over it, which go on top again.
struct Pending {
    blank: Vec<(u16, u16, Cell)>,
    image: Vec<u8>,
    over: Vec<(u16, u16, Cell)>,
}

impl InlineLayer {
    /// Keeps ratatui away from the cells that show an image and finds the images to write: new
    /// ones, moved ones, and ones with cells that something covered in the last frame. Called
    /// once the frame is drawn, before ratatui writes it.
    pub fn prepare(&mut self, buf: &mut Buffer, placements: &[Placement]) {
        // A resize clears the screen.
        if buf.area != self.screen {
            self.screen = buf.area;
            self.shown.clear();
        }
        self.pending.clear();
        let mut shown = Vec::new();
        for placement in placements {
            let Some(image) = &placement.inline else { continue };
            let area = placement.area.intersection(buf.area);
            let mut cells = Vec::with_capacity(area.area() as usize);
            for position in area.positions() {
                let cell = &mut buf[position];
                let showing = cell.symbol() == INLINE_CELL;
                if showing {
                    cell.set_diff_option(CellDiffOption::Skip);
                } else if cell.diff_option == CellDiffOption::Skip {
                    // Drawn over without being cleared, like a play button: ratatui writes it.
                    cell.set_diff_option(CellDiffOption::None);
                }
                cells.push(showing);
            }
            let before = self.shown.iter().find(|s| s.id == image.id && s.area == area && s.skip == placement.skip);
            let uncovered =
                before.is_none_or(|before| cells.iter().zip(&before.cells).any(|(now, then)| *now && !then));
            if uncovered && cells.contains(&true) {
                let mut bytes = Vec::new();
                let rows = placement.skip..placement.skip + area.height;
                if image.draw(&mut bytes, (area.x, area.y), rows).is_ok() {
                    let (mut blank, mut over) = (Vec::new(), Vec::new());
                    for (position, showing) in area.positions().zip(&cells) {
                        let cell = buf[position].clone();
                        if *showing {
                            let mut space = Cell::new(" ");
                            space.set_bg(cell.bg);
                            blank.push((position.x, position.y, space));
                        } else {
                            over.push((position.x, position.y, cell));
                        }
                    }
                    self.pending.push(Pending { blank, image: bytes, over });
                }
            }
            shown.push(Shown { id: image.id, area, skip: placement.skip, cells });
        }
        self.shown = shown;
    }

    /// Writes the images found by [`InlineLayer::prepare`], once ratatui has written the frame.
    /// Their cells are blanked first, so that see-through parts show the background rather than
    /// what was there before, and what was drawn over an image goes back on top of it.
    pub fn write<W: Write>(&mut self, backend: &mut CrosstermBackend<W>) -> io::Result<()> {
        for pending in self.pending.drain(..) {
            backend.draw(pending.blank.iter().map(|(x, y, cell)| (*x, *y, cell)))?;
            backend.write_all(&pending.image)?;
            backend.draw(pending.over.iter().map(|(x, y, cell)| (*x, *y, cell)))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Instant};

    use ratatui::{
        Terminal,
        style::Style,
        widgets::{Clear, Widget},
    };

    use super::*;
    use crate::loader::{Graphic, Picture};

    const FONT: FontSize = FontSize { width: 10, height: 20 };

    fn terminal(auto: ProtocolType) -> ImageTerminal {
        ImageTerminal { picker: Picker::halfblocks(), auto }
    }

    #[test]
    fn a_forced_protocol_wins_over_what_the_terminal_answered() {
        let images = terminal(ProtocolType::Sixel);
        let kind = |protocol| images.picker(protocol).protocol_type();
        assert_eq!(kind(ImageProtocol::Auto), ProtocolType::Sixel);
        assert_eq!(kind(ImageProtocol::Kitty), ProtocolType::Kitty);
        assert_eq!(kind(ImageProtocol::Sixel), ProtocolType::Sixel);
        assert_eq!(kind(ImageProtocol::Iterm2), ProtocolType::Iterm2);
        assert_eq!(kind(ImageProtocol::Blocks), ProtocolType::Halfblocks);
        assert_eq!(images.auto(), ImageProtocol::Sixel);
    }

    #[test]
    fn protocols_are_named_as_in_the_environment_variable() {
        assert_eq!(ImageProtocol::parse("sixel"), Some(ImageProtocol::Sixel));
        assert_eq!(ImageProtocol::parse("iterm2"), Some(ImageProtocol::Iterm2));
        assert_eq!(ImageProtocol::parse("Kitty"), Some(ImageProtocol::Kitty));
        assert_eq!(ImageProtocol::parse("blocks"), Some(ImageProtocol::Blocks));
        assert_eq!(ImageProtocol::parse("auto"), Some(ImageProtocol::Auto));
        assert_eq!(ImageProtocol::parse("png"), None);
    }

    #[test]
    fn auto_leaves_out_what_would_lose_covers() {
        use ProtocolType::{Halfblocks, Iterm2, Kitty, Sixel};
        let tmux = Host { tmux: true, ..Host::default() };
        let konsole = Host { konsole: true, ..Host::default() };
        let vscode = Host { vscode: true, ..Host::default() };
        assert_eq!(auto_protocol(Sixel, Host::default()), Sixel);
        assert_eq!(auto_protocol(Iterm2, Host::default()), Iterm2);
        assert_eq!(auto_protocol(Halfblocks, Host::default()), Halfblocks);
        assert_eq!(auto_protocol(Sixel, tmux), Halfblocks);
        assert_eq!(auto_protocol(Iterm2, tmux), Halfblocks);
        assert_eq!(auto_protocol(Kitty, tmux), Kitty);
        assert_eq!(auto_protocol(Halfblocks, konsole), Iterm2);
        assert_eq!(auto_protocol(Halfblocks, Host { tmux: true, ..konsole }), Halfblocks);
        assert_eq!(auto_protocol(Sixel, vscode), Sixel);
        assert_eq!(auto_protocol(Iterm2, vscode), Halfblocks);
    }

    /// An image in two colors: red on top, blue below.
    fn two_tone(width: u32, height: u32) -> DynamicImage {
        let image = RgbaImage::from_fn(width, height, |_, y| {
            if y < height / 2 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 255, 255]) }
        });
        image.into()
    }

    /// The images written in `bytes`, decoded: their width and height in pixels.
    fn sixels(bytes: &[u8]) -> Vec<(usize, usize)> {
        let text = String::from_utf8_lossy(bytes);
        text.match_indices("\x1bP")
            .map(|(at, _)| {
                let end = text[at..].find("\x1b\\").map(|end| at + end + 2).expect("terminated");
                let image = icy_sixel::SixelImage::decode(&bytes[at..end]).expect("valid sixel");
                (image.width, image.height)
            })
            .collect()
    }

    #[test]
    fn a_sixel_image_is_cut_to_whole_bands_inside_its_rows() {
        // 3×3 cells of 10×20 px: 30×60 px, ten bands.
        let inline = Inline::new(ProtocolType::Sixel, &two_tone(30, 60), Size::new(3, 3), FONT, false).unwrap();
        assert_eq!(inline.size(), Size::new(3, 3));
        let draw = |rows| {
            let mut out = Vec::new();
            inline.draw(&mut out, (0, 0), rows).unwrap();
            sixels(&out)
        };
        assert_eq!(draw(0..3), [(30, 60)]);
        // Row 1 starts at pixel 20, inside band 3: drawing starts with band 4, at pixel 24.
        assert_eq!(draw(1..3), [(30, 36)]);
        // Row 1 ends at pixel 40, inside band 6, which is left out.
        assert_eq!(draw(0..2), [(30, 36)]);
        assert_eq!(draw(1..2), [(30, 12)]);
    }

    #[test]
    fn a_sixel_image_keeps_its_colors_when_cut() {
        let inline = Inline::new(ProtocolType::Sixel, &two_tone(30, 60), Size::new(3, 3), FONT, false).unwrap();
        let mut out = Vec::new();
        inline.draw(&mut out, (0, 0), 2..3).unwrap();
        let start = out.windows(2).position(|pair| pair == b"\x1bP").unwrap();
        let image = icy_sixel::SixelImage::decode(&out[start..]).unwrap();
        // The bottom row is all blue.
        assert!(image.pixels.chunks(4).all(|pixel| pixel[..3] == [0, 0, 255]));
    }

    #[test]
    fn an_iterm2_image_has_a_row_per_row_of_cells() {
        let inline = Inline::new(ProtocolType::Iterm2, &two_tone(30, 60), Size::new(3, 3), FONT, false).unwrap();
        let mut out = Vec::new();
        inline.draw(&mut out, (5, 7), 1..3).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.matches("\x1b]1337;File=").count(), 2);
        assert!(text.starts_with("\x1b[8;6H"));
        assert!(text.contains("width=3;height=1;"));

        let tmux = Inline::new(ProtocolType::Iterm2, &two_tone(30, 60), Size::new(3, 3), FONT, true).unwrap();
        let mut out = Vec::new();
        tmux.draw(&mut out, (0, 0), 0..1).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\x1bPtmux;\x1b\x1b]1337;File="));
    }

    /// What the terminal is sent.
    #[derive(Clone, Default)]
    struct Capture(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Draws frames the way cloofy does, and returns what each one writes to the terminal.
    struct Screen {
        terminal: Terminal<CrosstermBackend<Capture>>,
        sent: Capture,
        layer: InlineLayer,
    }

    impl Screen {
        fn new() -> Self {
            let sent = Capture::default();
            let backend = CrosstermBackend::new(sent.clone());
            let viewport = ratatui::Viewport::Fixed(Rect::new(0, 0, 40, 20));
            let terminal = Terminal::with_options(backend, ratatui::TerminalOptions { viewport }).unwrap();
            Self { terminal, sent, layer: InlineLayer::default() }
        }

        /// Draws `covers` with their top-left corners at the given cells (a negative row is
        /// above the content area, which starts at row 2), then `over` on top.
        fn frame(&mut self, covers: &[(&Picture, (u16, i32))], over: impl FnOnce(&mut Buffer)) -> Vec<u8> {
            self.sent.0.borrow_mut().clear();
            let layer = &mut self.layer;
            self.terminal
                .draw(|frame| {
                    let clip = Rect::new(0, 2, 40, 16);
                    let placements: Vec<_> = covers
                        .iter()
                        .filter_map(|(picture, at)| crate::ui::picture_in(frame, picture, *at, picture.size(), clip))
                        .collect();
                    over(frame.buffer_mut());
                    layer.prepare(frame.buffer_mut(), &placements);
                })
                .unwrap();
            self.layer.write(self.terminal.backend_mut()).unwrap();
            self.sent.0.take()
        }
    }

    fn picture(kind: ProtocolType) -> Picture {
        let inline = Inline::new(kind, &two_tone(60, 60), Size::new(6, 3), FONT, false).unwrap();
        Picture { graphic: Graphic::Inline(Arc::new(inline)), source: None, terminal_bytes: 0 }
    }

    fn images(bytes: &[u8]) -> usize {
        let text = String::from_utf8_lossy(bytes);
        text.matches("\x1bP").count() + text.matches("\x1b]1337;").count()
    }

    #[test]
    fn a_cover_is_written_once_and_not_again_while_it_stays() {
        let (a, b) = (picture(ProtocolType::Sixel), picture(ProtocolType::Sixel));
        let mut screen = Screen::new();
        let both = [(&a, (2, 3)), (&b, (12, 3))];
        assert_eq!(images(&screen.frame(&both, |_| {})), 2);
        assert_eq!(images(&screen.frame(&both, |_| {})), 0);
        // Recoloring the cells, as a page fade does, changes nothing.
        let fade = |buf: &mut Buffer| buf.set_style(buf.area, Style::new().bg(ratatui::style::Color::Rgb(1, 2, 3)));
        assert_eq!(images(&screen.frame(&both, fade)), 0);
        // A new cover comes alone.
        let c = picture(ProtocolType::Sixel);
        assert_eq!(images(&screen.frame(&[(&a, (2, 3)), (&b, (12, 3)), (&c, (22, 3))], |_| {})), 1);
    }

    #[test]
    fn an_overlay_is_drawn_over_a_cover_and_the_cover_comes_back_whole() {
        let a = picture(ProtocolType::Sixel);
        let mut screen = Screen::new();
        let covers = [(&a, (2, 3))];
        screen.frame(&covers, |_| {});
        // A menu over the right half of the cover: its text is written, the cover is not.
        let menu = |buf: &mut Buffer| {
            let popup = Rect::new(4, 4, 20, 3);
            Clear.render(popup, buf);
            buf.set_string(5, 5, "Add to queue", Style::new());
        };
        let out = screen.frame(&covers, menu);
        assert_eq!(images(&out), 0);
        // Cells left as they were are skipped, so the words come apart.
        assert!(String::from_utf8_lossy(&out).contains("Add"));
        assert_eq!(images(&screen.frame(&covers, menu)), 0);
        // Closed: the cover is written again over what the menu left.
        assert_eq!(images(&screen.frame(&covers, |_| {})), 1);
        assert_eq!(images(&screen.frame(&covers, |_| {})), 0);
    }

    #[test]
    fn a_play_button_goes_back_on_top_when_its_cover_is_written() {
        let a = picture(ProtocolType::Sixel);
        let mut screen = Screen::new();
        let button = |buf: &mut Buffer| buf.set_string(4, 5, " ▶ ", Style::new());
        // Hovered as it appears: the image first, then the button over it.
        let out = String::from_utf8(screen.frame(&[(&a, (2, 3))], button)).unwrap();
        let (image, play) = (out.find("\x1bP").unwrap(), out.rfind('▶').unwrap());
        assert!(image < play);
        // The mouse leaves: the cover is written again, without the button.
        let out = screen.frame(&[(&a, (2, 3))], |_| {});
        assert_eq!(images(&out), 1);
        assert!(!String::from_utf8_lossy(&out).contains('▶'));
        // And comes back: only the button is written.
        let out = screen.frame(&[(&a, (2, 3))], button);
        assert_eq!(images(&out), 0);
        assert!(String::from_utf8_lossy(&out).contains('▶'));
    }

    #[test]
    fn a_cover_scrolled_partly_out_is_written_cut_to_its_visible_rows() {
        let a = picture(ProtocolType::Sixel);
        let mut screen = Screen::new();
        screen.frame(&[(&a, (2, 3))], |_| {});
        // One row above the content area, which starts at row 2.
        let out = screen.frame(&[(&a, (2, 1))], |_| {});
        assert_eq!(sixels(&out), [(60, 36)]);
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("\x1b[3;3H\x1bP"), "drawn from the first visible row");
        // Scrolled out entirely: nothing to write.
        assert_eq!(images(&screen.frame(&[(&a, (2, -2))], |_| {})), 0);
    }

    #[test]
    fn iterm2_covers_follow_the_same_rules() {
        let a = picture(ProtocolType::Iterm2);
        let mut screen = Screen::new();
        assert_eq!(images(&screen.frame(&[(&a, (2, 3))], |_| {})), 3);
        assert_eq!(images(&screen.frame(&[(&a, (2, 3))], |_| {})), 0);
        assert_eq!(images(&screen.frame(&[(&a, (2, 1))], |_| {})), 2);
    }

    #[test]
    fn a_cover_is_never_drawn_in_the_last_row() {
        let a = picture(ProtocolType::Sixel);
        let mut screen = Screen::new();
        let mut placed = None;
        screen.terminal.draw(|frame| placed = crate::ui::picture_at(frame, &a, Rect::new(0, 17, 6, 3))).unwrap();
        assert_eq!(placed.map(|p| p.area), Some(Rect::new(0, 17, 6, 2)));
    }

    /// Encode time and size of real covers, from the image cache:
    /// `cargo test -p cloofy-tui --release measure_covers -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn measure_covers() {
        let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/cloofy/images");
        let covers: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter_map(|entry| image::load_from_memory(&std::fs::read(entry.path()).ok()?).ok())
            .take(20)
            .collect();
        let font = FontSize { width: 10, height: 22 };
        // A Home card, the player bar, and Now Playing in a large window.
        for size in [Size::new(14, 7), Size::new(8, 3), Size::new(60, 27)] {
            for kind in [ProtocolType::Sixel, ProtocolType::Iterm2] {
                let start = Instant::now();
                let encoded: Vec<_> =
                    covers.iter().map(|image| Inline::new(kind, image, size, font, false).unwrap()).collect();
                let each = start.elapsed() / covers.len() as u32;
                let bytes = encoded.iter().map(Inline::bytes).sum::<usize>() / encoded.len();
                println!("{kind:?} {}x{} cells: {each:?} and {} KiB per cover", size.width, size.height, bytes / 1024);
            }
        }
    }
}
