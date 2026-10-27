<p align="center">
  <img src="https://raw.githubusercontent.com/parda21/cloofy/assets/logo.svg" alt="cloofy" height="84">
</p>

<p align="center">
  <b>Spotify in your terminal.</b><br>
  Plays on its own, with Home and the DJ, synced lyrics, real cover art and a mouse that works everywhere.
</p>

<p align="center">
  <a href="https://github.com/parda21/cloofy/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/parda21/cloofy?style=flat-square&color=8b5cf6&label=release"></a>
  <a href="https://github.com/parda21/cloofy/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/parda21/cloofy/ci.yml?branch=main&style=flat-square&label=ci"></a>
  <a href="https://ratatui.rs"><img alt="Built with Ratatui" src="https://img.shields.io/badge/built_with-ratatui-ff8fb1?style=flat-square"></a>
  <a href="LICENSE"><img alt="MIT license" src="https://img.shields.io/github/license/parda21/cloofy?style=flat-square&color=8b5cf6"></a>
</p>

<p align="center">
  <a href="#install">Install</a> · <a href="#setup">Setup</a> · <a href="#keys">Keys</a> · <a href="#faq">FAQ</a> · <a href="CHANGELOG.md">Changelog</a>
</p>

<p align="center">
  <img src="https://raw.githubusercontent.com/parda21/cloofy/assets/hero.webp" alt="cloofy in a terminal: the Home page with cover art, a hovered card showing its play button, a playlist with the playing song, and the now-playing view with synced lyrics">
  <br>
  <sub>Recorded from <code>cloofy demo</code>, which needs no account. The covers are Spotify's; the library is made up.</sub>
</p>

cloofy is a Spotify client that runs entirely in the terminal. It plays the music itself, with no official app or daemon in the background, and it shows up as a Spotify Connect device you can pick from your phone. It looks and works like the desktop client you know: your Home feed, your library in a sidebar, cover art, synced lyrics, and the DJ talking between tracks.

## Features

- **Plays on its own.** Audio goes through librespot, and cloofy is a Spotify Connect device. It controls your other devices too, and follows them live.
- **Home and the DJ.** Your shelves, Made For You mixes, and the DJ with its voice between tracks. Ask the DJ for something, as in Spotify's apps, or let it pick something different. Crossfade if you like.
- **Your library.** Playlists, albums, artists and podcasts, in a sidebar or as a full page with covers.
- **Every page.** Playlists, albums and artist pages with popular tracks and releases, plus search with artwork.
- **The queue.** See what plays next, on cloofy or the device it controls: the songs you queued, then the rest of the playlist or album. Skip ahead to any of them, or take a queued song out again.
- **Synced lyrics.** The current line lights up next to a large cover in the now-playing view.
- **Cover art.** Real images in terminals that show pictures through the Kitty, Sixel or iTerm2 protocol, and block art everywhere else.
- **Mouse and keyboard.** Click, hover, scroll, drag the seek and volume bars, right-click for actions. Every action also has a key, and <kbd>Ctrl</kbd> <kbd>K</kbd> finds the rest.
- **Settings in the app.** Crossfade, cover art, the accent color, icons, power saving and notifications, on a Settings page like Spotify's. Slow computer? Turn cover art off.
- **Instant.** The last page, list and track are back the moment it starts, and the controls answer before Spotify does.
- **At home on macOS and Linux.** Media keys and the system's media controls: Control Center on macOS, and on Linux GNOME, KDE, Waybar and playerctl. cloofy pauses before the computer sleeps, reconnects when it wakes, and moves less when the system asks for less motion or saves power. On macOS, AirPods and headphones pause playback when they disconnect.

## A closer look

### Now playing, with lyrics in time

<kbd>z</kbd> opens the track in full: a large cover, and lyrics that follow the song line by line.

<img src="https://raw.githubusercontent.com/parda21/cloofy/assets/nowplaying.webp" alt="The now-playing view: a large album cover on the left, synced lyrics on the right with the current line highlighted">

### Every page you know

Playlists and albums with the playing song marked, artist pages with popular tracks and play counts, search with covers, and your whole library as a grid.

<table>
  <tr>
    <td><img src="https://raw.githubusercontent.com/parda21/cloofy/assets/playlist.webp" alt="A playlist page with its mosaic cover, Play and Shuffle buttons, and the track list"></td>
    <td><img src="https://raw.githubusercontent.com/parda21/cloofy/assets/artist.webp" alt="An artist page with a round portrait, monthly listeners, and the popular tracks"></td>
  </tr>
  <tr>
    <td><img src="https://raw.githubusercontent.com/parda21/cloofy/assets/search.webp" alt="Search results for daft punk with album covers"></td>
    <td><img src="https://raw.githubusercontent.com/parda21/cloofy/assets/library.webp" alt="The library as a full page: a grid of playlist covers with filter tabs"></td>
  </tr>
</table>

### A key for everything, or a click

<kbd>Ctrl</kbd> <kbd>K</kbd> opens a command palette for every action and playlist, <kbd>?</kbd> lists all keys, and the hint bar at the bottom shows what works on the current page. All of it works with the mouse too.

<img src="https://raw.githubusercontent.com/parda21/cloofy/assets/palette.webp" alt="The command palette open over Home, listing actions with their keys">

## Try it first

```sh
cloofy demo
```

The demo opens the full interface with a sample library: real albums and artists, and a few made-up playlists. It needs no account and plays nothing, so it's a quick way to see how cloofy looks in your terminal. Every picture on this page comes from it.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/parda21/cloofy/main/install.sh | bash
```

The installer checks your system, puts the latest release in `~/.local/bin`, and walks you through the [setup](#setup): your Spotify app, its client ID, and signing in. cloofy offers new releases itself, see [updates](#updates).

<details>
<summary>Installer options, building from source</summary>

- `bash -s -- --yes` installs without asking anything.
- `bash -s -- --uninstall` removes cloofy.

On platforms without a prebuilt binary, the installer builds cloofy with cargo. To do that yourself (Rust 1.88 or newer):

```sh
cargo install --git https://github.com/parda21/cloofy --locked
```

On Linux the build needs the ALSA and PulseAudio headers:

| Distro | Packages |
| --- | --- |
| Debian, Ubuntu | `libasound2-dev libpulse-dev pkg-config` |
| Fedora | `alsa-lib-devel pulseaudio-libs-devel` |
| Arch | `alsa-lib libpulse` |

</details>

## Setup

> [!NOTE]
> Playing music needs **Spotify Premium**, and reading your library needs a **free Spotify developer app** of your own. The installer guides you through it in about two minutes.

1. Create an app at [developer.spotify.com/dashboard](https://developer.spotify.com/dashboard). Give it any name, set the redirect URI to `http://127.0.0.1:8974/callback`, and pick the Web API.
2. If you listen with another Spotify account than the dashboard's, add it under **User Management**.
3. Put the app's client ID into `~/.config/cloofy/config.toml`:

   ```toml
   client_id = "your client id"
   ```

4. Run `cloofy login`. The browser opens twice: once for your app, once for playback.
5. Run `cloofy doctor` to check that everything works, then `cloofy`.

## Keys

<details>
<summary>All shortcuts</summary>

| Key | Action |
| --- | --- |
| <kbd>space</kbd> | Play or pause |
| <kbd>[</kbd> <kbd>]</kbd> | Previous, next track |
| <kbd>⇧</kbd> <kbd>←</kbd> <kbd>→</kbd> | Seek 5 seconds |
| <kbd>+</kbd> <kbd>-</kbd> | Volume |
| <kbd>s</kbd> <kbd>r</kbd> | Shuffle, repeat |
| <kbd>1</kbd> <kbd>/</kbd> <kbd>3</kbd> | Home, search, your library |
| <kbd>enter</kbd> | Open or play the selection |
| <kbd>esc</kbd> | Back |
| <kbd>f</kbd> | Like the song, or save the album or playlist |
| <kbd>F</kbd> | Follow the artist |
| <kbd>q</kbd> <kbd>a</kbd> | Add to queue, add to a playlist |
| <kbd>.</kbd> | Actions for the selection |
| <kbd>z</kbd> | Now playing and lyrics |
| <kbd>Q</kbd> | Queue |
| <kbd>d</kbd> | Devices |
| <kbd>D</kbd> | Start the DJ; while it plays, ask it for something |
| <kbd>,</kbd> | Settings |
| <kbd>U</kbd> | Update cloofy |
| <kbd>Ctrl</kbd> <kbd>K</kbd> | Command palette |
| <kbd>?</kbd> | All shortcuts |
| <kbd>Ctrl</kbd> <kbd>C</kbd> | Quit |

</details>

Other commands:

```sh
cloofy play <uri or link>   # play without the interface; --shuffle to shuffle
cloofy connect              # wait as a Spotify Connect device until an app picks it
cloofy login --force        # sign in again
cloofy update               # update to the latest release
```

## Updates

When a new release is out, an update button shows in the top bar. Click it or press <kbd>U</kbd>: cloofy downloads the release, checks it against its published checksum and puts it in place of itself, then the same button restarts it. `cloofy update` does the same without the interface. Where cloofy can't write to its own folder, such as `/usr/local/bin`, update with the installer.

## Settings

<kbd>,</kbd>, the command palette, or a click on your name in the top bar opens the Settings page: crossfade, cover art and how covers are drawn, the accent color, icons, power saving, song notifications and update checks. Changes apply at once and cloofy saves them in `~/.config/cloofy/config.toml`, so there's no need to edit the file. You still can; it's read at start:

```toml
client_id = "your client id"
crossfade = 6              # seconds tracks overlap, 0 to 12; off when missing
covers = false             # no cover art, for slower computers; on when missing
accent = "violet"          # "green", "violet", "pink", "blue" or "orange"; follows the cover when missing
images = "blocks"          # "kitty", "sixel", "iterm2" or "blocks"; detected from the terminal when missing
icons = "nerd"             # "nerd" or "text"; detected from the terminal when missing
power_saving = "on"        # "on" or "off"; follows the system when missing
notifications = false      # no desktop notifications for new tracks; on when missing
update_check = false       # don't look for new releases at start; on when missing
credentials = "keychain"   # macOS: sign-ins in the Keychain instead of files (only here)
```

Power saving stops animations and redraws less often. Unless set otherwise, it turns on by itself with Low Power Mode on macOS and the Power Saver profile on Linux.

Inside tmux, notifications need `set -g focus-events on` and `set -g allow-passthrough all` in `~/.tmux.conf`. With `credentials = "keychain"`, cloofy moves its sign-ins into the macOS Keychain on the next start; macOS asks once after each update, because release builds aren't signed yet.

cloofy keeps its sign-ins (unless they are in the Keychain) and its session in `~/.local/share/cloofy`, and its caches and log in `~/.cache/cloofy`.

## Terminals

| Terminal | Cover art | Icons | Notifications |
| --- | --- | --- | --- |
| Ghostty, kitty, WezTerm | Real images | Built in | Yes |
| iTerm2 | Real images | With a Nerd Font | Yes |
| foot, Windows Terminal, Konsole, VS Code, xterm, mlterm | Real images | With a Nerd Font | No |
| Others | Real images with Sixel, else blocks | With a Nerd Font | No |

Every terminal needs true color. Without a Nerd Font, pick text icons in Settings. VS Code shows images once `terminal.integrated.enableImages` is on, xterm when started with `-ti 340`, and Windows Terminal from version 1.22. Inside tmux, covers are real images in Ghostty and kitty and blocks elsewhere, because tmux keeps only a few Sixel images at a time.

## FAQ

<details>
<summary><b>Why do I need my own Spotify app?</b></summary>

Spotify gives its Web API, which cloofy reads your library and searches through, only to registered apps. Apps in development mode are limited to a handful of users, so one shared app couldn't serve everyone. Yours is free and only yours, and the installer sets it up with you.
</details>

<details>
<summary><b>Does it work with a free account?</b></summary>

No. Outside its own apps, Spotify streams only to Premium accounts.
</details>

<details>
<summary><b>What does cloofy send where?</b></summary>

Requests to Spotify, and to [LRCLIB](https://lrclib.net) for lyrics Spotify doesn't have. Nothing else: no telemetry, no server of its own.
</details>

<details>
<summary><b>How does it work?</b></summary>

Playback runs through a [fork of librespot](https://github.com/parda21/librespot/tree/cloofy) that adds the DJ and crossfade from two open upstream pull requests. Your library and search come from the Web API through your own app. Home, artist pages and the DJ use Spotify's internal APIs, the same ones its own apps use. Those are not public and may change without notice.
</details>

## Thanks

To [librespot](https://github.com/librespot-org/librespot) for playing Spotify outside Spotify, including the open pull requests for the [DJ](https://github.com/librespot-org/librespot/pull/1797) and [crossfade](https://github.com/librespot-org/librespot/pull/1756); to [ratatui](https://ratatui.rs), [ratatui-image](https://github.com/ratatui/ratatui-image) and [tachyonfx](https://github.com/junkdog/tachyonfx) for the interface; and to [@yk-sgr](https://github.com/yk-sgr) for everything that makes cloofy feel at home on macOS.

cloofy is not affiliated with Spotify. It is [MIT](LICENSE) licensed.

---

<p align="center"><sub>
  <b>Was this repo created with the help of AI?</b> Yes.<br>
  <b>Do I care, as long as it works?</b> No.
</sub></p>
