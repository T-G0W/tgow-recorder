#!/bin/sh
set -eu

version="$1"
stage="$2"
out="$3"
root="$(mktemp -d)"
pkg="$root/tgow-recorder"

install -Dm755 "$stage/tgow-recorder" "$pkg/usr/bin/tgow-recorder"
install -Dm755 "$stage/tgow-rec" "$pkg/usr/bin/tgow-rec"
install -Dm755 "$stage/tgow-overlay" "$pkg/usr/bin/tgow-overlay"
install -Dm644 "$stage/tgow-recorder.desktop" "$pkg/usr/share/applications/tgow-recorder.desktop"
install -Dm644 "$stage/tgow-recorder.svg" "$pkg/usr/share/icons/hicolor/scalable/apps/tgow-recorder.svg"
install -Dm644 "$stage/logo-512.png" "$pkg/usr/share/icons/hicolor/512x512/apps/tgow-recorder.png"
install -Dm644 "$stage/README.md" "$pkg/usr/share/doc/tgow-recorder/README.md"
install -Dm644 "$stage/THIRD-PARTY-LICENSES.txt" "$pkg/usr/share/doc/tgow-recorder/THIRD-PARTY-LICENSES.txt"
{
    echo "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/"
    echo "Upstream-Name: tgow-recorder"
    echo "Source: https://github.com/T-G0W/tgow-recorder"
    echo
    echo "Files: *"
    echo "Copyright: 2026 T-GOW"
    echo "License: MIT or Apache-2.0"
    echo
    sed 's/^$/./; s/^/ /' "$stage/LICENSE-MIT"
} > "$pkg/usr/share/doc/tgow-recorder/copyright"

mkdir -p "$root/debian"
printf 'Source: tgow-recorder\n\nPackage: tgow-recorder\nArchitecture: amd64\n' > "$root/debian/control"
shlibs="$(cd "$root" && dpkg-shlibdeps -O "$pkg/usr/bin/tgow-recorder" "$pkg/usr/bin/tgow-rec" "$pkg/usr/bin/tgow-overlay" | sed -n 's/^shlibs:Depends=//p')"
size="$(du -sk "$pkg/usr" | cut -f1)"

mkdir -p "$pkg/DEBIAN"
cat > "$pkg/DEBIAN/control" <<CONTROL
Package: tgow-recorder
Version: $version
Section: video
Priority: optional
Architecture: amd64
Maintainer: T-GOW <210194347+T-G0W@users.noreply.github.com>
Installed-Size: $size
Depends: $shlibs, libegl1, libxkbcommon0, libwayland-client0, libwayland-egl1
Recommends: xdg-desktop-portal, pipewire-pulse | pulseaudio
Homepage: https://github.com/T-G0W/tgow-recorder
Description: Instant replay, recording, streaming and screenshots
 TGOW Recorder keeps the last minutes of your screen in memory and saves
 them with one hotkey. It also records, streams and takes screenshots,
 encodes on the GPU with NVENC, VA-API or Vulkan and comes with an
 overlay in the style of ShadowPlay.
CONTROL

mkdir -p "$out"
dpkg-deb --root-owner-group --build "$pkg" "$out/tgow-recorder-$version-linux-x86_64.deb"
rm -rf "$root"
