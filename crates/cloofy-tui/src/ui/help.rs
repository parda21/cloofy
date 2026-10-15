use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Widget},
};

use crate::{
    app::{App, Hit},
    theme::{FAINT, MUTED, RAISED, TEXT},
};

const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "NAVIGATION",
        &[
            ("↑ ↓  j k", "Move"),
            ("enter", "Open / play"),
            ("← →  h l", "Sidebar ↔ content"),
            ("tab", "Switch pane"),
            ("g  G", "Top / bottom"),
            ("esc", "Back"),
        ],
    ),
    (
        "PLAYBACK",
        &[
            ("space", "Play / Pause, takes over other device"),
            ("]  [", "Next / previous song"),
            ("⇧←  ⇧→  H L", "Seek 5 seconds"),
            ("+  -", "Volume"),
            ("s", "Shuffle"),
            ("r", "Repeat: off → all → one"),
            ("z", "Now playing with lyrics"),
            ("d", "Switch device"),
        ],
    ),
    ("LIBRARY", &[("f", "Like / save"), ("q", "Add to queue"), ("a", "Add to playlist")]),
    ("GENERAL", &[("^K  :", "Command palette"), ("/", "Search"), ("?", "This help"), ("^C", "Quit")]),
];

pub fn draw(buf: &mut Buffer, app: &mut App, area: Rect) {
    let rows: u16 = SECTIONS.iter().map(|(_, keys)| keys.len() as u16 + 2).sum();
    let popup = area.centered(Constraint::Length(58.min(area.width)), Constraint::Length((rows + 3).min(area.height)));
    Clear.render(popup, buf);
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(FAINT))
        .title(Line::from(" Keyboard shortcuts ").fg(TEXT).bold())
        .style(Style::new().bg(RAISED))
        .render(popup, buf);

    let mut y = popup.y + 2;
    for (title, keys) in SECTIONS {
        buf.set_string(popup.x + 3, y, *title, Style::new().fg(FAINT).bold());
        y += 1;
        for (key, label) in *keys {
            let line = Line::from(vec![
                Span::styled(format!("{key:<14}"), Style::new().fg(TEXT).bold()),
                Span::styled(*label, MUTED),
            ]);
            buf.set_line(popup.x + 3, y, &line, popup.width.saturating_sub(6));
            y += 1;
        }
        y += 1;
    }
    app.hits.push((popup, Hit::OverlayArea));
    if app.overlay_fx_pending {
        app.animate_overlay(popup);
    }
}
