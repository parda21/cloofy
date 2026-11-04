---
title: Spotify
description: Set up Spotify in cloofy, with your own free developer app and Spotify Premium.
---

cloofy plays Spotify itself, through [librespot](https://github.com/librespot-org/librespot), and is a Spotify Connect device you can pick from your phone. It also controls your other devices, and follows them live.

:::note[What you need]
Playing music needs **Spotify Premium**, and reading your library needs a **free Spotify developer app** of your own. The installer guides you through it in about two minutes.
:::

## Setup

1. Create an app at [developer.spotify.com/dashboard](https://developer.spotify.com/dashboard). Give it any name, set the redirect URI to `http://127.0.0.1:8974/callback`, and pick the Web API.
2. If you listen with another Spotify account than the dashboard's, add it under **User Management**.
3. Put the app's client ID into `~/.config/cloofy/config.toml`:

   ```toml
   client_id = "your client id"
   ```

4. Run `cloofy login spotify`. The browser opens twice: once for your app, once for playback.
5. Run `cloofy doctor` to check that everything works, then `cloofy`.

With both services installed, `cloofy login` signs in to each in turn.

## The DJ

<kbd>D</kbd> starts Spotify's DJ, with its voice between tracks. While it plays, <kbd>D</kbd> or the DJ button opens the DJ panel, as in Spotify's apps: type what you'd like to hear, or take one of Spotify's suggestions. **Pick something for me** jumps to different picks, and the DJ announces the change.

## Devices and remote mode

<kbd>d</kbd> lists your Spotify devices and moves playback between them. `cloofy connect` runs cloofy without the interface, as a Spotify Connect device that waits until an app picks it; `--name` sets the name it shows up as.

## Crossfade

<kbd>X</kbd> or the [Settings](/docs/settings/) page sets how many seconds tracks overlap, from 0 to 12.

## Why your own app?

Spotify gives its Web API, which cloofy reads your library and searches through, only to registered apps. Apps in development mode are limited to a handful of users, so one shared app couldn't serve everyone. Yours is free and only yours.
