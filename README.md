<p align="center">
  <img src="https://raw.githubusercontent.com/parda21/cloofy/assets/logo.svg" alt="cloofy" height="84">
</p>

<p align="center">
  <b>Spotify and YouTube Music in your terminal.</b><br>
  Plays on its own, with Home, Spotify's DJ, synced lyrics, real cover art and a mouse that works everywhere.
</p>

<p align="center">
  <a href="https://github.com/parda21/cloofy/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/parda21/cloofy?style=flat-square&color=8b5cf6&label=release"></a>
  <a href="https://github.com/parda21/cloofy/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/parda21/cloofy/ci.yml?branch=main&style=flat-square&label=ci"></a>
  <a href="https://ratatui.rs"><img alt="Built with Ratatui" src="https://img.shields.io/badge/built_with-ratatui-ff8fb1?style=flat-square"></a>
  <a href="LICENSE"><img alt="MIT license" src="https://img.shields.io/github/license/parda21/cloofy?style=flat-square&color=8b5cf6"></a>
</p>

<p align="center">
  <a href="#install">Install</a> · <a href="#setup">Setup</a> · <a href="#keys">Keys</a> · <a href="#omarchy">Omarchy</a> · <a href="#faq">FAQ</a> · <a href="CHANGELOG.md">Changelog</a>
</p>

<p align="center">
  <img src="https://raw.githubusercontent.com/parda21/cloofy/assets/hero.webp" alt="cloofy in a terminal: the Home page with cover art, a hovered card showing its play button, a playlist with the playing song, and the now-playing view with synced lyrics">
  <br>
  <sub>Recorded from <code>cloofy demo</code>, which needs no account. The covers are Spotify's; the library is made up.</sub>
</p>

cloofy plays Spotify and YouTube Music in the terminal, one of them or both side by side. It plays the music itself, with no official app in the background: Spotify through librespot, as a Spotify Connect device you can pick from your phone, and YouTube Music through mpv, with or without an account. It looks and works like the desktop client you know: your Home feed, your library in a sidebar, cover art, synced lyrics, and on Spotify the DJ talking between tracks.

## Features

- **Plays on its own.** Spotify plays through librespot, and cloofy is a Spotify Connect device. It controls your other devices too, and follows them live. YouTube Music plays through mpv, which cloofy starts and stops itself, with no gap between songs.
- **Spotify, YouTube Music or both.** Install one, or both side by side: <kbd>S</kbd> or the switch in the top bar shows the other's Home, library and pages, as you left them.
- **YouTube Music without an account.** Home, search, every page, lyrics and song radios work signed out. Sign in with your browser for your library and likes.
- **Home and the DJ.** Your shelves and mixes, and on Spotify Made For You and the DJ with its voice between tracks. Ask the DJ for something, as in Spotify's apps, or let it pick something different. Crossfade if you like.
- **Your library.** Playlists, albums, artists, and on Spotify podcasts, in a sidebar or as a full page with covers.
- **Every page.** Playlists, albums and artist pages with popular tracks and releases, plus search with artwork.
- **The queue.** See what plays next, on cloofy or the device it controls: the songs you queued, then the rest of the playlist or album. Skip ahead to any of them, or take a queued song out again. On YouTube Music, the radio of the last song follows once the playlist or album ends.
- **Synced lyrics.** The current line lights up next to a large cover in the now-playing view.
- **Cover art.** Real images in terminals that show pictures through the Kitty, Sixel or iTerm2 protocol, and block art everywhere else.
- **Mouse and keyboard.** Click, hover, scroll, drag the seek and volume bars, right-click for actions. Every action also has a key, and <kbd>Ctrl</kbd> <kbd>K</kbd> finds the rest.
- **Settings in the app.** Crossfade, cover art, the accent color, icons, power saving and notifications, on a Settings page like Spotify's. Slow computer? Turn cover art off.
- **Instant.** The last page, list and track are back the moment it starts, and the controls answer before Spotify does.
- **At home on macOS and Linux.** Media keys and the system's media controls: Control Center on macOS, and on Linux GNOME, KDE, Waybar and playerctl. cloofy pauses before the computer sleeps, reconnects when it wakes, and moves less when the system asks for less motion or saves power. On macOS, AirPods and headphones pause Spotify when they disconnect.
- **At home on Omarchy.** cloofy follows the theme, shows what plays in the bar, and has its place in the menu and the launcher. See [Omarchy](#omarchy).

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

The installer first asks what you listen to: Spotify, YouTube Music, or both. Each comes as a build of its own, so a YouTube Music install carries no Spotify code, and the other way around. Then it checks your system, puts the latest release in `~/.local/bin`, and walks you through the [setup](#setup). For Spotify that's your Spotify app, its client ID and signing in. For YouTube Music it's mpv, yt-dlp and deno, which it offers to install with your package manager, and an optional sign-in. On [Omarchy](https://omarchy.org) it also offers cloofy's [bar widget, theme, menu entries and launcher](#omarchy). Run it again to switch; cloofy offers new releases itself, see [updates](#updates).

Package managers have the same three builds:

| | Spotify | YouTube Music | Both |
| --- | --- | --- | --- |
| [Homebrew](https://brew.sh), macOS and Linux | `cloofy` | `cloofy-ytmusic` | `cloofy-full` |
| [mise](https://mise.jdx.dev), anywhere | `matching=spotify` | `matching=ytmusic` | `matching=full` |
| Arch Linux | `cloofy-bin` | `cloofy-ytmusic-bin` | `cloofy-full-bin` |

```sh
brew install parda21/cloofy/cloofy-ytmusic
mise use -g 'github:parda21/cloofy[matching=ytmusic]'
```

On Arch Linux, as a package built from the PKGBUILD that comes with each release:

```sh
curl -L -o PKGBUILD https://github.com/parda21/cloofy/releases/latest/download/cloofy-ytmusic-bin.PKGBUILD && makepkg -si
```

Homebrew's and Arch's YouTube Music packages bring mpv, yt-dlp and deno along; with mise, install those yourself. Each package replaces the others. Installed with a package manager, go through the [setup](#setup) yourself. The package manager keeps cloofy up to date.

<details>
<summary>Installer options, building from source</summary>

- `bash -s -- --service ytmusic` installs YouTube Music without asking about it: `spotify`, `ytmusic` or `both`.
- `bash -s -- --yes` installs without asking anything: the services you have, or Spotify the first time.
- `bash -s -- --uninstall` removes cloofy, and its Omarchy setup.

On platforms without a prebuilt binary, the installer builds cloofy with cargo, with only the services you picked. To do that yourself (Rust 1.88 or newer), name them as features; without any, you get both:

```sh
cargo install --git https://github.com/parda21/cloofy --locked cloofy --no-default-features --features ytmusic
```

On Linux the Spotify build needs the ALSA and PulseAudio headers:

| Distro | Packages |
| --- | --- |
| Debian, Ubuntu | `libasound2-dev libpulse-dev pkg-config` |
| Fedora | `alsa-lib-devel pulseaudio-libs-devel` |
| Arch | `alsa-lib libpulse` |

YouTube Music needs no headers to build, and [mpv](https://mpv.io), [yt-dlp](https://github.com/yt-dlp/yt-dlp) (2025.11.12 or newer) and [deno](https://deno.com) to play.

</details>

## Setup

Set up the services you installed; the installer does it with you. With both, `cloofy login` signs in to each in turn, and asks before YouTube Music, which plays without an account.

### Spotify

> [!NOTE]
> Playing music needs **Spotify Premium**, and reading your library needs a **free Spotify developer app** of your own. The installer guides you through it in about two minutes.

1. Create an app at [developer.spotify.com/dashboard](https://developer.spotify.com/dashboard). Give it any name, set the redirect URI to `http://127.0.0.1:8974/callback`, and pick the Web API.
2. If you listen with another Spotify account than the dashboard's, add it under **User Management**.
3. Put the app's client ID into `~/.config/cloofy/config.toml`:

   ```toml
   client_id = "your client id"
   ```

4. Run `cloofy login spotify`. The browser opens twice: once for your app, once for playback.
5. Run `cloofy doctor` to check that everything works, then `cloofy`.

### YouTube Music

YouTube Music needs no account. cloofy plays it with [mpv](https://mpv.io) and finds the streams with [yt-dlp](https://github.com/yt-dlp/yt-dlp) 2025.11.12 or newer, which needs [deno](https://deno.com) for YouTube's challenges. The installer offers to install all three with your package manager, and the Homebrew and Arch packages bring them along.

Signed out, Home, search, artist, album and playlist pages, lyrics and song radios work. Signing in adds your library: your playlists, albums, subscribed artists and liked songs, and liking, saving and adding to playlists.

1. Run `cloofy login ytmusic`. cloofy borrows the sign-in of your default browser: yt-dlp reads its cookies, which may ask for your keyring, and cloofy keeps only YouTube's. If the browser isn't signed in to YouTube yet, cloofy opens YouTube's sign-in there and goes on once you're signed in. With a browser cloofy doesn't know, or one it can't read, pick another from the list, or **Another way** to paste the Cookie header of a request to music.youtube.com from the browser's developer tools.
2. Run `cloofy doctor` to check mpv, yt-dlp, deno, music.youtube.com, the sign-in and a stream, then `cloofy`.

On Linux, yt-dlp reads the keyring of Chrome, Brave and other Chromium-based browsers with Python's secretstorage: the installer offers it, and the Arch packages bring it along.

`cloofy logout ytmusic` forgets the sign-in, and YouTube Music plays without an account again. A sign-in or sign-out shows the next time cloofy starts.

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
| <kbd>S</kbd> | Switch between Spotify and YouTube Music |
| <kbd>enter</kbd> | Open or play the selection |
| <kbd>esc</kbd> | Back |
| <kbd>f</kbd> | Like the song, or save the album or playlist |
| <kbd>F</kbd> | Follow the artist |
| <kbd>q</kbd> <kbd>a</kbd> | Add to queue, add to a playlist |
| <kbd>.</kbd> | Actions for the selection |
| <kbd>z</kbd> | Now playing and lyrics |
| <kbd>Q</kbd> | Queue |
| <kbd>d</kbd> | Devices, on Spotify |
| <kbd>D</kbd> | Start the DJ on Spotify; while it plays, ask it for something |
| <kbd>,</kbd> | Settings |
| <kbd>U</kbd> | Update cloofy |
| <kbd>Ctrl</kbd> <kbd>K</kbd> | Command palette |
| <kbd>?</kbd> | All shortcuts |
| <kbd>Ctrl</kbd> <kbd>C</kbd> | Quit |

</details>

Other commands:

```sh
cloofy play <uri or link>   # play a Spotify or YouTube link without the interface; --shuffle to shuffle
cloofy connect              # Spotify: wait as a Spotify Connect device until an app picks it
cloofy login ytmusic        # sign in to one service: spotify or ytmusic
cloofy login --force        # sign in again
cloofy logout ytmusic       # sign out of YouTube Music
cloofy doctor               # check what each service can reach
cloofy omarchy install      # set cloofy up in Omarchy
cloofy update               # update to the latest release
```

## Updates

When a new release is out, an update button shows in the top bar. Click it or press <kbd>U</kbd>: cloofy downloads the release, checks it against its published checksum and puts it in place of itself, then the same button restarts it. `cloofy update` does the same without the interface. An update keeps the build you have: Spotify, YouTube Music or both. Where cloofy can't write to its own folder, such as `/usr/local/bin`, update with the installer.

Installed with Homebrew, mise, a system package or Nix, cloofy leaves its updates to that tool: the button shows the new version, and clicking it or `cloofy update` tells you how to update, such as `brew upgrade cloofy`.

## Settings

<kbd>,</kbd>, the command palette, or a click on your name in the top bar opens the Settings page: crossfade, cover art and how covers are drawn, the accent color, icons, power saving, song notifications and update checks. Changes apply at once and cloofy saves them in `~/.config/cloofy/config.toml`, so there's no need to edit the file. You still can; it's read at start:

```toml
client_id = "your client id"
services = ["ytmusic"]     # "spotify", "ytmusic" or both, in the switch's order; every one built in when missing
crossfade = 6              # seconds tracks overlap, 0 to 12; off when missing
covers = false             # no cover art, for slower computers; on when missing
accent = "violet"          # "theme", "green", "violet", "pink", "blue" or "orange"; follows the cover when missing
images = "blocks"          # "kitty", "sixel", "iterm2" or "blocks"; detected from the terminal when missing
icons = "nerd"             # "nerd" or "text"; detected from the terminal when missing
power_saving = "on"        # "on" or "off"; follows the system when missing
notifications = false      # no desktop notifications for new tracks; on when missing
update_check = false       # don't look for new releases at start; on when missing
credentials = "keychain"   # macOS: sign-ins in the Keychain instead of files (only here)
```

The installer writes `services` with what you listen to. One config file serves every build, so a build leaves out the services it doesn't have. The `"theme"` accent is the Omarchy theme's, and cloofy's green elsewhere.

Power saving stops animations and redraws less often. Unless set otherwise, it turns on by itself with Low Power Mode on macOS and the Power Saver profile on Linux.

Inside tmux, notifications need `set -g focus-events on` and `set -g allow-passthrough all` in `~/.tmux.conf`. With `credentials = "keychain"`, cloofy moves its sign-ins into the macOS Keychain on the next start; macOS asks once after each update, because release builds aren't signed yet.

cloofy keeps its sign-ins (unless they are in the Keychain) and its session in `~/.local/share/cloofy`, and its caches and log in `~/.cache/cloofy`.

## Omarchy

On [Omarchy](https://omarchy.org), cloofy takes its colors from the theme and follows it when you switch, without a restart. Song notifications come from the desktop with the cover, in any terminal, and a click on one brings cloofy to the front. `cloofy omarchy install` adds the rest, and the installer offers to run it:

- **Bar widget.** Next to the clock: the service, a small equalizer while a song plays, and the title and artist, which scroll by when they don't fit. A click opens a card with the cover, the position, the controls and the volume. Middle click plays or pauses, right click brings cloofy to the front, and scrolling skips. Its settings in Omarchy choose the width, whether long titles scroll or fade out, and whether it shows while cloofy isn't open.
- **Theme.** A template Omarchy renders for every theme. Edit it in `~/.config/omarchy/themed/cloofy.toml.tpl` to change how every theme looks in cloofy, or give a theme its own `cloofy.toml`.
- **Menu.** Music in the Omarchy menu: open cloofy, play or pause, next and previous.
- **Launcher.** cloofy with its icon in the app launcher. It opens cloofy, or brings it to the front.
- **Super+Shift+M.** Opens cloofy instead of Spotify, if you say yes or add `--bind-music-key`.

`cloofy omarchy status` shows what is set up, and `cloofy omarchy remove` takes out exactly what `install` added. Running `install` again changes nothing, and running the installer again brings it up to date.

## Terminals

| Terminal | Cover art | Icons | Notifications |
| --- | --- | --- | --- |
| Ghostty, kitty, WezTerm | Real images | Built in | Yes |
| iTerm2 | Real images | With a Nerd Font | Yes |
| foot, Windows Terminal, Konsole, VS Code, xterm, mlterm | Real images | With a Nerd Font | No |
| Others | Real images with Sixel, else blocks | With a Nerd Font | No |

Every terminal needs true color. Without a Nerd Font, pick text icons in Settings. VS Code shows images once `terminal.integrated.enableImages` is on, xterm when started with `-ti 340`, and Windows Terminal from version 1.22. Inside tmux, covers are real images in Ghostty and kitty and blocks elsewhere, because tmux keeps only a few Sixel images at a time. On Omarchy, notifications come from the desktop in every terminal.

## FAQ

<details>
<summary><b>Why do I need my own Spotify app?</b></summary>

Spotify gives its Web API, which cloofy reads your library and searches through, only to registered apps. Apps in development mode are limited to a handful of users, so one shared app couldn't serve everyone. Yours is free and only yours, and the installer sets it up with you.
</details>

<details>
<summary><b>Does Spotify work with a free account?</b></summary>

No. Outside its own apps, Spotify streams only to Premium accounts.
</details>

<details>
<summary><b>Do I need YouTube Premium?</b></summary>

No. YouTube Music plays without any account, and signing in only adds your library and likes: cloofy gets every song's stream the same way either way, without your sign-in.
</details>

<details>
<summary><b>Why mpv and yt-dlp?</b></summary>

YouTube protects its audio streams with challenges that only a JavaScript runtime can solve. yt-dlp solves them, with deno, and an older yt-dlp gets no streams at all, so keep it up to date. mpv plays what yt-dlp finds, and cloofy hands it the next song ahead of time, so there's no gap. cloofy starts mpv in the background without your `mpv.conf` or scripts, and mpv quits with cloofy, even when cloofy crashes.
</details>

<details>
<summary><b>Is signing in to YouTube Music with my browser's cookies safe?</b></summary>

`cloofy login ytmusic` has yt-dlp export the browser's cookies into a private folder, keeps only youtube.com's, and deletes the export right away. They stay in `~/.local/share/cloofy/ytmusic/cookies.txt`, which only you can read, or in the macOS Keychain with `credentials = "keychain"`. cloofy sends them only to music.youtube.com, with its own requests there; streams, covers and lyrics from LRCLIB load without them. They are a signed-in session like the browser's, so treat the file like a password. `cloofy logout ytmusic` deletes it.
</details>

<details>
<summary><b>Do the media keys work on Linux, or do I need playerctl?</b></summary>

On GNOME, KDE and other desktops with media controls they work right away, through MPRIS. On bare window managers like Sway, i3 or Hyprland, the keys need a binding, usually to playerctl. Omarchy binds them already.
</details>

<details>
<summary><b>What does cloofy send where?</b></summary>

Requests to Spotify and YouTube Music, for the services you use, and to [LRCLIB](https://lrclib.net) for lyrics they don't have. For YouTube Music, yt-dlp and mpv fetch the streams from YouTube. cloofy also asks GitHub for new releases, unless `update_check = false`. Nothing else: no telemetry, no server of its own.
</details>

<details>
<summary><b>How does it work?</b></summary>

Spotify playback runs through a [fork of librespot](https://github.com/parda21/librespot/tree/cloofy) that adds the DJ and crossfade from two open upstream pull requests. Your library and search come from the Web API through your own app. Home, artist pages and the DJ use Spotify's internal APIs, the same ones its own apps use.

YouTube Music's pages, search, lyrics and your library come from InnerTube, the API music.youtube.com itself uses. yt-dlp finds each song's stream, and mpv plays it.

Spotify's internal APIs and InnerTube are not public and may change without notice.
</details>

## Thanks

To [librespot](https://github.com/librespot-org/librespot) for playing Spotify outside Spotify, including the open pull requests for the [DJ](https://github.com/librespot-org/librespot/pull/1797) and [crossfade](https://github.com/librespot-org/librespot/pull/1756); to [yt-dlp](https://github.com/yt-dlp/yt-dlp) and [mpv](https://mpv.io) for playing YouTube Music; to [ratatui](https://ratatui.rs), [ratatui-image](https://github.com/ratatui/ratatui-image) and [tachyonfx](https://github.com/junkdog/tachyonfx) for the interface; and to [@yk-sgr](https://github.com/yk-sgr) for everything that makes cloofy feel at home on macOS.

cloofy is not affiliated with Spotify or YouTube. It is [MIT](LICENSE) licensed.

---

<p align="center"><sub>
  <b>Was this repo created with the help of AI?</b> Yes.<br>
  <b>Do I care, as long as it works?</b> No.
</sub></p>
