//! Shows a made-up track in the system's media controls and prints each request that arrives,
//! for trying the integration without Spotify:
//!
//! ```sh
//! cargo run -p cloofy-os --example now_playing [--paused] [cover.jpg]
//! ```
//!
//! On Linux, `playerctl -p cloofy play-pause` or `busctl --user call org.mpris.MediaPlayer2.cloofy
//! /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player Seek x 10000000` send requests too.

use std::{sync::Arc, time::Duration};

use cloofy_os::media::{Command, Controls, State, Track};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paused = args.iter().any(|arg| arg == "--paused");
    let artwork = args
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .map(|path| Arc::from(std::fs::read(path).expect("cover is readable")));
    cloofy_os::run_main(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let controls = Controls::new(move |command| {
            let _ = tx.send(command);
        })
        .expect("this system has no media controls cloofy can use");
        let track = Track {
            title: "Example Song".into(),
            artist: "cloofy".into(),
            album: "Media Controls".into(),
            duration: Duration::from_secs(215),
            artwork,
        };
        let mut state = State { track: Some(track), playing: !paused, position: Duration::ZERO };
        let mut since = std::time::Instant::now();
        controls.update(state.clone());
        println!("Showing a track. Try the media keys, Control Center or playerctl; Ctrl+C quits.");
        for command in rx {
            println!("{command:?}");
            if state.playing {
                state.position += since.elapsed();
            }
            since = std::time::Instant::now();
            match command {
                Command::Play => state.playing = true,
                Command::Pause => state.playing = false,
                Command::Toggle => state.playing = !state.playing,
                Command::Seek(position) => state.position = position,
                Command::Next | Command::Previous => state.position = Duration::ZERO,
            }
            controls.update(state.clone());
        }
    });
}
