#!/usr/bin/env bash
# Fills in the Homebrew formula and the AUR packages for a release, from the templates next to
# this script and the release's checksums:
#
#   packaging/render.sh 0.3.0 <folder with the release's .sha256 files> <output folder>
#
# The release workflow runs it for each new tag before publish.sh pushes the output. To check
# the output for a published release:
#
#   gh release download v0.3.0 --repo parda21/cloofy --pattern '*.sha256' --dir /tmp/sums
#   packaging/render.sh 0.3.0 /tmp/sums /tmp/packages

set -euo pipefail

if [ $# -ne 3 ]; then
	echo "Usage: $0 <version> <checksums folder> <output folder>" >&2
	exit 2
fi
version=$1
checksums=$2
out=$3
here=$(cd "$(dirname "$0")" && pwd)

targets=(x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-apple-darwin aarch64-apple-darwin)
templates=(homebrew/cloofy.rb aur/cloofy-bin/PKGBUILD aur/cloofy-bin/.SRCINFO aur/cloofy/PKGBUILD aur/cloofy/.SRCINFO)

# pacman's versions can't hold a dash, and package managers get releases, not previews.
if ! [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
	echo "Not a release version: $version" >&2
	exit 1
fi

# Each placeholder such as @SHA256_X86_64_APPLE_DARWIN@ becomes the checksum of that build.
substitutions=(-e "s/@VERSION@/$version/g")
for target in "${targets[@]}"; do
	file="$checksums/cloofy-$target.tar.gz.sha256"
	if [ ! -f "$file" ]; then
		echo "Missing $file" >&2
		exit 1
	fi
	sum=$(cut -d ' ' -f 1 <"$file")
	if ! [[ $sum =~ ^[0-9a-f]{64}$ ]]; then
		echo "No SHA-256 checksum in $file" >&2
		exit 1
	fi
	placeholder=$(printf 'SHA256_%s' "$target" | tr 'a-z-' 'A-Z_')
	substitutions+=(-e "s/@$placeholder@/$sum/g")
done

for template in "${templates[@]}"; do
	mkdir -p "$out/$(dirname "$template")"
	sed "${substitutions[@]}" "$here/$template" >"$out/$template"
	if grep -n '@[A-Z0-9_]*@' "$out/$template" >&2; then
		echo "$template has placeholders left" >&2
		exit 1
	fi
	echo "$out/$template"
done
