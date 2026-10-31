#!/usr/bin/env bash
# Installs cloofy, the music client for the terminal, and sets it up step by step. It asks what
# you listen to, Spotify, YouTube Music or both, and installs the build with just that:
#
#   curl -fsSL https://raw.githubusercontent.com/parda21/cloofy/main/install.sh | bash
#
# Run it again to update, or to switch. Options, after `bash -s --` when piped:
#
#   --service S  spotify, ytmusic or both: install that without asking
#   --yes        no questions: keep the services you have (or Spotify) and leave the sign-ins
#                for later
#   --uninstall  remove cloofy, and its settings and sign-ins if you want
#
#   CLOOFY_INSTALL_DIR  where the binary goes (default: ~/.local/bin)
#   CLOOFY_VERSION      a release tag such as v0.1.0 (default: the latest release)
#   CLOOFY_RELEASE_URL  for testing: a folder of release archives and checksums to download from,
#                       in place of CLOOFY_VERSION's release on GitHub
#
# Works with the bash 3.2 that ships with macOS.

set -euo pipefail

repo="parda21/cloofy"
redirect_uri="http://127.0.0.1:8974/callback"
dashboard="https://developer.spotify.com/dashboard"
config_file="${XDG_CONFIG_HOME:-$HOME/.config}/cloofy/config.toml"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}/cloofy"
cache_dir="${XDG_CACHE_HOME:-$HOME/.cache}/cloofy"

usage() {
	cat <<'USAGE'
Installs cloofy, the music client for the terminal, and sets it up step by step.

  --service S  spotify, ytmusic or both: install that without asking
  --yes        no questions: keep the services you have (or Spotify), sign in later
  --uninstall  remove cloofy, and its settings and sign-ins if you want

  CLOOFY_INSTALL_DIR  where the binary goes (default: ~/.local/bin)
  CLOOFY_VERSION      a release tag such as v0.1.0 (default: the latest release)
USAGE
}

assume_yes=false
uninstall=false
# The build to install, as the release archives name them: spotify, ytmusic or full.
variant=""
while [ $# -gt 0 ]; do
	case "$1" in
	-y | --yes) assume_yes=true ;;
	--uninstall) uninstall=true ;;
	--service | --service=*)
		if [ "$1" = --service ]; then
			service=${2:-}
			[ $# -lt 2 ] || shift
		else
			service=${1#--service=}
		fi
		case "$service" in
		spotify | ytmusic) variant=$service ;;
		both | full) variant=full ;;
		"")
			printf -- '--service needs spotify, ytmusic or both\n' >&2
			exit 2
			;;
		*)
			printf 'Unknown service: %s (spotify, ytmusic or both)\n' "$service" >&2
			exit 2
			;;
		esac
		;;
	-h | --help)
		usage
		exit 0
		;;
	*)
		printf 'Unknown option: %s (try --help)\n' "$1" >&2
		exit 2
		;;
	esac
	shift
done

tmp=$(mktemp -d)
# The terminal's settings while a list reads single keys, to restore on any exit.
tty_state=""

cleanup() {
	if [ -n "$tty_state" ]; then
		stty "$tty_state" <&3 2>/dev/null || true
	fi
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

# Counted once the services are known, as each brings steps of its own.
step=0
steps=0
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
	printf '\n  %sSpotify and YouTube Music in the terminal%s\n' "$muted" "$reset"
}

# The option `choose` starts on, and takes without a terminal. Set it right before; it resets.
preselect=0

# Asks `question` with a list to pick from by arrow keys, j/k or number; each option is
# "label|hint". Sets $choice to the picked index; without a terminal, takes $preselect.
choose() {
	local question=$1
	shift
	local options=("$@") count=$# i label hint key rest width
	choice=$preselect
	preselect=0
	if ! $interactive; then
		return
	fi
	# Lines must not wrap: the list redraws itself by moving the cursor up one line per option.
	width=$(($(columns) - 8))
	question=$(fit "$question" "$width")
	printf '    %s%s%s\n' "$bold" "$question" "$reset"
	hide_cursor
	# Keys arrive one at a time and unechoed. bash's read -n would set this up only for
	# stdin, which is the script itself when piped from curl.
	tty_state=$(stty -g <&3)
	stty -icanon -echo min 1 time 0 <&3
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
		IFS= read -rn1 -u 3 key || key=""
		case "$key" in
		$'\033')
			IFS= read -rn2 -u 3 rest || rest=""
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
	stty "$tty_state" <&3
	tty_state=""
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

# Prints `path` with the home directory as ~. From a variable, as bash 5 expands a ~ written in
# the replacement back into the home directory.
pretty() {
	local tilde="~"
	printf '%s' "${1/#$HOME/$tilde}"
}

# The words as a list in a sentence: "a", "a and b", "a, b and c".
listing() {
	local out=$1
	shift
	while [ $# -gt 1 ]; do
		out="$out, $1"
		shift
	done
	if [ $# -eq 1 ]; then
		out="$out and $1"
	fi
	printf '%s' "$out"
}

# ── Packages ────────────────────────────────────────────────────────────────────────────────

# This system's package manager: brew, apt, dnf, zypper or pacman, or nothing.
package_manager() {
	local id
	if [ "$(uname -s)" = Darwin ]; then
		if has brew; then echo brew; fi
		return 0
	fi
	# shellcheck source=/dev/null # the file belongs to the system, not this script
	id=$(. /etc/os-release 2>/dev/null && echo "${ID:-} ${ID_LIKE:-}")
	case "$id" in
	*debian* | *ubuntu*) echo apt ;;
	*fedora* | *rhel*) echo dnf ;;
	*suse*) echo zypper ;;
	*arch*) echo pacman ;;
	esac
}

# The distro's install command for the given Debian, Fedora, Arch and openSUSE package names.
packages() {
	case "$(package_manager)" in
	apt) echo "sudo apt install $1" ;;
	dnf) echo "sudo dnf install $2" ;;
	pacman) echo "sudo pacman -S $3" ;;
	zypper) echo "sudo zypper install $4" ;;
	*) echo "your distribution's packages for ALSA and PulseAudio ($1)" ;;
	esac
}

# What each program YouTube Music plays with does, for the line that shows it.
tool_role() {
	case "$1" in
	mpv) echo "plays the music" ;;
	yt-dlp) echo "finds the streams" ;;
	deno) echo "solves YouTube's challenges for yt-dlp" ;;
	esac
}

# The command that installs packages with `pm`, as one types it.
pm_command() {
	case "$1" in
	brew) echo "brew install" ;;
	apt) echo "sudo apt install" ;;
	dnf) echo "sudo dnf install" ;;
	zypper) echo "sudo zypper install" ;;
	pacman) echo "sudo pacman -S" ;;
	esac
}

# Installs `packages` with package manager `pm`, unattended, as asking was the confirmation.
install_packages() {
	local pm=$1 pm_install
	shift
	case "$pm" in
	brew) pm_install=(brew install) ;;
	apt) pm_install=(env DEBIAN_FRONTEND=noninteractive apt-get install -y) ;;
	dnf) pm_install=(dnf install -y) ;;
	zypper) pm_install=(zypper --non-interactive install) ;;
	pacman) pm_install=(pacman -S --needed --noconfirm) ;;
	esac
	# All but Homebrew install as root. The password comes first, in the open, so the
	# install can run behind the spinner; there -n makes sudo fail rather than ask unseen.
	if [ "$pm" != brew ] && [ "$(id -u)" -ne 0 ]; then
		pm_install=(sudo -n "${pm_install[@]}")
	fi
	if [ "${pm_install[0]}" != sudo ] || sudo -v -p "    ${bold}Password for sudo$reset $green›$reset " <&3; then
		run "Installing $(listing "$@")" "${pm_install[@]}" "$@" || true
	fi
}

# Whether yt-dlp can read the keyring: a Python script's Python has secretstorage, or it's one
# of yt-dlp's own builds, which bring it along.
yt_dlp_reads_keyring() {
	local script interpreter
	script=$(command -v yt-dlp) || return 1
	[ "$(head -c 2 "$script" 2>/dev/null)" = "#!" ] || return 0
	# `#!/usr/bin/env python3` names the program after env.
	read -ra interpreter <<<"$(head -n 1 "$script" | sed 's/^#! *//')"
	"${interpreter[@]}" -c 'import secretstorage' 2>/dev/null
}

# The package of Python's secretstorage for package manager `pm`; nothing where it has none.
secretstorage_package() {
	case "$1" in
	apt | dnf) echo python3-secretstorage ;;
	zypper) echo python3-SecretStorage ;;
	pacman) echo python-secretstorage ;;
	esac
}

# Shows `tool` as installed, or as too old: since 2025.11.12, yt-dlp solves YouTube's
# challenges with deno, and older ones get no streams.
show_tool() {
	local version pattern='^([0-9]{4})\.([0-9]{2})\.([0-9]{2})'
	if [ "$1" = yt-dlp ]; then
		version=$(yt-dlp --version 2>/dev/null | head -n 1 || true)
		if [[ $version =~ $pattern ]] && [ "${BASH_REMATCH[1]}${BASH_REMATCH[2]}${BASH_REMATCH[3]}" -lt 20251112 ]; then
			outdated=true
			warn "yt-dlp $version is too old for YouTube, which needs 2025.11.12 or newer"
			note "Update it: https://github.com/yt-dlp/yt-dlp/wiki/Installation"
			return 0
		fi
	fi
	ok "$1 $(tool_role "$1")"
}

# Whether package manager `pm` has `tool` in a version that works with YouTube. Debian's and
# Ubuntu's yt-dlp lag months behind YouTube's changes, and Debian and Fedora have no deno.
packaged() {
	case "$1:$2" in
	:* | apt:yt-dlp | apt:deno | dnf:deno) return 1 ;;
	esac
}

# Where to get `tool` from without a package manager.
tool_source() {
	case "$1" in
	mpv) echo "mpv: https://mpv.io/installation" ;;
	yt-dlp) echo "yt-dlp: https://github.com/yt-dlp/yt-dlp/wiki/Installation" ;;
	deno) echo "deno: curl -fsSL https://deno.land/install.sh | sh" ;;
	esac
}

# `path` with its symlinks followed, as Homebrew links bin/cloofy to the file in its Cellar.
resolve() {
	local path=$1 link hops=0
	while [ -L "$path" ] && [ $hops -lt 20 ]; do
		link=$(readlink "$path")
		case "$link" in
		/*) path=$link ;;
		*) path=$(dirname "$path")/$link ;;
		esac
		hops=$((hops + 1))
	done
	printf '%s' "$path"
}

# The package manager that installed the cloofy at `path`, told apart by where each puts its
# files as cloofy's own updater does; nothing for a plain file such as this script installs.
manager_of() {
	case "$1" in */mise/shims/*)
		echo mise
		return 0
		;;
	esac
	case "$(resolve "$1")" in
	/nix/store/*) echo Nix ;;
	*/Cellar/*) echo Homebrew ;;
	*/mise/installs/*) echo mise ;;
	/usr/bin/*) echo "your package manager" ;;
	esac
}

# ── Services ────────────────────────────────────────────────────────────────────────────────

# What a build plays, for people.
variant_name() {
	case "$1" in
	spotify) echo Spotify ;;
	ytmusic) echo "YouTube Music" ;;
	full) echo "Spotify and YouTube Music" ;;
	esac
}

# The build for a list of services such as `"spotify", "ytmusic"`, or nothing.
variant_of_services() {
	case "$1" in
	*spotify*ytmusic* | *ytmusic*spotify*) echo full ;;
	*spotify*) echo spotify ;;
	*ytmusic*) echo ytmusic ;;
	esac
}

# The build of the cloofy at `path`, from its version, such as "cloofy 0.5.0 (spotify, ytmusic)".
# Releases before 0.5 play Spotify only and name no services.
variant_of_binary() {
	local version
	version=$("$1" --version </dev/null 2>/dev/null) || return 0
	case "$version" in
	*"("*")"*) variant_of_services "${version#*(}" ;;
	cloofy*) echo spotify ;;
	esac
}

# ── Settings ────────────────────────────────────────────────────────────────────────────────

# A value from config.toml, or nothing.
config_value() {
	[ -f "$config_file" ] || return 0
	sed -n "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*\"\{0,1\}\([^\"#]*\)\"\{0,1\}.*/\1/p" "$config_file" | head -n 1 | tr -d '[:space:]'
}

# The services config.toml turns on, as the list in it says them, or nothing.
config_services() {
	[ -f "$config_file" ] || return 0
	sed -n 's/^[[:space:]]*services[[:space:]]*=[[:space:]]*\[\([^]]*\)\].*/\1/p' "$config_file" | head -n 1
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
	manager=$(manager_of "$bin")
	[ -z "$manager" ] || fail "cloofy in $(pretty "$(dirname "$bin")") comes from $manager. Remove it there."
	printf '\n'
	choose "Remove cloofy from $(pretty "$bin")?" \
		"Remove the program|keeps your settings and sign-ins" \
		"Remove everything|also settings, sign-ins and caches" \
		"Cancel"
	[ "$choice" -eq 2 ] && exit 0
	# While the binary is still there: it knows what it added to Omarchy. `status` fails where
	# nothing is set up, as it does in a cloofy without Omarchy's setup.
	if [ "$(uname -s)" = Linux ] && "$bin" omarchy status </dev/null >/dev/null 2>&1; then
		if run "Removing cloofy from Omarchy" "$bin" omarchy remove; then
			ok "Removed cloofy from Omarchy"
		else
			warn "Some of cloofy may be left in Omarchy's settings"
		fi
	fi
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

# ── What to install ─────────────────────────────────────────────────────────────────────────

existing=$(command -v cloofy || true)
# The build of the cloofy found, else what config.toml says the last install turned on.
installed_variant=""
[ -z "$existing" ] || installed_variant=$(variant_of_binary "$existing")
kept_variant=$installed_variant
[ -n "$kept_variant" ] || kept_variant=$(variant_of_services "$(config_services)")
# Picked by --service or in the question, rather than kept; only then the settings change.
chosen=false
[ -z "$variant" ] || chosen=true

logo

if [ -z "$variant" ]; then
	case "$kept_variant" in
	ytmusic) preselect=1 ;;
	full) preselect=2 ;;
	esac
	if $interactive; then printf '\n'; fi
	choose "What do you listen to?" \
		"Spotify|needs Premium" \
		"YouTube Music|free, no account needed" \
		"Both|switch between them in cloofy"
	chosen=$interactive
	case "$choice" in
	0) variant=spotify ;;
	1) variant=ytmusic ;;
	*) variant=full ;;
	esac
fi

spotify=false
ytmusic=false
case "$variant" in
spotify) spotify=true ;;
ytmusic) ytmusic=true ;;
full) spotify=true ytmusic=true ;;
esac

# Omarchy gets a bar widget, its theme's colors, menu entries and a launcher.
omarchy=false
if [ "$(uname -s)" = Linux ] && { [ -d /usr/share/omarchy ] || [ -d "$HOME/.local/share/omarchy" ] || has omarchy; }; then
	omarchy=true
fi

# System, Install, Sign in, and each service's own.
steps=3
if $spotify; then steps=$((steps + 2)); fi
if $ytmusic; then steps=$((steps + 1)); fi
if $omarchy; then steps=$((steps + 1)); fi

# ── System ──────────────────────────────────────────────────────────────────────────────────

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
	elif $spotify; then
		# mpv, which plays YouTube Music, gets by with ALSA.
		warn "No PulseAudio or PipeWire running. cloofy needs one of them to play Spotify."
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

if [ -n "$existing" ]; then
	ok "Found $("$existing" --version 2>/dev/null || echo cloofy) in $(pretty "$(dirname "$existing")")"
fi

# ── Install ─────────────────────────────────────────────────────────────────────────────────

heading "Install"

# A package manager's cloofy stays with it: it updates it, and owns the file. Without
# CLOOFY_INSTALL_DIR, this script would put its build in that file's place.
manager=""
if [ -z "${CLOOFY_INSTALL_DIR:-}" ] && [ -n "$existing" ]; then
	manager=$(manager_of "$existing")
fi

if [ -n "$manager" ]; then
	if [ "$installed_variant" != "$variant" ]; then
		how="see https://github.com/$repo#install"
		if [ "$manager" = Homebrew ]; then
			formula=$(resolve "$existing")
			formula=${formula#*/Cellar/}
			new_formula=cloofy
			[ "$variant" = spotify ] || new_formula="cloofy-$variant"
			how="brew uninstall ${formula%%/*} && brew install $repo/$new_formula"
		fi
		fail "cloofy in $(pretty "$(dirname "$existing")") comes from $manager. Switch it to $(variant_name "$variant") there:
      $how"
	fi
	bin_dir=$(dirname "$existing")
	bin=$existing
	ok "$("$bin" --version) comes from $manager, which keeps it up to date"
else
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

	# A checkout of cloofy that has this script as a file installs itself, for trying out changes.
	# Piped from curl the script is no file, and the folder it runs in is anyone's.
	source_dir=""
	script=${BASH_SOURCE[0]:-}
	if [ -f "$script" ] && [ -f "$(dirname "$script")/crates/cloofy/Cargo.toml" ]; then
		source_dir=$(cd "$(dirname "$script")" && pwd)
	fi

	tag=""
	if [ -n "${CLOOFY_VERSION:-}" ]; then
		tag=$CLOOFY_VERSION
	elif [ -z "$source_dir" ]; then
		if download "https://api.github.com/repos/$repo/releases/latest" "$tmp/latest.json" 2>/dev/null; then
			tag=$(sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' "$tmp/latest.json" | head -n 1 || true)
		fi
		# Without a release, there would be nothing checked to download, and no code to build.
		[ -n "$tag" ] || fail "Couldn't look up the latest release on GitHub, which may limit requests for a while.
      Try again later, or pick one at https://github.com/$repo/releases and set CLOOFY_VERSION to its tag."
	fi

	# Copied next to the cloofy there, then moved over it in one step: one that is running keeps
	# its file, and nothing ever starts half of the new one.
	install_binary() {
		$sudo install -m 755 "$1" "$bin_dir/.cloofy.new"
		$sudo mv -f "$bin_dir/.cloofy.new" "$bin_dir/cloofy"
	}

	installed=false
	if [ -z "$source_dir" ]; then
		base=${CLOOFY_RELEASE_URL:-https://github.com/$repo/releases/download/$tag}
		archive="cloofy-$variant-$target.tar.gz"
		# Releases before 0.5 have Spotify's build only, named without the service.
		if [ "$variant" = spotify ] && ! download "$base/$archive.sha256" "$tmp/probe" 2>/dev/null; then
			archive="cloofy-$target.tar.gz"
		fi
		if run "Downloading cloofy $tag for $(variant_name "$variant")" download "$base/$archive" "$tmp/$archive"; then
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
		if $spotify && [ "$os" = linux ] && ! pkg-config --exists alsa libpulse 2>/dev/null; then
			fail "Building cloofy for Spotify needs the ALSA and PulseAudio headers. Install them with:
      $(packages 'libasound2-dev libpulse-dev pkg-config' 'alsa-lib-devel pulseaudio-libs-devel' 'alsa-lib libpulse' 'alsa-devel libpulse-devel pkg-config')"
		fi
		# Only the services picked go into the build.
		case "$variant" in
		spotify) features=(--no-default-features --features spotify) ;;
		ytmusic) features=(--no-default-features --features ytmusic) ;;
		full) features=(--no-default-features --features "spotify ytmusic") ;;
		esac
		if [ -n "$source_dir" ]; then
			run "Building cloofy from $(pretty "$source_dir")" \
				cargo install --locked --root "$tmp/build" --path "$source_dir/crates/cloofy" "${features[@]}" || exit 1
		else
			run "Building cloofy $tag from source, this takes a few minutes" \
				cargo install --locked --root "$tmp/build" --git "https://github.com/$repo" --tag "$tag" "${features[@]}" cloofy || exit 1
		fi
		install_binary "$tmp/build/bin/cloofy"
	fi
	bin="$bin_dir/cloofy"
	ok "Installed $("$bin" --version) to $(pretty "$bin_dir")"
	if [ -n "$installed_variant" ] && [ "$installed_variant" != "$variant" ]; then
		note "In place of the build for $(variant_name "$installed_variant"). Settings and sign-ins stay."
	fi
fi

# Kept as they are with --yes: there may be fewer turned on than the build has.
if $chosen; then
	case "$variant" in
	spotify) config_set services '["spotify"]' ;;
	ytmusic) config_set services '["ytmusic"]' ;;
	full) config_set services '["spotify", "ytmusic"]' ;;
	esac
fi

# Without -q grep reads all of it: quitting early would fail the pipe under pipefail.
if $spotify && [ "$os" = linux ] && has ldconfig && ! ldconfig -p 2>/dev/null | grep 'libpulse\.so\.0' >/dev/null; then
	warn "cloofy plays Spotify through PulseAudio. Install its library with:"
	note "$(packages 'libpulse0' 'pulseaudio-libs' 'libpulse' 'libpulse0')"
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

# ── Spotify app ─────────────────────────────────────────────────────────────────────────────

client_id=""
if $spotify; then
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
fi

# ── YouTube Music ───────────────────────────────────────────────────────────────────────────

# What YouTube Music still needs at the end, for the summary.
missing=()
outdated=false
if $ytmusic; then
	heading "YouTube Music"

	for tool in mpv yt-dlp deno; do
		if has "$tool"; then
			show_tool "$tool"
		else
			missing+=("$tool")
		fi
	done

	pm=$(package_manager)
	# The missing ones the package manager has, to install in one go.
	wanted=()
	if [ ${#missing[@]} -gt 0 ]; then
		for tool in "${missing[@]}"; do
			if packaged "$pm" "$tool"; then wanted+=("$tool"); fi
		done
	fi
	if [ ${#wanted[@]} -gt 0 ] && $interactive; then
		note "cloofy plays YouTube Music with mpv, and yt-dlp and deno find the streams."
		printf '\n'
		choose "Install $(listing "${wanted[@]}") now?" \
			"Yes|$(pm_command "$pm") ${wanted[*]}" \
			"Later|YouTube Music plays once they're installed"
		if [ "$choice" -eq 0 ]; then
			install_packages "$pm" "${wanted[@]}"
			left=()
			for tool in "${missing[@]}"; do
				if has "$tool"; then
					show_tool "$tool"
				else
					left+=("$tool")
				fi
			done
			missing=(${left[@]+"${left[@]}"})
		fi
	fi

	# Signing in reads the cookies of Chromium-based browsers with the key from the keyring,
	# which yt-dlp gets through Python's secretstorage. Its own builds bring it along; the
	# distributions' yt-dlp leaves it optional.
	if [ "$os" = linux ] && has yt-dlp && ! yt_dlp_reads_keyring; then
		package=$(secretstorage_package "$pm")
		if [ -n "$package" ] && $interactive; then
			printf '\n'
			note "To sign in with Chrome, Brave or Chromium, yt-dlp needs Python's secretstorage."
			choose "Install $package now?" \
				"Yes|$(pm_command "$pm") $package" \
				"Later|signing in tells you again"
			if [ "$choice" -eq 0 ]; then
				install_packages "$pm" "$package"
				if yt_dlp_reads_keyring; then ok "$package lets yt-dlp read your browser's keyring"; fi
			fi
		elif [ -n "$package" ]; then
			note "To sign in with Chrome, Brave or Chromium: $(pm_command "$pm") $package"
		fi
	fi

	if [ ${#missing[@]} -gt 0 ]; then
		warn "Install $(listing "${missing[@]}") to play YouTube Music:"
		if [ "$os" = macos ] && [ -z "$pm" ]; then
			note "With Homebrew from https://brew.sh: brew install ${missing[*]}"
		else
			still=()
			for tool in "${missing[@]}"; do
				if packaged "$pm" "$tool"; then still+=("$tool"); fi
			done
			if [ ${#still[@]} -gt 0 ]; then
				note "$(pm_command "$pm") ${still[*]}"
			fi
			for tool in "${missing[@]}"; do
				if ! packaged "$pm" "$tool"; then note "$(tool_source "$tool")"; fi
			done
		fi
	fi
fi

# ── Preferences ─────────────────────────────────────────────────────────────────────────────

if $spotify; then
	heading "Preferences"

	question="Fade between songs?"
	if $ytmusic; then question="Fade between Spotify songs?"; fi
	if [ -n "$(config_value crossfade)" ]; then
		ok "Crossfade: $(config_value crossfade) seconds"
	elif $interactive; then
		choose "$question" \
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
		ok "Defaults kept. Change them in cloofy's Settings: press ,"
	fi
fi

# ── Sign in ─────────────────────────────────────────────────────────────────────────────────

heading "Sign in"

# Without YouTube Music, `cloofy login` is Spotify's, as it has been before 0.5.
spotify_login=(login)
if $ytmusic; then spotify_login=(login spotify); fi

# Whether cloofy has the sign-in that `cloofy login` with these arguments makes. Asked rather than
# looked for, as with credentials = "keychain" it isn't in a file.
signed_in_with() {
	"$bin" "$@" --check </dev/null >/dev/null 2>&1
}

signed_in=false
if $spotify; then
	# A package manager's cloofy from before 0.5 knows no --check; its files tell then.
	if signed_in_with "${spotify_login[@]}" ||
		{ [ -f "$data_dir/webapi-token.json" ] && [ -d "$data_dir/session" ]; }; then
		signed_in=true
		ok "Signed in to Spotify"
	elif [ -z "$client_id" ]; then
		note "Spotify comes after its app."
	elif $interactive; then
		choose "Sign in to Spotify now?" \
			"Yes|your browser opens twice: for your app, then for playback" \
			"Later|run cloofy ${spotify_login[*]}"
		if [ "$choice" -eq 0 ]; then
			printf '\n'
			if "$bin" "${spotify_login[@]}" <&3; then
				signed_in=true
			else
				warn "Sign-in did not finish. Try again with: cloofy ${spotify_login[*]}"
			fi
		fi
	else
		note "Run cloofy ${spotify_login[*]} to sign in."
	fi
fi

if $ytmusic; then
	if $spotify; then printf '\n'; fi
	if signed_in_with login ytmusic; then
		ok "Signed in to YouTube Music"
	elif $interactive; then
		note "YouTube Music plays without an account. Signing in adds your library and likes:"
		note "cloofy reads your browser's YouTube cookies, and they stay on this computer."
		printf '\n'
		choose "Sign in to YouTube Music?" \
			"Yes|takes the sign-in of your browser" \
			"No|listen without an account"
		if [ "$choice" -eq 0 ]; then
			printf '\n'
			"$bin" login ytmusic <&3 || warn "Sign-in did not finish. Try again with: cloofy login ytmusic"
		fi
	else
		note "YouTube Music plays without an account. For your library: cloofy login ytmusic"
	fi
fi

# ── Omarchy ─────────────────────────────────────────────────────────────────────────────────

if $omarchy; then
	heading "Omarchy"

	if ! "$bin" omarchy --help </dev/null >/dev/null 2>&1; then
		note "This version of cloofy has no Omarchy setup yet."
	elif "$bin" omarchy status </dev/null >/dev/null 2>&1; then
		# Set up before, as `status` succeeds only then, so brought up to date: a release may
		# change the bar widget.
		if run "Updating cloofy in Omarchy" "$bin" omarchy install; then
			ok "Bar widget, theme, menu and launcher are up to date"
		fi
	elif $interactive; then
		choose "Set up cloofy for Omarchy?" \
			"Yes|bar widget, theme colors, menu and launcher" \
			"Not now|cloofy omarchy install, any time"
		if [ "$choice" -eq 0 ]; then
			printf '\n'
			"$bin" omarchy install <&3 || warn "That did not work. Try again with: cloofy omarchy install"
		fi
	else
		note "Set it up any time with: cloofy omarchy install"
	fi
fi

# ── Done ────────────────────────────────────────────────────────────────────────────────────

# Each step is "what|command".
title="cloofy is ready"
next=()
if $spotify && [ -z "$client_id" ]; then
	title="cloofy is installed"
	next=("Spotify app|run this installer again" "Then sign in|cloofy ${spotify_login[*]}")
elif $spotify && ! $signed_in; then
	title="Almost there"
	next=("Sign in|cloofy ${spotify_login[*]}")
fi
if [ ${#missing[@]} -gt 0 ]; then
	[ "$title" = "cloofy is installed" ] || title="Almost there"
	next+=("Install|$(listing "${missing[@]}")")
fi
if $outdated; then
	[ "$title" = "cloofy is installed" ] || title="Almost there"
	next+=("Update|yt-dlp")
fi
if [ ${#next[@]} -eq 0 ]; then
	next=("Start it|cloofy" "All shortcuts|press ? inside")
elif [ "$title" = "Almost there" ]; then
	next+=("Then start it|cloofy")
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
