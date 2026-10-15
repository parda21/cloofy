#!/bin/sh
# Installs cloofy, the Spotify client for the terminal:
#
#   curl -fsSL https://raw.githubusercontent.com/parda21/cloofy/main/install.sh | sh
#
# Takes the prebuilt binary from the latest release when there is one for this machine, and
# builds from source with cargo otherwise. Run it again to update.
#
#   CLOOFY_INSTALL_DIR  where the binary goes (default: ~/.local/bin)
#   CLOOFY_VERSION      a release tag such as v0.1.0 (default: the latest release)

set -eu

repo="parda21/cloofy"
bin_dir="${CLOOFY_INSTALL_DIR:-$HOME/.local/bin}"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

say() { printf '  %s\n' "$*"; }
fail() {
	printf '\n  error: %s\n\n' "$*" >&2
	exit 1
}
has() { command -v "$1" >/dev/null 2>&1; }

# The Rust target triple of this machine, as release archives are named.
target() {
	case "$(uname -m)" in
	x86_64 | amd64) arch=x86_64 ;;
	aarch64 | arm64) arch=aarch64 ;;
	*) return 1 ;;
	esac
	case "$(uname -s)" in
	Linux) echo "$arch-unknown-linux-gnu" ;;
	Darwin) echo "$arch-apple-darwin" ;;
	*) return 1 ;;
	esac
}

download() {
	if has curl; then
		curl -fsSL --retry 2 -o "$2" "$1"
	elif has wget; then
		wget -q -O "$2" "$1"
	else
		fail "curl or wget is needed to download cloofy"
	fi
}

sha256() {
	if has sha256sum; then
		sha256sum "$1" | cut -d ' ' -f 1
	else
		shasum -a 256 "$1" | cut -d ' ' -f 1
	fi
}

# The distro's package manager command for the given Debian, Fedora and Arch package names.
packages() {
	# shellcheck source=/dev/null # the file belongs to the system, not this script
	id=$(. /etc/os-release 2>/dev/null && echo "$ID ${ID_LIKE:-}")
	case "$id" in
	*debian* | *ubuntu*) echo "sudo apt install $1" ;;
	*fedora* | *rhel*) echo "sudo dnf install $2" ;;
	*arch*) echo "sudo pacman -S $3" ;;
	*) echo "the packages for ALSA and PulseAudio ($1)" ;;
	esac
}

install_release() {
	triple=$(target) || return 1
	if [ -n "${CLOOFY_VERSION:-}" ]; then
		base="https://github.com/$repo/releases/download/$CLOOFY_VERSION"
	else
		base="https://github.com/$repo/releases/latest/download"
	fi
	archive="cloofy-$triple.tar.gz"
	download "$base/$archive" "$tmp/$archive" 2>/dev/null || return 1
	say "Downloaded $archive"
	download "$base/$archive.sha256" "$tmp/$archive.sha256" || fail "no checksum published for $archive"
	[ "$(sha256 "$tmp/$archive")" = "$(cut -d ' ' -f 1 <"$tmp/$archive.sha256")" ] ||
		fail "$archive does not match its checksum"
	tar -xzf "$tmp/$archive" -C "$tmp"
	mkdir -p "$bin_dir"
	install -m 755 "$tmp/cloofy" "$bin_dir/cloofy"

	if [ "$(uname -s)" = Linux ] && has ldconfig; then
		if ! ldconfig -p | grep -q 'libpulse\.so\.0' || ! ldconfig -p | grep -q 'libasound\.so\.2'; then
			say "cloofy plays through PulseAudio or PipeWire. Install their libraries with:"
			say "  $(packages 'libpulse0 libasound2' 'pulseaudio-libs alsa-lib' 'libpulse alsa-lib')"
		fi
	fi
}

install_source() {
	has cargo || fail "there is no prebuilt cloofy for this machine, and building one needs Rust.
  Install it from https://rustup.rs and run this script again."
	if [ "$(uname -s)" = Linux ] && ! pkg-config --exists alsa libpulse 2>/dev/null; then
		fail "building cloofy needs the ALSA and PulseAudio headers. Install them with:
    $(packages 'libasound2-dev libpulse-dev pkg-config' 'alsa-lib-devel pulseaudio-libs-devel' 'alsa-lib libpulse')"
	fi
	say "Building from source, this takes a few minutes"
	ref=""
	if [ -n "${CLOOFY_VERSION:-}" ]; then
		ref="--tag $CLOOFY_VERSION"
	fi
	# shellcheck disable=SC2086 # $ref is empty or two words on purpose
	cargo install --quiet --locked --root "$tmp/build" --git "https://github.com/$repo" $ref cloofy
	mkdir -p "$bin_dir"
	install -m 755 "$tmp/build/bin/cloofy" "$bin_dir/cloofy"
}

printf '\n'
install_release || install_source
say "Installed $("$bin_dir/cloofy" --version) to $bin_dir"

case ":$PATH:" in
*":$bin_dir:"*) ;;
*)
	say ""
	say "$bin_dir is not on your PATH. Add it to your shell's startup file:"
	say "  export PATH=\"$bin_dir:\$PATH\""
	;;
esac

say ""
say "Next: set up your Spotify app (https://github.com/$repo#setup), then run"
say "  cloofy login"
printf '\n'
