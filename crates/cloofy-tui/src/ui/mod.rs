mod artist;
mod content;
mod help;
mod hints;
mod home;
mod library;
mod menu;
mod picker;
mod player_bar;
mod sidebar;
mod text;
mod topbar;
mod wordmark;
mod zen;

use std::time::Instant;

use ratatui::widgets::Widget;
use ratatui::{
    Frame,
    buffer::{Buffer, Cell},
    layout::{Constraint, Layout, Rect, Size},
    style::{Color, Modifier, Style},
};
use ratatui_image::{Image, protocol::Protocol};
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{App, Focus, Hit},
    icons::icons,
    loader::Picture,
    overlay::Overlay,
    snapshot::Placement,
    theme::{BG, TEXT},
    views::View,
};

const SIDEBAR_WIDTH: u16 = 32;
/// Below this width only the focused pane is shown.
const TWO_PANE_MIN_WIDTH: u16 = 90;
const PLAYER_HEIGHT: u16 = 5;
/// The character Kitty image cells are made of.
const KITTY_PLACEHOLDER: char = '\u{10EEEE}';

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.screen = area;
    app.hits.clear();
    app.placements.clear();
    frame.buffer_mut().set_style(area, Style::new().bg(BG).fg(TEXT));
    // Small windows get compact bars so there is room left for the content.
    let top_height = match area.height {
        30.. => topbar::HEIGHT,
        14.. => 1,
        _ => 0,
    };
    let player_height = match area.height {
        24.. => PLAYER_HEIGHT,
        10.. => 2,
        _ => 1,
    };
    let hints_height = u16::from(area.height >= 16);
    let [top, main, player, hints] = Layout::vertical([
        Constraint::Length(top_height),
        Constraint::Min(0),
        Constraint::Length(player_height),
        Constraint::Length(hints_height),
    ])
    .areas(area);

    if app.zen {
        zen::draw(frame, app, top.union(main).union(player));
    } else {
        if !top.is_empty() {
            topbar::draw(frame.buffer_mut(), app, top);
        }
        if main.width >= TWO_PANE_MIN_WIDTH {
            let [nav, content] =
                Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)]).areas(main);
            sidebar::draw(frame, app, nav);
            draw_content(frame, app, content);
        } else if app.focus == Focus::Nav {
            sidebar::draw(frame, app, main);
        } else {
            draw_content(frame, app, main);
        }
        player_bar::draw(frame, app, player);
    }
    if !hints.is_empty() {
        hints::draw(frame.buffer_mut(), app, hints);
    }

    match &app.overlay {
        Some(Overlay::Help) => help::draw(frame.buffer_mut(), app, area),
        Some(Overlay::Menu(_)) => menu::draw(frame.buffer_mut(), app, area),
        Some(_) => picker::draw(frame, app, area),
        None => {}
    }

    hover(frame.buffer_mut(), app);
    app.sync_hover();

    let elapsed = app.last_draw.elapsed();
    app.last_draw = Instant::now();
    app.effects.process_effects(elapsed.into(), frame.buffer_mut(), area);
}

fn draw_content(frame: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    match app.view {
        View::Home(_) => home::draw(frame, app, area),
        View::Library(_) => library::draw(frame, app, area),
        View::Artist(_) => artist::draw(frame, app, area),
        _ => content::draw(frame, app, area),
    }
}

/// Draws a cover centered in `slot`, or nothing while it loads. Returns whether it drew.
pub(super) fn cover(frame: &mut Frame, app: &mut App, url: Option<&str>, size: Size, round: bool, slot: Rect) -> bool {
    cover_in(frame, app, url, size, round, (slot.x, slot.y as i32), slot.as_size(), slot)
}

/// Like [`cover`], for a slot at `(x, y)` that may reach past `clip` while scrolling: the
/// visible part of the cover is drawn.
#[allow(clippy::too_many_arguments)]
pub(super) fn cover_in(
    frame: &mut Frame,
    app: &mut App,
    url: Option<&str>,
    size: Size,
    round: bool,
    (x, y): (u16, i32),
    slot: Size,
    clip: Rect,
) -> bool {
    let placed = app.cover(url, size, round).and_then(|picture| picture_in(frame, picture, (x, y), slot, clip));
    match placed {
        Some(placement) => {
            app.placements.push(placement);
            true
        }
        None => false,
    }
}

/// Draws an image centered in `slot`; returns where it went, for dev snapshots.
pub(super) fn picture_at(frame: &mut Frame, picture: &Picture, slot: Rect) -> Option<Placement> {
    picture_in(frame, picture, (slot.x, slot.y as i32), slot.as_size(), slot)
}

/// Draws the part of an image centered in a slot at `(x, y)` that lies inside `clip`.
/// Returns `None` when nothing was drawn: fully clipped, or a protocol that cannot be cut.
fn picture_in(frame: &mut Frame, picture: &Picture, (x, y): (u16, i32), slot: Size, clip: Rect) -> Option<Placement> {
    let size = picture.protocol.size();
    let size = Size::new(size.width.min(slot.width), size.height.min(slot.height));
    let left = x + slot.width.saturating_sub(size.width) / 2;
    let top = y + (slot.height.saturating_sub(size.height) / 2) as i32;
    let visible_top = top.max(clip.y as i32);
    let visible_bottom = (top + size.height as i32).min(clip.bottom() as i32);
    if visible_top >= visible_bottom || left + size.width > frame.area().right() {
        return None;
    }
    let skip = (visible_top - top) as u16;
    let area = Rect::new(left, visible_top as u16, size.width, (visible_bottom - visible_top) as u16);
    let upload = if area.height == size.height {
        frame.render_widget(Image::new(&picture.protocol), area);
        take_upload(&mut frame.buffer_mut()[(area.x, area.y)])
    } else {
        // Kitty placeholders and half blocks are plain cells, so rows can be cut; Sixel and
        // iTerm2 images are a single escape sequence and cannot.
        if !matches!(picture.protocol, Protocol::Kitty(_) | Protocol::Halfblocks(_)) {
            return None;
        }
        let mut scratch = Buffer::empty(Rect::new(0, 0, size.width, size.height));
        Image::new(&picture.protocol).render(scratch.area, &mut scratch);
        let upload = take_upload(&mut scratch[(0, 0)]);
        let buf = frame.buffer_mut();
        for row in 0..area.height {
            for column in 0..area.width {
                buf[(area.x + column, area.y + row)] = scratch[(column, skip + row)].clone();
            }
        }
        upload
    };
    let cells = match picture.source {
        Some(_) => area.rows().flat_map(|row| row.columns()).map(|p| frame.buffer_mut()[p].clone()).collect(),
        None => Vec::new(),
    };
    Some(Placement { area, size, skip, source: picture.source.clone(), cells, upload })
}

/// The first time a Kitty image is drawn, the upload of its pixels rides on its top-left cell.
/// Takes it off, so that whatever is drawn over that cell, like an overlay, cannot lose it; the
/// caller writes it out after the frame.
fn take_upload(cell: &mut Cell) -> Option<String> {
    let symbol = cell.symbol();
    let start = symbol.find(KITTY_PLACEHOLDER).filter(|&start| start > 0)?;
    let (upload, placeholder) = (symbol[..start].to_owned(), symbol[start..].to_owned());
    cell.set_symbol(&placeholder);
    Some(upload)
}

/// The play button of a hovered card, ending at column `right` in row `y`. A dark cell to
/// its left sets it off from covers in the accent color.
pub(super) fn play_button(buf: &mut Buffer, app: &mut App, right: u16, y: u16, hit: Hit) {
    let icons = icons();
    let area = Rect::new(right.saturating_sub(3), y, 3, 1).intersection(buf.area);
    let accent = app.accent.color();
    if let Some(edge) = area.x.checked_sub(1) {
        buf.set_string(edge, y, " ", Style::new().bg(BG));
    }
    buf.set_style(area, Style::new().bg(accent));
    buf.set_string(area.x + 1, y, icons.play, Style::new().fg(BG).bg(accent));
    app.hits.push((area, hit));
}

/// Where each of `names`, written out joined by ", " from column `x`, lies on screen, cut
/// off at `width` columns: one rect per visible name, with its index.
pub(super) fn name_spans(x: u16, y: u16, names: &[String], width: usize) -> Vec<(Rect, usize)> {
    let mut spans = Vec::new();
    let mut offset = 0;
    for (index, name) in names.iter().enumerate() {
        if offset >= width {
            break;
        }
        let name_width = name.width().min(width - offset);
        spans.push((Rect::new(x + offset as u16, y, name_width as u16, 1), index));
        offset += name.width() + 2;
    }
    spans
}

/// Whether the mouse is over `area` with nothing in front of it.
pub(super) fn hovered(app: &App, area: Rect) -> bool {
    app.overlay.is_none() && app.mouse.is_some_and(|position| area.contains(position))
}

/// A slim scrollbar in column `x`, shown only when the content is longer than the view.
pub(super) fn scrollbar(buf: &mut Buffer, x: u16, y: u16, height: u16, total: usize, offset: usize, visible: usize) {
    if total <= visible || height < 3 || x >= buf.area.right() {
        return;
    }
    let rows = height as usize;
    let thumb = (rows * visible / total).clamp(1, rows);
    let max_offset = total - visible;
    let start = (rows - thumb) * offset.min(max_offset) / max_offset;
    for row in 0..rows {
        let (symbol, color) = if (start..start + thumb).contains(&row) {
            ("┃", Color::Rgb(96, 96, 108))
        } else {
            ("│", Color::Rgb(30, 30, 37))
        };
        buf.set_string(x, y + row as u16, symbol, Style::new().fg(color));
    }
}

/// Lightens whatever clickable thing the mouse is over; links get underlined instead.
fn hover(buf: &mut Buffer, app: &App) {
    let Some(position) = app.mouse else { return };
    // Only the topmost target counts: an overlay's body hides the rows and cards under it.
    let Some((area, hit)) = app.hits.iter().rev().find(|(area, _)| area.contains(position)) else { return };
    if !hit.is_interactive() {
        return;
    }
    let area = area.intersection(buf.area);
    if let Hit::Link(_) = hit {
        buf.set_style(area, Style::new().fg(TEXT).add_modifier(Modifier::UNDERLINED));
        return;
    }
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            // Leave image cells alone: their colors carry the picture.
            if cell.symbol().contains(KITTY_PLACEHOLDER)
                || app.placements.iter().any(|p| p.area.contains((x, y).into()))
            {
                continue;
            }
            let base = match cell.bg {
                Color::Rgb(r, g, b) => crate::theme::Rgb(r, g, b),
                _ => crate::theme::Rgb(13, 13, 16),
            };
            cell.bg = base.mix(crate::theme::Rgb(255, 255, 255), 0.06).into();
        }
    }
}
