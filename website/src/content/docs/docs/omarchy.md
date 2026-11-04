---
title: Omarchy
description: cloofy on Omarchy, with the theme's colors, a bar widget, the menu, the launcher and Super+Shift+M.
---

On [Omarchy](https://omarchy.org), cloofy takes its colors from the theme and follows it when you switch, without a restart. Light themes work as well as dark ones. Song notifications come from the desktop with the cover, in any terminal, and a click on one brings cloofy to the front.

`cloofy omarchy install` adds the rest, and the installer offers to run it:

- **Bar widget.** Next to the clock: the service, a small equalizer while a song plays, and the title and artist, which scroll by when they don't fit. A click opens a card with the cover, the position, the controls and the volume. Middle click plays or pauses, right click brings cloofy to the front, and scrolling skips. Its settings in Omarchy choose the width, whether long titles scroll or fade out, and whether it shows while cloofy isn't open.
- **Theme.** A template Omarchy renders for every theme. Edit it in `~/.config/omarchy/themed/cloofy.toml.tpl` to change how every theme looks in cloofy, or give a theme its own `cloofy.toml`.
- **Menu.** Music in the Omarchy menu: open cloofy, play or pause, next and previous.
- **Launcher.** cloofy with its icon in the app launcher. It opens cloofy, or brings it to the front.
- **Super+Shift+M.** Opens cloofy instead of Spotify, if you say yes or add `--bind-music-key`. A binding of your own stays as it is.

`cloofy omarchy status` shows what is set up, and `cloofy omarchy remove` takes out exactly what `install` added. Running `install` again changes nothing, and running the installer again brings it up to date.

## The accent

The **Theme** accent in [Settings](/docs/settings/) takes the theme's accent color, and follows it when you switch themes.

## Media keys

Omarchy binds the media keys already, so they play, pause and skip in cloofy, and so do `playerctl` and the bar.
