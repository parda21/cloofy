use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use super::text;
use crate::{
    app::{App, COVER_SIZE, Hit, Link},
    commands::Command,
    icons::icons,
    playback::Repeat,
    theme::{DANGER, FAINT, MUTED, PLAYER, RAISED, TEXT},
};

const VOLUME_CELLS: u16 = 10;

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    frame.buffer_mut().set_style(area, Style::new().bg(PLAYER));
    if area.height < 4 || area.width < 40 {
        return compact(frame.buffer_mut(), app, area);
    }
    let cover = Rect::new(area.x + 2, area.y + 1, COVER_SIZE.width, COVER_SIZE.height);
    let placed = app.now.cover.as_ref().map(|c| super::picture_at(frame, &c.picture, cover));
    match placed {
        Some(placement) => app.placements.extend(placement),
        None => {
            let buf = frame.buffer_mut();
            buf.set_style(cover, Style::new().bg(RAISED));
            buf.set_string(
                cover.x + cover.width / 2 - 1,
                cover.y + cover.height / 2,
                icons().note,
                Style::new().fg(FAINT),
            );
        }
    }
    app.hits.push((cover, Hit::Cover));

    let x = cover.right() + 3;
    let info = Rect::new(x, area.y + 1, area.right().saturating_sub(x + 2), 3);
    let buf = frame.buffer_mut();
    let mut hits = Vec::new();
    match &app.now.track {
        Some(track) => {
            let accent = app.accent.color();
            let (controls, targets) = controls(app, accent);
            let controls_width = controls.width() as u16;
            let controls_x = info.right().saturating_sub(controls_width);
            let volume_width = VOLUME_CELLS + 7;
            let volume_x = info.right().saturating_sub(volume_width);

            let title_width = info.width.saturating_sub(controls_width + 7) as usize;
            let title = text::truncate(&track.name, title_width);
            buf.set_string(info.x, info.y, &title, Style::new().fg(TEXT).bold());
            let heart_x = info.x + title.width() as u16 + 2;
            let (heart, heart_style) = if app.is_saved(&track.uri) {
                (icons().saved, Style::new().fg(accent))
            } else {
                (icons().save, Style::new().fg(FAINT))
            };
            buf.set_string(heart_x, info.y, heart, heart_style);
            let note_x = heart_x + 4;
            let room = controls_x.saturating_sub(note_x + 3) as usize;
            if app.can_resume() {
                let hint = "Last played · space to resume";
                if hint.width() <= room {
                    buf.set_string(note_x, info.y, hint, Style::new().fg(accent));
                }
            } else if let Some(label) = app.playing_context().and_then(|uri| app.context_label(uri))
                && room > 12
            {
                // Where the music comes from, like the official client's "Playing from".
                let label = text::truncate(&label, room - 3);
                buf.set_string(note_x, info.y, format!("{}  {label}", icons().playlist), Style::new().fg(FAINT));
                let width = label.width() as u16 + 3;
                hits.push((Rect::new(note_x, info.y, width, 1), Hit::Link(Link::PlayingContext)));
            }
            hits.push((Rect::new(heart_x, info.y, 1, 1), Hit::LikePlaying));
            buf.set_line(controls_x, info.y, &controls, controls_width);
            hits.extend(targets.into_iter().map(|(offset, width, command)| {
                (Rect::new(controls_x + offset, info.y, width, 1), Hit::Command(command))
            }));

            let by = [track.artists.join(", "), track.album.clone()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let by_width = info.width.saturating_sub(volume_width + 4) as usize;
            buf.set_string(info.x, info.y + 1, text::truncate(&by, by_width), Style::new().fg(MUTED));
            let linked = &track.artists[..track.artist_uris.len().min(track.artists.len())];
            for (rect, artist) in super::name_spans(info.x, info.y + 1, linked, by_width.saturating_sub(1)) {
                hits.push((rect, Hit::Link(Link::PlayingArtist(artist))));
            }
            let album_x = track.artists.join(", ").width() + 3;
            if track.album_uri.is_some() && !track.artists.is_empty() && album_x + 1 < by_width {
                let width = track.album.width().min(by_width - album_x - 1) as u16;
                hits.push((Rect::new(info.x + album_x as u16, info.y + 1, width, 1), Hit::Link(Link::PlayingAlbum)));
            }
            buf.set_line(volume_x, info.y + 1, &volume(app.now.volume, accent), volume_width);
            hits.push((
                Rect::new(volume_x + 2, info.y + 1, VOLUME_CELLS, 1),
                Hit::Volume { x: volume_x + 2, width: VOLUME_CELLS },
            ));

            let bar = Rect { y: info.y + 2, height: 1, ..info };
            hits.extend(progress(
                buf,
                bar,
                app.seek_preview.unwrap_or(app.now.position_ms()),
                track.duration_ms,
                accent,
            ));
        }
        None if app.is_connected() => {
            buf.set_string(info.x, info.y, "Nothing playing", Style::new().fg(TEXT).bold());
            let hint = "Pick a song and press ↵ · ␣ takes over playback from your other device";
            buf.set_string(info.x, info.y + 1, text::truncate(hint, info.width as usize), Style::new().fg(MUTED));
        }
        None => match &app.player_error {
            Some(err) => {
                buf.set_string(info.x, info.y, "Playback unavailable", Style::new().fg(DANGER).bold());
                buf.set_string(info.x, info.y + 1, text::truncate(err, info.width as usize), Style::new().fg(MUTED));
            }
            None => {
                let line = format!("{}  Connecting to Spotify…", text::spinner(app.frame()));
                buf.set_string(info.x, info.y, line, Style::new().fg(MUTED));
            }
        },
    }
    app.hits.extend(hits);
}

/// Small windows: title and controls in one row, the progress bar below if there is room.
fn compact(buf: &mut Buffer, app: &mut App, area: Rect) {
    let Some(track) = &app.now.track else {
        buf.set_string(area.x + 2, area.y, "Nothing playing", Style::new().fg(MUTED));
        return;
    };
    let accent = app.accent.color();
    let (controls, targets) = controls(app, accent);
    let controls_width = controls.width() as u16;
    let show_controls = area.width >= controls_width + 24;
    let text_width = area.width.saturating_sub(if show_controls { controls_width + 6 } else { 4 }) as usize;
    let title = format!("{} · {}", track.name, track.artists.join(", "));
    buf.set_string(area.x + 2, area.y, text::truncate(&title, text_width), Style::new().fg(TEXT).bold());
    let mut hits = Vec::new();
    if show_controls {
        let x = area.right().saturating_sub(controls_width + 2);
        buf.set_line(x, area.y, &controls, controls_width);
        hits.extend(
            targets
                .into_iter()
                .map(|(offset, width, command)| (Rect::new(x + offset, area.y, width, 1), Hit::Command(command))),
        );
    }
    if area.height >= 2 {
        let bar = Rect::new(area.x + 2, area.y + 1, area.width.saturating_sub(4), 1);
        hits.extend(progress(buf, bar, app.seek_preview.unwrap_or(app.now.position_ms()), track.duration_ms, accent));
    }
    app.hits.extend(hits);
}

/// Transport controls plus the click targets within the line (offset, width, command).
pub(super) fn controls(app: &App, accent: Color) -> (Line<'static>, Vec<(u16, u16, Command)>) {
    const GAP: u16 = 3;
    let icons = icons();
    let toggle = |on: bool| if on { Style::new().fg(accent) } else { Style::new().fg(FAINT) };
    let repeat = if app.now.repeat == Repeat::Track { icons.repeat_one } else { icons.repeat };
    // A track on its way spins in place of play/pause.
    let play = match (&app.now.pending, app.now.playing) {
        (Some(_), _) => text::spinner(app.frame()),
        (None, true) => icons.pause,
        (None, false) => icons.play,
    };
    let parts = [
        (icons.shuffle, toggle(app.now.shuffle), Command::Shuffle),
        (icons.previous, Style::new().fg(MUTED), Command::Previous),
        (
            play,
            // The play button glows while it would continue the last session.
            if app.can_resume() { Style::new().fg(accent).bold() } else { Style::new().fg(TEXT).bold() },
            Command::PlayPause,
        ),
        (icons.next, Style::new().fg(MUTED), Command::Next),
        (repeat, toggle(app.now.repeat != Repeat::Off), Command::Repeat),
    ];
    let mut spans = Vec::new();
    let mut targets = Vec::new();
    let mut x = 0;
    for (index, (glyph, style, command)) in parts.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" ".repeat(GAP as usize)));
            x += GAP;
        }
        let width = glyph.width() as u16;
        spans.push(Span::styled(glyph, style));
        // A little slack around each glyph makes them easier to hit.
        targets.push((x.saturating_sub(1), width + 2, command));
        x += width;
    }
    (Line::from(spans), targets)
}

fn volume(volume: u16, accent: Color) -> Line<'static> {
    let filled = ((volume as f32 / u16::MAX as f32) * VOLUME_CELLS as f32).round() as usize;
    Line::from(vec![
        Span::styled(format!("{} ", icons().volume), FAINT),
        Span::styled("━".repeat(filled), accent),
        Span::styled("━".repeat(VOLUME_CELLS as usize - filled), RAISED),
        Span::styled(format!(" {:>3}%", (volume as u32 * 100 / u16::MAX as u32)), FAINT),
    ])
}

/// Draws `1:42 ━━━●──── 3:58` and returns the seek target of the bar.
pub(super) fn progress(
    buf: &mut Buffer,
    area: Rect,
    position_ms: u32,
    duration_ms: u32,
    accent: Color,
) -> Option<(Rect, Hit)> {
    let elapsed = text::duration(position_ms);
    let total = text::duration(duration_ms);
    let bar_x = area.x + elapsed.width() as u16 + 2;
    let bar_width = (area.width as usize).saturating_sub(elapsed.width() + total.width() + 4);
    if bar_width < 4 {
        return None;
    }
    let ratio = if duration_ms == 0 { 0.0 } else { position_ms as f64 / duration_ms as f64 };
    let filled = ((ratio * bar_width as f64) as usize).min(bar_width - 1);
    let line = Line::from(vec![
        Span::styled(format!("{elapsed}  "), MUTED),
        Span::styled("━".repeat(filled), accent),
        Span::styled("●", Style::new().fg(TEXT)),
        Span::styled("─".repeat(bar_width - filled - 1), FAINT),
        Span::styled(format!("  {total}"), MUTED),
    ]);
    buf.set_line(area.x, area.y, &line, area.width);
    let width = bar_width as u16;
    Some((Rect::new(bar_x, area.y, width, 1), Hit::Seek { x: bar_x, width }))
}
