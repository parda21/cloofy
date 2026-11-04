---
title: Updates
description: How cloofy updates itself, and how it leaves updates to package managers.
---

When a new release is out, an update button shows in the top bar. Click it or press <kbd>U</kbd>: cloofy downloads the release, checks it against its published checksum and puts it in place of itself, then the same button restarts it. `cloofy update` does the same without the interface.

An update keeps the build you have: Spotify, YouTube Music or both. To switch, run the [installer](/docs/install/) again. Where cloofy can't write to its own folder, such as `/usr/local/bin`, update with the installer.

## With a package manager

Installed with Homebrew, mise, a system package or Nix, cloofy leaves its updates to that tool: the button shows the new version, and clicking it or `cloofy update` tells you how to update, such as `brew upgrade cloofy`.

## Turning it off

`update_check = false` in `config.toml`, or **Update checks** on the [Settings](/docs/settings/) page, stops cloofy looking for new releases at start.

## What changed

Every release is in the [changelog](https://github.com/parda21/cloofy/blob/main/CHANGELOG.md) and on [GitHub's release page](https://github.com/parda21/cloofy/releases).
