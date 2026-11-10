//! Home: a greeting or chips, the shortcut grid, and shelves of cover cards or of songs.

use cloofy_core::home::{Chip, HomeItem, ItemKind, Shelf};
use ratatui::{
    Frame,
    layout::{Rect, Size},
    style::{Color, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use super::{text, wordmark};
use crate::{
    app::{App, Focus, Hit},
    theme,
    views::{self, SONG_ROWS, View},
};

/// Card cover in cells; cells are about twice as tall as wide, so this is roughly square.
const COVER: Size = Size { width: 14, height: 7 };
const CARD_GAP: u16 = 3;
/// Title, gap, cover, two text lines and spacing below.
pub(super) const SHELF_ROWS: u16 = 1 + 1 + 7 + 2 + 2;
const TILE_COVER: Size = Size { width: 6, height: 3 };
const TILE_GAP: u16 = 2;
/// A song's cover in a shelf of songs, as in the sidebar.
const SONG_COVER: Size = Size { width: 4, height: 2 };
/// Narrowest column of songs, and the room between columns.
const SONG_WIDTH: u16 = 30;
const SONG_GAP: u16 = 3;
/// Title, gap, the songs two rows each with a row between them, and spacing below.
const SONGS_ROWS: u16 = 1 + 1 + SONG_ROWS as u16 * 3 - 1 + 2;
/// The Play all button in a shelf's title row.
const PLAY_ALL: &str = "Play all";

/// What one shelf needs for drawing, copied out of the view.
pub(super) struct ShelfLayout {
    /// Row reported in click targets.
    pub index: usize,
    /// Screen row of the title; may lie above the visible area while scrolling.
    pub y: i32,
    pub title: String,
    pub strapline: String,
    pub shortcuts: bool,
    pub songs: bool,
    pub play_all: bool,
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

/// How many columns of songs fit next to each other.
fn song_columns(width: u16) -> usize {
    ((width + SONG_GAP) / (SONG_WIDTH + SONG_GAP)).max(1) as usize
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

/// Left column of the card in slot `column` of a row of `slots` cards across `width`. The
/// room left over widens the gaps, so a full row ends at the right edge like the shortcut
/// tiles and the pager above it. Short rows keep the same spacing and stay left.
pub(super) fn card_x(x: u16, width: u16, slots: usize, column: usize) -> u16 {
    let (slots, column) = (slots as u16, column as u16);
    if slots < 2 {
        return x + column * (COVER.width + CARD_GAP);
    }
    let spare = width.saturating_sub(slots * COVER.width);
    let (gap, rest) = (spare / (slots - 1), spare % (slots - 1));
    // The first gaps take one column more each until the rest is used up.
    x + column * (COVER.width + gap) + column.min(rest)
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
    let p = theme::palette();
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
            Some(err) => Span::styled(format!("✕  {err}"), p.danger),
            None => Span::styled(format!("{}  Loading Home…", text::spinner(frame_no)), p.muted),
        };
        splash(frame, app, inner, message);
        return;
    };

    // Everything scrolls together, greeting or chips included; positions are rows from the top.
    let heights: Vec<u16> = home.shelves.iter().map(|s| shelf_height(s, inner.width)).collect();
    let mut starts = Vec::with_capacity(heights.len());
    let mut total: u16 = 2;
    for height in &heights {
        starts.push(total);
        total += height;
    }
    let viewport = inner.height;
    if view.chip.is_some() && !view.detached {
        view.scroll = 0;
    } else if !view.detached && !heights.is_empty() {
        let shelf = view.shelf.min(heights.len() - 1);
        let top = if shelf == 0 { 0 } else { starts[shelf] };
        let bottom = starts[shelf] + heights[shelf].saturating_sub(2);
        if top < view.scroll {
            view.scroll = top;
        } else if bottom > view.scroll + viewport {
            // A shelf taller than the window shows its top, or every frame would flip between both.
            view.scroll = (bottom - viewport).min(top);
        }
    }
    view.scroll = view.scroll.min(total.saturating_sub(viewport));
    let scroll = view.scroll;

    let mut chip_hits = Vec::new();
    if scroll == 0 {
        let spinner = view.loading.then(|| Span::styled(format!("   {}", text::spinner(frame_no)), p.faint));
        if home.chips.is_empty() {
            let greeting = if home.greeting.is_empty() { "Hello".to_owned() } else { home.greeting.clone() };
            let mut spans = vec![Span::styled(greeting, Style::new().fg(p.text).bold())];
            spans.extend(spinner);
            frame.buffer_mut().set_line(inner.x, inner.y, &Line::from(spans), inner.width);
        } else {
            let cursor = view.chip.filter(|_| focused);
            let room = inner.width.saturating_sub(if spinner.is_some() { 4 } else { 0 });
            let (hits, end) =
                chips(frame, app.accent.color(), app.accent.selection(), inner, room, &home.chips, cursor);
            chip_hits = hits;
            if let Some(spinner) = spinner {
                frame.buffer_mut().set_span(end, inner.y, &spinner, 4);
            }
        }
    }

    view.offsets.resize(home.shelves.len(), 0);
    let cards = visible_cards(inner.width);
    let columns = song_columns(inner.width);
    view.cards = cards;
    view.columns = columns;
    let on_shelf = view.chip.is_none();
    let mut layouts = Vec::new();
    // The first page of the shelf below the screen, so scrolling down finds it loaded.
    let mut below = Vec::new();
    for (index, shelf) in home.shelves.iter().enumerate() {
        let y = inner.y as i32 + starts[index] as i32 - scroll as i32;
        if y + heights[index] as i32 <= inner.y as i32 {
            continue;
        }
        let page = if shelf.songs { columns * SONG_ROWS } else { cards };
        let size = if shelf.songs { SONG_COVER } else { COVER };
        if y >= inner.bottom() as i32 {
            if !shelf.shortcuts {
                let first = view.offsets[index];
                below.extend(shelf.items.iter().skip(first).take(page).map(|item| (cover_of(item), size)));
            }
            break;
        }
        let first = if shelf.shortcuts {
            0
        } else {
            let offset = &mut view.offsets[index];
            if index == view.shelf && on_shelf {
                if shelf.songs {
                    views::follow_songs(offset, view.item, columns);
                } else {
                    follow(offset, view.item, cards);
                }
            }
            let last_page = if shelf.songs {
                shelf.items.len().div_ceil(SONG_ROWS).saturating_sub(columns) * SONG_ROWS
            } else {
                shelf.items.len().saturating_sub(cards)
            };
            *offset = (*offset).min(last_page);
            *offset
        };
        let count = if shelf.shortcuts { shelf.items.len() } else { page };
        layouts.push(ShelfLayout {
            index,
            y,
            title: shelf.title.clone(),
            strapline: shelf.strapline.clone(),
            shortcuts: shelf.shortcuts,
            songs: shelf.songs,
            play_all: shelf.play_all.is_some(),
            total: shelf.items.len(),
            first,
            items: shelf.items.iter().skip(first).take(count).cloned().collect(),
            nearby: if shelf.shortcuts { Vec::new() } else { nearby(&shelf.items, first, page) },
        });
    }
    let selected = (view.shelf, view.item, on_shelf);
    app.hits.extend(chip_hits);
    for ((url, round), size) in below {
        app.prefetch(url.as_deref(), size, round);
    }

    for layout in layouts {
        let selected_item = (selected.0 == layout.index && selected.2).then_some(selected.1);
        if layout.shortcuts {
            draw_shortcuts(frame, app, inner, &layout, selected_item, focused);
        } else if layout.songs {
            draw_songs(frame, app, inner, &layout, selected_item, focused);
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

/// The chips in the top row of `area`, within `room` columns, scrolled so the one under the
/// cursor shows. Returns where they are, and the column after the last one drawn.
fn chips(
    frame: &mut Frame,
    accent: Color,
    selection: Color,
    area: Rect,
    room: u16,
    chips: &[Chip],
    cursor: Option<usize>,
) -> (Vec<(Rect, Hit)>, u16) {
    const GAP: u16 = 1;
    let p = theme::palette();
    let widths: Vec<u16> = chips.iter().map(|c| c.title.width() as u16 + 2).collect();
    let right = area.x + room;
    // Start as far left as possible while the chip under the cursor still fits.
    let mut start = 0;
    if let Some(cursor) = cursor.filter(|&c| c < chips.len()) {
        while start < cursor && widths[start..=cursor].iter().map(|w| w + GAP).sum::<u16>() > room {
            start += 1;
        }
    }
    let mut hits = Vec::new();
    let mut x = area.x;
    let buf = frame.buffer_mut();
    for (index, chip) in chips.iter().enumerate().skip(start) {
        let width = widths[index];
        if x + width > right {
            break;
        }
        let under_cursor = cursor == Some(index);
        let style = match (chip.selected, under_cursor) {
            (true, true) => Style::new().fg(p.bg).bg(accent).bold().underlined(),
            (true, false) => Style::new().fg(p.bg).bg(accent).bold(),
            (false, true) => Style::new().fg(p.text).bg(selection).bold(),
            (false, false) => Style::new().fg(p.text).bg(p.raised),
        };
        buf.set_string(x, area.y, format!(" {} ", chip.title), style);
        hits.push((Rect::new(x, area.y, width, 1), Hit::HomeChip(index)));
        x += width + GAP;
    }
    (hits, x)
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
    let strapline = u16::from(!shelf.strapline.is_empty());
    if shelf.shortcuts {
        let columns = shortcut_columns(width);
        let rows = shelf.items.len().div_ceil(columns as usize) as u16;
        rows * (TILE_COVER.height + 1) + 1
    } else if shelf.songs {
        // A short shelf of songs is only as tall as its longest column.
        let rows = shelf.items.len().clamp(1, SONG_ROWS) as u16;
        strapline + SONGS_ROWS - (SONG_ROWS as u16 - rows) * 3
    } else {
        strapline + SHELF_ROWS
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
    let p = theme::palette();
    let accent = app.accent.color();
    let selection = if focused { app.accent.selection() } else { p.unfocused };
    for (url, round) in &layout.nearby {
        app.prefetch(url.as_deref(), COVER, *round);
    }
    let marker = playing_marker(app);
    let top = header(frame, app, area, layout, selected.is_some());

    let cover_y = top + 2;
    // A card's box reaches one column past its cover on both sides, the first card's too.
    let padded = Rect { x: area.x.saturating_sub(1), width: area.width + 2, ..area };
    let slots = visible_cards(area.width);
    for (column, item) in layout.items.iter().enumerate() {
        let x = card_x(area.x, area.width, slots, column);
        let index = layout.first + column;
        let is_selected = selected == Some(index);
        let card = clip(padded, x.saturating_sub(1), COVER.width + 2, cover_y - 1, COVER.height + 4);
        if is_selected && let Some(card) = card {
            frame.buffer_mut().set_style(card, Style::new().bg(selection));
        }
        draw_cover(frame, app, (x, cover_y), area, item, COVER);
        let buf = frame.buffer_mut();
        let playing = is_playing(app, item);
        let title_style = card_title_style(accent, playing, is_selected);
        let width = COVER.width as usize;
        let text_y = cover_y + COVER.height as i32;
        if let Some(y) = row(area, text_y) {
            let title_width = if playing { width - 4 } else { width };
            buf.set_string(x, y, text::truncate(&item.title, title_width), title_style);
            // Below the cover, so it never lands on image cells.
            if playing {
                buf.set_string(x + COVER.width - 3, y, marker, Style::new().fg(accent));
            }
        }
        if let Some(y) = row(area, text_y + 1) {
            buf.set_string(x, y, text::truncate(&item.subtitle, width), Style::new().fg(p.muted));
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

/// A shelf's strapline and title, with Play all and the arrows that page it where it has
/// them. Returns the row of the title.
fn header(frame: &mut Frame, app: &mut App, area: Rect, layout: &ShelfLayout, selected: bool) -> i32 {
    let p = theme::palette();
    let mut y = layout.y;
    if !layout.strapline.is_empty() {
        if let Some(row) = row(area, y) {
            let strapline = text::truncate(&layout.strapline, area.width as usize);
            frame.buffer_mut().set_string(area.x, row, strapline, Style::new().fg(p.muted));
        }
        y += 1;
    }
    let Some(title_y) = row(area, y) else { return y };
    let pageable = layout.total > layout.items.len();
    let play_all_width = if layout.play_all { PLAY_ALL.len() as u16 + 4 } else { 0 };
    let reserved = if pageable { 28 } else { 2 } + play_all_width;
    let title_width = area.width.saturating_sub(reserved) as usize;
    let buf = frame.buffer_mut();
    buf.set_string(area.x, title_y, text::truncate(&layout.title, title_width), Style::new().fg(p.text).bold());
    let mut right = area.right();
    if pageable {
        let last = layout.first + layout.items.len();
        let pager = format!("{}–{} of {}", layout.first + 1, last, layout.total);
        let style = if selected { Style::new().fg(p.muted) } else { Style::new().fg(p.faint) };
        let next_x = area.right().saturating_sub(2);
        let prev_x = next_x.saturating_sub(4);
        right = prev_x.saturating_sub(pager.chars().count() as u16 + 3);
        buf.set_string(right, title_y, pager, style);
        let icons = crate::icons::icons();
        let arrows = [
            (prev_x, icons.chevron_left, false, layout.first > 0),
            (next_x, icons.chevron_right, true, last < layout.total),
        ];
        for (x, icon, forward, enabled) in arrows {
            let arrow = Rect::new(x.saturating_sub(1), title_y, 3, 1);
            let color = if enabled { p.text } else { p.disabled };
            buf.set_style(arrow, Style::new().bg(p.raised));
            buf.set_string(x, title_y, icon, Style::new().fg(color));
            if enabled {
                app.hits.push((arrow, Hit::ShelfScroll { shelf: layout.index, forward }));
            }
        }
    }
    if layout.play_all {
        let width = play_all_width - 1;
        let x = right.saturating_sub(width + 2);
        let button = Rect::new(x, title_y, width, 1);
        let hovered = super::hovered(app, button);
        let style = if hovered { Style::new().fg(p.bg).bg(p.text) } else { Style::new().fg(p.text).bg(p.raised) };
        let buf = frame.buffer_mut();
        buf.set_string(x, title_y, format!(" {} {PLAY_ALL} ", crate::icons::icons().play), style);
        app.hits.push((button, Hit::ShelfPlay(layout.index)));
    }
    y
}

/// A titled shelf of songs in columns of [`SONG_ROWS`], like YouTube Music's Quick picks:
/// a small cover, the title and the artists. `selected` is the song under the cursor.
fn draw_songs(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    layout: &ShelfLayout,
    selected: Option<usize>,
    focused: bool,
) {
    let p = theme::palette();
    let accent = app.accent.color();
    let selection = if focused { app.accent.selection() } else { p.unfocused };
    for (url, round) in &layout.nearby {
        app.prefetch(url.as_deref(), SONG_COVER, *round);
    }
    let marker = playing_marker(app);
    let top = header(frame, app, area, layout, selected.is_some());
    let columns = song_columns(area.width);
    let width = (area.width.saturating_sub(SONG_GAP * (columns as u16 - 1))) / columns as u16;
    let padded = Rect { x: area.x.saturating_sub(1), width: area.width + 2, ..area };
    for (position, item) in layout.items.iter().enumerate() {
        let (column, line) = ((position / SONG_ROWS) as u16, (position % SONG_ROWS) as i32);
        let x = area.x + column * (width + SONG_GAP);
        let y = top + 2 + line * 3;
        let index = layout.first + position;
        let is_selected = selected == Some(index);
        let Some(cell) = clip(padded, x.saturating_sub(1), width + 2, y, SONG_COVER.height) else { continue };
        if is_selected {
            frame.buffer_mut().set_style(cell, Style::new().bg(selection));
        }
        draw_cover(frame, app, (x, y), area, item, SONG_COVER);
        let playing = app.now.track.as_ref().is_some_and(|track| track.uri == item.context_uri);
        let hovered = super::hovered(app, cell);
        let text_x = x + SONG_COVER.width + 2;
        let reserved = SONG_COVER.width
            + 2
            + if hovered {
                super::PLAY_BUTTON_WIDTH + 1
            } else if playing {
                4
            } else {
                0
            };
        let room = width.saturating_sub(reserved) as usize;
        let buf = frame.buffer_mut();
        if let Some(row) = row(area, y) {
            let style = card_title_style(accent, playing, is_selected);
            buf.set_string(text_x, row, text::truncate(&item.title, room), style);
            if playing && !hovered {
                buf.set_string(x + width - 3, row, marker, Style::new().fg(accent));
            }
        }
        if let Some(row) = row(area, y + 1) {
            let room = width.saturating_sub(SONG_COVER.width + 2) as usize;
            buf.set_string(text_x, row, text::truncate(&item.subtitle, room), Style::new().fg(p.muted));
        }
        app.hits.push((cell, Hit::Card { shelf: layout.index, item: index }));
        if hovered && let Some(row) = row(area, y) {
            let hit = Hit::CardPlay { shelf: layout.index, item: index };
            super::play_button(frame.buffer_mut(), app, x + width, row, hit, false);
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
    let p = theme::palette();
    let accent = app.accent.color();
    let selection = if focused { app.accent.selection() } else { p.unfocused };
    let marker = playing_marker(app);
    let columns = shortcut_columns(area.width);
    let tile_width = (area.width - TILE_GAP * (columns - 1)) / columns;
    for (index, item) in layout.items.iter().enumerate() {
        let column = index as u16 % columns;
        let tile_y = layout.y + (index as u16 / columns * (TILE_COVER.height + 1)) as i32;
        let x = area.x + column * (tile_width + TILE_GAP);
        let Some(tile) = clip(area, x, tile_width, tile_y, TILE_COVER.height) else { continue };
        let is_selected = selected == Some(index);
        frame.buffer_mut().set_style(tile, Style::new().bg(if is_selected { selection } else { p.raised }));
        draw_cover(frame, app, (x, tile_y), area, item, TILE_COVER);
        let playing = is_playing(app, item);
        let style = if playing { Style::new().fg(accent).bold() } else { Style::new().fg(p.text).bold() };
        let hovered = super::hovered(app, tile);
        // The end of the row holds the play button while hovered, else the playing tile's
        // equalizer; the name stops short of either.
        let reserved = TILE_COVER.width
            + 3
            + if hovered {
                super::PLAY_BUTTON_WIDTH + 1
            } else if playing {
                4
            } else {
                0
            };
        let name = text::truncate(&item.title, tile_width.saturating_sub(reserved) as usize);
        // Vertically centered next to the cover.
        let name_y = row(area, tile_y + (TILE_COVER.height / 2) as i32);
        if let Some(y) = name_y {
            frame.buffer_mut().set_string(x + TILE_COVER.width + 2, y, name, style);
        }
        app.hits.push((tile, Hit::Card { shelf: layout.index, item: index }));
        let Some(y) = name_y else { continue };
        // The play button of a hovered tile takes the equalizer's place.
        if hovered {
            let hit = Hit::CardPlay { shelf: layout.index, item: index };
            super::play_button(frame.buffer_mut(), app, x + tile_width - 1, y, hit, false);
        } else if playing {
            frame.buffer_mut().set_string(x + tile_width - 4, y, marker, Style::new().fg(accent));
        }
    }
}

/// A card's title: the accent marks what plays, as in track lists. The selection shows as the
/// background, so its title is only bold.
pub(super) fn card_title_style(accent: Color, playing: bool, selected: bool) -> Style {
    let p = theme::palette();
    match (playing, selected) {
        (true, _) => Style::new().fg(accent).bold(),
        (false, true) => Style::new().fg(p.text).bold(),
        (false, false) => Style::new().fg(p.text),
    }
}

/// Whether `item` is what plays now: the playlist, album or show the music comes from, or the
/// episode itself, since an episode plays from its whole show.
fn is_playing(app: &App, item: &HomeItem) -> bool {
    match item.kind {
        ItemKind::Episode => app.now.track.as_ref().is_some_and(|track| track.uri == item.uri),
        _ => app.playing_context() == Some(item.uri.as_str()),
    }
}

/// What marks the playing card: a moving equalizer, or a speaker while paused, as in the
/// sidebar.
pub(super) fn playing_marker(app: &App) -> &'static str {
    if app.now.playing { super::equalizer(app) } else { crate::icons::icons().volume }
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
    let p = theme::palette();
    let round = item.kind == ItemKind::Artist;
    if super::cover_in(frame, app, item.image_url.as_deref(), size, round, (x, y), size, clip) {
        return;
    }
    let Some(part) = self::clip(clip, x, size.width, y, size.height) else { return };
    let buf = frame.buffer_mut();
    buf.set_style(part, Style::new().bg(p.raised));
    let icons = crate::icons::icons();
    let icon = match item.kind {
        ItemKind::Playlist => icons.playlist,
        ItemKind::Album => icons.album,
        ItemKind::Artist => icons.artist,
        ItemKind::Audiobook => icons.audiobook,
        ItemKind::Podcast | ItemKind::Episode => icons.podcast,
    };
    if let Some(row) = row(clip, y + (size.height / 2) as i32) {
        buf.set_string(x + size.width / 2, row, icon, Style::new().fg(p.faint));
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

    #[test]
    fn full_rows_reach_the_right_edge() {
        for width in COVER.width..200 {
            let slots = visible_cards(width);
            if slots > 1 {
                assert_eq!(card_x(0, width, slots, slots - 1) + COVER.width, width);
            }
            // Gaps never shrink below the minimum and differ by at most one column.
            let gaps: Vec<u16> = (1..slots)
                .map(|column| card_x(0, width, slots, column) - card_x(0, width, slots, column - 1) - COVER.width)
                .collect();
            if let (Some(widest), Some(narrowest)) = (gaps.iter().max(), gaps.iter().min()) {
                assert!(*narrowest >= CARD_GAP && widest - narrowest <= 1);
            }
        }
    }
}
