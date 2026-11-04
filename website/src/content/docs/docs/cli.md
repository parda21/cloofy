---
title: Command line
description: cloofy's commands, for signing in, checking, playing without the interface and more.
---

`cloofy` on its own opens the interface. Everything else:

```sh
cloofy play <uri or link>   # play a Spotify or YouTube link without the interface; --shuffle to shuffle
cloofy connect              # Spotify: wait as a Spotify Connect device until an app picks it
cloofy login                # sign in to every service in use
cloofy login ytmusic        # sign in to one service: spotify or ytmusic
cloofy login --force        # sign in again, with another account if you like
cloofy logout ytmusic       # sign out of YouTube Music
cloofy doctor               # check what each service can reach
cloofy demo                 # the interface with a sample library, no account needed
cloofy omarchy install      # set cloofy up in Omarchy
cloofy update               # update to the latest release
cloofy --version            # the version, and the services built in
```

## play

`cloofy play` takes a URI or a link to a song, album, playlist or artist, and plays it without the interface:

```sh
cloofy play https://open.spotify.com/album/2noRn2Aes5aoNVsU6iWThc
cloofy play https://music.youtube.com/watch?v=d7woYjGWs6k --shuffle
```

Media keys and the system's media controls work while it plays.

## login

`cloofy login` signs in to every service in use, or to the one named. With YouTube Music among others, it asks before signing in there, as YouTube Music plays without an account. `--force` signs in again where already signed in. `--check` only checks, quietly: it exits with 0 when signed in, 1 when not.

## doctor

`cloofy doctor` checks what each service can reach: for Spotify your app, the Web API and playback, for YouTube Music mpv, yt-dlp, deno, music.youtube.com, the sign-in with its account, and a stream. Start here when something doesn't work; see [Troubleshooting](/docs/troubleshooting/).

## omarchy

`cloofy omarchy install` sets cloofy up in [Omarchy](/docs/omarchy/); `--bind-music-key` also makes Super+Shift+M open cloofy, and `--yes` asks nothing. `cloofy omarchy status` shows what is set up, and `cloofy omarchy remove` takes out exactly what `install` added.
