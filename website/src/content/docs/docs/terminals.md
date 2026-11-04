---
title: Terminals
description: Which terminals show cover art as real images, icons and notifications.
---

| Terminal | Cover art | Icons | Notifications |
| --- | --- | --- | --- |
| Ghostty, kitty, WezTerm | Real images | Built in | Yes |
| iTerm2 | Real images | With a Nerd Font | Yes |
| foot, Windows Terminal, Konsole, VS Code, xterm, mlterm | Real images | With a Nerd Font | No |
| Others | Real images with Sixel, else blocks | With a Nerd Font | No |

Every terminal needs true color. Without a Nerd Font, pick text icons in [Settings](/docs/settings/).

## Real images

cloofy draws covers through the Kitty graphics protocol, Sixel or the iTerm2 protocol, whichever the terminal speaks, and as block art everywhere else. They scroll in and out of view row by row, stay whole under menus, the command palette and the DJ panel, and reach the terminal once instead of with every frame. `images` in `config.toml` picks one by hand.

- **VS Code** shows images once `terminal.integrated.enableImages` is on.
- **xterm** shows them when started with `-ti 340`.
- **Windows Terminal** shows them from version 1.22.

## tmux

Inside tmux, covers are real images in Ghostty and kitty and blocks elsewhere, because tmux keeps only a few Sixel images at a time. Notifications need `set -g focus-events on` and `set -g allow-passthrough all` in `~/.tmux.conf`.

## Omarchy

On [Omarchy](/docs/omarchy/), notifications come from the desktop in every terminal, with the cover.
