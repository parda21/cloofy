//! The cloofy wordmark: a pixel font drawn with half blocks, tinted with the accent color.

use ratatui::{buffer::Buffer, style::Style};

use crate::theme::Rgb;

/// Glyphs five pixels tall (the sixth row stays empty); two pixel rows make one text row.
const GLYPHS: [(char, [&str; 6]); 5] = [
    ('c', [".##", "#..", "#..", "#..", ".##", "..."]),
    ('l', ["#", "#", "#", "#", "#", "."]),
    ('o', [".#.", "#.#", "#.#", "#.#", ".#.", "..."]),
    ('f', [".##", "#..", "###", "#..", "#..", "..."]),
    ('y', ["#.#", "#.#", ".##", "..#", "##.", "..."]),
];
const WORD: &str = "cloofy";

pub const HEIGHT: u16 = 3;
pub const WIDTH: u16 = 21;

/// Draws the wordmark with its top-left corner at `x`, `y`.
pub fn draw(buf: &mut Buffer, x: u16, y: u16, accent: Rgb) {
    let columns = columns();
    let highlight = accent.mix(crate::theme::palette().highlight, 0.45);
    for (index, column) in columns.iter().enumerate() {
        let t = index as f32 / (columns.len().max(2) - 1) as f32;
        let color = highlight.mix(accent, t);
        for row in 0..HEIGHT as usize {
            let symbol = match (column[row * 2], column[row * 2 + 1]) {
                (true, true) => "█",
                (true, false) => "▀",
                (false, true) => "▄",
                (false, false) => continue,
            };
            buf.set_string(x + index as u16, y + row as u16, symbol, Style::new().fg(color.into()));
        }
    }
}

/// Pixel columns of the whole word, one blank column between letters.
fn columns() -> Vec<[bool; 6]> {
    let mut columns = Vec::new();
    for (i, letter) in WORD.chars().enumerate() {
        let glyph = GLYPHS.iter().find(|(c, _)| *c == letter).map(|(_, g)| g).expect("glyph for every letter");
        for x in 0..glyph[0].len() {
            columns.push(std::array::from_fn(|y| glyph[y].as_bytes()[x] == b'#'));
        }
        if i + 1 < WORD.len() {
            columns.push([false; 6]);
        }
    }
    columns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_matches_the_glyphs() {
        assert_eq!(columns().len() as u16, WIDTH);
    }
}
