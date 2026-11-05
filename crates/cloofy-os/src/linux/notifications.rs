//! Desktop notifications sent to the desktop's notification server itself, rather than through
//! the terminal: that way they carry the cover, and they also work in terminals without
//! notifications of their own.

use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process,
    sync::{Arc, Mutex, PoisonError, Weak},
};

use futures::StreamExt;
use zbus::{Connection, Proxy, Variant, fdo, names::WellKnownName};

use super::{bus, media};

const SERVER: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
/// A Nerd Font note, which Omarchy shows where a notification has no image.
const GLYPH: &str = "󰝚";

pub struct Notifications {
    connection: Connection,
    shown: Arc<Mutex<Shown>>,
}

/// The notification on screen, which the next one replaces, and its cover file.
#[derive(Default)]
struct Shown {
    id: u32,
    image: Option<PathBuf>,
    written: u64,
    /// Whether the server reads markup in the body, once asked.
    markup: Option<bool>,
}

/// Whether a click on a notification brings cloofy's window to the front: on Hyprland.
pub fn click_raises() -> bool {
    media::can_raise()
}

impl Notifications {
    /// `None` without a session bus, or without a notification server on it.
    pub fn new() -> Option<Self> {
        let connection = bus::run(async {
            let connection = bus::session().await?;
            has_server(&connection).await.then_some(connection)
        })?;
        let shown = Arc::default();
        if click_raises() {
            bus::spawn(raise_on_click(connection.clone(), Arc::downgrade(&shown)));
        }
        Some(Self { connection, shown })
    }

    /// Shows `title` and `body`, with `image` (JPEG, PNG or WebP) where there is one, in place of
    /// the notification shown before.
    pub fn show(&self, title: &str, body: &str, image: Option<Arc<[u8]>>) {
        let (connection, shown) = (self.connection.clone(), self.shown.clone());
        let (title, body) = (title.to_owned(), body.to_owned());
        bus::spawn(async move {
            let markup = shown.lock().unwrap_or_else(PoisonError::into_inner).markup;
            let markup = match markup {
                Some(markup) => markup,
                None => {
                    let markup = takes_markup(&connection).await;
                    shown.lock().unwrap_or_else(PoisonError::into_inner).markup = Some(markup);
                    markup
                }
            };
            let body = if markup { escape(&body) } else { body };
            let (replaces, image) = {
                let mut shown = shown.lock().unwrap_or_else(PoisonError::into_inner);
                (shown.id, image.and_then(|image| shown.write(&image)))
            };
            let arguments = notify_arguments(replaces, &title, &body, image.as_deref(), click_raises());
            let reply = connection.call_method(Some(SERVER), PATH, Some(SERVER), "Notify", &arguments).await;
            match reply.and_then(|reply| reply.body().deserialize::<u32>()) {
                Ok(id) => shown.lock().unwrap_or_else(PoisonError::into_inner).id = id,
                Err(err) => tracing::warn!("notification: {err}"),
            }
        });
    }
}

impl Drop for Notifications {
    fn drop(&mut self) {
        let image = self.shown.lock().unwrap_or_else(PoisonError::into_inner).image.take();
        if let Some(path) = image {
            let _ = fs::remove_file(path);
        }
    }
}

type NotifyArguments<'a> = (&'a str, u32, &'a str, &'a str, &'a str, &'a [&'a str], HashMap<&'a str, Variant<'a>>, i32);

/// What `Notify` takes: app, the notification it replaces, icon, title, body, actions, hints
/// and how long it shows (the server's choice). `clickable` where a click brings cloofy to the
/// front.
fn notify_arguments<'a>(
    replaces: u32,
    title: &'a str,
    body: &'a str,
    image: Option<&Path>,
    clickable: bool,
) -> NotifyArguments<'a> {
    let mut hints = HashMap::from([
        ("desktop-entry", Variant::from("cloofy")),
        ("omarchy-glyph", Variant::from(GLYPH)),
        // Low: a new song is worth a glance, not an interruption.
        ("urgency", Variant::U8(0)),
    ]);
    if let Some(url) = image.and_then(|path| url::Url::from_file_path(path).ok()) {
        hints.insert("image-path", Variant::from(url.to_string()));
    }
    // The action a click on the notification takes; it shows no button. A slice, since a fixed
    // array would go over the bus as a structure, which `Notify` doesn't take.
    let actions: &[&str] = if clickable { &["default", "Show cloofy"] } else { &[] };
    ("cloofy", replaces, "cloofy", title, body, actions, hints, -1)
}

/// Whether a notification server runs or the bus starts one when asked: without, notifications
/// would go nowhere, as on a desktop without one or a server reached over SSH.
async fn has_server(connection: &Connection) -> bool {
    let found = async {
        let bus = fdo::DBusProxy::new(connection).await?;
        if bus.name_has_owner(WellKnownName::from_static_str_unchecked(SERVER).into()).await? {
            return Ok(true);
        }
        let startable = bus.list_activatable_names().await?;
        Ok::<_, fdo::Error>(startable.iter().any(|name| name.as_str() == SERVER))
    };
    found.await.inspect_err(|err| tracing::debug!("notification server: {err}")).unwrap_or(false)
}

/// Whether the server reads markup in the body; asked only once there is something to show, as
/// asking starts a server that isn't running yet.
async fn takes_markup(connection: &Connection) -> bool {
    let reply = connection.call_method(Some(SERVER), PATH, Some(SERVER), "GetCapabilities", &()).await;
    match reply.and_then(|reply| reply.body().deserialize::<Vec<String>>()) {
        Ok(capabilities) => capabilities.iter().any(|capability| capability == "body-markup"),
        Err(err) => {
            tracing::debug!("notification server's capabilities: {err}");
            false
        }
    }
}

/// Brings cloofy's window to the front when the notification on screen is clicked, for as long
/// as there are notifications.
async fn raise_on_click(connection: Connection, shown: Weak<Mutex<Shown>>) {
    let clicks = async { Proxy::new(&connection, SERVER, PATH, SERVER).await?.receive_signal("ActionInvoked").await };
    let mut clicks = match clicks.await {
        Ok(clicks) => clicks,
        Err(err) => return tracing::debug!("notification clicks: {err}"),
    };
    while let Some(click) = clicks.next().await {
        let Some(shown) = shown.upgrade() else { return };
        let current = shown.lock().unwrap_or_else(PoisonError::into_inner).id;
        if let Ok((id, action)) = click.body().deserialize::<(u32, &str)>()
            && id == current
            && action == "default"
        {
            media::raise();
        }
    }
}

impl Shown {
    /// Writes `image` to a file of its own, the server taking images by path, and removes the
    /// one before: the notification it belonged to is being replaced.
    fn write(&mut self, image: &[u8]) -> Option<PathBuf> {
        if let Some(old) = self.image.take() {
            let _ = fs::remove_file(old);
        }
        self.written += 1;
        let name = format!("cloofy-{}-notification-{}.{}", process::id(), self.written, media::extension(image));
        let path = media::cover_dir().join(name);
        // A new file only, as with the media controls' covers.
        let written = OpenOptions::new().write(true).create_new(true).open(&path).and_then(|mut f| f.write_all(image));
        if let Err(err) = written {
            tracing::warn!("cover for the notification: {}: {err}", path.display());
            let _ = fs::remove_file(&path);
            return None;
        }
        self.image = Some(path.clone());
        Some(path)
    }
}

/// For servers that read markup in the body, text that looks like it is escaped.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use zbus::DynamicType;

    use super::*;

    #[test]
    fn calls_notify_with_its_signature() {
        let arguments = notify_arguments(3, "Strobe", "deadmau5", Some(Path::new("/run/cover.jpg")), true);
        assert_eq!(arguments.signature().to_string(), "(susssasa{sv}i)");
        assert_eq!(arguments.1, 3, "replaces the one before");
        assert_eq!(arguments.5, ["default", "Show cloofy"]);
        assert_eq!(arguments.6["image-path"], Variant::from("file:///run/cover.jpg"));
        let arguments = notify_arguments(0, "Strobe", "deadmau5", None, false);
        assert!(arguments.5.is_empty(), "no click to offer where it does nothing");
        assert_eq!(arguments.signature().to_string(), "(susssasa{sv}i)");
    }

    #[test]
    fn escapes_what_looks_like_markup() {
        assert_eq!(escape("Simon & Garfunkel <live>"), "Simon &amp; Garfunkel &lt;live&gt;");
    }

    #[test]
    fn keeps_one_cover_file() {
        let mut shown = Shown::default();
        let first = shown.write(b"\xff\xd8\xffjpeg").expect("written");
        assert!(first.extension().is_some_and(|ext| ext == "jpg"));
        let second = shown.write(b"\x89PNG\r\n\x1a\npng").expect("written");
        assert!(!first.exists(), "the replaced notification's cover is gone");
        assert_eq!(fs::read(&second).unwrap(), b"\x89PNG\r\n\x1a\npng");
        fs::remove_file(second).unwrap();
    }
}
