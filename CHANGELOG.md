# Changelog

All notable changes to cloofy. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Media keys.** On macOS, the media keys, AirPods and Control Center play, pause, skip and seek, and Control Center shows the track with its cover. This works in `cloofy play` and `cloofy connect` too.
- **Follows your headphones.** On macOS, playback moves to AirPods or headphones as they connect, and pauses when the output in use disconnects instead of carrying on through the speakers.
- **Reconnecting.** cloofy reconnects to Spotify by itself when the connection drops, after sleep or a network outage, and offers to continue where playback stopped. On macOS it pauses before the Mac sleeps and reconnects as soon as it wakes.
- **Reduce Motion and Low Power Mode.** On macOS with Reduce Motion, slides become fades, lyrics jump to the current line and the equalizer stands still. In Low Power Mode animations stop and cloofy wakes the computer less often.
- **Notifications.** Each new track shows a desktop notification while cloofy's window is in the background, in kitty, Ghostty, WezTerm and iTerm2, also inside tmux with `focus-events on` and `allow-passthrough all`. `notifications = false` turns them off.
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

[Unreleased]: https://github.com/parda21/cloofy/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/parda21/cloofy/releases/tag/v0.1.0
