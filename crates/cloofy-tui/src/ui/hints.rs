//! The bottom line: context hints (clickable), status messages and the playing device.

use crossterm::event::KeyCode;
use ratatui::{buffer::Buffer, layout::Rect, style::Style, text::Span};
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{App, Focus, HintAction, Hit},
    commands::Command,
    icons::icons,
    theme::{DANGER, FAINT, MUTED},
    views::View,
};

type Hint = (&'static str, &'static str, Option<HintAction>);

pub fn draw(buf: &mut Buffer, app: &mut App, area: Rect) {
    let device = match (app.transferring_to(), &app.now.device, app.is_connected()) {
        (Some(name), ..) => {
            Span::styled(format!("{}  → {name} …", crate::ui::text::spinner(app.frame())), app.accent.color())
        }
        (None, Some(name), _) => Span::styled(format!("{}  Playing on {name}", icons().speaker), app.accent.color()),
        (None, None, true) => Span::styled(format!("{}  cloofy", icons().computer), app.accent.color()),
        (None, None, false) => Span::styled(format!("{}  cloofy", icons().computer), FAINT),
    };
    let device_width = device.width() as u16;
    let device_x = area.right().saturating_sub(device_width + 2);
    buf.set_span(device_x, area.y, &device, device_width);
    app.hits.push((Rect::new(device_x, area.y, device_width, 1), Hit::Command(Command::Devices)));
    let width = area.width.saturating_sub(device_width + 6);

    if let Some(status) = &app.status {
        let color = if status.error { DANGER } else { app.accent.color() };
        let text = crate::ui::text::truncate(&status.text, width as usize);
        buf.set_string(area.x + 2, area.y, text, Style::new().fg(color));
        return;
    }

    let mut x = area.x + 2;
    for (key, label, action) in hints(app) {
        let len = (key.width() + 1 + label.width()) as u16;
        if x + len > area.x + width {
            break;
        }
        buf.set_string(x, area.y, *key, Style::new().fg(MUTED).bold());
        buf.set_string(x + key.width() as u16 + 1, area.y, *label, Style::new().fg(FAINT));
        if let Some(action) = action {
            app.hits.push((Rect::new(x, area.y, len, 1), Hit::Hint(*action)));
        }
        x += len + 3;
    }
}

fn hints(app: &App) -> &'static [Hint] {
    use Command::*;
    use HintAction::{Activate, Back, Key, Run, ToggleFocus};
    match (app.focus, &app.view) {
        _ if app.zen => &[
            ("z", "close", Some(Back)),
            ("space", "play/pause", Some(Run(PlayPause))),
            ("]", "next", Some(Run(Next))),
            ("[", "previous", Some(Run(Previous))),
            ("f", "like", Some(Run(Like))),
            ("a", "add to playlist", Some(Run(AddToPlaylist))),
            ("^K", "commands", Some(Run(Palette))),
        ],
        // Enter and esc mean something else while typing; a click does the same.
        (Focus::SearchInput, _) => {
            &[("enter", "to results", Some(Key(KeyCode::Enter))), ("esc", "back", Some(Key(KeyCode::Esc)))]
        }
        (Focus::Nav, _) => &[
            ("enter", "open", Some(Activate)),
            ("← →", "filter", Some(Key(KeyCode::Right))),
            ("tab", "to content", Some(ToggleFocus)),
            (".", "actions", Some(Run(Menu))),
            ("/", "search", Some(Run(Search))),
            ("^K", "commands", Some(Run(Palette))),
            ("?", "help", Some(Run(Help))),
        ],
        (Focus::Content, View::Artist(_)) => &[
            ("enter", "play/open", Some(Activate)),
            ("F", "follow", Some(Key(KeyCode::Char('F')))),
            (".", "actions", Some(Run(Menu))),
            ("esc", "back", Some(Back)),
            ("^K", "commands", Some(Run(Palette))),
            ("?", "help", Some(Run(Help))),
        ],
        (Focus::Content, View::Library(_)) => &[
            ("enter", "open", Some(Activate)),
            ("< >", "filter", Some(Key(KeyCode::Char('>')))),
            (".", "actions", Some(Run(Menu))),
            ("esc", "back", Some(Back)),
            ("^K", "commands", Some(Run(Palette))),
            ("?", "help", Some(Run(Help))),
        ],
        (Focus::Content, View::Home(_)) => &[
            ("enter", "open", Some(Activate)),
            ("p", "play", Some(Key(KeyCode::Char('p')))),
            (".", "actions", Some(Run(Menu))),
            ("D", "DJ", Some(Run(Dj))),
            ("/", "search", Some(Run(Search))),
            ("^K", "commands", Some(Run(Palette))),
            ("?", "help", Some(Run(Help))),
        ],
        (Focus::Content, View::Search(_)) => &[
            ("enter", "play/open", Some(Activate)),
            (".", "actions", Some(Run(Menu))),
            ("f", "like", Some(Run(Like))),
            ("q", "queue", Some(Run(Queue))),
            ("/", "new search", Some(Run(Search))),
            ("esc", "back", Some(Back)),
            ("^K", "commands", Some(Run(Palette))),
        ],
        (Focus::Content, View::Settings(_)) => &[
            ("↑ ↓", "choose", None),
            ("← →", "change", Some(Key(KeyCode::Right))),
            ("esc", "back", Some(Back)),
            ("^K", "commands", Some(Run(Palette))),
            ("?", "help", Some(Run(Help))),
        ],
        (Focus::Content, View::Tracks(_)) => &[
            ("enter", "play", Some(Activate)),
            (".", "actions", Some(Run(Menu))),
            ("f", "like", Some(Run(Like))),
            ("q", "queue", Some(Run(Queue))),
            ("a", "add to playlist", Some(Run(AddToPlaylist))),
            ("z", "now playing", Some(Run(NowPlaying))),
            ("esc", "back", Some(Back)),
            ("^K", "commands", Some(Run(Palette))),
            ("?", "help", Some(Run(Help))),
        ],
    }
}
