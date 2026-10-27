//! The connections to D-Bus. cloofy-os is synchronous and its callers may run no async runtime
//! at all, so the connections live on a thread of their own, with a small tokio runtime.

use std::{
    future::{self, Future},
    sync::{OnceLock, mpsc},
    thread,
    time::Duration,
};

use tokio::{
    runtime::{Builder, Handle},
    sync::OnceCell,
};
use zbus::Connection;

/// How long a caller waits for the bus before it goes on without: one that doesn't answer must
/// not hold up cloofy.
const TIMEOUT: Duration = Duration::from_secs(2);

fn runtime() -> Option<&'static Handle> {
    static RUNTIME: OnceLock<Option<Handle>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .inspect_err(|err| tracing::warn!("cannot start the D-Bus thread: {err}"))
                .ok()?;
            let handle = runtime.handle().clone();
            thread::Builder::new()
                .name("cloofy-dbus".into())
                .spawn(move || runtime.block_on(future::pending::<()>()))
                .inspect_err(|err| tracing::warn!("cannot start the D-Bus thread: {err}"))
                .ok()?;
            Some(handle)
        })
        .as_ref()
}

/// Runs `task` in the background, on the D-Bus thread.
pub fn spawn(task: impl Future<Output = ()> + Send + 'static) {
    if let Some(runtime) = runtime() {
        runtime.spawn(task);
    }
}

/// Runs `task` on the D-Bus thread and waits for its result, at most [`TIMEOUT`]. A result that
/// comes too late is dropped there.
pub fn run<T: Send + 'static>(task: impl Future<Output = Option<T>> + Send + 'static) -> Option<T> {
    let (tx, rx) = mpsc::sync_channel(1);
    spawn(async move {
        let _ = tx.send(task.await);
    });
    rx.recv_timeout(TIMEOUT).ok().flatten()
}

/// The session bus, shared by everything that only listens on it; `None` without one.
pub async fn session() -> Option<Connection> {
    static SESSION: OnceCell<Option<Connection>> = OnceCell::const_new();
    SESSION
        .get_or_init(|| async {
            Connection::session().await.inspect_err(|err| tracing::debug!("no session bus: {err}")).ok()
        })
        .await
        .clone()
}

/// The system bus, shared like [`session`].
pub async fn system() -> Option<Connection> {
    static SYSTEM: OnceCell<Option<Connection>> = OnceCell::const_new();
    SYSTEM
        .get_or_init(|| async {
            Connection::system().await.inspect_err(|err| tracing::debug!("no system bus: {err}")).ok()
        })
        .await
        .clone()
}
