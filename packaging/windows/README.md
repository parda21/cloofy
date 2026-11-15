# Windows

cloofy on Windows: the [install.ps1](install.ps1) route is the one that works today, and the
[scoop manifests](#scoop) are templates for when releases ship Windows builds.

## Install

You need Rust 1.90 or newer with the MSVC toolchain, and CMake and NASM for Spotify's audio
(`scoop install cmake nasm`, or the Visual Studio Build Tools' CMake). YouTube Music needs
[mpv](https://mpv.io), [yt-dlp](https://github.com/yt-dlp/yt-dlp) and
[deno](https://deno.com) on `PATH` to play; `scoop install mpv yt-dlp deno` has all three.

From a checkout:

```powershell
.\packaging\windows\install.ps1                    # YouTube Music
.\packaging\windows\install.ps1 -Service spotify   # Spotify
.\packaging\windows\install.ps1 -Service both      # both
.\packaging\windows\install.ps1 -AddToPath         # and `cloofy` on your PATH
```

It builds the release binary, copies it to `%LOCALAPPDATA%\cloofy\bin`, runs
`cloofy windows install` for the Start Menu shortcut, and only with `-AddToPath` adds that folder
to your user `PATH`. Without it, run cloofy as `%LOCALAPPDATA%\cloofy\bin\cloofy.exe`.

Then set it up as on Linux: `cloofy login` signs in to the services built in, and `cloofy doctor`
checks mpv, yt-dlp, deno, the service and a stream.

## The Start Menu shortcut

`cloofy windows install` writes `%APPDATA%\Microsoft\Windows\Start Menu\Programs\cloofy.lnk`,
pointing at the running `cloofy.exe`, with the AppUserModelID `dev.cloofy.cloofy`. Windows only
shows toasts for an app whose shortcut carries one, so song notifications need it; without it
cloofy says so in its log and sends none. `cloofy windows status` shows whether the shortcut is
there, where it points and whether its AppUserModelID matches, and `cloofy windows remove` deletes
the shortcut and nothing else.

`install.ps1` again brings the shortcut up to date after an update or a move.

## Scoop

[`cloofy-ytmusic.json`](cloofy-ytmusic.json) and [`cloofy-spotify.json`](cloofy-spotify.json) are
templates, like the Homebrew formulas and AUR PKGBUILDs: they name a release archive that no
release produces yet, and a checksum to fill in then. Their `post_install` and `pre_uninstall`
run `cloofy windows install` and `cloofy windows remove`.

## Uninstall

```powershell
.\packaging\windows\uninstall.ps1
```

It removes the shortcut, deletes `%LOCALAPPDATA%\cloofy\bin` and takes that one folder off your
user `PATH`, where `install.ps1` put it. Your configuration and cache in `%APPDATA%\cloofy` and
`%LOCALAPPDATA%\cloofy\cache`, and your sign-ins wherever `credentials` keeps them, stay.

## Check by hand

Tests cover the interface and the system's API calls, but these need ears and eyes:

- Play a song with `CLOOFY_REQUIRE_MPV=1 cargo test -p cloofy-ytmusic` with mpv installed: the
  tests start mpv and talk to it over the named pipe.
- Media keys play, pause and skip, and the system's media panel shows the song, the artist and
  the cover.
- A new song in the background shows a toast with the cover, and the next one replaces it.
- After `cloofy windows remove`, no toast shows.
- Sleeping and waking the computer leaves playback working.
- In Settings, **Reduce motion** follows Windows' animation setting, and Low Power Mode follows
  battery saver.
- `cloofy login ytmusic` lists the browsers with a profile, and importing from one works.
- A Spotify build plays sound, with Spotify Premium and your own client ID.
