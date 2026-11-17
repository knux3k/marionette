#!/bin/sh
# Release archive of the marionette binary (decision 0141), used by .github/workflows/release.yml.
#   scripts/release-archive.sh <name> <target-triple> <binary> <out-dir>
# Writes <out-dir>/marionette-<version>-<name>.(tar.gz|zip) holding the binary, the README,
# changelog and human guides, and licenses/ (engine licenses plus THIRD-PARTY.txt for every
# linked crate). Prints the archive path.
set -eu
name=$1 triple=$2 bin=$3 out=$4
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
stage="$out/marionette-$version-$name"
rm -rf "$stage"
mkdir -p "$stage/docs"
cp "$bin" "$stage/"
cp README.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE "$stage/"
cp docs/GETTING-STARTED.md docs/CONNECT-YOUR-AI.md "$stage/docs/"
cargo run --locked -q -p marionette-package --example engine_licenses -- "$triple" "$stage/licenses" >&2
cd "$out"
case "$name" in
    windows-*) 7z a -tzip -bso0 "marionette-$version-$name.zip" "marionette-$version-$name" >&2
               echo "$out/marionette-$version-$name.zip" ;;
    *) tar czf "marionette-$version-$name.tar.gz" "marionette-$version-$name"
       echo "$out/marionette-$version-$name.tar.gz" ;;
esac
