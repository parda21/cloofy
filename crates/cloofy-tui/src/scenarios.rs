//! The whole app on the demo, driven by keys and clicks through [`Harness`]. Screens are kept in
//! `tests/screens`; `CLOOFY_UPDATE_SCREENS=1 cargo test -p cloofy-tui` takes new ones, and the
//! diff shows what changed.

use std::time::Duration;

use crossterm::event::KeyCode::{self, Char, Down, Enter, Esc};

use crate::harness::Harness;

mod playback;

const DEBOUNCE: Duration = Duration::from_millis(250);

#[tokio::test]
async fn starts_at_home() {
    let h = Harness::demo().await;
    assert!(h.shows("Good evening"));
    assert!(h.shows("Around the World"), "the demo's song is in the player bar");
    h.assert_screen("home");
}

#[tokio::test]
async fn library_lists_what_is_saved() {
    let mut h = Harness::demo().await;
    h.press(Char('3')).await;
    assert!(h.shows("All  Playlists  Albums  Artists"));
    h.assert_screen("library");
}

#[tokio::test]
async fn search_waits_for_typing_to_stop() {
    let mut h = Harness::demo().await;
    h.press(Char('/')).await.type_text("daft").await;
    assert!(!h.shows("One More Time"), "no search while typing");
    h.wait(DEBOUNCE).await;
    assert!(h.shows("One More Time"));
    h.press(Enter).await;
    assert!(h.shows("enter play/open"), "results take the keys after enter");
    h.assert_screen("search");
}

#[tokio::test]
async fn a_playlist_opens_and_esc_goes_back() {
    let mut h = Harness::demo().await;
    h.click_on("Deep Focus").await;
    assert!(h.shows("By Sam · 10 songs"));
    h.assert_screen("playlist");
    h.press(Esc).await;
    assert!(h.shows("Good evening"));
}

#[tokio::test]
async fn the_demo_says_it_plays_nothing() {
    let mut h = Harness::demo().await;
    h.press(Char('/')).await.type_text("daft").await;
    h.wait(DEBOUNCE).await;
    h.press(Enter).await.press(Down).await.press(Enter).await;
    assert!(h.shows("This is the demo: nothing plays here"));
    assert!(h.shows("Around the World"), "the song stays");
    assert!(h.shows("Playlist · Late Night Coding"), "and so does what it plays from:\n{}", h.screen());
    h.wait(Duration::from_secs(5)).await;
    assert!(!h.shows("nothing plays here"), "the notice goes away");
}

#[tokio::test]
async fn the_queue_page_lists_what_plays_next() {
    let mut h = Harness::demo().await;
    h.press(Char('Q')).await;
    let (now, next) = (h.find("Now playing").unwrap(), h.find("Next in queue").unwrap());
    let (playing, queued) = (h.find("Around the World  ").unwrap(), h.find("Digital Love").unwrap());
    assert!(now.1 < playing.1 && playing.1 < next.1 && next.1 < queued.1, "{}", h.screen());
}

#[tokio::test]
async fn now_playing_shows_the_lyrics() {
    let mut h = Harness::demo().await;
    h.press(Char('z')).await;
    assert!(h.shows("Lyrics · Musixmatch"));
    h.assert_screen("now_playing");
    h.press(Char('z')).await;
    assert!(h.shows("Good evening"));
}

#[tokio::test]
async fn the_palette_runs_commands() {
    let mut h = Harness::demo().await;
    h.ctrl('k').await.type_text("sett").await;
    h.press(Enter).await;
    assert!(h.shows("Cover art"), "{}", h.screen());
}

#[tokio::test]
async fn small_windows_get_compact_bars() {
    let h = Harness::demo_sized(64, 16).await;
    h.assert_screen("compact");
}

/// Layout bugs show up at sizes nobody tries: every page, drawn at many sizes, must not panic,
/// and a frame drawn again without anything happening must come out the same.
#[tokio::test]
async fn every_page_draws_at_any_size() {
    const WIDTHS: [u16; 9] = [1, 8, 20, 33, 47, 64, 80, 121, 240];
    const HEIGHTS: [u16; 10] = [1, 3, 6, 9, 10, 14, 16, 24, 30, 70];
    let pages: [(&str, &[KeyCode]); 9] = [
        ("home", &[]),
        ("library", &[Char('3')]),
        ("search", &[Char('/'), Char('d'), Char('a')]),
        ("playlist", &[Char('3'), Down, Enter]),
        ("queue", &[Char('Q')]),
        ("now playing", &[Char('z')]),
        ("settings", &[Char(',')]),
        ("help", &[Char('?')]),
        ("palette", &[Char(':')]),
    ];
    for (name, keys) in pages {
        let mut h = Harness::demo().await;
        for key in keys {
            h.press(*key).await;
        }
        h.wait(DEBOUNCE).await;
        for width in WIDTHS {
            for height in HEIGHTS {
                h.resize(width, height).await;
                let before = h.screen();
                h.redraw();
                assert_eq!(before, h.screen(), "{name} at {width}×{height} changes from frame to frame");
            }
        }
        h.resize(120, 36).await;
        assert!(h.shows("cloofy"), "{name} recovers at full size:\n{}", h.screen());
    }
}

#[tokio::test]
async fn shuffling_in_the_demo_keeps_what_plays() {
    let mut h = Harness::demo().await;
    h.click_on("Deep Focus").await.click_on("Shuffle").await;
    assert!(h.shows("This is the demo: nothing plays here"));
    assert!(h.shows("Playlist · Late Night Coding"), "{}", h.screen());
}
