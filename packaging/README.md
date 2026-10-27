# Packages

cloofy's Homebrew formula and AUR packages, as templates. For each release tag, the `packages` job in [`release.yml`](../.github/workflows/release.yml) fills in the version and the release's checksums and pushes the results:

| Template | Published to | Installs with |
| --- | --- | --- |
| [`homebrew/cloofy.rb`](homebrew/cloofy.rb) | `Formula/cloofy.rb` in [parda21/homebrew-cloofy](https://github.com/parda21/homebrew-cloofy) | `brew install parda21/cloofy/cloofy` |
| [`aur/cloofy-bin/`](aur/cloofy-bin) | [aur.archlinux.org/packages/cloofy-bin](https://aur.archlinux.org/packages/cloofy-bin) | `yay -S cloofy-bin` |
| [`aur/cloofy/`](aur/cloofy) | [aur.archlinux.org/packages/cloofy](https://aur.archlinux.org/packages/cloofy) | `yay -S cloofy` |

The formula and `cloofy-bin` install the release builds; `cloofy` builds the tag from source with cargo, submodules included. Both repositories stay as they are until their secret is set: the job then skips them with a notice, and the release itself is unaffected.

## One-time setup

### Homebrew tap

1. Create a public repository named **`homebrew-cloofy`** under `parda21` on GitHub. Empty is fine; the first release adds `Formula/cloofy.rb`. Homebrew finds it as the tap `parda21/cloofy`.
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

`cloofy` and `cloofy-bin` don't exist on the AUR yet; the first push creates them, with your account as their maintainer. The PKGBUILDs name `parda21` and GitHub's no-reply address as maintainer; put your own contact in the `# Maintainer:` line of both templates if you like.

### Publishing the current release

The job runs for tags pushed from now on. To publish a release that is already out, such as 0.3.0, run the same scripts from a checkout:

```sh
gh release download v0.3.0 --repo parda21/cloofy --pattern '*.sha256' --dir /tmp/cloofy-sums
packaging/render.sh 0.3.0 /tmp/cloofy-sums /tmp/cloofy-packages
read -rs HOMEBREW_TAP_TOKEN && export HOMEBREW_TAP_TOKEN    # paste the token
AUR_SSH_PRIVATE_KEY="$(cat ~/.ssh/aur)" packaging/publish.sh /tmp/cloofy-packages
```

Then try it: `brew install parda21/cloofy/cloofy` on a Mac, and `yay -S cloofy-bin` on Arch Linux.

## Changing a package

- Edit the template here, never the published files: the next release overwrites them.
- A PKGBUILD's `.SRCINFO` is kept by hand. After changing a PKGBUILD, run `packaging/check.sh`: it fills in the templates and compares each `.SRCINFO` with what `makepkg --printsrcinfo` makes of its PKGBUILD, in an Arch Linux container if makepkg isn't installed. CI runs it too.
- Every release starts at `pkgrel=1`. To fix a package between releases, push it to the AUR by hand with `pkgrel=2`.
- Placeholders: `@VERSION@`, such as `0.3.0`, and `@SHA256_<TARGET>@` for the checksum of each release archive, such as `@SHA256_AARCH64_APPLE_DARWIN@`.

## Notes

- **Unsigned builds.** Homebrew sets macOS's quarantine flag only on casks, never on what a formula downloads, so Gatekeeper doesn't stop the unsigned binary. That's why cloofy ships as a formula. Apple silicon runs only signed code, and the ad-hoc signature that Apple's linker gives each release build is enough for that.
- **Linux libraries.** The release builds link the system's `libasound.so.2`, `libpulse.so.0` and `libpulse-simple.so.0`. Homebrew on Linux leaves prebuilt binaries as they are, so they use the system's libraries, not Homebrew's; the formula's caveats say so. The AUR packages depend on `alsa-lib`, `libpulse`, `libgcc` and `glibc`.
- **AUR host key.** `publish.sh` only talks to the AUR server whose key has the fingerprint `SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4`, as the [AUR's home page](https://aur.archlinux.org) lists it. Should the AUR ever change its key, update `aur_host_key` there.
- **Updates.** cloofy doesn't replace a binary that Homebrew (under a `Cellar`), pacman (in `/usr/bin`) or Nix (in `/nix/store`) installed. It still shows new releases, and says how to update instead.
