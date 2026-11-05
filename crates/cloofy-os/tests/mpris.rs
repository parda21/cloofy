//! The media controls on a real session bus, as playerctl and the desktop's widgets see them. CI
//! runs this under `dbus-run-session` with `CLOOFY_REQUIRE_DBUS=1`; elsewhere, without a session
//! bus, it passes without looking.
#![cfg(target_os = "linux")]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use cloofy_os::media::{Command, Controls, State, Track};
use zbus::{Connection, OwnedVariant, Proxy, Variant, proxy::CacheProperties};

const NAME: &str = "org.mpris.MediaPlayer2.cloofy";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

#[tokio::test]
async fn playerctl_sees_the_track_and_controls_it() {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        assert!(std::env::var_os("CLOOFY_REQUIRE_DBUS").is_none(), "no session bus");
        return eprintln!("no session bus, skipped");
    }
    let commands = Arc::new(Mutex::new(Vec::new()));
    let controls = Controls::new({
        let commands = commands.clone();
        move |command| commands.lock().unwrap().push(command)
    })
    .expect("media controls on the session bus");
    controls.update(State {
        track: Some(Track {
            title: "Around the World".into(),
            artist: "Daft Punk".into(),
            album: "Homework".into(),
            duration: Duration::from_secs(429),
            artwork: None,
            link: Some("https://open.spotify.com/track/1pKYYY0dkg23sQQXi0Q5zN".into()),
        }),
        playing: true,
        position: Duration::from_secs(81),
        volume: u16::MAX / 2,
    });

    let bus = Connection::session().await.unwrap();
    let player = zbus::proxy::Builder::<Proxy>::new(&bus)
        .destination(NAME)
        .path(PATH)
        .interface(PLAYER)
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .unwrap();

    let metadata = eventually("the track", || async {
        let metadata: HashMap<String, OwnedVariant> = player.get_property("Metadata").await.ok()?;
        metadata.contains_key("xesam:title").then_some(metadata)
    })
    .await;
    let field = |key: &str| metadata.get(key).unwrap_or_else(|| panic!("no {key}")).try_clone().unwrap();
    assert_eq!(String::try_from(field("xesam:title")).unwrap(), "Around the World");
    assert_eq!(Vec::<String>::try_from(field("xesam:artist")).unwrap(), ["Daft Punk"]);
    assert_eq!(String::try_from(field("xesam:album")).unwrap(), "Homework");
    assert_eq!(i64::try_from(field("mpris:length")).unwrap(), 429_000_000);
    assert_eq!(player.get_property::<String>("PlaybackStatus").await.unwrap(), "Playing");
    let position: i64 = player.get_property("Position").await.unwrap();
    assert!(position >= 81_000_000, "{position}");

    for (method, expected) in [("PlayPause", Command::Toggle), ("Next", Command::Next), ("Previous", Command::Previous)]
    {
        player.call_method(method, &()).await.unwrap();
        eventually(method, || async { commands.lock().unwrap().contains(&expected).then_some(()) }).await;
    }
    player.set_property("Volume", Variant::from(0.25)).await.unwrap();
    eventually("the volume", || async {
        commands
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, Command::Volume(v) if v.abs_diff(u16::MAX / 4) <= 1))
            .then_some(())
    })
    .await;

    drop(controls);
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
