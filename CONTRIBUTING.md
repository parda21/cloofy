# Contributing

cloofy builds with Rust 1.90 or newer. `git clone --recursive`, or `git submodule update --init` after cloning: the workspace patches librespot and ratatui-image with the forks in `vendor/`.

## Tests

```sh
cargo test --workspace
```

runs everything that needs neither an account nor the network:

- **Unit tests** next to the code, and the parsers of both services against recorded answers in `crates/*/tests/fixtures`.
- **The interface, whole.** `crates/cloofy-tui/src/harness.rs` drives the app with keys, clicks and resizes and reads the screen back as text. Time stands still until a test moves it on with `wait`. `scenarios.rs` runs on the demo's sample, and `scenarios/playback.rs` on fake services (`backend/fake.rs`), whose players play and record what they're told.
- **Screens.** Some scenarios compare the screen with `crates/cloofy-tui/tests/screens/*.txt`. After a change to how something looks, take the new screens and check the diff before committing:

  ```sh
  CLOOFY_UPDATE_SCREENS=1 cargo test -p cloofy-tui
  ```

- **The binary** in a terminal of its own (`crates/cloofy/tests/smoke.rs`): it starts, takes keys, and leaves the terminal as it found it.

Some tests need a program and skip themselves without it. CI has them all and sets these variables, so there they fail instead of skipping:

| Needs | Tests | In CI |
|---|---|---|
| mpv | YouTube Music's player | `CLOOFY_REQUIRE_MPV=1` |
| a D-Bus session bus (Linux) | the media controls over MPRIS, the desktop's notifications | `CLOOFY_REQUIRE_DBUS=1`, under `dbus-run-session` |

Tests that go online are `#[ignore = "online"]`. Run them with:

```sh
cargo test -p cloofy-ytmusic -- --ignored
```

The nightly workflow runs the catalog's (`--test live`) against the live service. The stream test, which asks yt-dlp for a song's audio, only works from a home connection: from GitHub's servers, YouTube asks yt-dlp to sign in. Run it before a release, ideally without deno on `PATH`, since cloofy should need no JavaScript runtime.

## Fixes

A fix comes with a test that fails without it: a scenario when it shows on screen, a unit test otherwise.
