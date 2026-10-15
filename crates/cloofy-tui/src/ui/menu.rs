//! The context menu, opened next to the mouse or the selection.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    widgets::{Block, BorderType, Clear, Widget},
};
use unicode_width::UnicodeWidthStr;

use super::text;
use crate::{
    app::{App, Hit},
    overlay::Overlay,
    theme::{FAINT, MUTED, RAISED, TEXT},
};

const MAX_WIDTH: u16 = 44;

pub fn draw(buf: &mut Buffer, app: &mut App, screen: Rect) {
    let accent = app.accent.color();
    let selection = app.accent.selection();
    let Some(Overlay::Menu(menu)) = &app.overlay else { return };

    let widest = menu.items.iter().map(|(_, label)| label.width()).max().unwrap_or(0);
    let width = ((widest.max(menu.target.name.width().min(30)) + 6) as u16).min(MAX_WIDTH).min(screen.width);
    let height = (menu.items.len() as u16 + 4).min(screen.height);
    let (anchor_x, anchor_y) = menu.anchor;
    let x = anchor_x.min(screen.right().saturating_sub(width));
    let y = if anchor_y + height <= screen.bottom() { anchor_y } else { anchor_y.saturating_sub(height) };
    let popup = Rect::new(x, y, width, height);

    Clear.render(popup, buf);
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(FAINT))
        .style(Style::new().bg(RAISED))
        .render(popup, buf);
    let inner_width = popup.width.saturating_sub(4) as usize;
    buf.set_string(
        popup.x + 2,
        popup.y + 1,
        text::truncate(&menu.target.name, inner_width),
        Style::new().fg(MUTED).bold(),
    );
    buf.set_string(
        popup.x + 1,
        popup.y + 2,
        "─".repeat(popup.width.saturating_sub(2) as usize),
        Style::new().fg(FAINT),
    );

    let mut hits = vec![(popup, Hit::OverlayArea)];
    for (index, (_, label)) in menu.items.iter().enumerate() {
        let row_y = popup.y + 3 + index as u16;
        if row_y >= popup.bottom().saturating_sub(1) {
            break;
        }
        let row = Rect::new(popup.x + 1, row_y, popup.width.saturating_sub(2), 1);
        let selected = index == menu.cursor;
        if selected {
            buf.set_style(row, Style::new().bg(selection));
            buf.set_string(row.x, row_y, "▌", Style::new().fg(accent));
        }
        let style = if selected { Style::new().fg(TEXT).bold() } else { Style::new().fg(TEXT) };
        buf.set_string(popup.x + 2, row_y, text::truncate(label, inner_width), style);
        hits.push((row, Hit::OverlayItem(index)));
    }
    app.hits.extend(hits);
    if app.overlay_fx_pending {
        app.animate_overlay(popup);
    }
}
