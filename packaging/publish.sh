#!/usr/bin/env bash
# Pushes what render.sh wrote to the Homebrew tap and the AUR, each only when its secret is set,
# so releases work the same before the setup in packaging/README.md:
#
#   HOMEBREW_TAP_TOKEN   a token that can push to parda21/homebrew-cloofy
#   AUR_SSH_PRIVATE_KEY  the private SSH key of the AUR account that maintains cloofy
#
#   packaging/publish.sh <folder render.sh wrote>

set -euo pipefail

if [ $# -ne 1 ]; then
	echo "Usage: $0 <folder render.sh wrote>" >&2
	exit 2
fi
packages=$1
tap="parda21/homebrew-cloofy"
# One per build: Spotify, YouTube Music, both. Each installs a cloofy binary and conflicts with
# the others.
formulas=(cloofy cloofy-ytmusic cloofy-full)
aur_packages=(cloofy-bin cloofy-ytmusic-bin cloofy-full-bin cloofy)
# aur.archlinux.org's key, SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4 as listed on the AUR's home page.
aur_host_key="aur.archlinux.org ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEuBKrPzbawxA/k2g6NcyV5jmqwJ2s+zpgZGZ7tpLIcN"

files=()
for formula in "${formulas[@]}"; do
	files+=("homebrew/$formula.rb")
done
for package in "${aur_packages[@]}"; do
	files+=("aur/$package/PKGBUILD" "aur/$package/.SRCINFO")
done
for file in "${files[@]}"; do
	if [ ! -f "$packages/$file" ]; then
		echo "Missing $packages/$file; run render.sh first" >&2
		exit 1
	fi
done
version=$(sed -n 's/^pkgver=//p' "$packages/aur/cloofy-bin/PKGBUILD")

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# A note in the workflow run's summary, or on the terminal.
notice() {
	if [ -n "${GITHUB_ACTIONS:-}" ]; then
		echo "::notice title=$1::$2"
	else
		echo "$1: $2"
	fi
}

# Commits all changes in the clone at `dir` and pushes them to `branch`, if there are any.
commit_and_push() {
	local dir=$1 message=$2 branch=$3
	git -C "$dir" add -A
	if git -C "$dir" diff --cached --quiet; then
		echo "${dir##*/}: up to date"
		return
	fi
	git -C "$dir" -c user.name="github-actions[bot]" \
		-c user.email="41898282+github-actions[bot]@users.noreply.github.com" commit -q -m "$message"
	git -C "$dir" push -q origin "HEAD:$branch"
	echo "${dir##*/}: pushed $message"
}

publish_homebrew() {
	if [ -z "${HOMEBREW_TAP_TOKEN:-}" ]; then
		notice "Homebrew" "HOMEBREW_TAP_TOKEN is not set, so $tap stays as it is"
	else
		git clone -q "https://x-access-token:$HOMEBREW_TAP_TOKEN@github.com/$tap.git" "$tmp/homebrew-cloofy"
		mkdir -p "$tmp/homebrew-cloofy/Formula"
		for formula in "${formulas[@]}"; do
			cp "$packages/homebrew/$formula.rb" "$tmp/homebrew-cloofy/Formula/$formula.rb"
		done
		# The tap's default branch, also while the tap is still empty.
		commit_and_push "$tmp/homebrew-cloofy" "cloofy $version" "$(git -C "$tmp/homebrew-cloofy" symbolic-ref --short HEAD)"
	fi
}

publish_aur() {
	if [ -z "${AUR_SSH_PRIVATE_KEY:-}" ]; then
		notice "AUR" "AUR_SSH_PRIVATE_KEY is not set, so ${aur_packages[*]} stay as they are on the AUR"
	else
		mkdir -m 700 "$tmp/ssh"
		printf '%s\n' "$AUR_SSH_PRIVATE_KEY" >"$tmp/ssh/key"
		chmod 600 "$tmp/ssh/key"
		echo "$aur_host_key" >"$tmp/ssh/known_hosts"
		export GIT_SSH_COMMAND="ssh -i $tmp/ssh/key -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=$tmp/ssh/known_hosts"
		for package in "${aur_packages[@]}"; do
			# A package new to the AUR clones empty, and the first push creates it.
			git clone -q "ssh://aur@aur.archlinux.org/$package.git" "$tmp/$package"
			cp "$packages/aur/$package/PKGBUILD" "$packages/aur/$package/.SRCINFO" "$tmp/$package/"
			# The AUR takes pushes to master only.
			commit_and_push "$tmp/$package" "Update to $version" master
		done
	fi
}

# Each on its own, so one that fails doesn't keep the other from its update. `set -e` holds in
# the subshells only outside a condition, hence the statuses taken afterwards.
set +e
(set -e; publish_homebrew)
homebrew=$?
(set -e; publish_aur)
aur=$?
set -e
[ $homebrew -eq 0 ] || echo "Publishing to the Homebrew tap failed" >&2
[ $aur -eq 0 ] || echo "Publishing to the AUR failed" >&2
[ $homebrew -eq 0 ] && [ $aur -eq 0 ]
