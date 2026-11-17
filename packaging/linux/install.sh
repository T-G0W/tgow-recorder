#!/bin/sh
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
app="$data/tgow-recorder"
bin="$HOME/.local/bin"
desktop="$data/applications/tgow-recorder.desktop"
icon="$data/icons/hicolor/scalable/apps/tgow-recorder.svg"

refresh() {
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$data/applications" || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$data/icons/hicolor" || true
}

remove() {
    if [ -x "$app/tgow-recorder" ]; then
        "$app/tgow-recorder" ctl quit >/dev/null 2>&1 || true
    fi
    rm -f "$bin/tgow-recorder" "$desktop" "$icon"
    rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/tgow-recorder.desktop"
    rm -rf "$app"
    refresh
    echo "TGOW Recorder was removed. Your settings in ~/.config/tgow-recorder and your recordings were kept."
}

setup() {
    for file in tgow-recorder tgow-rec tgow-overlay tgow-recorder.desktop tgow-recorder.svg; do
        if [ ! -f "$here/$file" ]; then
            echo "$file is missing next to install.sh" >&2
            exit 1
        fi
    done
    if [ -x "$app/tgow-recorder" ]; then
        "$app/tgow-recorder" ctl quit >/dev/null 2>&1 || true
    fi
    mkdir -p "$app" "$bin" "$(dirname "$desktop")" "$(dirname "$icon")"
    for file in tgow-recorder tgow-rec tgow-overlay; do
        install -m 755 "$here/$file" "$app/$file"
    done
    for file in tgow-recorder.svg logo-512.png README.md LICENSE-MIT LICENSE-APACHE THIRD-PARTY-LICENSES.txt install.sh; do
        if [ -f "$here/$file" ]; then
            install -m 644 "$here/$file" "$app/$file"
        fi
    done
    chmod 755 "$app/install.sh"
    install -m 644 "$here/tgow-recorder.svg" "$icon"
    sed "s|^Exec=tgow-recorder|Exec=\"$app/tgow-recorder\"|" "$here/tgow-recorder.desktop" > "$desktop"
    ln -sf "$app/tgow-recorder" "$bin/tgow-recorder"
    refresh
    echo "TGOW Recorder is installed. Start it from your app menu or run: tgow-recorder --show"
    echo "To remove it again run: $app/install.sh --uninstall"
}

case "${1:-}" in
    --uninstall | uninstall) remove ;;
    "" | --install | install) setup ;;
    *)
        echo "usage: install.sh [--uninstall]" >&2
        exit 2
        ;;
esac
