---
title: Terminals
description: Which terminals show cover art as real images, icons and notifications.
---

| Terminal | Cover art | Icons | Notifications |
| --- | --- | --- | --- |
| Ghostty, kitty, WezTerm | Real images | Built in | Yes |
| iTerm2 | Real images | With a Nerd Font | Yes |
| foot, Konsole, VS Code, xterm, mlterm | Real images | With a Nerd Font | On Linux |
| Windows Terminal | Real images | With a Nerd Font | No |
| Others | Real images with Sixel, else blocks | With a Nerd Font | On Linux |

Every terminal needs true color. Without a Nerd Font, pick text icons in [Settings](/docs/settings/).

## Real images

cloofy draws covers through the Kitty graphics protocol, Sixel or the iTerm2 protocol, whichever the terminal speaks, and as block art everywhere else. They scroll in and out of view row by row, stay whole under menus, the command palette and the DJ panel, and reach the terminal once instead of with every frame. `images` in `config.toml` picks one by hand.

- **VS Code** shows images once `terminal.integrated.enableImages` is on.
- **xterm** shows them when started with `-ti 340`.
- **Windows Terminal** shows them from version 1.22.

## tmux

Inside tmux, covers are real images in Ghostty and kitty and blocks elsewhere, because tmux keeps only a few Sixel images at a time. Notifications need `set -g focus-events on`, and the terminal's own also `set -g allow-passthrough all`, in `~/.tmux.conf`.

## Notifications

kitty, Ghostty, WezTerm and iTerm2 show notifications themselves, and a click on one brings their window to the front. On Linux, every other terminal gets them from the desktop instead, with the cover: from GNOME, KDE Plasma or a notification daemon like mako or dunst. On Hyprland, [Omarchy](/docs/omarchy/) included, they always come from the desktop, in every terminal, and a click on one brings cloofy to the front. Over SSH, only terminals with notifications of their own show them.
