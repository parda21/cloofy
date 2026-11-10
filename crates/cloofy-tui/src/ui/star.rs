//! Asks for a star on GitHub, once cloofy is clearly in use: a short note and two buttons.

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Style, Stylize},
    text::Line,
    widgets::{Block, BorderType, Clear, Paragraph, Widget, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{App, Hit},
    overlay::Overlay,
    theme,
};

const WIDTH: u16 = 54;
const HEIGHT: u16 = 10;
/// A star that every terminal font has.
const STAR_ICON: &str = "★";
const NOTE: &str = "cloofy is free and made in spare time. If you like it, a star on GitHub helps \
                    other people find it.";
/// The two buttons, in the order the cursor goes through them.
pub const STAR: &str = "Star on GitHub";
pub const NO_THANKS: &str = "No thanks";

pub fn draw(buf: &mut Buffer, app: &mut App, area: Rect) {
    let p = theme::palette();
    let Some(Overlay::Star { cursor }) = app.overlay else { return };
    let popup = area.centered(Constraint::Length(WIDTH.min(area.width)), Constraint::Length(HEIGHT.min(area.height)));
    Clear.render(popup, buf);
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(p.faint))
        .title(Line::from(format!(" {STAR_ICON} Enjoying cloofy? ")).fg(p.text).bold())
        .style(Style::new().bg(p.raised))
        .render(popup, buf);
    app.hits.push((popup, Hit::OverlayArea));

    let inner = Rect { x: popup.x + 3, y: popup.y + 2, width: popup.width.saturating_sub(6), height: 3 };
    Paragraph::new(NOTE).wrap(Wrap { trim: true }).style(Style::new().fg(p.muted)).render(inner, buf);

    let y = popup.y + 6;
    if y >= popup.bottom().saturating_sub(1) {
        return;
    }
    let labels = [format!(" {STAR_ICON} {STAR} "), format!(" {NO_THANKS} ")];
    let widths: Vec<u16> = labels.iter().map(|label| label.width() as u16).collect();
    let total = widths.iter().sum::<u16>() + 3;
    let mut x = popup.x + popup.width.saturating_sub(total) / 2;
    let accent = app.accent.color();
    for (index, label) in labels.iter().enumerate() {
        let button = Rect::new(x, y, widths[index], 1).intersection(popup);
        let style = if index == cursor {
            Style::new().fg(p.bg).bg(accent).bold()
        } else {
            Style::new().fg(p.text).bg(p.unfocused)
        };
        buf.set_string(button.x, y, label, style);
        app.hits.push((button, Hit::OverlayItem(index)));
        x += widths[index] + 3;
    }
    let hint = "enter choose · esc close";
    let hint_x = popup.x + popup.width.saturating_sub(hint.width() as u16) / 2;
    buf.set_string(hint_x, y + 2, hint, Style::new().fg(p.faint));
    if app.overlay_fx_pending {
        app.animate_overlay(popup);
    }
}
