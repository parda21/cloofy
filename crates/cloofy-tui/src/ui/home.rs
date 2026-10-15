//! Home: a greeting, the shortcut grid and shelves of cover cards.

use cloofy_spotify::home::{HomeItem, ItemKind, Shelf};
use ratatui::{
    Frame,
    layout::{Rect, Size},
    style::{Color, Style},
    text::{Line, Span},
};

use super::{text, wordmark};
use crate::{
    app::{App, Focus, Hit},
    theme::{DANGER, FAINT, MUTED, RAISED, TEXT},
    views::View,
};

/// Card cover in cells; cells are about twice as tall as wide, so this is roughly square.
const COVER: Size = Size { width: 14, height: 7 };
const CARD_GAP: u16 = 3;
/// Title, gap, cover, two text lines and spacing below.
pub(super) const SHELF_ROWS: u16 = 1 + 1 + 7 + 2 + 2;
const TILE_COVER: Size = Size { width: 6, height: 3 };
const TILE_GAP: u16 = 2;
const UNFOCUSED_SELECTION: Color = Color::Rgb(25, 25, 31);

/// What one shelf needs for drawing, copied out of the view.
pub(super) struct ShelfLayout {
    /// Row reported in click targets.
    pub index: usize,
    /// Screen row of the title; may lie above the visible area while scrolling.
    pub y: i32,
    pub title: String,
    pub shortcuts: bool,
    pub total: usize,
    /// Index of the first visible card.
    pub first: usize,
    pub items: Vec<HomeItem>,
    /// Covers of the pages before and after the visible one, loaded ahead.
    pub nearby: Vec<(Option<String>, bool)>,
}

/// How many cards fit next to each other.
pub(super) fn visible_cards(width: u16) -> usize {
    ((width + CARD_GAP) / (COVER.width + CARD_GAP)).max(1) as usize
}

/// Cover URLs, and whether they are round, of the pages left and right of the `cards` items
/// from `first`. Loading them ahead lets paging show the next cards at once.
pub(super) fn nearby(items: &[HomeItem], first: usize, cards: usize) -> Vec<(Option<String>, bool)> {
    let before = first.saturating_sub(cards)..first;
    let after = (first + cards).min(items.len())..(first + 2 * cards).min(items.len());
    before.chain(after).map(|index| cover_of(&items[index])).collect()
}

/// An item's cover URL and whether it is drawn round.
fn cover_of(item: &HomeItem) -> (Option<String>, bool) {
    (item.image_url.clone(), item.kind == ItemKind::Artist)
}

/// Scrolls a shelf horizontally so the selected card stays visible.
pub(super) fn follow(offset: &mut usize, item: usize, cards: usize) {
    if item < *offset {
        *offset = item;
    } else if item >= *offset + cards {
        *offset = item + 1 - cards;
    }
}

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    app.content_area = area;
    app.hits.push((area, Hit::ContentArea));
    let inner = Rect {
        x: area.x + 3,
        y: area.y + 1,
        width: area.width.saturating_sub(6),
        height: area.height.saturating_sub(1),
    };
    if inner.width < 20 || inner.height < 3 {
        return;
    }
    let focused = app.focus == Focus::Content;
    let frame_no = app.frame();
    let View::Home(view) = &mut app.view else { return };

    let Some(home) = &view.home else {
        let message = match &view.error {
            Some(err) => Span::styled(format!("✕  {err}"), DANGER),
            None => Span::styled(format!("{}  Loading Home…", text::spinner(frame_no)), MUTED),
        };
        splash(frame, app, inner, message);
        return;
    };

    // Everything scrolls together, greeting included; positions are rows from the top.
    let heights: Vec<u16> = home.shelves.iter().map(|s| shelf_height(s, inner.width)).collect();
    let mut starts = Vec::with_capacity(heights.len());
    let mut total: u16 = 2;
    for height in &heights {
        starts.push(total);
        total += height;
    }
    let viewport = inner.height;
    if !view.detached && !heights.is_empty() {
        let shelf = view.shelf.min(heights.len() - 1);
        let top = if shelf == 0 { 0 } else { starts[shelf] };
        let bottom = starts[shelf] + heights[shelf].saturating_sub(2);
        if top < view.scroll {
            view.scroll = top;
        } else if bottom > view.scroll + viewport {
            view.scroll = bottom - viewport;
        }
    }
    view.scroll = view.scroll.min(total.saturating_sub(viewport));
    let scroll = view.scroll;

    if scroll == 0 {
        let greeting = if home.greeting.is_empty() { "Hello".to_owned() } else { home.greeting.clone() };
        let mut spans = vec![Span::styled(greeting, Style::new().fg(TEXT).bold())];
        if view.loading {
            spans.push(Span::styled(format!("   {}", text::spinner(frame_no)), FAINT));
        }
        frame.buffer_mut().set_line(inner.x, inner.y, &Line::from(spans), inner.width);
    }

    view.offsets.resize(home.shelves.len(), 0);
    let cards = visible_cards(inner.width);
    view.cards = cards;
    let mut layouts = Vec::new();
    // The first page of the shelf below the screen, so scrolling down finds it loaded.
    let mut below = Vec::new();
    for (index, shelf) in home.shelves.iter().enumerate() {
        let y = inner.y as i32 + starts[index] as i32 - scroll as i32;
        if y + heights[index] as i32 <= inner.y as i32 {
            continue;
        }
        if y >= inner.bottom() as i32 {
            if !shelf.shortcuts {
                let first = view.offsets[index];
                below.extend(shelf.items.iter().skip(first).take(cards).map(cover_of));
            }
            break;
        }
        let first = if shelf.shortcuts {
            0
        } else {
            let offset = &mut view.offsets[index];
            if index == view.shelf {
                follow(offset, view.item, cards);
            }
            *offset = (*offset).min(shelf.items.len().saturating_sub(cards));
            *offset
        };
        let count = if shelf.shortcuts { shelf.items.len() } else { cards };
        layouts.push(ShelfLayout {
            index,
            y,
            title: shelf.title.clone(),
            shortcuts: shelf.shortcuts,
            total: shelf.items.len(),
            first,
            items: shelf.items.iter().skip(first).take(count).cloned().collect(),
            nearby: if shelf.shortcuts { Vec::new() } else { nearby(&shelf.items, first, cards) },
        });
    }
    let selected = (view.shelf, view.item);
    for (url, round) in below {
        app.prefetch(url.as_deref(), COVER, round);
    }

    for layout in layouts {
        let selected_item = (selected.0 == layout.index).then_some(selected.1);
        if layout.shortcuts {
            draw_shortcuts(frame, app, inner, &layout, selected_item, focused);
        } else {
            draw_shelf(frame, app, inner, &layout, selected_item, focused);
        }
    }
    super::scrollbar(
        frame.buffer_mut(),
        inner.right() + 1,
        inner.y,
        inner.height,
        total as usize,
        scroll as usize,
        viewport as usize,
    );
}

/// The wordmark with a status line, while Home loads for the first time.
fn splash(frame: &mut Frame, app: &mut App, area: Rect, message: Span<'static>) {
    let top = area.y + area.height.saturating_sub(wordmark::HEIGHT + 3) / 2;
    let buf = frame.buffer_mut();
    wordmark::draw(buf, area.x + area.width.saturating_sub(wordmark::WIDTH) / 2, top, app.accent.rgb());
    let width = message.width() as u16;
    buf.set_span(area.x + area.width.saturating_sub(width) / 2, top + wordmark::HEIGHT + 2, &message, width);
}

fn shelf_height(shelf: &Shelf, width: u16) -> u16 {
    if shelf.shortcuts {
        let columns = shortcut_columns(width);
        let rows = shelf.items.len().div_ceil(columns as usize) as u16;
        rows * (TILE_COVER.height + 1) + 1
    } else {
        SHELF_ROWS
    }
}

fn shortcut_columns(width: u16) -> u16 {
    if width >= 110 { 4 } else { 2 }
}

/// The part of rows `y..y + height` that lies inside `area`, across the given columns.
pub(super) fn clip(area: Rect, x: u16, width: u16, y: i32, height: u16) -> Option<Rect> {
    let top = y.max(area.y as i32);
    let bottom = (y + height as i32).min(area.bottom() as i32);
    (top < bottom).then(|| Rect::new(x, top as u16, width, (bottom - top) as u16).intersection(area))
}

/// `y` as a screen row, if it is inside `area`.
pub(super) fn row(area: Rect, y: i32) -> Option<u16> {
    (y >= area.y as i32 && y < area.bottom() as i32).then_some(y as u16)
}

/// A titled row of cover cards; `selected` is the selected card if this shelf has the cursor.
/// Shelves cut off at the top or bottom draw what is visible.
pub(super) fn draw_shelf(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    layout: &ShelfLayout,
    selected: Option<usize>,
    focused: bool,
) {
    let accent = app.accent.color();
    let selection = if focused { app.accent.selection() } else { UNFOCUSED_SELECTION };
    for (url, round) in &layout.nearby {
        app.prefetch(url.as_deref(), COVER, *round);
    }
    if let Some(y) = row(area, layout.y) {
        let pageable = layout.total > layout.items.len();
        let title_width = area.width.saturating_sub(if pageable { 28 } else { 2 }) as usize;
        let buf = frame.buffer_mut();
        buf.set_string(area.x, y, text::truncate(&layout.title, title_width), Style::new().fg(TEXT).bold());
        if pageable {
            let last = layout.first + layout.items.len();
            let pager = format!("{}–{} of {}", layout.first + 1, last, layout.total);
            let style = if selected.is_some() { Style::new().fg(MUTED) } else { Style::new().fg(FAINT) };
            let next_x = area.right().saturating_sub(2);
            let prev_x = next_x.saturating_sub(4);
            buf.set_string(prev_x.saturating_sub(pager.chars().count() as u16 + 3), y, pager, style);
            let icons = crate::icons::icons();
            let arrows = [
                (prev_x, icons.chevron_left, false, layout.first > 0),
                (next_x, icons.chevron_right, true, last < layout.total),
            ];
            for (x, icon, forward, enabled) in arrows {
                let arrow = Rect::new(x.saturating_sub(1), y, 3, 1);
                let color = if enabled { TEXT } else { Color::Rgb(52, 52, 60) };
                buf.set_style(arrow, Style::new().bg(RAISED));
                buf.set_string(x, y, icon, Style::new().fg(color));
                if enabled {
                    app.hits.push((arrow, Hit::ShelfScroll { shelf: layout.index, forward }));
                }
            }
        }
    }

    let cover_y = layout.y + 2;
    for (column, item) in layout.items.iter().enumerate() {
        let x = area.x + column as u16 * (COVER.width + CARD_GAP);
        let index = layout.first + column;
        let is_selected = selected == Some(index);
        let card = clip(area, x.saturating_sub(1), COVER.width + 2, cover_y - 1, COVER.height + 4);
        if is_selected && let Some(card) = card {
            frame.buffer_mut().set_style(card, Style::new().bg(selection));
        }
        draw_cover(frame, app, (x, cover_y), area, item, COVER);
        let buf = frame.buffer_mut();
        let title_style = if is_selected { Style::new().fg(accent).bold() } else { Style::new().fg(TEXT) };
        let width = COVER.width as usize;
        let text_y = cover_y + COVER.height as i32;
        if let Some(y) = row(area, text_y) {
            buf.set_string(x, y, text::truncate(&item.title, width), title_style);
        }
        if let Some(y) = row(area, text_y + 1) {
            buf.set_string(x, y, text::truncate(&item.subtitle, width), Style::new().fg(MUTED));
        }
        if let Some(card) = card {
            app.hits.push((card, Hit::Card { shelf: layout.index, item: index }));
            let button_y = cover_y + COVER.height as i32 - 1;
            if super::hovered(app, card)
                && let Some(y) = row(area, button_y)
            {
                let hit = Hit::CardPlay { shelf: layout.index, item: index };
                super::play_button(frame.buffer_mut(), app, x + COVER.width, y, hit, true);
            }
        }
    }
}

fn draw_shortcuts(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    layout: &ShelfLayout,
    selected: Option<usize>,
    focused: bool,
) {
    let accent = app.accent.color();
    let selection = if focused { app.accent.selection() } else { UNFOCUSED_SELECTION };
    let columns = shortcut_columns(area.width);
    let tile_width = (area.width - TILE_GAP * (columns - 1)) / columns;
    for (index, item) in layout.items.iter().enumerate() {
        let column = index as u16 % columns;
        let tile_y = layout.y + (index as u16 / columns * (TILE_COVER.height + 1)) as i32;
        let x = area.x + column * (tile_width + TILE_GAP);
        let Some(tile) = clip(area, x, tile_width, tile_y, TILE_COVER.height) else { continue };
        let is_selected = selected == Some(index);
        frame.buffer_mut().set_style(tile, Style::new().bg(if is_selected { selection } else { RAISED }));
        draw_cover(frame, app, (x, tile_y), area, item, TILE_COVER);
        let style = if is_selected { Style::new().fg(accent).bold() } else { Style::new().fg(TEXT).bold() };
        let hovered = super::hovered(app, tile);
        // A hovered tile shows its play button at the right end; the name stops short of it.
        let reserved = TILE_COVER.width + 3 + if hovered { super::PLAY_BUTTON_WIDTH + 1 } else { 0 };
        let name = text::truncate(&item.title, tile_width.saturating_sub(reserved) as usize);
        // Vertically centered next to the cover.
        if let Some(y) = row(area, tile_y + (TILE_COVER.height / 2) as i32) {
            frame.buffer_mut().set_string(x + TILE_COVER.width + 2, y, name, style);
        }
        app.hits.push((tile, Hit::Card { shelf: layout.index, item: index }));
        if hovered && let Some(y) = row(area, tile_y + (TILE_COVER.height / 2) as i32) {
            let hit = Hit::CardPlay { shelf: layout.index, item: index };
            super::play_button(frame.buffer_mut(), app, x + tile_width - 1, y, hit, false);
        }
    }
}

/// A card's cover at `(x, y)`, cut to `clip`; an icon on a plain square while it loads.
pub(super) fn draw_cover(
    frame: &mut Frame,
    app: &mut App,
    (x, y): (u16, i32),
    clip: Rect,
    item: &HomeItem,
    size: Size,
) {
    let round = item.kind == ItemKind::Artist;
    if super::cover_in(frame, app, item.image_url.as_deref(), size, round, (x, y), size, clip) {
        return;
    }
    let Some(part) = self::clip(clip, x, size.width, y, size.height) else { return };
    let buf = frame.buffer_mut();
    buf.set_style(part, Style::new().bg(RAISED));
    let icons = crate::icons::icons();
    let icon = match item.kind {
        ItemKind::Playlist => icons.playlist,
        ItemKind::Album => icons.album,
        ItemKind::Artist => icons.artist,
        ItemKind::Audiobook => icons.audiobook,
        ItemKind::Podcast | ItemKind::Episode => icons.podcast,
    };
    if let Some(row) = row(clip, y + (size.height / 2) as i32) {
        buf.set_string(x + size.width / 2, row, icon, Style::new().fg(FAINT));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(count: usize) -> Vec<HomeItem> {
        (0..count)
            .map(|i| HomeItem {
                uri: format!("spotify:playlist:{i}"),
                kind: ItemKind::Playlist,
                title: String::new(),
                subtitle: String::new(),
                image_url: Some(i.to_string()),
                owner_id: String::new(),
                owner_name: String::new(),
                context_uri: String::new(),
            })
            .collect()
    }

    fn urls(nearby: Vec<(Option<String>, bool)>) -> Vec<String> {
        nearby.into_iter().filter_map(|(url, _)| url).collect()
    }

    #[test]
    fn nearby_covers_are_the_pages_on_either_side() {
        assert_eq!(urls(nearby(&items(20), 6, 3)), ["3", "4", "5", "9", "10", "11"]);
    }

    #[test]
    fn nearby_covers_stop_at_the_ends_of_the_shelf() {
        assert_eq!(urls(nearby(&items(5), 0, 3)), ["3", "4"]);
        assert_eq!(urls(nearby(&items(5), 2, 3)), ["0", "1"]);
        assert!(nearby(&items(2), 0, 3).is_empty());
    }
}
