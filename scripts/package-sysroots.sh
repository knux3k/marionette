#!/bin/sh
# Link-time sysroots for Linux packages (decision 0140): the ALSA and udev (gamepads) libraries of Ubuntu 20.04
# (glibc 2.31, the Steam Linux Runtime 3.0 floor) for x86-64 and ARM64. `marionette package`
# links against these instead of the build machine's newer libasound.
#   scripts/package-sysroots.sh [dir]   (default target/sysroots)
# then: export MARIONETTE_SYSROOT_X86_64=<dir>/x86_64 MARIONETTE_SYSROOT_AARCH64=<dir>/aarch64
set -eu
dir=${1:-target/sysroots}
fetch() { # arch, mirror, folder
    mkdir -p "$dir/$3"
    for spec in a/alsa-lib/libasound2_1.2.2-2.1ubuntu2.5 a/alsa-lib/libasound2-dev_1.2.2-2.1ubuntu2.5 \
        s/systemd/libudev1_245.4-4ubuntu3 s/systemd/libudev-dev_245.4-4ubuntu3; do
        deb="${spec##*/}_$1.deb"
        if [ ! -f "$dir/$deb" ]; then
            curl -fsSL -o "$dir/$deb.part" "$2/pool/main/${spec}_$1.deb"
            mv "$dir/$deb.part" "$dir/$deb"
        fi
        dpkg-deb -x "$dir/$deb" "$dir/$3"
    done
}
fetch amd64 http://archive.ubuntu.com/ubuntu x86_64
fetch arm64 http://ports.ubuntu.com/ubuntu-ports aarch64
echo "MARIONETTE_SYSROOT_X86_64=$(cd "$dir/x86_64" && pwd)"
echo "MARIONETTE_SYSROOT_AARCH64=$(cd "$dir/aarch64" && pwd)"
