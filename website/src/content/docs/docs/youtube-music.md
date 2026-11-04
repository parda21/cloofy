---
title: YouTube Music
description: Play YouTube Music in cloofy through mpv and yt-dlp, signed out or with your library.
---

YouTube Music needs no account. cloofy plays it with [mpv](https://mpv.io) and finds the streams with [yt-dlp](https://github.com/yt-dlp/yt-dlp) 2025.11.12 or newer, which needs [deno](https://deno.com) for YouTube's challenges. The installer offers to install all three with your package manager, and the Homebrew and Arch packages bring them along.

Signed out, Home, search, artist, album and playlist pages, lyrics and song radios work. Signing in adds your library: your playlists, albums, subscribed artists and liked songs, and liking, saving and adding to playlists.

Songs follow each other without a gap, and when a playlist or album ends, the radio of its last song follows, as on YouTube Music. The queue, shuffle, repeat, seeking, the volume and the system's media controls work as with Spotify.

## Signing in

1. Run `cloofy login ytmusic`. cloofy borrows the sign-in of your default browser: yt-dlp reads its cookies, which may ask for your keyring, and cloofy keeps only YouTube's.
2. If the browser isn't signed in to YouTube yet, cloofy opens YouTube's sign-in there and goes on once you're signed in.
3. Pick the account. The browser's cookies sign in every Google account signed in to it, so cloofy lists them by their address, brand accounts too:

   ```
   Sign in to YouTube Music as
   › 1  Ada Lovelace · ada@work.example
     2  Ada Lovelace · ada@gmail.com
     3  Another account: sign in to it in Brave
   ```

   A work account signed in first is never taken by itself. **Another account** opens Google's sign-in for one more in the browser, and cloofy goes on with it.
4. Run `cloofy doctor` to check mpv, yt-dlp, deno, music.youtube.com, the sign-in and its account, and a stream, then `cloofy`.

With a browser cloofy doesn't know, or one it can't read, pick another from the list, or **Another way** to paste the Cookie header of a request to music.youtube.com from the browser's developer tools.

On Linux, yt-dlp reads the keyring of Chrome, Brave and other Chromium-based browsers with Python's secretstorage: the installer offers it, and the Arch packages bring it along.

`cloofy login ytmusic --force` signs in again, with another account if you like. `cloofy logout ytmusic` forgets the sign-in, and YouTube Music plays without an account again. A sign-in or sign-out shows the next time cloofy starts.

## Links

`cloofy play` takes music.youtube.com, youtube.com and youtu.be links, and plays a song with its radio after it:

```sh
cloofy play https://music.youtube.com/watch?v=d7woYjGWs6k
```

## Both services side by side

With Spotify and YouTube Music installed, a switch in the top bar names each, the one shown lit. A click on it, <kbd>S</kbd> or **Switch to …** in the command palette shows the other service's Home, library, search and pages, as you left them. The player bar shows what plays on either, and playing on one pauses the other.

The DJ, devices, crossfade and podcasts are Spotify's.

## Your cookies

`cloofy login ytmusic` has yt-dlp export the browser's cookies into a private folder, keeps only youtube.com's, and deletes the export right away. They stay in `~/.local/share/cloofy/ytmusic/cookies.txt`, which only you can read, or in the macOS Keychain with `credentials = "keychain"`. cloofy sends them only to music.youtube.com, with its own requests there; streams, covers and lyrics from LRCLIB load without them. They are a signed-in session like the browser's, so treat the file like a password. `cloofy logout ytmusic` deletes it.
