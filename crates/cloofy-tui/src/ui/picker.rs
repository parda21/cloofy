//! Command palette and playlist picker: a floating, filterable list.

use ratatui::{
    Frame,
    layout::{Rect, Size},
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Widget},
};
use unicode_width::UnicodeWidthStr;

use super::text::{self, fit};
use crate::{
    app::{App, Hit},
    overlay::{Overlay, Picker},
    theme::{FAINT, MUTED, RAISED, TEXT},
};

const WIDTH: u16 = 64;
const MAX_ROWS: u16 = 14;
const THUMB: Size = Size { width: 4, height: 2 };

/// One visible entry, copied out of the picker.
struct Row {
    label: String,
    detail: String,
    image_url: Option<String>,
}

pub fn draw(frame: &mut Frame, app: &mut App, screen: Rect) {
    let accent = app.accent.color();
    let selection = app.accent.selection();
    let frame_no = app.frame();
    let Some(overlay) = &mut app.overlay else { return };
    let title = overlay.picker_title().unwrap_or_default();
    // A fresh list loads behind the last one.
    let refreshing = matches!(overlay, Overlay::Devices { refreshing: true, .. });
    let (query, rows) = match overlay {
        Overlay::Palette(picker) => (picker.query.clone(), rows(picker)),
        Overlay::AddToPlaylist { picker, .. } | Overlay::Devices { picker, .. } => (picker.query.clone(), rows(picker)),
        Overlay::Help | Overlay::Menu(_) => return,
    };
    let cursor = match overlay {
        Overlay::Palette(picker) => &mut picker.cursor,
        Overlay::AddToPlaylist { picker, .. } | Overlay::Devices { picker, .. } => &mut picker.cursor,
        Overlay::Help | Overlay::Menu(_) => return,
    };
    // Pickers with covers use two-line rows: name and detail next to a thumbnail.
    let row_height = if rows.iter().any(|r| r.image_url.is_some()) { THUMB.height + 1 } else { 1 };

    let list_height = (rows.len() as u16 * row_height).clamp(1, MAX_ROWS);
    let height = (list_height + 5).min(screen.height);
    let width = WIDTH.min(screen.width.saturating_sub(4));
    let popup = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + screen.height.saturating_sub(height) / 4,
        width,
        height,
    );
    let buf = frame.buffer_mut();
    Clear.render(popup, buf);
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(FAINT))
        .style(Style::new().bg(RAISED))
        .render(popup, buf);

    let inner = Rect::new(popup.x + 2, popup.y + 1, popup.width.saturating_sub(4), popup.height.saturating_sub(2));
    let mut input = vec![Span::styled("›  ", accent)];
    if query.is_empty() {
        input.push(Span::styled(title, FAINT));
    } else {
        input.push(Span::styled(query, Style::new().fg(TEXT).bold()));
    }
    input.push(Span::styled("▏", accent));
    buf.set_line(inner.x, inner.y, &Line::from(input), inner.width);
    if refreshing {
        buf.set_string(inner.right().saturating_sub(1), inner.y, text::spinner(frame_no), Style::new().fg(FAINT));
    }
    buf.set_string(
        popup.x + 1,
        inner.y + 1,
        "─".repeat(popup.width.saturating_sub(2) as usize),
        Style::new().fg(FAINT),
    );

    let list = Rect::new(inner.x, inner.y + 2, inner.width, list_height);
    let mut hits = vec![(popup, Hit::OverlayArea)];
    if rows.is_empty() {
        buf.set_string(list.x, list.y, "Nothing found.", Style::new().fg(MUTED));
    }
    let visible = cursor.visible((list.height / row_height).max(1) as usize, rows.len());
    let selected = cursor.selected;
    let mut thumbs = Vec::new();
    for (slot, position) in visible.enumerate() {
        let y = list.y + slot as u16 * row_height;
        let row = &rows[position];
        let is_selected = position == selected;
        let row_area = Rect::new(popup.x + 1, y, popup.width.saturating_sub(2), row_height.min(THUMB.height));
        if is_selected {
            buf.set_style(row_area, Style::new().bg(selection));
            buf.set_string(popup.x + 1, y, "▌", Style::new().fg(accent));
        }
        let label_style = if is_selected { Style::new().fg(TEXT).bold() } else { Style::new().fg(TEXT) };
        if row_height > 1 {
            let x = list.x + THUMB.width + 2;
            let width = list.right().saturating_sub(x) as usize;
            buf.set_string(x, y, text::truncate(&row.label, width), label_style);
            buf.set_string(x, y + 1, text::truncate(&row.detail, width), Style::new().fg(MUTED));
            thumbs.push((Rect::new(list.x, y, THUMB.width, THUMB.height), row.image_url.clone()));
        } else {
            let detail_width = row.detail.width().min(14);
            let label_width = (list.width as usize).saturating_sub(detail_width + 2);
            let line = Line::from(vec![
                Span::styled(fit(&row.label, label_width), label_style),
                Span::raw("  "),
                Span::styled(text::truncate(&row.detail, detail_width), FAINT),
            ]);
            buf.set_line(list.x, y, &line, list.width);
        }
        hits.push((row_area, Hit::OverlayItem(position)));
    }
    let footer = "↑↓ move   enter select   esc close";
    buf.set_string(inner.x, popup.bottom().saturating_sub(2), footer, Style::new().fg(FAINT));

    for (area, url) in thumbs {
        if !super::cover(frame, app, url.as_deref(), THUMB, false, area) {
            frame.buffer_mut().set_style(area, Style::new().bg(ratatui::style::Color::Rgb(40, 40, 48)));
        }
    }
    app.hits.extend(hits);
    if app.overlay_fx_pending {
        app.animate_overlay(popup);
    }
}

fn rows<T>(picker: &Picker<T>) -> Vec<Row> {
    picker
        .matches
        .iter()
        .map(|&index| {
            let item = &picker.items[index];
            Row { label: item.label.clone(), detail: item.detail.clone(), image_url: item.image_url.clone() }
        })
        .collect()
}
