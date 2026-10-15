//! "Your Library": filter tabs and library items with thumbnails.

use ratatui::{
    Frame,
    layout::{Rect, Size},
    style::{Color, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use super::text;
use crate::{
    app::{App, Focus, Hit, LibraryEntry, LibraryFilter, NavItem},
    art::LIKED_COVER,
    commands::Command,
    icons::icons,
    theme::{FAINT, MUTED, SURFACE, TEXT},
    views::{ListCursor, View},
};

const THUMB: Size = Size { width: 4, height: 2 };
/// Two text lines next to the thumbnail, plus a gap.
const ENTRY_ROWS: u16 = 3;
/// Rows above the list: title and filter tabs.
const LIST_TOP: u16 = 5;

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    frame.buffer_mut().set_style(area, Style::new().bg(SURFACE));
    app.hits.push((area, Hit::NavArea));
    if area.height < LIST_TOP + ENTRY_ROWS {
        return;
    }
    let accent = app.accent.color();
    let page_open = matches!(app.view, View::Library(_));

    let mut title = vec![Span::styled(format!("{}  Your Library", icons().library), Style::new().fg(TEXT).bold())];
    if app.playlists_loading || app.library_loading {
        title.push(Span::styled(format!("  {}", text::spinner(app.frame())), FAINT));
    }
    let buf = frame.buffer_mut();
    buf.set_line(area.x + 3, area.y + 1, &Line::from(title), area.width.saturating_sub(8));
    let expand = Rect::new(area.right().saturating_sub(4), area.y + 1, 2, 1);
    let expand_style = if page_open { Style::new().fg(accent) } else { Style::new().fg(MUTED) };
    buf.set_string(expand.x, expand.y, icons().expand, expand_style);
    app.hits.push((expand, Hit::Command(Command::Library)));

    let filters = app.library_filters();
    let active = app.library_filter;
    let chips =
        tabs(frame.buffer_mut(), area.x + 3, area.y + 3, area.right().saturating_sub(2), &filters, active, accent);
    app.hits.extend(chips.into_iter().map(|(rect, filter)| (rect, Hit::Chip(filter))));

    let entries = app.library_entries();
    let list = Rect::new(area.x, area.y + LIST_TOP, area.width, area.bottom().saturating_sub(area.y + LIST_TOP));
    let mut cursor = ListCursor {
        selected: app.nav.selected.saturating_sub(NavItem::FIXED),
        offset: app.nav.offset,
        detached: app.nav.detached,
    };
    let visible = cursor.visible((list.height / ENTRY_ROWS) as usize, entries.len());
    app.nav.offset = cursor.offset;
    let ahead = visible.end..(visible.end + visible.len()).min(entries.len());
    for (row, position) in visible.enumerate() {
        let y = list.y + row as u16 * ENTRY_ROWS;
        entry(frame, app, area, y, position + NavItem::FIXED, entries[position]);
    }
    // The next screenful of thumbnails, so scrolling down finds them loaded.
    for position in ahead {
        let (_, _, image_url, round) = describe(app, entries[position]);
        app.prefetch(image_url.as_deref(), THUMB, round);
    }
    let fits = (list.height / ENTRY_ROWS) as usize;
    super::scrollbar(frame.buffer_mut(), area.right() - 1, list.y, list.height, entries.len(), app.nav.offset, fits);
}

/// Text tabs: the active one bold with an accent underline. Returns their click areas.
///
/// Tabs that don't fit hide behind a chevron at either end; clicking one selects the next
/// hidden filter in that direction. The active tab always stays in view.
pub(super) fn tabs(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    right: u16,
    filters: &[LibraryFilter],
    active: LibraryFilter,
    accent: Color,
) -> Vec<(Rect, LibraryFilter)> {
    const GAP: u16 = 2;
    /// A chevron plus the gap between it and the tabs.
    const CHEVRON: u16 = 2;
    let widths: Vec<u16> = filters.iter().map(|f| f.label().width() as u16).collect();
    // Width of tabs `range` side by side.
    let span = |range: std::ops::Range<usize>| -> u16 {
        let count = range.len() as u16;
        widths[range].iter().sum::<u16>() + GAP * count.saturating_sub(1)
    };
    let room = right.saturating_sub(x);
    let active_index = filters.iter().position(|f| *f == active).unwrap_or(0);
    // Start as far left as possible while the active tab still fits, with room for the
    // chevrons it takes.
    let mut start = 0;
    if span(0..filters.len()) > room {
        while start < active_index {
            let left = if start > 0 { CHEVRON } else { 0 };
            let more = if active_index + 1 < filters.len() { CHEVRON } else { 0 };
            if left + span(start..active_index + 1) + more <= room {
                break;
            }
            start += 1;
        }
    }

    let mut hits = Vec::new();
    let mut x = x;
    let icons = icons();
    if start > 0 {
        buf.set_string(x, y, icons.chevron_left, Style::new().fg(MUTED));
        hits.push((Rect::new(x, y, CHEVRON, 1), filters[start - 1]));
        x += CHEVRON;
    }
    for (index, &filter) in filters.iter().enumerate().skip(start) {
        // The rest fit as they are, or this tab must leave room for the chevron.
        let limit = if x + span(index..filters.len()) <= right { right } else { right.saturating_sub(CHEVRON) };
        let width = widths[index];
        if x + width > limit {
            buf.set_string(right.saturating_sub(1), y, icons.chevron_right, Style::new().fg(MUTED));
            hits.push((Rect::new(right.saturating_sub(CHEVRON), y, CHEVRON, 1), filter));
            break;
        }
        let style = if filter == active {
            Style::new().fg(TEXT).bold().underlined().underline_color(accent)
        } else {
            Style::new().fg(MUTED)
        };
        buf.set_string(x, y, filter.label(), style);
        hits.push((Rect::new(x, y, width, 1), filter));
        x += width + GAP;
    }
    hits
}

fn entry(frame: &mut Frame, app: &mut App, area: Rect, y: u16, index: usize, entry: LibraryEntry) {
    let (title, subtitle, image_url, round) = describe(app, entry);
    let cursor = app.focus == Focus::Nav && app.nav.selected == index;
    let open = app.is_open(NavItem::Library(entry));
    let row = Rect::new(area.x, y, area.width, THUMB.height);
    if cursor {
        frame.buffer_mut().set_style(row, Style::new().bg(app.accent.selection()));
    }
    if open {
        let marker = Style::new().fg(app.accent.color());
        frame.buffer_mut().set_string(area.x + 1, y, "▎", marker);
        frame.buffer_mut().set_string(area.x + 1, y + 1, "▎", marker);
    }

    let thumb = Rect::new(area.x + 3, y, THUMB.width, THUMB.height);
    super::cover(frame, app, image_url.as_deref(), THUMB, round, thumb);

    let x = thumb.right() + 2;
    // What is playing shows in the accent color with a moving equalizer, like Spotify's speaker.
    let playing = app.playing_context().is_some_and(|context| app.entry_uri(entry).as_deref() == Some(context));
    // Text stays out of the last column, where the scrollbar goes. The equalizer ends a column
    // before it, and only the title row makes room for it.
    let width = area.right().saturating_sub(x + 1) as usize;
    let title_width = area.right().saturating_sub(x + if playing { 6 } else { 1 }) as usize;
    let title_style = match (playing, open) {
        (true, _) => Style::new().fg(app.accent.color()).bold(),
        (false, true) => Style::new().fg(TEXT).bold(),
        (false, false) => Style::new().fg(TEXT),
    };
    let equalizer = match (app.now.playing, app.calm()) {
        (true, false) => text::equalizer(app.frame()),
        (true, true) => text::equalizer(0),
        (false, _) => icons().volume,
    };
    let buf = frame.buffer_mut();
    buf.set_string(x, y, text::truncate(&title, title_width), title_style);
    if playing {
        buf.set_string(area.right().saturating_sub(5), y, equalizer, Style::new().fg(app.accent.color()));
    }
    buf.set_string(x, y + 1, text::truncate(&subtitle, width), Style::new().fg(MUTED));
    app.hits.push((row, Hit::Nav(index)));
}

/// Title, subtitle, cover URL and whether the cover is round.
pub(super) fn describe(app: &App, entry: LibraryEntry) -> (String, String, Option<String>, bool) {
    match entry {
        LibraryEntry::Liked => {
            let subtitle = match app.liked_total {
                Some(total) => format!("Playlist · {total} songs"),
                None => "Playlist".into(),
            };
            ("Liked Songs".into(), subtitle, Some(LIKED_COVER.into()), false)
        }
        LibraryEntry::Playlist(i) => {
            let p = &app.playlists[i];
            (p.name.clone(), format!("Playlist · {}", p.owner_name), p.image_url.clone(), false)
        }
        LibraryEntry::Album(i) => {
            let a = &app.albums[i];
            (a.name.clone(), format!("Album · {}", a.artists.join(", ")), a.image_url.clone(), false)
        }
        LibraryEntry::Artist(i) => {
            let a = &app.artists[i];
            (a.name.clone(), "Artist".into(), a.image_url.clone(), true)
        }
        LibraryEntry::Show(i) => {
            let s = &app.shows[i];
            (s.name.clone(), "Podcast".into(), s.image_url.clone(), false)
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;

    use super::*;

    const ALL_KINDS: [LibraryFilter; 4] =
        [LibraryFilter::Playlists, LibraryFilter::Albums, LibraryFilter::Artists, LibraryFilter::Podcasts];

    /// Tabs as drawn in the 32-column sidebar, with the text of their row.
    fn sidebar_tabs(active: LibraryFilter) -> (Vec<(Rect, LibraryFilter)>, String) {
        let mut buf = Buffer::empty(Rect::new(0, 0, 32, 1));
        let hits = tabs(&mut buf, 3, 0, 30, &ALL_KINDS, active, Color::Reset);
        let line = buf.content.iter().map(|cell| cell.symbol()).collect::<String>();
        (hits, line.trim_end().to_owned())
    }

    #[test]
    fn hidden_tabs_sit_behind_a_chevron() {
        let (hits, line) = sidebar_tabs(LibraryFilter::All);
        assert_eq!(line, "   Playlists  Albums         ›");
        assert_eq!(hits.last(), Some(&(Rect::new(28, 0, 2, 1), LibraryFilter::Artists)));
    }

    #[test]
    fn the_active_tab_stays_in_view() {
        let (hits, line) = sidebar_tabs(LibraryFilter::Podcasts);
        assert_eq!(line, "   ‹ Albums  Artists  Podcasts");
        assert_eq!(hits.first(), Some(&(Rect::new(3, 0, 2, 1), LibraryFilter::Playlists)));
    }
}
