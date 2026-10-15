use std::collections::HashMap;

use cloofy_spotify::catalog::Track;
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Rect, Size},
    style::{Color, Style},
    text::{Line, Span},
};

use unicode_width::UnicodeWidthStr;

use super::text::{self, fit, right};
use crate::{
    app::{App, Focus, HeaderButton, Hit, Link},
    art::LIKED_COVER,
    icons::icons,
    theme::{DANGER, FAINT, MUTED, RAISED, TEXT},
    views::{SearchEntry, SearchView, TrackSource, TrackView, View},
};

/// Rows above a track list: label, title, meta line, gap and column header.
const HEADER_ROWS: u16 = 5;
/// Cover in the header of playlists and albums, when the window is tall enough for it.
const HEADER_COVER: Size = Size { width: 14, height: 7 };
const COVER_HEADER_MIN_HEIGHT: u16 = 22;
const UNFOCUSED_SELECTION: Color = Color::Rgb(25, 25, 31);
/// Search result thumbnails, the size the sidebar uses.
const THUMB: Size = Size { width: 4, height: 2 };
/// Rows per search result with a thumbnail: two lines of text beside it, then a gap.
const RESULT_ROWS: u16 = THUMB.height + 1;
/// Shorter result lists drop the thumbnails and give each result one row.
const RESULT_THUMBS_MIN_HEIGHT: u16 = 3 * RESULT_ROWS;

/// A thumbnail to draw once the buffer is free: slot, cover URL and whether it is round.
type Thumb = (Rect, Option<String>, bool);

/// Colors and state shared by every row of a list.
struct RowStyle<'a> {
    accent: Color,
    selection: Color,
    focused: bool,
    playing_uri: Option<String>,
    playing: bool,
    /// The track on its way to the player.
    loading_uri: Option<String>,
    frame: u64,
    saved: &'a HashMap<String, bool>,
}

impl RowStyle<'_> {
    fn is_saved(&self, uri: &str) -> bool {
        self.saved.get(uri).copied().unwrap_or(false)
    }

    fn heart(&self, uri: &str) -> Span<'static> {
        if self.is_saved(uri) { Span::styled(format!("{} ", icons().saved), self.accent) } else { Span::raw("  ") }
    }
}

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    app.content_area = area;
    let inner = Rect {
        x: area.x + 3,
        y: area.y + 1,
        width: area.width.saturating_sub(6),
        height: area.height.saturating_sub(1),
    };
    if inner.width < 16 || inner.height < 2 {
        return;
    }
    app.hits.push((area, Hit::ContentArea));

    let cover_url = match &app.view {
        View::Tracks(v) if inner.height >= COVER_HEADER_MIN_HEIGHT => Some(match &v.source {
            TrackSource::Liked => Some(LIKED_COVER.to_owned()),
            TrackSource::Playlist(p) => p.image_url.clone(),
            TrackSource::Album(a) => a.image_url.clone(),
        }),
        _ => None,
    };
    if let Some(url) = &cover_url {
        let slot = Rect::new(inner.x, inner.y, HEADER_COVER.width, HEADER_COVER.height);
        if !super::cover(frame, app, url.as_deref(), HEADER_COVER, false, slot) {
            frame.buffer_mut().set_style(slot, Style::new().bg(RAISED));
        }
    }

    // Save button state: own playlists and Liked Songs have none.
    let page_saved = match &app.view {
        View::Tracks(v) => match &v.source {
            TrackSource::Liked => None,
            TrackSource::Playlist(p) if app.me.as_ref().is_some_and(|me| me.id == p.owner_id) => None,
            TrackSource::Playlist(p) => Some(app.is_saved(&p.uri)),
            TrackSource::Album(a) => Some(app.is_saved(&a.uri)),
        },
        _ => None,
    };

    let buf = frame.buffer_mut();
    let style = RowStyle {
        accent: app.accent.color(),
        selection: app.accent.selection(),
        focused: app.focus == Focus::Content,
        playing_uri: app.now.track.as_ref().map(|t| t.uri.clone()),
        playing: app.now.playing,
        loading_uri: app.now.pending.as_ref().and_then(|p| p.uri.clone()),
        frame: app.frame(),
        saved: &app.saved,
    };
    let searching = app.focus == Focus::SearchInput;
    let mut thumbs = Vec::new();
    app.page_height = match &mut app.view {
        View::Tracks(view) => tracks(buf, view, inner, &style, &mut app.hits, cover_url.is_some(), page_saved),
        View::Search(view) => search(buf, view, inner, &style, searching, &mut app.hits, &mut thumbs),
        View::Home(_) | View::Artist(_) | View::Library(_) => 0,
    };
    for (slot, url, round) in thumbs {
        if !super::cover(frame, app, url.as_deref(), THUMB, round, slot) {
            frame.buffer_mut().set_style(slot, Style::new().bg(RAISED));
        }
    }
}

/// Draws a track list view and returns the number of list rows.
fn tracks(
    buf: &mut Buffer,
    view: &mut TrackView,
    area: Rect,
    style: &RowStyle,
    hits: &mut Vec<(Rect, Hit)>,
    with_cover: bool,
    saved: Option<bool>,
) -> usize {
    let count = view.total.unwrap_or(view.tracks.len() as u32);
    let length = if view.loading {
        "…".to_owned()
    } else {
        text::total_duration(view.tracks.iter().map(|t| t.duration_ms as u64).sum())
    };
    let (label, title, meta) = match &view.source {
        TrackSource::Liked => ("PLAYLIST", "Liked Songs".to_owned(), format!("{count} songs · {length}")),
        TrackSource::Playlist(p) => {
            ("PLAYLIST", p.name.clone(), format!("By {} · {count} songs · {length}", p.owner_name))
        }
        TrackSource::Album(a) => {
            ("ALBUM", a.name.clone(), format!("{} · {} · {count} songs · {length}", a.artists.join(", "), a.year))
        }
    };
    if area.height < 12 {
        return track_rows(buf, view, area, style, hits);
    }
    // With a cover the text sits beside it, vertically centered.
    let (text_x, text_y, header_y) = if with_cover {
        (area.x + HEADER_COVER.width + 3, area.y + 1, area.y + HEADER_COVER.height + 1)
    } else {
        (area.x, area.y, area.y + HEADER_ROWS - 1)
    };
    let width = area.right().saturating_sub(text_x) as usize;
    buf.set_string(text_x, text_y, label, Style::new().fg(FAINT).bold());
    buf.set_string(text_x, text_y + 1, text::truncate(&title, width), Style::new().fg(TEXT).bold());
    buf.set_string(text_x, text_y + 2, text::truncate(&meta, width), Style::new().fg(MUTED));
    let mut row = vec![
        (HeaderButton::Play, format!("{} Play", icons().play), true),
        (HeaderButton::Shuffle, format!("{} Shuffle", icons().shuffle), false),
    ];
    if let Some(saved) = saved {
        let label = if saved { format!("{} Saved", icons().saved) } else { format!("{} Save", icons().save) };
        row.push((HeaderButton::Save, label, false));
    }
    row.push((HeaderButton::More, icons().more.to_owned(), false));
    let buttons_y = if with_cover { text_y + 4 } else { text_y + 3 };
    buttons(buf, text_x, buttons_y, &row, style.accent, hits);

    let columns = Columns::new(area.width);
    buf.set_line(area.x, header_y, &columns.header(), area.width);

    let list = Rect { y: header_y + 2, height: area.bottom().saturating_sub(header_y + 2), ..area };
    if view.tracks.is_empty() {
        let message = match (&view.error, view.loading) {
            (Some(err), _) => Span::styled(format!("✕  {err}"), DANGER),
            (None, true) => Span::styled(format!("{}  Loading…", text::spinner(style.frame)), MUTED),
            (None, false) => Span::styled("Nothing here yet.", MUTED),
        };
        buf.set_line(list.x, list.y, &Line::from(message), list.width);
        return list.height as usize;
    }

    track_rows(buf, view, list, style, hits)
}

/// The track list itself; also all there is of a track view in small windows.
fn track_rows(
    buf: &mut Buffer,
    view: &mut TrackView,
    list: Rect,
    style: &RowStyle,
    hits: &mut Vec<(Rect, Hit)>,
) -> usize {
    let columns = Columns::new(list.width);
    let visible = view.list.visible(list.height as usize, view.tracks.len());
    for (row, index) in visible.enumerate() {
        let y = list.y + row as u16;
        highlight(buf, list, y, 1, index == view.list.selected, style);
        let track = &view.tracks[index];
        let line = columns.row(index, track, style);
        buf.set_line(list.x, y, &line, list.width);
        hits.push((Rect { y, height: 1, ..list }, Hit::Row(index)));
        // Artist and album names lead to their pages.
        if columns.artist > 0 {
            let x = list.x + (Columns::INDEX + columns.title + 2) as u16;
            let linked = &track.artists[..track.artist_uris.len().min(track.artists.len())];
            for (rect, artist) in super::name_spans(x, y, linked, columns.artist.saturating_sub(1)) {
                hits.push((rect, Hit::Link(Link::TrackArtist { row: index, artist })));
            }
        }
        if columns.album > 0 && !track.album_uri.is_empty() {
            let x = list.x + (Columns::INDEX + columns.title + 2 + columns.artist + 2) as u16;
            let width = track.album.width().min(columns.album.saturating_sub(1)) as u16;
            hits.push((Rect::new(x, y, width, 1), Hit::Link(Link::TrackAlbum(index))));
        }
    }
    let offset = view.list.offset;
    super::scrollbar(buf, list.right() + 1, list.y, list.height, view.tracks.len(), offset, list.height as usize);
    list.height as usize
}

/// Draws the search results and returns how many fit on a page. Thumbnails are left in
/// `thumbs` for the caller.
fn search(
    buf: &mut Buffer,
    view: &mut SearchView,
    area: Rect,
    style: &RowStyle,
    typing: bool,
    hits: &mut Vec<(Rect, Hit)>,
    thumbs: &mut Vec<Thumb>,
) -> usize {
    let list = area;
    let query = view.query.trim();
    if let Some(err) = &view.error {
        buf.set_string(
            list.x,
            list.y,
            text::truncate(&format!("✕  {err}"), list.width as usize),
            Style::new().fg(DANGER),
        );
        return list.height as usize;
    }
    if query.is_empty() {
        buf.set_string(list.x, list.y, "Search songs, artists, albums and playlists.", Style::new().fg(MUTED));
        buf.set_string(
            list.x,
            list.y + 1,
            "Just start typing above · enter jumps to the results.",
            Style::new().fg(FAINT),
        );
        return list.height as usize;
    }
    if view.entries.is_empty() {
        if !view.loading && view.searched == query {
            let message = format!("No results found for “{query}”.");
            buf.set_string(list.x, list.y, message, Style::new().fg(MUTED));
        }
        return list.height as usize;
    }

    let grid = ResultGrid::new(list.width, list.height >= RESULT_THUMBS_MIN_HEIGHT);
    view.entry_rows = grid.rows;
    let fitting = (list.height / grid.rows) as usize;
    let visible = view.list.visible(fitting, view.entries.len());
    for (slot, index) in visible.enumerate() {
        let y = list.y + slot as u16 * grid.rows;
        let entry = &view.entries[index];
        let Some(result) = Summary::of(entry) else {
            if let SearchEntry::Header(title) = entry {
                // A blank row above each section title, where results have room to breathe.
                let y = if grid.thumbnails { y + 1 } else { y };
                buf.set_string(list.x, y, *title, Style::new().fg(TEXT).bold());
            }
            continue;
        };
        let row = Rect::new(list.x, y, list.width, grid.rows.min(THUMB.height));
        highlight(buf, list, y, row.height, index == view.list.selected && !typing, style);
        hits.push((row, Hit::Row(index)));
        grid.draw(buf, row, index, &result, style, hits, thumbs);
    }
    let offset = view.list.offset;
    super::scrollbar(buf, list.right() + 1, list.y, list.height, view.entries.len(), offset, fitting);
    fitting
}

/// What a search result shows, whatever its kind.
struct Summary<'a> {
    uri: &'a str,
    name: &'a str,
    song: bool,
    /// Under the name, or beside it in one-row results.
    meta: String,
    /// The names in `meta` that lead to artist pages: a song's artists.
    linked: &'a [String],
    /// A song's album, beside its name where there is room, and whether it can be opened.
    album: Option<(&'a str, bool)>,
    /// Right-aligned at the end: a song's length or an album's year.
    field: String,
    icon: &'static str,
    image_url: Option<&'a str>,
    round: bool,
}

impl<'a> Summary<'a> {
    fn of(entry: &'a SearchEntry) -> Option<Self> {
        let icons = icons();
        let plain = |uri: &'a str, name: &'a str, meta: String, icon, image_url: &'a Option<String>| Summary {
            uri,
            name,
            song: false,
            meta,
            linked: &[],
            album: None,
            field: String::new(),
            icon,
            image_url: image_url.as_deref(),
            round: false,
        };
        Some(match entry {
            SearchEntry::Header(_) => return None,
            SearchEntry::Track(t) => Summary {
                song: true,
                linked: &t.artists[..t.artist_uris.len().min(t.artists.len())],
                album: Some((&t.album, !t.album_uri.is_empty())),
                field: text::duration(t.duration_ms),
                ..plain(&t.uri, &t.name, t.artists.join(", "), icons.note, &t.image_url)
            },
            SearchEntry::Artist(a) => {
                Summary { round: true, ..plain(&a.uri, &a.name, "Artist".into(), icons.artist, &a.image_url) }
            }
            SearchEntry::Album(a) => Summary {
                field: a.year.clone(),
                ..plain(&a.uri, &a.name, a.artists.join(", "), icons.album, &a.image_url)
            },
            SearchEntry::Playlist(p) => {
                let meta = format!("By {} · {} songs", p.owner_name, p.total);
                plain(&p.uri, &p.name, meta, icons.playlist, &p.image_url)
            }
        })
    }
}

/// The column grid every search result follows, so names, details, hearts and the right
/// edge line up from one section to the next: a lead (thumbnail or icon), the name, a second
/// column and a tail with the heart and a short right-aligned field.
struct ResultGrid {
    thumbnails: bool,
    /// Screen rows per result.
    rows: u16,
    lead: u16,
    name: usize,
    /// With thumbnails a song's album, else the meta line; 0 when there is no room.
    second: usize,
}

impl ResultGrid {
    /// Heart plus a five-column field.
    const TAIL: usize = 7;

    fn new(width: u16, thumbnails: bool) -> Self {
        let lead = if thumbnails { THUMB.width + 2 } else { 5 };
        let text = (width as usize).saturating_sub(lead as usize + 2 + Self::TAIL);
        // With thumbnails the meta line sits under the name, so the second column only holds
        // a song's album and is worth it in wide windows only.
        let name = if thumbnails && width < 80 { text } else { text * 55 / 100 };
        let second = text.saturating_sub(name + 2);
        Self { thumbnails, rows: if thumbnails { RESULT_ROWS } else { 1 }, lead, name, second }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        buf: &mut Buffer,
        row: Rect,
        index: usize,
        result: &Summary,
        style: &RowStyle,
        hits: &mut Vec<(Rect, Hit)>,
        thumbs: &mut Vec<Thumb>,
    ) {
        let (x, y) = (row.x + self.lead, row.y);
        let second_x = x + (self.name + 2) as u16;
        if self.thumbnails {
            let slot = Rect::new(row.x, y, THUMB.width, THUMB.height);
            thumbs.push((slot, result.image_url.map(str::to_owned), result.round));
        } else {
            // Songs get a quiet note; the other kinds stand out.
            let color = if result.song { FAINT } else { style.accent };
            buf.set_string(row.x + 2, y, result.icon, Style::new().fg(color));
        }

        // The playing song shows in the accent color with a moving equalizer, as in the sidebar.
        let current = style.playing_uri.as_deref() == Some(result.uri);
        let name = text::truncate(result.name, self.name.saturating_sub(if current { 5 } else { 0 }));
        let name_style = if current { Style::new().fg(style.accent).bold() } else { Style::new().fg(TEXT) };
        buf.set_string(x, y, &name, name_style);
        if current {
            let marker = if style.playing { text::equalizer(style.frame) } else { icons().volume };
            buf.set_string(x + name.width() as u16 + 2, y, marker, Style::new().fg(style.accent));
        }

        let (meta_x, meta_y, meta_width) =
            if self.thumbnails { (x, y + 1, self.name + 2 + self.second) } else { (second_x, y, self.second) };
        if meta_width > 0 {
            buf.set_string(meta_x, meta_y, text::truncate(&result.meta, meta_width), Style::new().fg(MUTED));
            for (rect, artist) in super::name_spans(meta_x, meta_y, result.linked, meta_width.saturating_sub(1)) {
                hits.push((rect, Hit::Link(Link::TrackArtist { row: index, artist })));
            }
        }
        if let Some((album, linked)) = result.album
            && self.thumbnails
            && self.second > 0
        {
            buf.set_string(second_x, y, text::truncate(album, self.second), Style::new().fg(MUTED));
            if linked {
                let width = album.width().min(self.second.saturating_sub(1)) as u16;
                hits.push((Rect::new(second_x, y, width, 1), Hit::Link(Link::TrackAlbum(index))));
            }
        }

        let tail = Line::from(vec![style.heart(result.uri), Span::styled(right(&result.field, 5), FAINT)]);
        buf.set_line(row.right().saturating_sub(Self::TAIL as u16), y, &tail, Self::TAIL as u16);
    }
}

/// A row of pill buttons; the primary one is filled with the accent color.
pub(super) fn buttons(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    row: &[(HeaderButton, String, bool)],
    accent: Color,
    hits: &mut Vec<(Rect, Hit)>,
) {
    let mut x = x;
    for (button, label, primary) in row {
        let text = format!(" {label} ");
        let width = text.width() as u16;
        let style = if *primary {
            Style::new().bg(accent).fg(crate::theme::BG).bold()
        } else {
            Style::new().bg(RAISED).fg(TEXT)
        };
        buf.set_string(x, y, &text, style);
        hits.push((Rect::new(x, y, width, 1), Hit::Header(*button)));
        x += width + 2;
    }
}

/// Selection background plus the accent marker in the left padding, over `height` rows.
fn highlight(buf: &mut Buffer, list: Rect, y: u16, height: u16, selected: bool, style: &RowStyle) {
    if !selected {
        return;
    }
    let bg = if style.focused { style.selection } else { UNFOCUSED_SELECTION };
    buf.set_style(Rect::new(list.x.saturating_sub(1), y, list.width + 2, height), Style::new().bg(bg));
    if style.focused {
        for row in y..y + height {
            buf.set_string(list.x.saturating_sub(2), row, "▌", Style::new().fg(style.accent));
        }
    }
}

/// Column widths of a track list; narrower windows drop the album, then the artist.
struct Columns {
    title: usize,
    artist: usize,
    album: usize,
}

impl Columns {
    const INDEX: usize = 5;
    /// Heart plus duration.
    const TAIL: usize = 9;

    fn new(width: u16) -> Self {
        let width = width as usize;
        let flexible = width.saturating_sub(Self::INDEX + Self::TAIL);
        if width >= 100 {
            let flexible = flexible.saturating_sub(4);
            let title = flexible * 42 / 100;
            let artist = flexible * 28 / 100;
            Self { title, artist, album: flexible - title - artist }
        } else if width >= 64 {
            let flexible = flexible.saturating_sub(2);
            let title = flexible * 58 / 100;
            Self { title, artist: flexible - title, album: 0 }
        } else {
            Self { title: flexible, artist: 0, album: 0 }
        }
    }

    fn header(&self) -> Line<'static> {
        let style = Style::new().fg(FAINT).bold();
        let mut spans =
            vec![Span::styled(fit("  #", Self::INDEX), style), Span::styled(fit("TITLE", self.title), style)];
        if self.artist > 0 {
            spans.push(Span::styled(format!("  {}", fit("ARTIST", self.artist)), style));
        }
        if self.album > 0 {
            spans.push(Span::styled(format!("  {}", fit("ALBUM", self.album)), style));
        }
        spans.push(Span::styled(right("◷", Self::TAIL - 2), style));
        Line::from(spans)
    }

    fn row(&self, index: usize, track: &Track, style: &RowStyle) -> Line<'static> {
        let is_current = style.playing_uri.as_deref() == Some(track.uri.as_str());
        let number = if style.loading_uri.as_deref() == Some(track.uri.as_str()) {
            Span::styled(fit(&right(text::spinner(style.frame), 3), Self::INDEX), style.accent)
        } else if !is_current {
            Span::styled(fit(&right(&(index + 1).to_string(), 3), Self::INDEX), FAINT)
        } else if style.playing {
            Span::styled(fit(&format!(" {}", text::equalizer(style.frame)), Self::INDEX), style.accent)
        } else {
            Span::styled(fit("  ▶", Self::INDEX), style.accent)
        };
        let title_style = if is_current { Style::new().fg(style.accent).bold() } else { Style::new().fg(TEXT) };
        let mut spans = vec![number, Span::styled(fit(&track.name, self.title), title_style)];
        if self.artist > 0 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(fit(&track.artists.join(", "), self.artist), MUTED));
        }
        if self.album > 0 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(fit(&track.album, self.album), MUTED));
        }
        spans.push(Span::raw("  "));
        spans.push(style.heart(&track.uri));
        spans.push(Span::styled(right(&text::duration(track.duration_ms), 5), FAINT));
        Line::from(spans)
    }
}
