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
pub(super) fn tabs(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    right: u16,
    filters: &[LibraryFilter],
    active: LibraryFilter,
    accent: Color,
) -> Vec<(Rect, LibraryFilter)> {
    let mut x = x;
    let mut hits = Vec::new();
    for &filter in filters {
        let label = filter.label();
        let width = label.width() as u16;
        if x + width > right {
            break;
        }
        let style = if filter == active {
            Style::new().fg(TEXT).bold().underlined().underline_color(accent)
        } else {
            Style::new().fg(MUTED)
        };
        buf.set_string(x, y, label, style);
        hits.push((Rect::new(x, y, width, 1), filter));
        x += width + 3;
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
    let width = area.right().saturating_sub(x + if playing { 4 } else { 1 }) as usize;
    let title_style = match (playing, open) {
        (true, _) => Style::new().fg(app.accent.color()).bold(),
        (false, true) => Style::new().fg(TEXT).bold(),
        (false, false) => Style::new().fg(TEXT),
    };
    let equalizer = if app.now.playing { text::equalizer(app.frame()) } else { icons().volume };
    let buf = frame.buffer_mut();
    buf.set_string(x, y, text::truncate(&title, width), title_style);
    if playing {
        buf.set_string(area.right().saturating_sub(3), y, equalizer, Style::new().fg(app.accent.color()));
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
