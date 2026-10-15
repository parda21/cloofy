//! "Your Library" as a full page: filter tabs and a grid of covers.

use ratatui::{
    Frame,
    layout::{Rect, Size},
    style::{Color, Style},
};

use super::{home, sidebar, text};
use crate::{
    app::{App, Focus, Hit, LibraryFilter},
    theme::{MUTED, RAISED, TEXT},
    views::View,
};

const COVER: Size = Size { width: 14, height: 7 };
const GAP: u16 = 3;
/// Cover, title, subtitle and spacing.
const CARD_ROWS: u16 = COVER.height + 2 + 2;
/// Title, gap and tabs above the grid.
const HEADER_ROWS: u16 = 5;
const UNFOCUSED_SELECTION: Color = Color::Rgb(25, 25, 31);

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    app.content_area = area;
    app.hits.push((area, Hit::ContentArea));
    let inner = Rect::new(area.x + 3, area.y + 1, area.width.saturating_sub(6), area.height.saturating_sub(1));
    if inner.width < 16 || inner.height < 3 {
        return;
    }
    let accent = app.accent.color();
    let selection = if app.focus == Focus::Content { app.accent.selection() } else { UNFOCUSED_SELECTION };
    let View::Library(view) = &app.view else { return };
    let filter = view.filter;

    let buf = frame.buffer_mut();
    buf.set_string(inner.x, inner.y, "Your Library", Style::new().fg(TEXT).bold());
    let filters: Vec<_> = std::iter::once(LibraryFilter::All).chain(app.library_filters()).collect();
    let tabs = sidebar::tabs(buf, inner.x, inner.y + 2, inner.right(), &filters, filter, accent);
    app.hits.extend(tabs.into_iter().map(|(rect, filter)| (rect, Hit::PageChip(filter))));

    let entries = app.library_entries_for(filter);
    let grid = Rect::new(inner.x, inner.y + HEADER_ROWS, inner.width, inner.height.saturating_sub(HEADER_ROWS));
    let loading = app.playlists_loading || app.library_loading;
    if entries.is_empty() {
        let message = if loading {
            format!("{}  Loading…", text::spinner(app.frame()))
        } else {
            "Nothing here yet.".to_owned()
        };
        frame.buffer_mut().set_string(grid.x, grid.y, message, Style::new().fg(MUTED));
        return;
    }
    // Too little room for covers: one entry per row instead.
    let list_mode = grid.height < CARD_ROWS || grid.width < COVER.width;
    let (columns, row_height) =
        if list_mode { (1, 1) } else { (((grid.width + GAP) / (COVER.width + GAP)).max(1) as usize, CARD_ROWS) };
    let (cursor, mut top, detached) = match &app.view {
        View::Library(v) => (v.cursor.min(entries.len() - 1), v.top, v.detached),
        _ => return,
    };
    // Scrolling is by screen rows, so cards slide in and out of view.
    let row_height = row_height as usize;
    let viewport = grid.height as usize;
    let total = entries.len().div_ceil(columns) * row_height;
    if !detached {
        let card_top = cursor / columns * row_height;
        let card_bottom = card_top + row_height - if list_mode { 0 } else { 2 };
        if card_top < top {
            top = card_top;
        } else if card_bottom > top + viewport {
            top = card_bottom - viewport;
        }
    }
    top = top.min(total.saturating_sub(viewport));
    super::scrollbar(frame.buffer_mut(), grid.right() + 1, grid.y, grid.height, total, top, viewport);
    if let View::Library(v) = &mut app.view {
        v.columns = columns;
        v.top = top;
        // The library arrives in parts; a restored cursor may point at one still on its way.
        if !loading {
            v.cursor = cursor;
        }
    }

    let first = top / row_height * columns;
    let last = ((top + viewport).div_ceil(row_height) * columns).min(entries.len());
    let row_y = |index: usize| grid.y as i32 + (index / columns * row_height) as i32 - top as i32;
    if list_mode {
        for (index, &entry) in entries.iter().enumerate().take(last).skip(first) {
            let Some(y) = home::row(grid, row_y(index)) else { continue };
            let (title, subtitle, _, _) = sidebar::describe(app, entry);
            let row = Rect::new(grid.x, y, grid.width, 1);
            let selected = index == cursor;
            let buf = frame.buffer_mut();
            if selected {
                buf.set_style(row, Style::new().bg(selection));
            }
            let title_style = if selected { Style::new().fg(accent).bold() } else { Style::new().fg(TEXT) };
            let width = grid.width as usize;
            buf.set_string(row.x, y, text::truncate(&title, width / 2), title_style);
            buf.set_string(
                row.x + (width / 2) as u16 + 2,
                y,
                text::truncate(&subtitle, width / 2 - 2),
                Style::new().fg(MUTED),
            );
            app.hits.push((row, Hit::Row(index)));
        }
        return;
    }
    // The next screenful of covers, so scrolling down finds them loaded.
    for &entry in entries.iter().take(last + (last - first)).skip(last) {
        let (_, _, image_url, round) = sidebar::describe(app, entry);
        app.prefetch(image_url.as_deref(), COVER, round);
    }
    // The selection reaches one row above the cards, into the gap below the tabs.
    let clip = Rect { y: grid.y - 1, height: grid.height + 1, ..grid };
    for (index, &entry) in entries.iter().enumerate().take(last).skip(first) {
        let x = grid.x + (index % columns) as u16 * (COVER.width + GAP);
        let y = row_y(index);
        let card = home::clip(clip, x.saturating_sub(1), COVER.width + 2, y - 1, COVER.height + 4);
        let selected = index == cursor;
        if selected && let Some(card) = card {
            frame.buffer_mut().set_style(card, Style::new().bg(selection));
        }
        let (title, subtitle, image_url, round) = sidebar::describe(app, entry);
        if !super::cover_in(frame, app, image_url.as_deref(), COVER, round, (x, y), COVER, grid)
            && let Some(part) = home::clip(grid, x, COVER.width, y, COVER.height)
        {
            frame.buffer_mut().set_style(part, Style::new().bg(RAISED));
        }
        let title_style = if selected { Style::new().fg(accent).bold() } else { Style::new().fg(TEXT) };
        let buf = frame.buffer_mut();
        let text_y = y + COVER.height as i32;
        if let Some(row) = home::row(grid, text_y) {
            buf.set_string(x, row, text::truncate(&title, COVER.width as usize), title_style);
        }
        if let Some(row) = home::row(grid, text_y + 1) {
            buf.set_string(x, row, text::truncate(&subtitle, COVER.width as usize), Style::new().fg(MUTED));
        }
        if let Some(card) = card {
            app.hits.push((card, Hit::Row(index)));
            if super::hovered(app, card)
                && let Some(row) = home::row(grid, y + COVER.height as i32 - 1)
            {
                super::play_button(frame.buffer_mut(), app, x + COVER.width, row, Hit::RowPlay(index), true);
            }
        }
    }
}
