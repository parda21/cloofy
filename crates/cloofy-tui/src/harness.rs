//! Drives the whole interface the way someone at the keyboard does, without a terminal: keys and
//! clicks go in, frames are drawn into a buffer, and the screen comes out as text.
//!
//! Every input is followed by [`Harness::settle`], which hands the app what its background tasks
//! send and draws until nothing is left to do. The clock stands still unless [`Harness::wait`]
//! moves it on, so screens come out the same on every run.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use cloofy_core::paths::Paths;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{App, AppEvent, Start},
    backend::Service,
    clock,
    demo::Demo,
    graphics::ImageTerminal,
    loader::Loader,
    preferences::Preferences,
    ui,
};

/// How long [`Harness::settle`] waits for the app before failing the test.
const SETTLE_LIMIT: Duration = Duration::from_secs(10);

pub struct Harness {
    pub app: App,
    events: UnboundedReceiver<AppEvent>,
    terminal: Terminal<TestBackend>,
    root: PathBuf,
}

impl Harness {
    /// `cloofy demo` in a 120×36 window, with its first screen drawn.
    pub async fn demo() -> Self {
        Self::demo_sized(120, 36).await
    }

    pub async fn demo_sized(width: u16, height: u16) -> Self {
        static RUNS: AtomicUsize = AtomicUsize::new(0);
        let run = RUNS.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("cloofy-harness-{}-{run}", std::process::id()));
        let paths =
            Paths { config: root.join("config"), data: root.join("data"), cache: root.join("cache"), keychain: false };
        let demo = Arc::new(Demo::sample().expect("the demo sample loads"));
        let (tx, events) = mpsc::unbounded_channel();
        let service = Service::of(&demo.playing.track.uri);
        let images = paths.cache.join("images");
        let loader = Loader::new(Vec::new(), service, tx, ImageTerminal::halfblocks(), images, false)
            .with_demo(Some(demo.clone()));
        let app = App::new(loader, Preferences::default(), Start::Demo(&demo), paths, None, None);
        let terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        let mut harness = Self { app, events, terminal, root };
        harness.settle().await;
        harness
    }

    /// Presses `code`.
    pub async fn press(&mut self, code: KeyCode) -> &mut Self {
        self.input(Event::Key(KeyEvent::new(code, KeyModifiers::NONE))).await
    }

    /// Presses `c` with Ctrl held.
    pub async fn ctrl(&mut self, c: char) -> &mut Self {
        self.input(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))).await
    }

    /// Types `text`, a key per character.
    pub async fn type_text(&mut self, text: &str) -> &mut Self {
        for c in text.chars() {
            self.press(KeyCode::Char(c)).await;
        }
        self
    }

    /// Clicks the first place on screen that shows `text`.
    pub async fn click_on(&mut self, text: &str) -> &mut Self {
        let (column, row) = self.find(text).unwrap_or_else(|| panic!("{text:?} isn't on screen:\n{}", self.screen()));
        for kind in [MouseEventKind::Down(MouseButton::Left), MouseEventKind::Up(MouseButton::Left)] {
            let mouse = MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE };
            self.input(Event::Mouse(mouse)).await;
        }
        self
    }

    /// Resizes the window.
    pub async fn resize(&mut self, width: u16, height: u16) -> &mut Self {
        self.terminal.backend_mut().resize(width, height);
        self.input(Event::Resize(width, height)).await
    }

    /// Lets `duration` pass, ticking as the app's timer does.
    pub async fn wait(&mut self, duration: Duration) -> &mut Self {
        const TICK: Duration = Duration::from_millis(50);
        let mut left = duration;
        while !left.is_zero() {
            let step = left.min(TICK);
            clock::advance(step);
            left -= step;
            self.app.on_tick();
        }
        self.settle().await;
        self
    }

    async fn input(&mut self, event: Event) -> &mut Self {
        self.app.on_input(event);
        self.settle().await;
        self
    }

    /// Hands the app everything its tasks send, drawing after each round, until no task is left
    /// and a frame asks for nothing new.
    pub async fn settle(&mut self) {
        let started = std::time::Instant::now();
        loop {
            let mut handled = false;
            while let Ok(event) = self.events.try_recv() {
                self.app.on_event(event);
                handled = true;
            }
            self.redraw();
            // A tick after the frame sends for what its layout asked, like the large cover.
            self.app.on_tick();
            if !handled && self.events.is_empty() && self.app.is_idle() {
                self.redraw();
                return;
            }
            assert!(started.elapsed() < SETTLE_LIMIT, "Still busy after {SETTLE_LIMIT:?}:\n{}", self.screen());
            // Real time, for downloads and the blocking pool.
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    /// Draws a frame, as the app does whenever something may have changed.
    pub fn redraw(&mut self) {
        self.terminal.draw(|frame| ui::draw(frame, &mut self.app)).expect("a frame");
        // Image uploads and escape codes for a real terminal.
        for placement in &mut self.app.placements {
            placement.upload = None;
        }
        self.app.terminal_out.clear();
    }

    /// The screen as text, a line per row, without trailing spaces.
    pub fn screen(&self) -> String {
        let buffer = self.terminal.backend().buffer();
        let mut screen = String::new();
        for y in 0..buffer.area.height {
            let mut line = String::new();
            let mut skip = 0;
            for x in 0..buffer.area.width {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let symbol = buffer[(x, y)].symbol();
                skip = symbol.width().saturating_sub(1);
                line.push_str(symbol);
            }
            screen.push_str(line.trim_end());
            screen.push('\n');
        }
        screen
    }

    /// The column and row where `text` first shows.
    pub fn find(&self, text: &str) -> Option<(u16, u16)> {
        self.screen().lines().enumerate().find_map(|(row, line)| {
            let at = line.find(text)?;
            Some((line[..at].width() as u16, row as u16))
        })
    }

    pub fn shows(&self, text: &str) -> bool {
        self.find(text).is_some()
    }

    /// Compares the screen with `tests/screens/<name>.txt`. `CLOOFY_UPDATE_SCREENS=1` writes the
    /// screen there instead, as does a first run.
    #[track_caller]
    pub fn assert_screen(&self, name: &str) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/screens").join(format!("{name}.txt"));
        let screen = self.screen();
        let update = std::env::var_os("CLOOFY_UPDATE_SCREENS").is_some_and(|v| v != "0");
        let saved = std::fs::read_to_string(&path).ok();
        if update || saved.is_none() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &screen).unwrap();
            assert!(update, "New screen {name}, saved to {}; check it and run again", path.display());
            return;
        }
        let saved = saved.unwrap();
        if saved != screen {
            panic!("Screen {name} differs; CLOOFY_UPDATE_SCREENS=1 takes the new one.\n{}", diff(&saved, &screen));
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The rows that differ, before and after.
fn diff(saved: &str, screen: &str) -> String {
    let (saved, screen): (Vec<_>, Vec<_>) = (saved.lines().collect(), screen.lines().collect());
    let mut out = String::new();
    for row in 0..saved.len().max(screen.len()) {
        let (before, after) = (saved.get(row).copied(), screen.get(row).copied());
        if before != after {
            out.push_str(&format!("{row:>3} - {}\n{row:>3} + {}\n", before.unwrap_or(""), after.unwrap_or("")));
        }
    }
    out
}
