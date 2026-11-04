---
title: Troubleshooting
description: What to check when something doesn't work, and what to send along with a bug report.
---

## Start with doctor

```sh
cloofy doctor
```

It checks what each service can reach, one line per check: for Spotify your app, the Web API, the session and playback; for YouTube Music mpv, yt-dlp, deno, music.youtube.com, the sign-in with its account, and a stream. A line with ✗ says what failed.

## Spotify

- **"Spotify refused the key to a song."** Spotify turns down librespot's requests for the keys to songs at times, for every program built on librespot ([librespot#1649](https://github.com/librespot-org/librespot/issues/1649)). For many accounts it acts like a limit: cloofy asks for one key at a time, and after a refusal waits 3, then 5, then 10 seconds before it asks again, which usually ends it. Skipping through many songs quickly brings it on. Some accounts get every key refused, which only Spotify can change; YouTube Music isn't affected.
- **"Playback didn't start" or no devices.** Spotify streams only to Premium accounts. In `cloofy doctor`, **Profile** and **Session** should show the same account; if not, sign in again with `cloofy login spotify --force`.
- **Your library is empty or the Web API fails.** Check the client ID in `~/.config/cloofy/config.toml`, the redirect URI `http://127.0.0.1:8974/callback` in your app, and that your account is under **User Management** if it isn't the dashboard's.

## YouTube Music

- **Songs don't start.** YouTube changes its challenges now and then, and an older yt-dlp gets no streams at all. Update yt-dlp (2025.11.12 or newer) and make sure deno is installed; `cloofy doctor` checks both.
- **The wrong account.** Run `cloofy login ytmusic --force` and pick the right one. See [Signing in](/docs/youtube-music/#signing-in).
- **The sign-in has expired.** When YouTube no longer takes the sign-in, for example after you signed out of Google in the browser, cloofy says so. Run `cloofy login ytmusic` again.
- **yt-dlp can't decrypt the browser's cookies.** On Linux, yt-dlp needs Python's secretstorage to read the keyring of Chromium-based browsers: install `python-secretstorage` (Arch) or `python3-secretstorage` (Debian, Ubuntu, Fedora).

## No sound

cloofy's sound shows up in your system's mixer as **cloofy**, with the song's title. If it plays there but you hear nothing, check the output device and its volume in the mixer; another app playing silently points to the system rather than cloofy.

## The log

The interface writes its log to `~/.cache/cloofy/cloofy.log`. `CLOOFY_LOG` sets how much goes in, for example:

```sh
CLOOFY_LOG=info,librespot=debug cloofy
```

Commands such as `cloofy play` log to the terminal instead:

```sh
CLOOFY_LOG=info cloofy play https://music.youtube.com/watch?v=d7woYjGWs6k 2> play.log
```

## Reporting a bug

Open an issue on [GitHub](https://github.com/parda21/cloofy/issues) with `cloofy --version`, your system and terminal, the output of `cloofy doctor`, and the end of the log. Remove anything private first: the log and doctor may show your account's name.
