#!/usr/bin/env bash
# Checks the templates after a change: fills them in for the version in Cargo.toml with made-up
# checksums, checks the syntax of the formula and the PKGBUILDs, and that each .SRCINFO says
# what makepkg reads from its PKGBUILD. Without makepkg, that runs in an Arch Linux container.
#
#   packaging/check.sh

set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$here/../Cargo.toml" | head -n 1)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

mkdir "$tmp/checksums"
for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-apple-darwin aarch64-apple-darwin; do
	printf '%064d  cloofy-%s.tar.gz\n' 0 "$target" >"$tmp/checksums/cloofy-$target.tar.gz.sha256"
done
"$here/render.sh" "$version" "$tmp/checksums" "$tmp/out" >/dev/null

if command -v ruby >/dev/null; then
	ruby -c "$tmp/out/homebrew/cloofy.rb" >/dev/null
else
	echo "No Ruby here, so the formula's syntax goes unchecked"
fi

if command -v makepkg >/dev/null; then
	srcinfo() { (cd "$tmp/out/aur/$1" && makepkg --printsrcinfo); }
else
	# makepkg doesn't run as root.
	srcinfo() {
		docker run --rm -v "$tmp/out/aur/$1:/pkg:ro" archlinux:base-devel bash -c \
			'useradd -m builder && cp -r /pkg /home/builder/pkg && chown -R builder /home/builder/pkg &&
			cd /home/builder/pkg && su builder -c "makepkg --printsrcinfo"'
	}
fi
for package in cloofy-bin cloofy; do
	bash -n "$tmp/out/aur/$package/PKGBUILD"
	if ! diff -u "$tmp/out/aur/$package/.SRCINFO" <(srcinfo "$package"); then
		echo "packaging/aur/$package/.SRCINFO doesn't match its PKGBUILD" >&2
		exit 1
	fi
done
echo "The formula and PKGBUILDs for $version check out"
