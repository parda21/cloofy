//! The Settings page: every setting with a line on what it does, and its value, changed in
//! place with the arrow keys or a click.

use std::borrow::Cow;

use ratatui::{Frame, buffer::Buffer, layout::Rect, style::Style};
use unicode_width::UnicodeWidthStr;

use super::{content::UNFOCUSED_SELECTION, text};
use crate::{
    app::{App, Focus, Hit},
    graphics::ImageProtocol,
    preferences::{SECTIONS, Setting},
    theme,
    views::View,
};

/// Room for the longest choice, so the arrows around it stay put while it changes.
const CHOICE_WIDTH: usize = 10;

/// What one screen row of the page shows.
enum Row {
    Title,
    Section(&'static str),
    /// The name and value of the `n`th setting.
    Name(usize, Setting),
    /// The line under it that says what it does.
    About(Setting),
    Blank,
    Footer,
}

/// The page's rows, with the settings `shown`; a section without any is left out.
fn rows(shown: &[Setting]) -> Vec<Row> {
    let mut rows = vec![Row::Title, Row::Blank];
    let mut index = 0;
    for (title, settings) in SECTIONS {
        let settings: Vec<Setting> = settings.iter().copied().filter(|setting| shown.contains(setting)).collect();
        if settings.is_empty() {
            continue;
        }
        rows.extend([Row::Section(title), Row::Blank]);
        for setting in settings {
            rows.extend([Row::Name(index, setting), Row::About(setting), Row::Blank]);
            index += 1;
        }
        rows.push(Row::Blank);
    }
    rows.push(Row::Footer);
    rows
}

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let p = theme::palette();
    app.content_area = area;
    let inner = Rect {
        x: area.x + 3,
        y: area.y + 1,
        width: area.width.saturating_sub(6),
        height: area.height.saturating_sub(1),
    };
    if inner.width < 24 || inner.height < 3 {
        return;
    }
    app.hits.push((area, Hit::ContentArea));
    let View::Settings(view) = &mut app.view else { return };

    let rows = rows(&view.settings);
    let height = inner.height as usize;
    // Follow the selection, with its section title in view, unless the wheel moved the page.
    if !view.detached {
        let name = rows.iter().position(|row| matches!(row, Row::Name(i, _) if *i == view.selected)).unwrap_or(0);
        let first_shown = if view.selected == 0 { 0 } else { name.saturating_sub(3) };
        if first_shown < view.top {
            view.top = first_shown;
        }
        if name + 2 > view.top + height {
            view.top = name + 2 - height;
        }
    }
    view.top = view.top.min(rows.len().saturating_sub(height));
    let (top, selected) = (view.top, view.selected);

    let buf = frame.buffer_mut();
    let focused = app.focus == Focus::Content;
    let accent = app.accent.color();
    let selection = if focused { app.accent.selection() } else { UNFOCUSED_SELECTION };
    for (offset, row) in rows.iter().skip(top).take(height).enumerate() {
        let y = inner.y + offset as u16;
        match *row {
            Row::Title => {
                buf.set_string(inner.x, y, "Settings", Style::new().fg(p.text).bold());
            }
            Row::Section(title) => {
                buf.set_string(inner.x, y, title, Style::new().fg(p.text).bold());
            }
            Row::Name(index, setting) => {
                // The band covers the name and the line under it, like a two-row list entry.
                let band = super::content::row_band(inner, y, 2).intersection(area);
                if index == selected {
                    buf.set_style(band, Style::new().bg(selection));
                    if focused {
                        for row in band.top()..band.bottom() {
                            buf.set_string(inner.x.saturating_sub(2), row, "▌", Style::new().fg(accent));
                        }
                    }
                }
                app.hits.push((band, Hit::Row(index)));
                let value_x = value(buf, app, index, setting, inner.right(), y);
                let width = value_x.saturating_sub(inner.x + 2) as usize;
                buf.set_string(inner.x, y, text::truncate(setting.label(), width), Style::new().fg(p.text));
            }
            Row::About(setting) => {
                let about = about(app, setting);
                buf.set_string(inner.x, y, text::truncate(&about, inner.width as usize), Style::new().fg(p.muted));
            }
            Row::Blank => {}
            Row::Footer => {
                let saved = if app.is_demo() {
                    "The demo keeps changes until it closes".to_owned()
                } else {
                    format!("Saved in {}", app.config_path())
                };
                let footer = format!("cloofy {} · {saved}", env!("CARGO_PKG_VERSION"));
                buf.set_string(inner.x, y, text::truncate(&footer, inner.width as usize), Style::new().fg(p.faint));
            }
        }
    }
}

/// What the setting does, or why it can't do anything here.
fn about(app: &App, setting: Setting) -> Cow<'static, str> {
    match setting {
        Setting::Notifications if !app.can_notify() && !app.is_demo() => {
            "This terminal can't show notifications; see the README for which ones can.".into()
        }
        Setting::Images if app.prefs.images == ImageProtocol::Auto => {
            format!("How covers reach the terminal. Auto picked {} here.", app.auto_image_protocol().label()).into()
        }
        _ => setting.description().into(),
    }
}

/// Draws the value of `setting` ending at column `right`, with its click targets. Returns
/// where it starts.
fn value(buf: &mut Buffer, app: &mut App, index: usize, setting: Setting, right: u16, y: u16) -> u16 {
    let p = theme::palette();
    let accent = app.accent.color();
    let shown = app.prefs.value(setting);
    if let Some(on) = app.prefs.switch(setting) {
        // A pill like the page buttons: filled with the accent while on.
        let label = format!(" {shown} ");
        let width = label.width() as u16;
        let x = right.saturating_sub(width);
        let style = if on { Style::new().bg(accent).fg(p.bg).bold() } else { Style::new().bg(p.raised).fg(p.muted) };
        buf.set_string(x, y, &label, style);
        app.hits.push((Rect::new(x, y, width, 1), Hit::SettingStep { index, forward: true }));
        return x;
    }
    let choice = format!("{shown:^CHOICE_WIDTH$}");
    let width = choice.width() as u16 + 6;
    let x = right.saturating_sub(width);
    let pill = Style::new().bg(p.raised);
    buf.set_string(x, y, " ‹ ", pill.fg(p.muted));
    buf.set_string(x + 3, y, &choice, pill.fg(p.text));
    buf.set_string(x + width - 3, y, " › ", pill.fg(p.muted));
    app.hits.push((Rect::new(x, y, 3, 1), Hit::SettingStep { index, forward: false }));
    app.hits.push((Rect::new(x + 3, y, width - 3, 1), Hit::SettingStep { index, forward: true }));
    x
}
