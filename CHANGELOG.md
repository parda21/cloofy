# Changelog

All notable changes to cloofy. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

YouTube Music joins Spotify, on its own or side by side, and cloofy becomes part of Omarchy.

### Added

- **YouTube Music.** Home, search, artist, album and playlist pages, lyrics and song radios, without an account. cloofy plays it through mpv, with the streams from yt-dlp, and songs follow each other without a gap. The queue, shuffle, repeat, seeking, the volume and the system's media controls work as with Spotify, the last song comes back after a restart, and when a playlist or album ends, the radio of its last song follows, as on YouTube Music. It needs mpv, yt-dlp 2025.11.12 or newer, and deno. The DJ, devices, crossfade and podcasts stay Spotify's; while YouTube Music plays, the hints, the help and the command palette leave out the DJ, devices and crossfade.
- **Your YouTube Music library.** `cloofy login ytmusic` borrows the sign-in of a browser where you're signed in to music.youtube.com: pick the browser from a list, or paste a Cookie header from its developer tools. Signed in, cloofy shows your playlists, albums, subscribed artists and liked songs, and you can like songs, save albums and playlists, subscribe to artists and add songs to playlists. Cookies YouTube turns down aren't kept. `cloofy logout ytmusic` forgets the sign-in.
- **Both side by side.** With both services in use, a switch in the top bar names each, the one shown lit. A click on it, `S` or "Switch to …" in the command palette shows the other service's Home, library, search and pages, as you left them. The player bar shows what plays on either, and its links and the queue open in the service the song comes from. Playing on one pauses the other, and cloofy starts on the service you left it on.
- **What you listen to, and nothing else.** The installer first asks: Spotify, YouTube Music or both. Each is a build of its own, so a YouTube Music install carries no Spotify code, and the other way around. `--service spotify`, `ytmusic` or `both` answers ahead, and running it again with another answer swaps the build, keeping settings and sign-ins. For YouTube Music it checks mpv, yt-dlp and deno, offers to install the missing ones with pacman, apt, dnf, zypper or Homebrew where they have them, and offers the sign-in.
- **A package for each build.** Homebrew has `cloofy`, `cloofy-ytmusic` and `cloofy-full`, and each release comes with PKGBUILDs for `cloofy-bin`, `cloofy-ytmusic-bin` and `cloofy-full-bin` on Arch Linux. Each replaces the others, and the YouTube Music ones bring mpv, yt-dlp and deno along. With mise, `matching=spotify`, `matching=ytmusic` or `matching=full` picks the build.
- **YouTube Music from the command line.** `cloofy play` takes music.youtube.com, youtube.com and youtu.be links; a song plays with its radio after it. `cloofy doctor` checks mpv, yt-dlp and deno, music.youtube.com, the sign-in and a stream.
- **Omarchy's theme.** On Omarchy, cloofy takes its colors from the theme and follows it when it changes, without a restart. Light themes work as well as dark ones, and a theme can bring its own `cloofy.toml`. The accent has a new choice, Theme, which takes the theme's accent, and cloofy's green elsewhere.
- **cloofy in Omarchy's bar.** `cloofy omarchy install` puts a widget next to the clock: the service, a small equalizer while a song plays, and the title and artist, which scroll by when they don't fit. A click opens a card with the cover, the album, the position to click or drag, the controls and the volume. Middle click plays or pauses, right click brings cloofy to the front, and scrolling skips.
- **Omarchy's menu, launcher and Super+Shift+M.** The same command adds Music to the Omarchy menu, to open cloofy, play or pause and skip, and cloofy with its icon to the app launcher. Asked, or with `--bind-music-key`, Super+Shift+M opens cloofy instead of Spotify. `cloofy omarchy status` shows what is set up, and `cloofy omarchy remove` takes out exactly what it added. The installer offers it on Omarchy, and `--uninstall` takes it out again.
- **Notifications with the cover.** On Omarchy, song notifications come from the desktop with the cover, also in terminals without notifications of their own, like Alacritty. Each replaces the one before, and a click brings cloofy's window to the front.
- **More for desktop widgets.** On Linux, they can read and set cloofy's volume and get a link to the song on the web, which also tells them the service. On Hyprland they can bring cloofy's window to the front.

### Changed

- **`cloofy login`** signs in to every service in use, or to the one named, as in `cloofy login spotify`. With YouTube Music among others, it asks before signing in there, as YouTube Music plays without an account. With Spotify only, it works as before.
- **One service doesn't stop the other.** A service that can't start, such as Spotify without a sign-in, no longer keeps cloofy from starting: its pages say why, and cloofy starts on one that works.
- **Updates keep the build.** cloofy updates to the build it is: Spotify, YouTube Music or both. cloofy 0.4 updates to the Spotify build; run the installer to switch.
- **mise** installs cloofy with its github backend, `github:parda21/cloofy`, where `matching` picks the build.
- **`cloofy --version`** names the services built in, such as `(spotify, ytmusic)`.

### Fixed

- Songs LRCLIB has no lyrics for say "No lyrics for this song." instead of "Lyrics are unavailable right now.", as LRCLIB sometimes answers with an error when it has none.
- With bash 5, the installer shows paths in your home folder with `~` too.

## [0.4.0] - 2026-10-27

Settings move into the app, the queue shows what plays next, and cloofy feels at home on Linux.

### Added

- **Settings.** `,`, the command palette or a click on your name in the top bar opens a Settings page: crossfade, cover art and how covers are drawn, the accent color, icons, power saving, song notifications and update checks. Changes apply at once and are saved to `config.toml`, so nobody has to edit it by hand.
- **Queue.** See what plays next, as in Spotify's apps: the song playing now, the songs you queued under "Next in queue" and the rest of the playlist or album under "Next from". `Q`, the queue button next to the volume or the command palette opens it. It keeps up as songs change and as you add songs with `q`, on cloofy and on the device it controls. Enter or a double click skips ahead to a song, and Delete or its menu takes a song you queued out again.
- **Cover art off.** For slower computers: without cover art, cloofy loads and draws no covers at all. Pages show plain tiles in their place, and Now Playing gives the lyrics the whole screen.
- **Power saving, your way.** Power saving can stay on or off for good; Auto follows the system as before.
- **Accent color.** The accent can stay one color instead of following the cover.
- **At home on Linux.** Media keys, playerctl and the media controls of GNOME, KDE and Waybar play, pause, skip and seek, and show the track with its cover, in `cloofy play` and `cloofy connect` too. cloofy pauses before the computer sleeps and reconnects as soon as it wakes. With animations turned off in the desktop's settings, slides become fades, lyrics jump to the current line and the equalizer stands still. In Power Saver mode animations stop and cloofy wakes the computer less often.
- **Covers in more terminals.** Covers are real images in iTerm2, foot, Windows Terminal, Konsole, VS Code, xterm and other terminals that show Sixel or iTerm2 images, no longer only in kitty, Ghostty and WezTerm. They scroll in and out of view row by row, stay whole under menus, the command palette and the DJ panel, and reach the terminal once instead of with every frame. Inside tmux they stay blocks unless the terminal around it is kitty or Ghostty.
- **Package managers.** Install cloofy with `brew install parda21/cloofy/cloofy` on macOS and Linux, with `mise use -g ubi:parda21/cloofy`, or on Arch Linux with `makepkg` from the PKGBUILD that comes with each release. Installed this way, cloofy still tells you about new releases, and leaves installing them to the package manager.

### Changed

- Crossfade (`X`) is saved with the other settings in `config.toml`. The length chosen last moves there by itself.

## [0.3.0] - 2026-10-16

The DJ takes requests, and cloofy updates itself.

### Added

- **Ask DJ.** While DJ plays on cloofy, `D` or the DJ button next to the controls opens the DJ panel, as in Spotify's apps. Type what you'd like to hear or take one of Spotify's suggestions: DJ acknowledges the request and plays what it put together for it. "Pick something for me" jumps to different picks, and DJ announces the change.
- **Updates in one click.** When a new release is out, a button in the top bar says so. Click it or press `U`, and cloofy downloads the release, checks it against its checksum and replaces itself; the same button then restarts into it. `cloofy update` does it from the command line, and `update_check = false` stops cloofy looking.

## [0.2.0] - 2026-10-15

cloofy feels at home on macOS, starts instantly, and can be tried without an account. Thanks to [@yk-sgr](https://github.com/yk-sgr) for everything on macOS.

### Added

- **Media keys.** On macOS, the media keys, AirPods and Control Center play, pause, skip and seek, and Control Center shows the track with its cover. This works in `cloofy play` and `cloofy connect` too.
- **Follows your headphones.** On macOS, playback moves to AirPods or headphones as they connect, and pauses when the output in use disconnects instead of carrying on through the speakers.
- **Reconnecting.** cloofy reconnects to Spotify by itself when the connection drops, after sleep or a network outage, and offers to continue where playback stopped. On macOS it pauses before the Mac sleeps and reconnects as soon as it wakes.
- **Reduce Motion and Low Power Mode.** On macOS with Reduce Motion, slides become fades, lyrics jump to the current line and the equalizer stands still. In Low Power Mode animations stop and cloofy wakes the computer less often.
- **Notifications.** Each new track shows a desktop notification while cloofy's window is in the background, in kitty, Ghostty, WezTerm and iTerm2, also inside tmux with `focus-events on` and `allow-passthrough all`. `notifications = false` turns them off.
- **Keychain.** On macOS, `credentials = "keychain"` keeps sign-ins in the Keychain instead of files. It's off by default, because macOS asks for access again after each update of an unsigned build.
- **Demo.** `cloofy demo` shows the whole interface with a sample library, no account needed.
- **Instant start.** Home, your library and the list you had open show at once from the last run, then refresh in the background.
- **Search with covers.** Results show their artwork, laid out on one grid.

### Changed

- **Smoother screen.** Each frame reaches the terminal whole, so pages and covers no longer tear, and keys and clicks go ahead of loading data.
- **Lighter covers.** Kitty terminals scale covers themselves. Covers load a few at a time, visible ones first and the next page ahead, and non-square ones fill their slot. cloofy needs much less memory for them.
- **Controls answer at once.** Play, pause, skip, seek and volume show on screen right away, on cloofy and on other devices, and don't jump back while Spotify catches up.
- **Other devices.** cloofy follows them through Spotify's own updates instead of asking every 3 seconds.
- **Faster pages.** Pages opened again show at once. Long lists and followed playlists show while they load. Likes, saves and follows show in your library right away.
- **Page changes.** A new page shows at once with a short fade instead of a wipe.
- **Smaller and leaner.** Release builds are smaller and faster, and tracks are buffered on disk instead of in memory.

### Fixed

- On macOS, playback no longer stalls for good when the headphones or AirPods in use disconnect.
- The saved volume applies on start. cloofy always started at 50% and then saved that.
- Help and the command palette no longer crash in small windows.
- Stalled requests give up, rate-limited ones are tried again, and a failed connection to Spotify shows instead of pages that load forever.
- Every key has a mouse action: hint clicks do what their keys do, right-click works on names, play buttons and the player, and clicked targets light up.
- Many layout fixes: columns, highlights, spacing and card rows line up across pages and window sizes.

## [0.1.0] - 2026-10-15

The first release.

### Added

- **Playback.** cloofy plays music itself through librespot and shows up as a Spotify Connect device. It can also control your other devices.
- **Home.** Your shelves and Made For You, the DJ with its voice between tracks, and crossfade.
- **Library.** Playlists, albums, artists and podcasts, in the sidebar and as a full page.
- **Pages and search.** Playlist, album and artist pages, and search.
- **Lyrics.** Synced lyrics in a now-playing view with a large cover.
- **Cover art.** Shown through the Kitty graphics protocol, as half blocks elsewhere.
- **Mouse.** Clicks, hover, the wheel, context menus, and dragging the seek and volume bars.
- **Keyboard.** Shortcuts for everything, plus a command palette.
- **Session restore.** The page, the selection and the last track come back after a restart.
- **Installer.** It sets everything up step by step.

[Unreleased]: https://github.com/parda21/cloofy/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/parda21/cloofy/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/parda21/cloofy/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/parda21/cloofy/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/parda21/cloofy/releases/tag/v0.1.0
