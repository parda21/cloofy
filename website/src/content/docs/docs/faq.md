---
title: FAQ
description: Questions about Spotify, YouTube Music, privacy and how cloofy works.
---

## Why do I need my own Spotify app?

Spotify gives its Web API, which cloofy reads your library and searches through, only to registered apps. Apps in development mode are limited to a handful of users, so one shared app couldn't serve everyone. Yours is free and only yours, and the installer sets it up with you.

## Does Spotify work with a free account?

No. Outside its own apps, Spotify streams only to Premium accounts.

## Do I need YouTube Premium?

No. YouTube Music plays without any account, and signing in only adds your library and likes: cloofy gets every song's stream the same way either way, without your sign-in.

## Why mpv and yt-dlp?

YouTube protects its audio streams with challenges that only a JavaScript runtime can solve. yt-dlp solves them, with deno, and an older yt-dlp gets no streams at all, so keep it up to date. mpv plays what yt-dlp finds, and cloofy hands it the next song ahead of time, so there's no gap. cloofy starts mpv in the background without your `mpv.conf` or scripts, and mpv quits with cloofy, even when cloofy crashes.

## Is signing in to YouTube Music with my browser's cookies safe?

`cloofy login ytmusic` has yt-dlp export the browser's cookies into a private folder, keeps only youtube.com's, and deletes the export right away. They stay in `~/.local/share/cloofy/ytmusic/cookies.txt`, which only you can read, or in the macOS Keychain with `credentials = "keychain"`. cloofy sends them only to music.youtube.com, with its own requests there; streams, covers and lyrics from LRCLIB load without them. They are a signed-in session like the browser's, so treat the file like a password. `cloofy logout ytmusic` deletes it.

## cloofy signed in to YouTube Music with my work account. How do I switch?

Run `cloofy login ytmusic --force` and pick the other account from the list. cloofy lists every Google account signed in to your browser by its address; if yours isn't there, **Another account** signs in to it in the browser first.

## Do the media keys work on Linux, or do I need playerctl?

On GNOME, KDE and other desktops with media controls they work right away, through MPRIS. On bare window managers like Sway, i3 or Hyprland, the keys need a binding, usually to playerctl. Omarchy binds them already.

## What does cloofy send where?

Requests to Spotify and YouTube Music, for the services you use, and to [LRCLIB](https://lrclib.net) for lyrics they don't have. For YouTube Music, yt-dlp and mpv fetch the streams from YouTube. cloofy also asks GitHub for new releases, unless `update_check = false`. Nothing else: no telemetry, no server of its own.

## How does it work?

Spotify playback runs through a [fork of librespot](https://github.com/parda21/librespot/tree/cloofy) that adds the DJ and crossfade from two open upstream pull requests. Your library and search come from the Web API through your own app. Home, artist pages and the DJ use Spotify's internal APIs, the same ones its own apps use.

YouTube Music's pages, search, lyrics and your library come from InnerTube, the API music.youtube.com itself uses. yt-dlp finds each song's stream, and mpv plays it.

Spotify's internal APIs and InnerTube are not public and may change without notice.

## Is cloofy affiliated with Spotify or YouTube?

No. cloofy is an independent open-source project, [MIT](https://github.com/parda21/cloofy/blob/main/LICENSE) licensed.
