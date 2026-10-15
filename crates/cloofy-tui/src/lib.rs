mod app;
mod art;
mod commands;
mod icons;
mod loader;
mod overlay;
mod playback;
mod snapshot;
mod state;
mod theme;
mod ui;
mod views;

use std::{
    io::{BufWriter, Stdout, Write},
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, bail};
use cloofy_spotify::{config::Config, paths::Paths, session, webapi::WebApi};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, EventStream},
    terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate},
};
use futures::{FutureExt, StreamExt};
use ratatui::{Terminal, backend::CrosstermBackend, buffer::Buffer};
use ratatui_image::picker::Picker;
use tokio::{
    sync::{mpsc, watch},
    time::MissedTickBehavior,
};

use crate::{
    app::{App, AppEvent},
    loader::{DataEvent, Loader},
};

/// Timer resolution for animations and debouncing; draws only happen when something changed.
const TICK: Duration = Duration::from_millis(25);
/// Most loaded images taken in per frame. They arrive by the dozen when a page opens, and the
/// frame sends the pixels of each new one, which holds up input until it is written.
const IMAGES_PER_FRAME: usize = 12;

/// Frames are collected in a buffer and written in one go, instead of in the 1 KiB pieces
/// stdout's own buffer would send.
type Term = Terminal<CrosstermBackend<BufWriter<Stdout>>>;

pub async fn run(paths: &Paths) -> Result<()> {
    let config = Config::load(paths)?;
    let api = Arc::new(WebApi::new(config.client_id(paths)?, paths.webapi_token())?);
    if !api.is_logged_in().await || !session::is_logged_in(paths)? {
        bail!("Not logged in yet – run `cloofy login` first.");
    }

    #[cfg(unix)]
    let _stderr = StderrToLog::redirect(&paths.log_file());
    icons::init(config.icons.as_deref());
    // Raw mode, the alternate screen and the panic hook; frames go through `terminal` below.
    let _ = ratatui::init();
    let mut terminal = Terminal::new(CrosstermBackend::new(BufWriter::with_capacity(64 * 1024, std::io::stdout())))?;
    // Must run after entering the alternate screen and before reading input events.
    let picker = image_picker();
    crossterm::execute!(std::io::stdout(), EnableMouseCapture)?;
    let crossfade = config.crossfade.unwrap_or(0).min(12);
    let result = run_app(&mut terminal, paths, api, picker, crossfade).await;
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

async fn run_app(terminal: &mut Term, paths: &Paths, api: Arc<WebApi>, picker: Picker, crossfade: u64) -> Result<()> {
    let (tx, mut events) = mpsc::unbounded_channel();
    let (session_tx, session_rx) = watch::channel(None);
    let snapshotter = snapshot::Snapshotter::from_env().map(Arc::new);
    let loader = Loader::new(
        api,
        tx.clone(),
        session_rx,
        picker,
        paths.pathfinder_hashes(),
        paths.cache.join("images"),
        snapshotter.is_some(),
    );
    loader.prune_image_cache();
    let notices = tx.clone();
    playback::spawn(paths.clone(), Duration::from_secs(crossfade), tx);
    let saved = state::load(&paths.state_file());
    let mut app = App::new(loader, session_tx, crossfade, saved, paths.state_file());

    let mut input = EventStream::new();
    let mut closed = shutdown_signals()?;
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    draw(terminal, &mut app, false)?;
    while !app.quit {
        let redraw = tokio::select! {
            // Input first, so a key or click never waits behind loaded data.
            biased;
            event = input.next() => match event {
                Some(event) => {
                    // Handle everything already queued (a fast scroll sends dozens) before drawing once.
                    let mut redraw = app.on_input(event?);
                    while let Some(Some(event)) = input.next().now_or_never() {
                        redraw |= app.on_input(event?);
                    }
                    redraw
                }
                None => break,
            },
            _ = closed.recv() => break,
            Some(event) = events.recv() => {
                // Likewise handle what else has arrived, such as a burst of covers, before
                // drawing once.
                let mut redraw = app.on_event(event);
                let mut images = 0;
                while images < IMAGES_PER_FRAME
                    && let Ok(event) = events.try_recv()
                {
                    images += usize::from(matches!(event, AppEvent::Data(DataEvent::Image { .. })));
                    redraw |= app.on_event(event);
                }
                redraw
            }
            _ = ticker.tick() => app.on_tick(),
        };
        if redraw || app.snapshot_requested {
            let snapshot = std::mem::take(&mut app.snapshot_requested) && snapshotter.is_some();
            if let Some(buffer) = draw(terminal, &mut app, snapshot)?
                && let Some(snapshotter) = &snapshotter
            {
                let placements = app.placements.clone();
                let (snapshotter, notices) = (snapshotter.clone(), notices.clone());
                tokio::task::spawn_blocking(move || {
                    let text = match snapshotter.save(&buffer, &placements) {
                        Ok(path) => format!("Snapshot: {}", path.display()),
                        Err(err) => format!("Snapshot failed: {err:#}"),
                    };
                    let _ = notices.send(app::AppEvent::Data(loader::DataEvent::Notice(text)));
                });
            }
        }
    }
    app.shutdown();
    Ok(())
}

/// Draws a frame as one synchronized update, so the terminal shows it whole rather than half
/// painted. Returns the frame when `snapshot` asks for it.
fn draw(terminal: &mut Term, app: &mut App, snapshot: bool) -> Result<Option<Buffer>> {
    crossterm::queue!(terminal.backend_mut(), BeginSynchronizedUpdate)?;
    let frame = terminal.draw(|frame| ui::draw(frame, app))?;
    let buffer = snapshot.then(|| frame.buffer.clone());
    // Images drawn for the first time send their pixels now. The upload does not move the
    // cursor, and the terminal fills in cells that wait for the image once it arrives.
    for placement in &mut app.placements {
        if let Some(upload) = placement.upload.take() {
            terminal.backend_mut().write_all(upload.as_bytes())?;
        }
    }
    crossterm::execute!(terminal.backend_mut(), EndSynchronizedUpdate)?;
    Ok(buffer)
}

/// Fires when the terminal window is closed (SIGHUP) or cloofy is asked to stop (SIGTERM),
/// so the session still gets saved.
#[cfg(unix)]
fn shutdown_signals() -> Result<mpsc::Receiver<()>> {
    use tokio::signal::unix::{SignalKind, signal};
    let (tx, rx) = mpsc::channel(1);
    for kind in [SignalKind::hangup(), SignalKind::terminate()] {
        let mut stream = signal(kind)?;
        let tx = tx.clone();
        tokio::spawn(async move {
            stream.recv().await;
            let _ = tx.send(()).await;
        });
    }
    Ok(rx)
}

#[cfg(not(unix))]
fn shutdown_signals() -> Result<mpsc::Receiver<()>> {
    let (tx, rx) = mpsc::channel(1);
    std::mem::forget(tx);
    Ok(rx)
}

/// Sends everything written to stderr into the log file while the TUI owns the screen.
/// Audio libraries print errors there, which would otherwise draw over the interface.
#[cfg(unix)]
struct StderrToLog {
    saved: libc::c_int,
}

#[cfg(unix)]
impl StderrToLog {
    fn redirect(path: &std::path::Path) -> Option<Self> {
        use std::os::fd::AsRawFd;
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
        // SAFETY: only duplicates file descriptors; fd 2 keeps the file open after `file` drops.
        unsafe {
            let saved = libc::dup(libc::STDERR_FILENO);
            if saved < 0 {
                return None;
            }
            if libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) < 0 {
                libc::close(saved);
                return None;
            }
            Some(Self { saved })
        }
    }
}

#[cfg(unix)]
impl Drop for StderrToLog {
    fn drop(&mut self) {
        // SAFETY: restores the descriptor saved in `redirect`.
        unsafe {
            libc::dup2(self.saved, libc::STDERR_FILENO);
            libc::close(self.saved);
        }
    }
}

/// Hands memory freed inside the allocator back to the system, so the process shrinks after
/// dropping caches.
pub(crate) fn release_freed_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim only releases free pages; it is safe to call at any time.
    unsafe {
        libc::malloc_trim(0);
    }
}

/// Asks the terminal how it shows images. `CLOOFY_IMAGES=kitty` forces the Kitty protocol, for
/// measuring it where the terminal cannot answer (tmux in tests).
fn image_picker() -> Picker {
    if std::env::var("CLOOFY_IMAGES").as_deref() == Ok("kitty") {
        // Deprecated because a guessed font size draws images at the wrong size; fine here.
        #[allow(deprecated)]
        let mut picker = Picker::from_fontsize(ratatui_image::FontSize { width: 10, height: 22 });
        picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
        return picker;
    }
    let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    let font = picker.font_size();
    tracing::info!("images: {:?}, cells of {}x{} px", picker.protocol_type(), font.width, font.height);
    picker
}
