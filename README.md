# cloofy

A Spotify client for the terminal.

cloofy plays music itself, with no official client or daemon running in the background. It shows your Home feed, your library, artist pages, lyrics and cover art, and works with the keyboard and the mouse alike.

## Features

- **Plays on its own.** Audio goes through librespot, and cloofy shows up as a Spotify Connect device. It can also control your other devices.
- **Home and DJ.** Your personal shelves, Made For You mixes, and the DJ with its voice between tracks.
- **Your library.** Playlists, albums, artists and podcasts, in the sidebar or as a full page.
- **Pages and search.** Playlists, albums, and artists with their popular tracks and releases, plus search.
- **Lyrics.** Synced lyrics next to a large cover in the now-playing view.
- **Cover art.** Real images in terminals that speak the Kitty graphics protocol (kitty, Ghostty, WezTerm), and blocks elsewhere.
- **Mouse.** Click, hover, scroll, and drag the seek and volume bars.
- **Notifications.** In kitty, Ghostty, WezTerm and iTerm2, each new track shows a desktop notification while cloofy's window is in the background.
- **Media keys.** On macOS, the media keys, AirPods and Control Center play, pause, skip and seek, and Control Center shows the track with its cover.
- **Follows your headphones.** On macOS, playback moves to AirPods or headphones as they connect, and pauses when they disconnect.
- **Stays connected.** cloofy reconnects by itself after sleep or a lost network. On macOS it also pauses before the Mac sleeps.
- **Easy on motion and battery.** On macOS, cloofy follows Reduce Motion and saves energy in Low Power Mode.
- **Picks up where you left off.** The same page, the same selection, the last track ready to resume.

## Requirements

- Spotify Premium
- Linux with PulseAudio or PipeWire, or macOS
- A terminal with true color. kitty, Ghostty or WezTerm also get cover art and icons.
- A free Spotify developer app (see [Setup](#setup))

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/parda21/cloofy/main/install.sh | bash
```

The installer checks your system and installs the latest release. Then it walks you through the [setup](#setup): creating your Spotify app, entering its client ID, and signing in. Without a prebuilt binary for your platform, it builds cloofy from source with cargo. Run it again to update.

- `bash -s -- --yes` installs without asking anything.
- `bash -s -- --uninstall` removes cloofy.

To build from source yourself (Rust 1.88 or newer):

```sh
cargo install --git https://github.com/parda21/cloofy --locked
```

On Linux the build needs the ALSA and PulseAudio headers:

| Distro | Packages |
| --- | --- |
| Debian, Ubuntu | `libasound2-dev libpulse-dev pkg-config` |
| Fedora | `alsa-lib-devel pulseaudio-libs-devel` |
| Arch | `alsa-lib libpulse` |

## Setup

The installer does all of this with you. To do it by hand:

1. Create an app at [developer.spotify.com/dashboard](https://developer.spotify.com/dashboard). Give it any name, set the redirect URI to `http://127.0.0.1:8974/callback`, and pick the Web API.
2. Under **User Management**, add the Spotify account you listen with. Apps in development mode only work for the accounts listed there.
3. Put the app's client ID into `~/.config/cloofy/config.toml`:

   ```toml
   client_id = "your client id"
   ```

4. Run `cloofy login`. You sign in twice in the browser: once for your app, which reads your library and searches, and once for playback.
5. Run `cloofy doctor` to check that everything works, then `cloofy` to start.

## Usage

| Key | Action |
| --- | --- |
| `space` | Play or pause |
| `[` `]` | Previous, next track |
| `⇧←` `⇧→` | Seek 5 seconds |
| `+` `-` | Volume |
| `s` `r` | Shuffle, repeat |
| `1` `/` `3` | Home, search, your library |
| `enter` | Open or play the selection |
| `esc` | Back |
| `f` | Save to or remove from your library |
| `q` `a` | Add to queue, add to playlist |
| `.` | Actions for the selection |
| `z` | Now playing and lyrics |
| `d` | Devices |
| `D` | Start the DJ |
| `ctrl+k` | Command palette |
| `?` | All shortcuts |
| `ctrl+c` | Quit |

Everything also works with the mouse. Right-click a track, card or library entry for its actions.

Other commands:

```sh
cloofy play <uri or link>   # play without the interface; --shuffle to shuffle
cloofy connect              # wait as a Spotify Connect device until an app picks it
cloofy login --force        # sign in again
```

## Configuration

`~/.config/cloofy/config.toml`:

```toml
client_id = "your client id"
crossfade = 6     # seconds tracks overlap, 0 to 12; off when missing
icons = "nerd"    # "nerd" or "text"; detected from the terminal when missing
notifications = false  # no desktop notifications for new tracks; on when missing
```

cloofy keeps its sign-ins and session in `~/.local/share/cloofy`, and its caches and log in `~/.cache/cloofy`.

## How it works

Playback runs through a [fork of librespot](https://github.com/parda21/librespot/tree/cloofy) that adds the DJ and crossfade from two open upstream pull requests. Your library and search come from the Web API through your own app. Home, artist pages and the DJ use Spotify's internal APIs, the same ones its own apps use. Those are not public and may change without notice.

cloofy is not affiliated with Spotify.

## License

[MIT](LICENSE)
