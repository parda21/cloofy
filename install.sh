#!/usr/bin/env bash
# Installs cloofy, the Spotify client for the terminal, and sets it up step by step:
#
#   curl -fsSL https://raw.githubusercontent.com/parda21/cloofy/main/install.sh | bash
#
# Run it again to update. Options, after `bash -s --` when piped:
#
#   --yes        no questions: install with the defaults and leave the Spotify setup for later
#   --uninstall  remove cloofy, and its settings and sign-ins if you want
#
#   CLOOFY_INSTALL_DIR  where the binary goes (default: ~/.local/bin)
#   CLOOFY_VERSION      a release tag such as v0.1.0 (default: the latest release)
#
# Works with the bash 3.2 that ships with macOS.

set -euo pipefail

repo="parda21/cloofy"
redirect_uri="http://127.0.0.1:8974/callback"
dashboard="https://developer.spotify.com/dashboard"
config_file="${XDG_CONFIG_HOME:-$HOME/.config}/cloofy/config.toml"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}/cloofy"
cache_dir="${XDG_CACHE_HOME:-$HOME/.cache}/cloofy"

assume_yes=false
uninstall=false
for arg in "$@"; do
	case "$arg" in
	-y | --yes) assume_yes=true ;;
	--uninstall) uninstall=true ;;
	-h | --help)
		cat <<'USAGE'
Installs cloofy, the Spotify client for the terminal, and sets it up step by step.

  --yes        no questions: install with the defaults and leave the Spotify setup for later
  --uninstall  remove cloofy, and its settings and sign-ins if you want

  CLOOFY_INSTALL_DIR  where the binary goes (default: ~/.local/bin)
  CLOOFY_VERSION      a release tag such as v0.1.0 (default: the latest release)
USAGE
		exit 0
		;;
	*)
		printf 'Unknown option: %s (try --help)\n' "$arg" >&2
		exit 2
		;;
	esac
done

tmp=$(mktemp -d)
cleanup() {
	if [ -t 2 ]; then
		printf '\033[?25h' >&2
	fi
	rm -rf "$tmp"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# Questions read from the terminal, as stdin is the script itself when piped from curl.
interactive=false
if ! $assume_yes && [ -t 1 ] && (: </dev/tty) 2>/dev/null; then
	exec 3</dev/tty
	interactive=true
fi

# ── Output ──────────────────────────────────────────────────────────────────────────────────

has() { command -v "$1" >/dev/null 2>&1; }

# Decided here, once: inside $(...) stdout is a pipe, never the terminal.
color=false
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
	color=true
fi
truecolor=false
case "${COLORTERM:-}:${TERM_PROGRAM:-}:${TERM:-}" in
*truecolor* | *24bit* | *ghostty* | *WezTerm* | *iTerm* | *vscode* | *kitty*) truecolor=true ;;
esac

# The escape sequence for a foreground color, as true color or the closest of 256 colors.
rgb() {
	if ! $color; then
		return
	fi
	if $truecolor; then
		printf '\033[38;2;%d;%d;%dm' "$1" "$2" "$3"
	else
		printf '\033[38;5;%dm' $((16 + 36 * ($1 * 5 / 255) + 6 * ($2 * 5 / 255) + $3 * 5 / 255))
	fi
}

if $color; then
	reset=$'\033[0m'
	bold=$'\033[1m'
else
	reset=""
	bold=""
fi
green=$(rgb 30 215 96)
red=$(rgb 255 99 99)
yellow=$(rgb 255 196 87)
text=$(rgb 236 236 241)
muted=$(rgb 140 140 152)
faint=$(rgb 88 88 100)

# The terminal's width in columns, for keeping list lines from wrapping.
columns() {
	local size
	if $interactive && size=$(stty size <&3 2>/dev/null); then
		echo "${size#* }"
	else
		echo 80
	fi
}

# `text` cut to `width` characters, ending in … when cut.
fit() {
	if [ "${#1}" -le "$2" ]; then
		printf '%s' "$1"
	elif [ "$2" -gt 1 ]; then
		printf '%s…' "${1:0:$(($2 - 1))}"
	fi
}

hide_cursor() { if [ -t 1 ]; then printf '\033[?25l'; fi; }
show_cursor() { if [ -t 1 ]; then printf '\033[?25h'; fi; }

ok() { printf '    %s✓%s %s\n' "$green" "$reset" "$*"; }
warn() { printf '    %s!%s %s\n' "$yellow" "$reset" "$*"; }
note() { printf '      %s%s%s\n' "$muted" "$*" "$reset"; }
fail() {
	printf '\n    %s✗%s %s\n\n' "$red" "$reset" "$*" >&2
	exit 1
}

step=0
steps=5
heading() {
	step=$((step + 1))
	printf '\n  %s●%s %s%s%s  %s%d/%d%s\n\n' "$green" "$reset" "$bold" "$1" "$reset" "$faint" "$step" "$steps" "$reset"
}

# The cloofy wordmark from the app's pixel font, two cells per pixel, lighter on the left.
logo() {
	local rows=(
		".##.#..#...#...##.#.#"
		"#...#.#.#.#.#.#...#.#"
		"#...#.#.#.#.#.###..##"
		"#...#.#.#.#.#.#.....#"
		".##.#..#...#..#...##."
	)
	local row x line
	printf '\n'
	for row in "${rows[@]}"; do
		line="  "
		for ((x = 0; x < ${#row}; x++)); do
			if [ "${row:x:1}" = "#" ]; then
				line+="$(rgb $((131 - 101 * x / 20)) $((233 - 18 * x / 20)) $((168 - 72 * x / 20)))██"
			else
				line+="  "
			fi
		done
		printf '%s%s\n' "$line" "$reset"
	done
	printf '\n  %sA Spotify client for the terminal%s\n' "$muted" "$reset"
}

# Asks `question` with a list to pick from by arrow keys, j/k or number; each option is
# "label|hint". Sets $choice to the picked index; without a terminal, takes the first.
choose() {
	local question=$1
	shift
	local options=("$@") count=$# i label hint key rest width
	choice=0
	if ! $interactive; then
		return
	fi
	# Lines must not wrap: the list redraws itself by moving the cursor up one line per option.
	width=$(($(columns) - 8))
	question=$(fit "$question" "$width")
	printf '    %s%s%s\n' "$bold" "$question" "$reset"
	hide_cursor
	while true; do
		for ((i = 0; i < count; i++)); do
			label=$(fit "${options[i]%%|*}" "$width")
			hint=""
			case "${options[i]}" in *"|"*) hint=$(fit "${options[i]#*|}" $((width - ${#label} - 2))) ;; esac
			if [ "$i" -eq "$choice" ]; then
				printf '\033[2K    %s❯ %s%s%s  %s%s%s\n' "$green" "$bold" "$label" "$reset" "$muted" "$hint" "$reset"
			else
				printf '\033[2K      %s%s%s  %s%s%s\n' "$text" "$label" "$reset" "$faint" "$hint" "$reset"
			fi
		done
		IFS= read -rsn1 -u 3 key || key=""
		case "$key" in
		$'\033')
			IFS= read -rsn2 -u 3 rest || rest=""
			case "$rest" in
			"[A" | "OA") choice=$(((choice + count - 1) % count)) ;;
			"[B" | "OB") choice=$(((choice + 1) % count)) ;;
			esac
			;;
		k) choice=$(((choice + count - 1) % count)) ;;
		j) choice=$(((choice + 1) % count)) ;;
		[1-9]) [ "$key" -le "$count" ] && choice=$((key - 1)) ;;
		"") break ;;
		esac
		printf '\033[%dA' "$count"
	done
	# Collapse the list into one line with the answer.
	printf '\033[%dA' $((count + 1))
	for ((i = 0; i <= count; i++)); do
		printf '\033[2K\n'
	done
	printf '\033[%dA' $((count + 1))
	label=${options[choice]%%|*}
	printf '    %s%s%s  %s%s%s\n' "$text" "$(fit "$question" $((width - ${#label} - 2)))" "$reset" "$green" "$label" "$reset"
	show_cursor
}

# Asks for a line of text after `label`; sets $answer.
ask() {
	printf '    %s%s%s %s›%s ' "$bold" "$1" "$reset" "$green" "$reset"
	show_cursor
	IFS= read -r -u 3 answer || answer=""
}

# Runs a command behind a spinner, its output kept for when it fails.
run() {
	local message=$1
	shift
	local frames=(⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏) i=0 start=$SECONDS pid
	"$@" >"$tmp/log" 2>&1 &
	pid=$!
	if [ -t 1 ]; then
		hide_cursor
		while kill -0 "$pid" 2>/dev/null; do
			printf '\r\033[2K    %s%s%s %s  %s%ds%s' "$green" "${frames[i % 10]}" "$reset" "$message" "$faint" $((SECONDS - start)) "$reset"
			i=$((i + 1))
			sleep 0.1
		done
		printf '\r\033[2K'
		show_cursor
	fi
	if wait "$pid"; then
		return 0
	fi
	printf '    %s✗%s %s failed:\n\n' "$red" "$reset" "$message"
	tail -n 20 "$tmp/log" | sed 's/^/      /'
	printf '\n'
	return 1
}

open_url() {
	if has xdg-open; then
		xdg-open "$1" >/dev/null 2>&1 &
	elif has open; then
		open "$1"
	else
		return 1
	fi
}

copy() {
	if has pbcopy; then
		printf '%s' "$1" | pbcopy
	elif [ -n "${WAYLAND_DISPLAY:-}" ] && has wl-copy; then
		printf '%s' "$1" | wl-copy
	elif has xclip; then
		printf '%s' "$1" | xclip -selection clipboard
	elif has xsel; then
		printf '%s' "$1" | xsel --clipboard --input
	else
		return 1
	fi
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

# The distro's install command for the given Debian, Fedora and Arch package names.
packages() {
	local id
	# shellcheck source=/dev/null # the file belongs to the system, not this script
	id=$(. /etc/os-release 2>/dev/null && echo "${ID:-} ${ID_LIKE:-}")
	case "$id" in
	*debian* | *ubuntu*) echo "sudo apt install $1" ;;
	*fedora* | *rhel*) echo "sudo dnf install $2" ;;
	*arch*) echo "sudo pacman -S $3" ;;
	*) echo "your distribution's packages for ALSA and PulseAudio ($1)" ;;
	esac
}

# Prints `path` with the home directory as ~.
pretty() { printf '%s' "${1/#$HOME/~}"; }

# A value from config.toml, or nothing.
config_value() {
	[ -f "$config_file" ] || return 0
	sed -n "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*\"\{0,1\}\([^\"#]*\)\"\{0,1\}.*/\1/p" "$config_file" | head -n 1 | tr -d '[:space:]'
}

# Sets `key = value` in config.toml, keeping everything else in it.
config_set() {
	mkdir -p "$(dirname "$config_file")"
	if [ ! -f "$config_file" ]; then
		printf '# cloofy settings. All options: https://github.com/%s#configuration\n\n' "$repo" >"$config_file"
	fi
	if grep -q "^[[:space:]]*$1[[:space:]]*=" "$config_file"; then
		sed "s|^[[:space:]]*$1[[:space:]]*=.*|$1 = $2|" "$config_file" >"$tmp/config" && cat "$tmp/config" >"$config_file"
	else
		printf '%s = %s\n' "$1" "$2" >>"$config_file"
	fi
}

# ── Uninstall ───────────────────────────────────────────────────────────────────────────────

if $uninstall; then
	logo
	bin=$(command -v cloofy || true)
	[ -n "$bin" ] || bin="${CLOOFY_INSTALL_DIR:-$HOME/.local/bin}/cloofy"
	[ -e "$bin" ] || fail "cloofy is not installed"
	printf '\n'
	choose "Remove cloofy from $(pretty "$bin")?" \
		"Remove the program|keeps your settings and sign-ins" \
		"Remove everything|also settings, sign-ins and caches" \
		"Cancel"
	[ "$choice" -eq 2 ] && exit 0
	sudo=""
	[ -w "$(dirname "$bin")" ] || sudo="sudo"
	$sudo rm -f "$bin"
	ok "Removed $(pretty "$bin")"
	if [ "$choice" -eq 1 ]; then
		rm -rf "$(dirname "$config_file")" "$data_dir" "$cache_dir"
		ok "Removed settings, sign-ins and caches"
	fi
	printf '\n'
	exit 0
fi

# ── 1. System ───────────────────────────────────────────────────────────────────────────────

logo
heading "System"

case "$(uname -m)" in
x86_64 | amd64) arch=x86_64 ;;
aarch64 | arm64) arch=aarch64 ;;
*) fail "cloofy runs on x86_64 and arm64, not $(uname -m)" ;;
esac
case "$(uname -s)" in
Linux)
	os=linux
	target="$arch-unknown-linux-gnu"
	# shellcheck source=/dev/null
	ok "$(. /etc/os-release 2>/dev/null && echo "${PRETTY_NAME:-Linux}" || echo Linux) · $arch"
	if has pactl && server=$(pactl info 2>/dev/null | sed -n 's/^Server Name: //p') && [ -n "$server" ]; then
		ok "Audio through $server"
	elif [ -S "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/pulse/native" ]; then
		if pgrep -x pipewire >/dev/null 2>&1; then ok "Audio through PipeWire"; else ok "Audio through PulseAudio"; fi
	else
		warn "No PulseAudio or PipeWire running. cloofy needs one of them to play music."
	fi
	;;
Darwin)
	os=macos
	target="$arch-apple-darwin"
	ok "macOS $(sw_vers -productVersion 2>/dev/null || true) · $arch"
	ok "Audio through Core Audio"
	;;
*) fail "cloofy runs on Linux and macOS, not $(uname -s)" ;;
esac

terminal="${TERM_PROGRAM:-}"
case "$terminal:${TERM:-}:${KITTY_WINDOW_ID:-}" in
ghostty* | WezTerm* | *kitty* | *:*:?*)
	case "$terminal" in ghostty) terminal=Ghostty ;; "") terminal=kitty ;; esac
	ok "$terminal shows cover art and icons"
	;;
*)
	ok "Terminal: ${terminal:-${TERM:-unknown}}"
	note "Cover art shows as blocks here. kitty, Ghostty and WezTerm show real images."
	;;
esac

existing=$(command -v cloofy || true)
if [ -n "$existing" ]; then
	ok "Found $("$existing" --version 2>/dev/null || echo cloofy) in $(pretty "$(dirname "$existing")")"
fi

# ── 2. Install ──────────────────────────────────────────────────────────────────────────────

heading "Install"

if [ -n "${CLOOFY_INSTALL_DIR:-}" ]; then
	bin_dir=$CLOOFY_INSTALL_DIR
elif [ -n "$existing" ]; then
	bin_dir=$(dirname "$existing")
else
	# shellcheck disable=SC2088 # a label, shown as typed
	choose "Where should cloofy go?" \
		"~/.local/bin|recommended, no sudo needed" \
		"/usr/local/bin|for every user on this machine, asks for sudo"
	bin_dir="$HOME/.local/bin"
	[ "$choice" -eq 1 ] && bin_dir=/usr/local/bin
fi
sudo=""
if ! mkdir -p "$bin_dir" 2>/dev/null || [ ! -w "$bin_dir" ]; then
	sudo="sudo"
	$sudo mkdir -p "$bin_dir"
fi

if [ -n "${CLOOFY_VERSION:-}" ]; then
	tag=$CLOOFY_VERSION
else
	tag=$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" 2>/dev/null |
		sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1 || true)
fi

# A checkout of cloofy next to the script installs itself, for trying out changes.
source_dir=$(dirname "$0")
if [ -f "$source_dir/crates/cloofy/Cargo.toml" ]; then
	source_dir=$(cd "$source_dir" && pwd)
else
	source_dir=""
fi

install_binary() {
	$sudo install -m 755 "$1" "$bin_dir/cloofy"
}

installed=false
if [ -z "$source_dir" ] && [ -n "$tag" ]; then
	archive="cloofy-$target.tar.gz"
	base="https://github.com/$repo/releases/download/$tag"
	if run "Downloading cloofy $tag" download "$base/$archive" "$tmp/$archive"; then
		download "$base/$archive.sha256" "$tmp/$archive.sha256" || fail "No checksum published for $archive"
		[ "$(sha256 "$tmp/$archive")" = "$(cut -d ' ' -f 1 <"$tmp/$archive.sha256")" ] ||
			fail "$archive does not match its checksum"
		tar -xzf "$tmp/$archive" -C "$tmp"
		install_binary "$tmp/cloofy"
		installed=true
	else
		warn "No prebuilt cloofy $tag for $target, building it instead"
	fi
fi

if ! $installed; then
	has cargo || fail "Building cloofy needs Rust. Install it from https://rustup.rs and run this again."
	if [ "$os" = linux ] && ! pkg-config --exists alsa libpulse 2>/dev/null; then
		fail "Building cloofy needs the ALSA and PulseAudio headers. Install them with:
      $(packages 'libasound2-dev libpulse-dev pkg-config' 'alsa-lib-devel pulseaudio-libs-devel' 'alsa-lib libpulse')"
	fi
	if [ -n "$source_dir" ]; then
		run "Building cloofy from $(pretty "$source_dir")" \
			cargo install --locked --root "$tmp/build" --path "$source_dir/crates/cloofy" || exit 1
	else
		ref=()
		[ -n "$tag" ] && ref=(--tag "$tag")
		run "Building cloofy from source, this takes a few minutes" \
			cargo install --locked --root "$tmp/build" --git "https://github.com/$repo" ${ref[@]+"${ref[@]}"} cloofy || exit 1
	fi
	install_binary "$tmp/build/bin/cloofy"
fi
bin="$bin_dir/cloofy"
ok "Installed $("$bin" --version) to $(pretty "$bin_dir")"

# Without -q grep reads all of it: quitting early would fail the pipe under pipefail.
if [ "$os" = linux ] && has ldconfig && ! ldconfig -p 2>/dev/null | grep 'libpulse\.so\.0' >/dev/null; then
	warn "cloofy plays through PulseAudio. Install its library with:"
	note "$(packages 'libpulse0' 'pulseaudio-libs' 'libpulse')"
fi

case ":$PATH:" in
*":$bin_dir:"*) ;;
*)
	case "${SHELL##*/}" in
	zsh) rc="$HOME/.zshrc" ;;
	bash) rc="$HOME/.bashrc" && [ "$os" = macos ] && rc="$HOME/.bash_profile" ;;
	*) rc="" ;;
	esac
	if [ -n "$rc" ] && $interactive; then
		choose "$(pretty "$bin_dir") is not on your PATH yet. Add it?" \
			"Yes|in $(pretty "$rc")" \
			"No|I'll do it myself"
		if [ "$choice" -eq 0 ]; then
			# shellcheck disable=SC2016 # $PATH expands when the shell starts
			printf '\nexport PATH="%s:$PATH"\n' "$bin_dir" >>"$rc"
			ok "Added to $(pretty "$rc"), open a new terminal to use it"
		fi
	else
		warn "$(pretty "$bin_dir") is not on your PATH. Add it in your shell's startup file."
	fi
	;;
esac

# ── 3. Spotify app ──────────────────────────────────────────────────────────────────────────

heading "Spotify app"

client_id=$(config_value client_id)
if [ -n "$client_id" ]; then
	ok "Using your app $client_id"
elif ! $interactive; then
	warn "Set up your Spotify app later: run this script again, or see https://github.com/$repo#setup"
else
	note "cloofy reads your library through a Spotify app of your own."
	note "It's free and takes a minute. Playing music needs Premium."
	printf '\n'
	choose "How do you want to set it up?" \
		"Walk me through it|opens the Spotify dashboard" \
		"I already have a client ID" \
		"Later|run this script again when you're ready"
	if [ "$choice" -eq 0 ]; then
		printf '\n'
		open_url "$dashboard" && opened="opened in your browser" || opened=""
		copy "$redirect_uri" 2>/dev/null && copied="copied" || copied=""
		instruction() { printf '    %s%s%s  %s  %s%s%s\n' "$green" "$1" "$reset" "$2" "$muted" "${3:-}" "$reset"; }
		instruction 1 "Open ${bold}developer.spotify.com/dashboard$reset" "$opened"
		instruction 2 "Choose ${bold}Create app$reset, any name and description"
		instruction 3 "Redirect URI: $bold$redirect_uri$reset" "$copied"
		instruction 4 "Tick ${bold}Web API$reset, accept the terms, save"
		instruction 5 "Copy the ${bold}Client ID$reset from the app's settings"
		printf '\n'
	fi
	if [ "$choice" -le 1 ]; then
		while true; do
			ask "Client ID"
			answer=$(printf '%s' "$answer" | tr -d '[:space:]' | tr 'A-F' 'a-f')
			if [ -z "$answer" ]; then
				warn "Skipped. Run this script again when you have it."
				break
			elif [[ "$answer" =~ ^[0-9a-f]{32}$ ]]; then
				client_id=$answer
				config_set client_id "\"$client_id\""
				ok "Saved to $(pretty "$config_file")"
				break
			fi
			warn "That's not a client ID: 32 characters of 0-9 and a-f."
			note "Try again, or press enter to skip."
		done
		if [ -n "$client_id" ]; then
			note "Listening with another account than the dashboard's?"
			note "Add it to the app under User Management."
		fi
	fi
fi

# ── 4. Preferences ──────────────────────────────────────────────────────────────────────────

heading "Preferences"

if [ -n "$(config_value crossfade)" ]; then
	ok "Crossfade: $(config_value crossfade) seconds"
elif $interactive; then
	choose "Fade between songs?" \
		"Off|songs play back to back, gapless" \
		"3 seconds|a short blend" \
		"6 seconds|smooth, like a mix" \
		"12 seconds|long blends"
	case "$choice" in
	1) config_set crossfade 3 ;;
	2) config_set crossfade 6 ;;
	3) config_set crossfade 12 ;;
	esac
else
	ok "Defaults kept. Change them in $(pretty "$config_file")"
fi

# ── 5. Sign in ──────────────────────────────────────────────────────────────────────────────

heading "Sign in"

signed_in=false
if [ -f "$data_dir/webapi-token.json" ] && [ -d "$data_dir/session" ]; then
	signed_in=true
	ok "Already signed in"
elif [ -z "$client_id" ]; then
	note "Comes after the Spotify app."
elif $interactive; then
	choose "Sign in to Spotify now?" \
		"Yes|your browser opens twice: for your app, then for playback" \
		"Later|run cloofy login"
	if [ "$choice" -eq 0 ]; then
		printf '\n'
		if "$bin" login <&3; then
			signed_in=true
		else
			warn "Sign-in did not finish. Try again with: cloofy login"
		fi
	fi
else
	note "Run cloofy login to sign in."
fi

# ── Done ────────────────────────────────────────────────────────────────────────────────────

# Each step is "what|command".
if [ -z "$client_id" ]; then
	title="cloofy is installed"
	next=("Spotify app|run this installer again" "Then sign in|cloofy login")
elif ! $signed_in; then
	title="Almost there"
	next=("Sign in|cloofy login" "Then start it|cloofy")
else
	title="cloofy is ready"
	next=("Start it|cloofy" "All shortcuts|press ? inside")
fi
width=48
border=$(printf '%*s' "$width" '' | sed 's/ /─/g')
row() { printf '  %s│%s%s%s│%s\n' "$faint" "$reset" "$1" "$faint" "$reset"; }
printf '\n  %s╭%s╮%s\n' "$faint" "$border" "$reset"
row "$(printf '%*s' "$width" '')"
row "$(printf '   %s%-*s%s' "$green$bold" $((width - 3)) "$title" "$reset")"
row "$(printf '%*s' "$width" '')"
for item in "${next[@]}"; do
	row "$(printf '   %s%-16s%s%s%-*s%s' "$muted" "${item%%|*}" "$reset" "$bold" $((width - 19)) "${item#*|}" "$reset")"
done
row "$(printf '%*s' "$width" '')"
printf '  %s╰%s╯%s\n\n' "$faint" "$border" "$reset"
