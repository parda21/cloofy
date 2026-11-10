//! Playing, on fake services (see [`crate::backend::fake`]): what the player bar, the queue and
//! the notices show as the player reports, and the commands the player gets.

use std::time::Duration;

use cloofy_core::{player::PlayerEvent, service::Service};
use crossterm::event::KeyCode::{Char, Down, Enter};

use crate::{backend::fake::SONG, harness::Harness, views::View};

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

/// YouTube Music's Home as in its own apps: chips above, Quick picks as songs in columns that
/// play at once, and a Play all that plays them in order.
#[tokio::test]
async fn youtube_music_home_has_chips_and_quick_picks() {
    let mut h = Harness::fakes(&[YOUTUBE]).await;
    assert!(h.shows(" Relax ") && h.shows("Quick picks") && h.shows("MADE FOR YOU"), "{}", h.screen());
    h.assert_screen("ytmusic_home");

    // Down the first column, then to the next one: the song plays at once.
    h.press(Down).await.press(Char('l')).await.press(Enter).await;
    assert_eq!(h.fake(YOUTUBE).commands(), ["play ytmusic:track:p1-1"]);
    assert!(h.player_bar().contains("YouTube Music Mix Song 1"), "{}", h.player_bar());

    h.click_on("Play all").await;
    let commands = h.fake(YOUTUBE).commands();
    assert!(commands[1].starts_with("play ytmusic:songs:a1-1,a1-2,"), "{commands:?}");
    assert!(h.player_bar().contains("YouTube Music Song 1"), "{}", h.player_bar());
}

#[tokio::test]
async fn a_chip_narrows_youtube_music_home_down_and_back() {
    let mut h = Harness::fakes(&[YOUTUBE]).await;
    h.click_on("Workout").await;
    assert!(h.shows("Workout mixes"), "{}", h.screen());
    // Up from the first shelf to the chips; Enter on the picked one shows the whole Home again.
    h.press(Char('k')).await.press(Enter).await;
    assert!(!h.shows("Workout mixes") && h.shows("Quick picks"), "{}", h.screen());
}

/// A chip whose Home doesn't load left the old Home with the chip picked, and nothing said.
/// The Home shown stays, its chip too, and the bottom says why.
#[tokio::test]
async fn a_chip_that_fails_to_load_says_so() {
    let mut h = Harness::fakes(&[YOUTUBE]).await;
    h.fake(YOUTUBE).take_home_down();
    h.click_on("Workout").await;
    assert!(h.shows("Couldn't load Workout: The fake's Home is down") && h.shows("Quick picks"), "{}", h.screen());
    let View::Home(v) = &h.app.view else { panic!("{}", h.screen()) };
    assert!(v.filter.is_none() && !v.loading && v.chips().iter().all(|c| !c.selected));
}

/// On the second day of use cloofy asks for a star on GitHub, once, a few seconds in.
#[tokio::test]
async fn the_star_on_github_is_asked_for_once() {
    let mut h = Harness::fakes(&[YOUTUBE]).await;
    h.app.ask_for_star_after(Duration::from_secs(5));
    h.wait(Duration::from_secs(4)).await;
    assert!(!h.shows("Enjoying cloofy?"), "not right at the start");
    h.wait(Duration::from_secs(2)).await;
    assert!(h.shows("Enjoying cloofy?"), "{}", h.screen());
    assert!(h.app.usage.star_asked, "it counts as asked the moment it shows");
    h.assert_screen("star");

    h.press(Enter).await;
    assert!(!h.shows("Enjoying cloofy?"));
    assert_eq!(h.app.opened, [cloofy_core::update::REPO_URL]);
}

#[tokio::test]
async fn no_thanks_closes_the_star_for_good() {
    let mut h = Harness::fakes(&[YOUTUBE]).await;
    h.app.ask_for_star_after(Duration::ZERO);
    h.wait(Duration::from_millis(100)).await;
    h.click_on("No thanks").await;
    assert!(!h.shows("Enjoying cloofy?") && h.app.opened.is_empty());
    assert!(h.app.usage.star_asked);
}

/// On a short terminal the star's popup leaves out what doesn't fit inside its border, instead
/// of drawing over the border and the page under it.
#[tokio::test]
async fn the_star_keeps_inside_its_border_on_a_short_terminal() {
    let mut h = Harness::fakes(&[YOUTUBE]).await;
    h.app.ask_for_star_after(Duration::ZERO);
    h.wait(Duration::from_millis(100)).await;
    for height in [9, 7, 5] {
        h.resize(80, height).await;
        let screen = h.screen();
        let bottom = screen.lines().find(|line| line.contains('╰')).unwrap_or_else(|| panic!("{screen}"));
        let border: String = bottom.chars().skip_while(|&c| c != '╰').take_while(|&c| c != '╯').collect();
        assert!(border.chars().all(|c| c == '╰' || c == '─'), "{height} rows:\n{screen}");
    }
}

/// Issue #27: the expired sign-in went away after a few seconds, before anyone read it or
/// could copy the command. It stays now, under other messages, until closed.
#[tokio::test]
async fn an_expired_sign_in_stays_until_closed() {
    let mut h = Harness::fakes(&[YOUTUBE]).await;
    let expired = "YouTube doesn't accept the saved sign-in any more – run `cloofy login ytmusic` again";
    h.app.error(expired.into());
    h.wait(Duration::from_secs(60)).await;
    assert!(h.shows("run `cloofy login ytmusic` again") && h.shows("click to copy"), "{}", h.screen());

    // A message in between, and the sign-in is back after it.
    play_album(&mut h, YOUTUBE).await;
    h.fake(YOUTUBE).send(PlayerEvent::Unavailable { uri: "ytmusic:track:a1-2".into(), refused: false });
    h.settle().await;
    assert!(h.shows("A song couldn't load"), "{}", h.screen());
    h.wait(Duration::from_secs(5)).await;
    assert!(h.shows("cloofy login ytmusic"), "{}", h.screen());

    h.click_on("click to copy").await;
    assert!(h.shows("copied"), "{}", h.screen());
    h.click_on("✕").await;
    assert!(!h.shows("cloofy login ytmusic"), "{}", h.screen());
}

/// YouTube's bot check stopped every song without a word: it says so now, until music plays.
#[tokio::test]
async fn the_bot_check_says_why_nothing_plays() {
    let mut h = Harness::fakes(&[YOUTUBE]).await;
    play_album(&mut h, YOUTUBE).await;
    let reason = "YouTube asks this connection to prove it isn't a bot, so songs can't play for now.";
    h.fake(YOUTUBE).send(PlayerEvent::Blocked { reason: reason.into() });
    h.wait(Duration::from_secs(30)).await;
    assert!(h.shows("prove it isn't a bot"), "{}", h.screen());
    assert!(!h.shows("click to copy"), "there's no command to run");
    h.fake(YOUTUBE).send(PlayerEvent::Playing { position_ms: 0 });
    h.settle().await;
    assert!(!h.shows("prove it isn't a bot"), "{}", h.screen());
}
