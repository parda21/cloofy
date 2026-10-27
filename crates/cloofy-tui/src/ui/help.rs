use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Widget},
};

use super::text;
use crate::{
    app::{App, Hit},
    theme,
};

type Section = (&'static str, &'static [(&'static str, &'static str)]);

const NAVIGATION: Section = (
    "NAVIGATION",
    &[
        ("↑ ↓  j k", "Move"),
        ("← →  h l", "Move between cards"),
        ("←  h", "To the sidebar"),
        ("← →", "Filter (in sidebar)"),
        ("enter", "Open / play"),
        ("tab", "Switch pane"),
        ("g  G", "Top / bottom"),
        ("PgUp PgDn", "Page up / down"),
        ("^U  ^D", "Half a page up / down"),
        ("1  2  3", "Home, Search, Library"),
        ("<  >", "Filter (Library page)"),
        ("esc", "Back"),
    ],
);
const PLAYBACK: Section = (
    "PLAYBACK",
    &[
        ("space", "Play / pause"),
        ("]  [", "Next / previous song"),
        ("⇧←  ⇧→  H L", "Seek 5 seconds"),
        ("+  -", "Volume"),
        ("s", "Shuffle"),
        ("r", "Repeat: off, all, one"),
        ("p", "Play (Home, artist)"),
        ("D", "Start DJ / ask DJ"),
        ("X", "Crossfade length"),
        ("z", "Now playing & lyrics"),
        ("d", "Switch device"),
    ],
);
const LIBRARY: Section = (
    "LIBRARY",
    &[
        ("f", "Like / save"),
        ("F", "Follow artist"),
        ("q  Q", "Add to queue / show queue"),
        ("a", "Add to playlist"),
        (".", "Actions menu"),
    ],
);
const GENERAL: Section = (
    "GENERAL",
    &[
        ("^K  :", "Command palette"),
        ("/", "Search"),
        (",", "Settings"),
        ("?", "This help"),
        ("U", "Update cloofy"),
        ("^C  ^Q", "Quit"),
    ],
);

/// Everything in one column where the window is tall enough.
const ONE_COLUMN: &[&[Section]] = &[&[NAVIGATION, PLAYBACK, LIBRARY, GENERAL]];
/// Short windows get two columns side by side.
const TWO_COLUMNS: &[&[Section]] = &[&[NAVIGATION, LIBRARY], &[PLAYBACK, GENERAL]];
const ONE_COLUMN_WIDTH: u16 = 58;
const TWO_COLUMN_WIDTH: u16 = 100;
/// Two columns narrower than this would cut most descriptions short.
const TWO_COLUMN_MIN_WIDTH: u16 = 72;
/// Room for the keys in front of each description.
const KEY_WIDTH: usize = 13;
/// Space between the border and the text, and between columns.
const PADDING: u16 = 3;

pub fn draw(buf: &mut Buffer, app: &mut App, area: Rect) {
    let p = theme::palette();
    let fits = height(ONE_COLUMN) <= area.height || area.width < TWO_COLUMN_MIN_WIDTH;
    let (columns, width) = if fits { (ONE_COLUMN, ONE_COLUMN_WIDTH) } else { (TWO_COLUMNS, TWO_COLUMN_WIDTH) };
    let popup =
        area.centered(Constraint::Length(width.min(area.width)), Constraint::Length(height(columns).min(area.height)));
    Clear.render(popup, buf);
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(p.faint))
        .title(Line::from(" Keyboard shortcuts ").fg(p.text).bold())
        .style(Style::new().bg(p.raised))
        .render(popup, buf);

    // Whatever doesn't fit above the bottom border is left out; the buffer ends right there
    // in short windows.
    let bottom = popup.bottom().saturating_sub(1);
    let count = columns.len() as u16;
    let column_width = popup.width.saturating_sub(PADDING * (count + 1)) / count;
    for (index, sections) in columns.iter().enumerate() {
        let x = popup.x + PADDING + index as u16 * (column_width + PADDING);
        let mut y = popup.y + 2;
        'column: for (title, keys) in sections.iter() {
            if y >= bottom {
                break;
            }
            buf.set_stringn(x, y, *title, column_width as usize, Style::new().fg(p.faint).bold());
            y += 1;
            for (key, label) in *keys {
                if y >= bottom {
                    break 'column;
                }
                let label_width = (column_width as usize).saturating_sub(KEY_WIDTH);
                let line = Line::from(vec![
                    Span::styled(text::fit(key, KEY_WIDTH), Style::new().fg(p.text).bold()),
                    Span::styled(text::truncate(label, label_width), p.muted),
                ]);
                buf.set_line(x, y, &line, column_width);
                y += 1;
            }
            y += 1;
        }
    }
    app.hits.push((popup, Hit::OverlayArea));
    if app.overlay_fx_pending {
        app.animate_overlay(popup);
    }
}

/// Rows the popup needs for `columns`: borders, a blank line at the top and each section's
/// title, keys and the blank line after it.
fn height(columns: &[&[Section]]) -> u16 {
    let rows = |sections: &[Section]| sections.iter().map(|(_, keys)| keys.len() as u16 + 2).sum::<u16>();
    columns.iter().map(|sections| rows(sections)).max().unwrap_or(0) + 3
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr;

    use super::*;

    #[test]
    fn fits_a_classic_terminal_in_two_columns() {
        assert!(height(TWO_COLUMNS) <= 24);
    }

    #[test]
    fn keys_fit_their_column() {
        for (_, keys) in ONE_COLUMN.iter().flat_map(|column| column.iter()) {
            for (key, _) in *keys {
                assert!(key.width() < KEY_WIDTH, "{key}");
            }
        }
    }
}
