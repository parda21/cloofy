#!/usr/bin/env bash
# Checks the templates after a change: fills them in for the version in Cargo.toml with made-up
# checksums, checks the syntax of the formulas and the PKGBUILDs, and that each .SRCINFO says
# what makepkg reads from its PKGBUILD. Without makepkg, that runs in an Arch Linux container.
#
#   packaging/check.sh

set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$here/../Cargo.toml" | head -n 1)
formulas=(cloofy cloofy-ytmusic cloofy-full)
aur_packages=(cloofy-bin cloofy-ytmusic-bin cloofy-full-bin cloofy)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

mkdir "$tmp/checksums"
for variant in spotify ytmusic full; do
	for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-apple-darwin aarch64-apple-darwin; do
		archive="cloofy-$variant-$target.tar.gz"
		printf '%064d  %s\n' 0 "$archive" >"$tmp/checksums/$archive.sha256"
	done
done
"$here/render.sh" "$version" "$tmp/checksums" "$tmp/out" >/dev/null

# Each package downloads its own build: the one its name says, Spotify's when it names none.
variant_of() {
	case "$1" in
	*ytmusic*) echo ytmusic ;;
	*full*) echo full ;;
	*) echo spotify ;;
	esac
}
for formula in "${formulas[@]}"; do
	file="$tmp/out/homebrew/$formula.rb"
	if grep -n 'releases/download/' "$file" | grep -v "/cloofy-$(variant_of "$formula")-" >&2; then
		echo "packaging/homebrew/$formula.rb downloads another build than $(variant_of "$formula")'s" >&2
		exit 1
	fi
done
for package in cloofy-bin cloofy-ytmusic-bin cloofy-full-bin; do
	file="$tmp/out/aur/$package/PKGBUILD"
	if grep -n 'releases/download/' "$file" | grep -v "/cloofy-$(variant_of "$package")-" >&2; then
		echo "packaging/aur/$package/PKGBUILD downloads another build than $(variant_of "$package")'s" >&2
		exit 1
	fi
done

if command -v ruby >/dev/null; then
	for formula in "${formulas[@]}"; do
		ruby -c "$tmp/out/homebrew/$formula.rb" >/dev/null
	done
else
	echo "No Ruby here, so the formulas' syntax goes unchecked"
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
for package in "${aur_packages[@]}"; do
	bash -n "$tmp/out/aur/$package/PKGBUILD"
	if ! diff -u "$tmp/out/aur/$package/.SRCINFO" <(srcinfo "$package"); then
		echo "packaging/aur/$package/.SRCINFO doesn't match its PKGBUILD" >&2
		exit 1
	fi
done
echo "The formulas and PKGBUILDs for $version check out"
