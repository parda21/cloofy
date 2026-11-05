//! Playing, on fake services (see [`crate::backend::fake`]): what the player bar, the queue and
//! the notices show as the player reports, and the commands the player gets.

use std::time::Duration;

use cloofy_core::{player::PlayerEvent, service::Service};
use crossterm::event::KeyCode::{Char, Down, Enter};

use crate::{backend::fake::SONG, harness::Harness};

const SPOTIFY: Service = Service::Spotify;
const YOUTUBE: Service = Service::YouTubeMusic;

/// Opens the album of the service shown and plays its first song.
async fn play_album(h: &mut Harness, service: Service) {
    h.click_on(&format!("{} Album", service.name())).await.press(Enter).await;
}

#[tokio::test]
async fn a_song_plays_and_the_next_follows() {
    let mut h = Harness::fakes(&[SPOTIFY]).await;
    play_album(&mut h, SPOTIFY).await;
    assert_eq!(h.fake(SPOTIFY).commands(), ["play spotify:album:a1 at spotify:track:a1-1"]);
    assert!(h.player_bar().contains("Spotify Song 1"), "{}", h.player_bar());
    h.wait(SONG + Duration::from_secs(1)).await;
    assert!(h.player_bar().contains("Spotify Song 2"), "{}", h.player_bar());
    assert!(h.player_bar().contains("0:01"), "{}", h.player_bar());
}

#[tokio::test]
async fn space_pauses_and_the_position_holds() {
    let mut h = Harness::fakes(&[SPOTIFY]).await;
    play_album(&mut h, SPOTIFY).await;
    h.wait(Duration::from_secs(10)).await.press(Char(' ')).await;
    h.wait(Duration::from_secs(30)).await;
    assert!(h.player_bar().contains("0:10"), "paused at 0:10:\n{}", h.player_bar());
    h.press(Char(' ')).await.wait(Duration::from_secs(5)).await;
    assert!(h.player_bar().contains("0:15"), "{}", h.player_bar());
    assert_eq!(h.fake(SPOTIFY).commands()[1..], ["toggle", "toggle"]);
}

#[tokio::test]
async fn seeking_moves_the_position() {
    let mut h = Harness::fakes(&[SPOTIFY]).await;
    play_album(&mut h, SPOTIFY).await;
    h.press(Char('L')).await.press(Char('L')).await;
    assert!(h.player_bar().contains("0:10"), "{}", h.player_bar());
    assert_eq!(h.fake(SPOTIFY).commands().last().unwrap(), "seek 10000 ms");
}

/// Each service keeps its own list of what plays next; switching back and forth shows the one of
/// the service that plays. (8b495bf's own tests are in `app::browsing` and `cloofy-ytmusic`.)
#[tokio::test]
async fn the_queue_page_shows_the_service_that_plays() {
    let mut h = Harness::fakes(&[YOUTUBE, SPOTIFY]).await;
    play_album(&mut h, YOUTUBE).await;
    h.press(Char('S')).await;
    play_album(&mut h, SPOTIFY).await;
    h.press(Char('Q')).await;
    assert!(h.shows("Spotify Song 2") && !h.shows("YouTube Music Song"), "{}", h.screen());
    h.press(Char('S')).await;
    play_album(&mut h, YOUTUBE).await;
    h.press(Char('Q')).await;
    assert!(h.shows("YouTube Music Song 2") && !h.shows("Spotify Song"), "{}", h.screen());
}

/// From 3729e30: a song Spotify refused the key to was skipped without a word.
#[tokio::test]
async fn a_skipped_song_says_why() {
    let mut h = Harness::fakes(&[SPOTIFY]).await;
    play_album(&mut h, SPOTIFY).await;
    let uri = "spotify:track:a1-2".to_owned();
    h.fake(SPOTIFY).send(PlayerEvent::Unavailable { uri: uri.clone(), refused: true });
    h.settle().await;
    assert!(h.shows("Spotify refused the key to a song"), "{}", h.screen());
    h.wait(Duration::from_secs(5)).await;
    h.fake(SPOTIFY).send(PlayerEvent::Unavailable { uri, refused: false });
    h.settle().await;
    assert!(h.shows("A song couldn't load, so it was skipped"), "{}", h.screen());
}

/// From 7211331, as far as the interface goes: Home, the library and search don't wait for
/// the player, and playing says it is still connecting.
#[tokio::test]
async fn browsing_works_while_the_player_connects() {
    let mut h = Harness::fakes_connecting(&[SPOTIFY]).await;
    assert!(h.shows("Hello from Spotify") && h.shows("Spotify Mix"), "{}", h.screen());
    h.press(Char('/')).await.type_text("song 3").await.wait(Duration::from_millis(250)).await;
    h.press(Enter).await.press(Enter).await;
    assert!(h.shows("Still connecting to Spotify"), "{}", h.screen());
    assert!(h.fake(SPOTIFY).commands().is_empty());
    h.fake(SPOTIFY).connect();
    h.settle().await;
    h.press(Enter).await;
    // A song from search plays on with the rest of its album.
    assert_eq!(h.fake(SPOTIFY).commands(), ["play spotify:album:a1 at spotify:track:a1-3"]);
}

#[tokio::test]
async fn a_lost_connection_keeps_the_song_to_go_on_with() {
    let mut h = Harness::fakes(&[SPOTIFY]).await;
    play_album(&mut h, SPOTIFY).await;
    h.wait(Duration::from_secs(30)).await;
    h.fake(SPOTIFY).lose_connection();
    h.settle().await;
    assert!(h.shows("Connection to Spotify lost, reconnecting"), "{}", h.screen());
    assert!(h.player_bar().contains("Spotify Song 1"), "{}", h.player_bar());
    h.fake(SPOTIFY).connect();
    h.settle().await;
    h.press(Char(' ')).await;
    assert_eq!(h.fake(SPOTIFY).commands().last().unwrap(), "resume spotify:track:a1-1 at 30000 ms");
}

#[tokio::test]
async fn a_queued_song_plays_next() {
    let mut h = Harness::fakes(&[SPOTIFY]).await;
    play_album(&mut h, SPOTIFY).await;
    for _ in 0..4 {
        h.press(Down).await;
    }
    h.press(Char('q')).await;
    assert_eq!(h.fake(SPOTIFY).commands().last().unwrap(), "queue spotify:track:a1-5");
    h.press(Char('Q')).await;
    let (queued, rest) = (h.find("Spotify Song 5").unwrap(), h.find("Spotify Song 2").unwrap());
    assert!(queued.1 < rest.1, "the queued song comes first:\n{}", h.screen());
    h.press(Char(']')).await;
    assert!(h.player_bar().contains("Spotify Song 5"), "{}", h.player_bar());
}

#[tokio::test]
async fn liking_a_song_saves_it() {
    let mut h = Harness::fakes(&[SPOTIFY]).await;
    play_album(&mut h, SPOTIFY).await;
    for _ in 0..3 {
        h.press(Down).await;
    }
    h.press(Char('f')).await.wait(Duration::from_secs(5)).await;
    assert!(
        h.fake(SPOTIFY).commands().contains(&"save spotify:track:a1-4".to_owned()),
        "{:?}",
        h.fake(SPOTIFY).commands()
    );
}
