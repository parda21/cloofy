//! Playing without the interface, as `cloofy play` does for every service: a line per song
//! and a progress bar under it.

use std::{
    io::Write,
    time::{Duration, Instant},
};

use cloofy_core::player::CurrentTrack;

use crate::style;

const BAR_WIDTH: usize = 40;

pub fn describe(track: &CurrentTrack) -> String {
    format!("{}  {}", style::bold(&track.name), style::dim(&track.artists.join(", ")))
}

pub fn draw_progress(position: Duration, duration: Duration) {
    let ratio = if duration.is_zero() { 0.0 } else { position.as_secs_f64() / duration.as_secs_f64() };
    let filled = ((ratio * BAR_WIDTH as f64).round() as usize).min(BAR_WIDTH);
    print!(
        "\r\x1b[2K  {}  {}{}{}  {}",
        style::dim(&clock(position)),
        style::green(&"━".repeat(filled)),
        style::green("●"),
        style::dim(&"─".repeat(BAR_WIDTH - filled)),
        style::dim(&clock(duration)),
    );
    let _ = std::io::stdout().flush();
}

fn clock(duration: Duration) -> String {
    let secs = duration.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Playback position, extrapolated between player events.
#[derive(Default)]
pub struct Clock {
    base_ms: u32,
    since: Option<Instant>,
}

impl Clock {
    pub fn resume(&mut self, position_ms: u32) {
        self.base_ms = position_ms;
        self.since = Some(Instant::now());
    }

    pub fn pause(&mut self, position_ms: u32) {
        self.base_ms = position_ms;
        self.since = None;
    }

    pub fn position(&self) -> Duration {
        Duration::from_millis(self.base_ms.into()) + self.since.map_or(Duration::ZERO, |s| s.elapsed())
    }
}
