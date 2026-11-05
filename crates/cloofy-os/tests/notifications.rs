//! Song notifications on a real session bus, as the desktop's notification server gets them. CI
//! runs this under `dbus-run-session` with `CLOOFY_REQUIRE_DBUS=1`; elsewhere it passes without
//! looking, also on a desktop whose own server holds the name.
#![cfg(target_os = "linux")]

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use cloofy_os::notifications::Notifications;
use zbus::{Connection, OwnedVariant, fdo, interface, names::WellKnownName};

const SERVER: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// What a notification server got: title, body and the cover's path.
type Shown = Arc<Mutex<Vec<(String, String, Option<String>)>>>;

/// A notification server, with markup in the body or without.
struct Server {
    markup: bool,
    shown: Shown,
}

#[interface(name = "org.freedesktop.Notifications")]
impl Server {
    fn get_capabilities(&self) -> Vec<&'static str> {
        if self.markup { vec!["body", "body-markup", "actions"] } else { vec!["body", "actions"] }
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        _app: String,
        _replaces: u32,
        _icon: String,
        title: String,
        body: String,
        _actions: Vec<String>,
        hints: HashMap<String, OwnedVariant>,
        _timeout: i32,
    ) -> u32 {
        let image = hints.get("image-path").and_then(|path| String::try_from(path.try_clone().ok()?).ok());
        let mut shown = self.shown.lock().unwrap();
        shown.push((title, body, image));
        shown.len() as u32
    }

    fn get_server_information(&self) -> (&str, &str, &str, &str) {
        ("fake", "cloofy", "1", "1.2")
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn shows_songs_on_the_desktop_s_server_with_the_cover() {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        assert!(std::env::var_os("CLOOFY_REQUIRE_DBUS").is_none(), "no session bus");
        return eprintln!("no session bus, skipped");
    }
    let bus = fdo::DBusProxy::new(&Connection::session().await.unwrap()).await.unwrap();
    let name = || WellKnownName::from_static_str_unchecked(SERVER).into();
    if bus.name_has_owner(name()).await.unwrap() {
        assert!(std::env::var_os("CLOOFY_REQUIRE_DBUS").is_none(), "a server runs already");
        return eprintln!("the desktop's notification server runs, skipped");
    }
    let startable = bus.list_activatable_names().await.unwrap();
    if !startable.iter().any(|name| name.as_str() == SERVER) {
        assert!(Notifications::new().is_none(), "nowhere to show them without a server");
    }

    let shown = Shown::default();
    let server = serve(true, shown.clone()).await;
    let notifications = Notifications::new().expect("a server to show them");
    let cover = b"\xff\xd8\xffjpeg".as_slice();
    notifications.show("Strobe", "deadmau5 · <live> & more", Some(cover.into()));
    let (title, body, image) =
        eventually("the notification", || async { shown.lock().unwrap().first().cloned() }).await;
    assert_eq!(title, "Strobe");
    assert_eq!(body, "deadmau5 · &lt;live&gt; &amp; more", "escaped for a server that reads markup");
    let image = url::Url::parse(&image.expect("the cover")).unwrap().to_file_path().unwrap();
    assert_eq!(std::fs::read(&image).unwrap(), cover);
    drop(notifications);
    assert!(!image.exists(), "the cover goes with cloofy");
    server.release_name(SERVER).await.unwrap();

    let plain = Shown::default();
    let _server = serve(false, plain.clone()).await;
    let notifications = Notifications::new().expect("a server to show them");
    notifications.show("Strobe", "deadmau5 · <live> & more", None);
    let (_, body, image) = eventually("the notification", || async { plain.lock().unwrap().first().cloned() }).await;
    assert_eq!(body, "deadmau5 · <live> & more", "as it is for a server that shows text as it is");
    assert_eq!(image, None::<String>);
}

/// A notification server on the session bus, until the connection goes.
async fn serve(markup: bool, shown: Shown) -> Connection {
    zbus::connection::Builder::session()
        .serve_at(PATH, Server { markup, shown })
        .name(SERVER)
        .build()
        .await
        .expect("the notification server's name")
}

/// Asks until `check` has an answer, for at most five seconds.
async fn eventually<T, F: Future<Output = Option<T>>>(what: &str, mut check: impl FnMut() -> F) -> T {
    let started = Instant::now();
    loop {
        if let Some(found) = check().await {
            return found;
        }
        assert!(started.elapsed() < Duration::from_secs(5), "{what} never came");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
