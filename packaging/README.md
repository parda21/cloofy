# Packages

cloofy's Homebrew formulas and AUR packages, as templates. For each release tag, the `packages` job in [`release.yml`](../.github/workflows/release.yml) fills in the version and the release's checksums and pushes the results.

Each release has three builds, and a package for each: Spotify, YouTube Music, or both. A build has only the services it's named for, so a YouTube Music install carries no Spotify code and the other way around. The names without a service stay Spotify's, as cloofy was before 0.5.

| Build | Homebrew, in [parda21/homebrew-cloofy](https://github.com/parda21/homebrew-cloofy) | AUR | Release archives |
| --- | --- | --- | --- |
| Spotify | [`cloofy`](homebrew/cloofy.rb) | [`cloofy-bin`](aur/cloofy-bin), [`cloofy`](aur/cloofy) from source | `cloofy-spotify-<target>.tar.gz` |
| YouTube Music | [`cloofy-ytmusic`](homebrew/cloofy-ytmusic.rb) | [`cloofy-ytmusic-bin`](aur/cloofy-ytmusic-bin) | `cloofy-ytmusic-<target>.tar.gz` |
| Both | [`cloofy-full`](homebrew/cloofy-full.rb) | [`cloofy-full-bin`](aur/cloofy-full-bin) | `cloofy-full-<target>.tar.gz` |

They install with `brew install parda21/cloofy/cloofy-ytmusic` and `yay -S cloofy-ytmusic-bin`, and so on. Each installs a `cloofy` binary, so each conflicts with the others: Homebrew's `conflicts_with`, and on the AUR they all provide and conflict with `cloofy`, so pacman swaps one for another. The YouTube Music packages depend on mpv, yt-dlp and deno.

The formulas and the `-bin` packages install the release builds; `cloofy` builds the tag from source with cargo, submodules included, with `--no-default-features --features spotify`. Releases also carry `cloofy-<target>.tar.gz`, a copy of the Spotify build that cloofy 0.4 and older update from, and the three `-bin` PKGBUILDs as `cloofy-bin.PKGBUILD` and so on, plus `PKGBUILD`, Spotify's under the name 0.4's instructions download.

Both repositories stay as they are until their secret is set: the job then skips them with a notice, and the release itself is unaffected.

## One-time setup

### Homebrew tap

1. Create a public repository named **`homebrew-cloofy`** under `parda21` on GitHub. Empty is fine; the first release adds `Formula/cloofy.rb`, `cloofy-ytmusic.rb` and `cloofy-full.rb`. Homebrew finds it as the tap `parda21/cloofy`.
2. Create a fine-grained token at GitHub → Settings → Developer settings → Personal access tokens → [Fine-grained tokens](https://github.com/settings/personal-access-tokens/new):
   - Repository access: **Only select repositories** → `parda21/homebrew-cloofy`
   - Permissions: **Contents: Read and write** (Metadata: Read-only comes with it)
   - Expiration: as long as you like. When it runs out, the job fails at the tap until you put in a new token.
3. In [parda21/cloofy → Settings → Secrets and variables → Actions](https://github.com/parda21/cloofy/settings/secrets/actions), add it as a repository secret named **`HOMEBREW_TAP_TOKEN`**.

### AUR

1. Register an account at [aur.archlinux.org/register](https://aur.archlinux.org/register).
2. Make an SSH key for publishing only, without a passphrase, since the workflow uses it:

   ```sh
   ssh-keygen -t ed25519 -C "cloofy AUR" -N "" -f ~/.ssh/aur
   ```

3. On the AUR, under **My Account**, paste `~/.ssh/aur.pub` into **SSH Public Key** and save.
4. Add the private key, all of `~/.ssh/aur` including its `BEGIN` and `END` lines, as a repository secret named **`AUR_SSH_PRIVATE_KEY`**.

`cloofy`, `cloofy-bin`, `cloofy-ytmusic-bin` and `cloofy-full-bin` don't exist on the AUR yet; the first push creates them, with your account as their maintainer. The PKGBUILDs name `parda21` and GitHub's no-reply address as maintainer; put your own contact in the `# Maintainer:` line of the templates if you like.

### Publishing the current release

The job runs for tags pushed from now on. To publish a release that is already out, such as 0.5.0, run the same scripts from a checkout. Releases before 0.5 have only the Spotify build, under the old name, so the templates here don't fit them.

```sh
gh release download v0.5.0 --repo parda21/cloofy --pattern '*.sha256' --dir /tmp/cloofy-sums
packaging/render.sh 0.5.0 /tmp/cloofy-sums /tmp/cloofy-packages
read -rs HOMEBREW_TAP_TOKEN && export HOMEBREW_TAP_TOKEN    # paste the token
AUR_SSH_PRIVATE_KEY="$(cat ~/.ssh/aur)" packaging/publish.sh /tmp/cloofy-packages
```

Then try it: `brew install parda21/cloofy/cloofy-ytmusic` on a Mac, and `yay -S cloofy-ytmusic-bin` on Arch Linux.

## Changing a package

- Edit the template here, never the published files: the next release overwrites them.
- A PKGBUILD's `.SRCINFO` is kept by hand. After changing a PKGBUILD, run `packaging/check.sh`: it fills in the templates, checks that each package downloads its own build, and compares each `.SRCINFO` with what `makepkg --printsrcinfo` makes of its PKGBUILD, in an Arch Linux container if makepkg isn't installed. CI runs it too.
- A new package goes into the lists at the top of `render.sh`, `publish.sh` and `check.sh`, and the release workflow's PKGBUILD step if it's a `-bin` one.
- Every release starts at `pkgrel=1`. To fix a package between releases, push it to the AUR by hand with `pkgrel=2`.
- Placeholders: `@VERSION@`, such as `0.5.0`, and `@SHA256_<BUILD>_<TARGET>@` for the checksum of each release archive, such as `@SHA256_YTMUSIC_AARCH64_APPLE_DARWIN@` for `cloofy-ytmusic-aarch64-apple-darwin.tar.gz`.

## Notes

- **Unsigned builds.** Homebrew sets macOS's quarantine flag only on casks, never on what a formula downloads, so Gatekeeper doesn't stop the unsigned binary. That's why cloofy ships as a formula. Apple silicon runs only signed code, and the ad-hoc signature that Apple's linker gives each release build is enough for that.
- **Linux libraries.** The Spotify and full builds link the system's `libasound.so.2`, `libpulse.so.0` and `libpulse-simple.so.0`, for librespot; the YouTube Music build links neither, as mpv plays its music, and the release workflow fails if it does. Homebrew on Linux leaves prebuilt binaries as they are, so they use the system's libraries, not Homebrew's; the formulas' caveats say so. The AUR packages depend on `libgcc` and `glibc`, plus `alsa-lib` and `libpulse` with Spotify, and `mpv`, `yt-dlp` and `deno` with YouTube Music.
- **YouTube Music's programs.** cloofy plays YouTube Music with mpv and finds the streams with yt-dlp, which needs deno for YouTube's challenges (yt-dlp 2025.11.12 or newer). The formulas and AUR packages with YouTube Music depend on all three; the installer offers to install them with the system's package manager.
- **AUR host key.** `publish.sh` only talks to the AUR server whose key has the fingerprint `SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4`, as the [AUR's home page](https://aur.archlinux.org) lists it. Should the AUR ever change its key, update `aur_host_key` there.
- **Updates.** cloofy doesn't replace a binary that Homebrew (under a `Cellar`), pacman (in `/usr/bin`) or Nix (in `/nix/store`) installed. It still shows new releases, and says how to update instead.
