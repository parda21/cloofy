---
title: Settings
description: cloofy's Settings page, and every option of config.toml.
---

<kbd>,</kbd>, the command palette, or a click on your name in the top bar opens the Settings page: crossfade, cover art and how covers are drawn, the colors, the accent color, icons, power saving, song notifications and update checks. Changes apply at once and cloofy saves them in `~/.config/cloofy/config.toml`, so there's no need to edit the file.

## config.toml

You still can; it's read at start:

```toml
client_id = "your client id"
services = ["ytmusic"]     # "spotify", "ytmusic" or both, in the switch's order; every one built in when missing
crossfade = 6              # seconds tracks overlap, 0 to 12; off when missing
covers = false             # no cover art, for slower computers; on when missing
colors = "terminal"        # "cloofy" or "terminal"; Omarchy's theme there, cloofy's own elsewhere, when missing
accent = "violet"          # "theme", "green", "violet", "pink", "blue" or "orange"; follows the cover when missing
images = "blocks"          # "kitty", "sixel", "iterm2" or "blocks"; detected from the terminal when missing
icons = "nerd"             # "nerd" or "text"; detected from the terminal when missing
power_saving = "on"        # "on" or "off"; follows the system when missing
notifications = false      # no desktop notifications for new tracks; on when missing
update_check = false       # don't look for new releases at start; on when missing
credentials = "keychain"   # macOS: sign-ins in the Keychain instead of files (only here)
```

The installer writes `services` with what you listen to. One config file serves every build, so a build leaves out the services it doesn't have. The `"theme"` accent is the one of the colors in use: `theme.toml`'s when it sets one, else the [Omarchy](/docs/omarchy/) theme's with Colors on Auto, and cloofy's green otherwise, also with Terminal.

## Colors

cloofy draws with its own dark colors, and on Omarchy with the [theme's](/docs/omarchy/). Set Colors to **Terminal** and it takes your terminal's background and text instead, with the shades in between mixed from them, so a purple terminal stays purple. Terminals that don't tell their colors keep cloofy's. **cloofy** keeps cloofy's own colors, on Omarchy too.

To pick colors yourself, put any of them in `~/.config/cloofy/theme.toml`. They go over whichever colors the setting gives, and cloofy follows the file as you edit it, without a restart:

```toml
bg = "#2b0a3d"        # behind everything
surface = "#33124a"   # the sidebar
player = "#30104a"    # the player bar
raised = "#3d1a55"    # cards, menus and buttons
text = "#f4eefa"
muted = "#b7a3c4"     # artists, counts and other secondary text
accent = "#e95420"    # the accent while no cover sets one, and the "Theme" accent
```

The others are `faint`, `danger`, `unfocused`, `disabled`, `placeholder`, `thumb`, `lyric_near`, `lyric_far` and `highlight`; [Omarchy's template](https://github.com/parda21/cloofy/blob/main/integrations/omarchy/cloofy.toml.tpl) says what each is for.

## Cover art

Slow computer? Turn cover art off: cloofy then loads and draws no covers at all. Pages show plain tiles in their place, and Now Playing gives the lyrics the whole screen. Which terminals show real images is on [Terminals](/docs/terminals/).

## Power saving

Power saving stops animations and redraws less often. Unless set otherwise, it turns on by itself with Low Power Mode on macOS and the Power Saver profile on Linux. With animations turned off in the system's settings, slides become fades, lyrics jump to the current line and the equalizer stands still.

## Notifications

Each new track shows a desktop notification while cloofy's window is in the background, in kitty, Ghostty, WezTerm and iTerm2. Inside tmux, notifications need `set -g focus-events on` and `set -g allow-passthrough all` in `~/.tmux.conf`. On Omarchy they come from the desktop, with the cover, in every terminal.

## Keychain

On macOS, `credentials = "keychain"` keeps sign-ins in the Keychain instead of files: cloofy moves them there on the next start. macOS asks once after each update, because release builds aren't signed yet.

## Where cloofy keeps its files

| What | Where |
| --- | --- |
| Settings | `~/.config/cloofy/config.toml` |
| Your own colors | `~/.config/cloofy/theme.toml` |
| Sign-ins (unless in the Keychain) and session | `~/.local/share/cloofy` |
| Caches and the log | `~/.cache/cloofy` |
