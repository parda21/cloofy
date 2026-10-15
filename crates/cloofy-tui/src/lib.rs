mod app;
mod art;
mod commands;
mod icons;
mod loader;
mod media;
mod overlay;
mod playback;
mod snapshot;
mod state;
mod theme;
mod ui;
mod views;

use std::{sync::Arc, time::Duration};

use anyhow::{Result, bail};
use cloofy_spotify::{config::Config, paths::Paths, session, webapi::WebApi};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, EventStream};
use futures::{FutureExt, StreamExt};
use ratatui::DefaultTerminal;
use ratatui_image::picker::Picker;
use tokio::{
    sync::{mpsc, watch},
    time::MissedTickBehavior,
};

use crate::{app::App, loader::Loader};

/// Timer resolution for animations and debouncing; draws only happen when something changed.
const TICK: Duration = Duration::from_millis(25);

pub async fn run(paths: &Paths) -> Result<()> {
    let config = Config::load(paths)?;
    let api = Arc::new(WebApi::new(config.client_id(paths)?, paths.webapi_token())?);
    if !api.is_logged_in().await || !session::is_logged_in(paths)? {
        bail!("Not logged in yet – run `cloofy login` first.");
    }

    #[cfg(unix)]
    let _stderr = StderrToLog::redirect(&paths.log_file());
    icons::init(config.icons.as_deref());
    let mut terminal = ratatui::init();
    // Must run after entering the alternate screen and before reading input events.
    let picker = image_picker();
    crossterm::execute!(std::io::stdout(), EnableMouseCapture)?;
    let crossfade = config.crossfade.unwrap_or(0).min(12);
    let result = run_app(&mut terminal, paths, api, picker, crossfade).await;
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

async fn run_app(
    terminal: &mut DefaultTerminal,
    paths: &Paths,
    api: Arc<WebApi>,
    picker: Picker,
    crossfade: u64,
) -> Result<()> {
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
    let notices = tx.clone();
    let media = cloofy_os::media::Controls::new({
        let tx = tx.clone();
        move |command| {
            let _ = tx.send(app::AppEvent::Media(command));
        }
    });
    playback::spawn(paths.clone(), Duration::from_secs(crossfade), tx);
    let saved = state::load(&paths.state_file());
    let mut app = App::new(loader, session_tx, crossfade, saved, paths.state_file(), media);

    let mut input = EventStream::new();
    let mut closed = shutdown_signals()?;
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    terminal.draw(|frame| ui::draw(frame, &mut app))?;
    while !app.quit {
        let redraw = tokio::select! {
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
            Some(event) = events.recv() => {
                app.on_event(event);
                true
            }
            _ = ticker.tick() => app.on_tick(),
            _ = closed.recv() => break,
        };
        app.sync_media();
        if redraw || app.snapshot_requested {
            let frame = terminal.draw(|frame| ui::draw(frame, &mut app))?;
            if std::mem::take(&mut app.snapshot_requested)
                && let Some(snapshotter) = &snapshotter
            {
                let (buffer, placements) = (frame.buffer.clone(), app.placements.clone());
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
