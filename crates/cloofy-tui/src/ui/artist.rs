//! Artist page: header with portrait and stats, popular tracks, then shelves.

use ratatui::{
    Frame,
    layout::{Rect, Size},
    style::{Color, Style},
    text::{Line, Span},
};

use super::{
    content,
    home::{self, SHELF_ROWS, ShelfLayout},
    text::{self, fit, right},
};
use crate::{
    app::{App, Focus, HeaderButton, Hit},
    icons::icons,
    theme::{DANGER, FAINT, MUTED, RAISED, TEXT},
    views::View,
};

const PORTRAIT: Size = Size { width: 14, height: 7 };
/// Portrait plus a gap.
const HEADER_ROWS: u16 = PORTRAIT.height + 1;
const UNFOCUSED_SELECTION: Color = Color::Rgb(25, 25, 31);

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    app.content_area = area;
    app.hits.push((area, Hit::ContentArea));
    let inner = Rect {
        x: area.x + 3,
        y: area.y + 1,
        width: area.width.saturating_sub(6),
        height: area.height.saturating_sub(1),
    };
    if inner.width < 24 || inner.height < 3 {
        return;
    }
    let focused = app.focus == Focus::Content;
    let frame_no = app.frame();
    let equalizer = super::equalizer(app);
    let accent = app.accent.color();
    let selection = if focused { app.accent.selection() } else { UNFOCUSED_SELECTION };
    let playing_uri = app.now.track.as_ref().map(|t| t.uri.clone());
    let playing = app.now.playing;
    let loading_uri = app.now.pending.as_ref().and_then(|p| p.uri.clone());
    let View::Artist(view) = &mut app.view else { return };

    let Some(page) = &view.page else {
        // Laid out like the loaded header, so nothing moves when the page arrives: an empty
        // portrait, the name beside it and the status where the stats will be.
        let buf = frame.buffer_mut();
        let portrait = Rect::new(inner.x, inner.y, PORTRAIT.width, PORTRAIT.height).intersection(inner);
        buf.set_style(portrait, Style::new().bg(RAISED));
        let text_x = inner.x + PORTRAIT.width + 3;
        let width = inner.right().saturating_sub(text_x) as usize;
        let status = match &view.error {
            Some(err) => (format!("✕  {err}"), Style::new().fg(DANGER)),
            None => (format!("{}  Loading…", text::spinner(frame_no)), Style::new().fg(MUTED)),
        };
        let lines = [
            ("ARTIST".to_owned(), Style::new().fg(FAINT).bold()),
            (view.name.clone(), Style::new().fg(TEXT).bold()),
            status,
        ];
        for (y, (line, style)) in (inner.y + 1..inner.bottom()).zip(lines) {
            buf.set_string(text_x, y, text::truncate(&line, width), style);
        }
        return;
    };

    // Virtual layout: header, "Popular" with its tracks, then the shelves. The heading has a
    // blank row before its tracks, like shelf titles before their covers.
    let tracks_top = HEADER_ROWS + 3;
    let shelves_top = tracks_top + page.top_tracks.len() as u16 + 2;
    let track_count = page.top_tracks.len();
    let (selected_top, selected_bottom) = match view.row.checked_sub(track_count) {
        None => (tracks_top + view.row as u16, tracks_top + view.row as u16 + 1),
        Some(shelf) => {
            let top = shelves_top + shelf as u16 * SHELF_ROWS;
            (top, top + SHELF_ROWS - 2)
        }
    };
    let total = shelves_top + page.shelves.len() as u16 * SHELF_ROWS;
    if !view.detached {
        // Keep the header visible while the first tracks are selected.
        let anchor = if view.row < 3 { 0 } else { selected_top };
        if anchor < view.scroll {
            view.scroll = anchor;
        } else if selected_bottom > view.scroll + inner.height {
            view.scroll = selected_bottom - inner.height;
        }
    }
    view.scroll = view.scroll.min(total.saturating_sub(inner.height));
    let scroll = view.scroll;
    let screen_y = |virtual_y: u16| -> Option<u16> {
        let y = (inner.y + virtual_y).checked_sub(scroll)?;
        (y >= inner.y && y < inner.bottom()).then_some(y)
    };

    let followed_label = view.uri.clone();
    let cards = home::visible_cards(inner.width);
    view.cards = cards;
    view.offsets.resize(page.shelves.len(), 0);
    if let Some(shelf) = view.row.checked_sub(track_count) {
        home::follow(&mut view.offsets[shelf], view.item, cards);
    }
    let layouts: Vec<ShelfLayout> = page
        .shelves
        .iter()
        .enumerate()
        .filter_map(|(index, shelf)| {
            let y = inner.y as i32 + (shelves_top + index as u16 * SHELF_ROWS) as i32 - scroll as i32;
            if y + SHELF_ROWS as i32 <= inner.y as i32 || y >= inner.bottom() as i32 {
                return None;
            }
            let first = view.offsets[index];
            Some(ShelfLayout {
                index: track_count + index,
                y,
                title: shelf.title.clone(),
                shortcuts: false,
                total: shelf.items.len(),
                first,
                items: shelf.items.iter().skip(first).take(cards).cloned().collect(),
                nearby: home::nearby(&shelf.items, first, cards),
            })
        })
        .collect();
    let header = (page.name.clone(), page.image_url.clone(), page.monthly_listeners, page.followers);
    let tracks: Vec<_> = page.top_tracks.iter().map(|t| (t.track.clone(), t.playcount)).collect();
    let (row, item) = (view.row, view.item);

    // Header: round portrait with name and stats beside it; scrolls out of view row by row.
    let header_y = inner.y as i32 - scroll as i32;
    if header_y + HEADER_ROWS as i32 > inner.y as i32 {
        let slot = (inner.x, header_y);
        if !super::cover_in(frame, app, header.1.as_deref(), PORTRAIT, true, slot, PORTRAIT, inner) {
            let rows = (header_y + PORTRAIT.height as i32 - inner.y as i32).min(PORTRAIT.height as i32) as u16;
            let top = header_y.max(inner.y as i32) as u16;
            frame.buffer_mut().set_style(Rect::new(inner.x, top, PORTRAIT.width, rows), Style::new().bg(RAISED));
        }
        let text_x = inner.x + PORTRAIT.width + 3;
        let width = inner.right().saturating_sub(text_x) as usize;
        let mut stats = Vec::new();
        if let Some(listeners) = header.2 {
            stats.push(format!("{} monthly listeners", text::grouped(listeners)));
        }
        if let Some(followers) = header.3 {
            stats.push(format!("{} followers", text::grouped(followers)));
        }
        let following = app.is_saved(&followed_label);
        let visible = |offset: u16| screen_y(offset);
        let buf = frame.buffer_mut();
        if let Some(y) = visible(1) {
            buf.set_string(text_x, y, "ARTIST", Style::new().fg(FAINT).bold());
        }
        if let Some(y) = visible(2) {
            buf.set_string(text_x, y, text::truncate(&header.0, width), Style::new().fg(TEXT).bold());
        }
        if let Some(y) = visible(3) {
            buf.set_string(text_x, y, text::truncate(&stats.join(" · "), width), Style::new().fg(MUTED));
        }
        let row = [
            (HeaderButton::Play, text::icon_label(icons().play, "Play"), true),
            (HeaderButton::Shuffle, text::icon_label(icons().shuffle, "Shuffle"), false),
            (
                HeaderButton::Save,
                if following {
                    text::icon_label(icons().saved, "Following")
                } else {
                    text::icon_label(icons().save, "Follow")
                },
                false,
            ),
            (HeaderButton::More, icons().more.to_owned(), false),
        ];
        if let Some(y) = visible(5) {
            content::buttons(buf, text_x, y, &row, accent, &mut app.hits);
        }
    }

    if let Some(y) = screen_y(tracks_top - 2) {
        frame.buffer_mut().set_string(inner.x, y, "Popular", Style::new().fg(TEXT).bold());
    }
    let title_width = (inner.width as usize).saturating_sub(5 + 16 + 9);
    for (index, (track, playcount)) in tracks.iter().enumerate() {
        let Some(y) = screen_y(tracks_top + index as u16) else { continue };
        let is_selected = row == index;
        let buf = frame.buffer_mut();
        if is_selected {
            buf.set_style(content::row_band(inner, y, 1), Style::new().bg(selection));
            if focused {
                buf.set_string(inner.x.saturating_sub(2), y, "▌", Style::new().fg(accent));
            }
        }
        let is_current = playing_uri.as_deref() == Some(track.uri.as_str());
        let number = match (is_current, playing) {
            _ if loading_uri.as_deref() == Some(track.uri.as_str()) => {
                Span::styled(fit(&right(text::spinner(frame_no), 3), 5), accent)
            }
            (false, _) => Span::styled(fit(&right(&(index + 1).to_string(), 3), 5), FAINT),
            (true, true) => Span::styled(fit(&format!(" {}", equalizer), 5), accent),
            (true, false) => Span::styled(fit("  ▶", 5), accent),
        };
        let title_style = if is_current { Style::new().fg(accent).bold() } else { Style::new().fg(TEXT) };
        let plays = playcount.map(text::grouped).unwrap_or_default();
        let heart = if app.is_saved(&track.uri) {
            Span::styled(format!("{} ", icons().saved), accent)
        } else {
            Span::raw("  ")
        };
        let line = Line::from(vec![
            number,
            Span::styled(fit(&track.name, title_width), title_style),
            Span::styled(right(&plays, 16), MUTED),
            Span::raw("  "),
            heart,
            Span::styled(right(&text::duration(track.duration_ms), 5), FAINT),
        ]);
        frame.buffer_mut().set_line(inner.x, y, &line, inner.width);
        app.hits.push((content::row_band(inner, y, 1), Hit::Row(index)));
    }

    for layout in layouts {
        let selected = (row == layout.index).then_some(item);
        home::draw_shelf(frame, app, inner, &layout, selected, focused);
    }
    super::scrollbar(
        frame.buffer_mut(),
        inner.right() + 1,
        inner.y,
        inner.height,
        total as usize,
        scroll as usize,
        inner.height as usize,
    );
}
