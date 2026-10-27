# Changelog

All notable changes to cloofy. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Settings.** `,`, the command palette or a click on your name in the top bar opens a Settings page: crossfade, cover art, the accent color, icons, power saving, song notifications and update checks. Changes apply at once and are saved to `config.toml`, so nobody has to edit it by hand.
- **Queue.** See what plays next, as in Spotify's apps: the song playing now, the songs you queued under "Next in queue" and the rest of the playlist or album under "Next from". `Q`, the queue button next to the volume or the command palette opens it. It keeps up as songs change and as you add songs with `q`, on cloofy and on the device it controls. Enter or a double click skips ahead to a song, and Delete or its menu takes a song you queued out again.
- **Cover art off.** For slower computers: without cover art, cloofy loads and draws no covers at all. Pages show plain tiles in their place, and Now Playing gives the lyrics the whole screen.
- **Power saving, your way.** Power saving can stay on or off for good; Auto follows the system as before.
- **Accent color.** The accent can stay one color instead of following the cover.
- **At home on Linux.** Media keys, playerctl and the media controls of GNOME, KDE and Waybar play, pause, skip and seek, and show the track with its cover, in `cloofy play` and `cloofy connect` too. cloofy pauses before the computer sleeps and reconnects as soon as it wakes. With animations turned off in the desktop's settings, slides become fades, lyrics jump to the current line and the equalizer stands still. In Power Saver mode animations stop and cloofy wakes the computer less often.
- **Covers in more terminals.** Covers are real images in iTerm2, foot, Windows Terminal, Konsole, VS Code, xterm and other terminals that show Sixel or iTerm2 images, no longer only in kitty, Ghostty and WezTerm. They scroll in and out of view row by row, stay whole under menus, the command palette and the DJ panel, and reach the terminal once instead of with every frame. Inside tmux they stay blocks unless the terminal around it is kitty or Ghostty.

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

[Unreleased]: https://github.com/parda21/cloofy/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/parda21/cloofy/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/parda21/cloofy/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/parda21/cloofy/releases/tag/v0.1.0
