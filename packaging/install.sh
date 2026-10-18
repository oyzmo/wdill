#!/usr/bin/env bash
# wdill — install / uninstall helper for the prebuilt distribution package.
#
# Run with no arguments for an interactive menu, or use a subcommand directly:
#
#   ./install.sh install
#       App + GNOME app grid entry. Per-user, no root needed.
#       Set PREFIX to change the location (default ~/.local).
#
#   sudo ./install.sh thumbnails
#       Nautilus preview-thumbnail support. Needs root: GNOME's
#       gnome-desktop-thumbnail-factory runs thumbnailers inside a bwrap
#       sandbox that only bind-mounts /usr (never $HOME), so the thumbnailer
#       executable has to live under a system prefix to be reachable at all.
#       Set PREFIX to change it (default /usr/local).
#
#   ./install.sh uninstall
#       Removes the app install; also removes the thumbnailer install if run
#       with sudo (skipped with a note otherwise).
#
# The menu re-runs itself under sudo (via exec) for whichever single action
# needs root, so you never have to know in advance which one that is.
set -euo pipefail

APP_ID="no.oyzmo.Wdill"
# Releases before the oyzmo.no rename used io.github.wdill. A leftover file
# under the old ID is not inert: a user-dir .thumbnailer shadows the working
# system one for the same MimeType, so clean the old name up everywhere.
LEGACY_APP_ID="io.github.wdill"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Under sudo, $HOME is root's — but the per-user app install belongs to whoever
# invoked sudo. Resolve their home, or `sudo ./install.sh uninstall` (and the
# menu, which re-runs uninstall under sudo) cleans /root/.local instead.
if [ "$(id -u)" -eq 0 ] && [ -n "${SUDO_USER:-}" ]; then
    USER_HOME="$(getent passwd "$SUDO_USER" | cut -d: -f6)"
else
    USER_HOME="$HOME"
fi
APP_PREFIX="${PREFIX:-$USER_HOME/.local}"
THUMB_PREFIX="${PREFIX:-/usr/local}"

# The app half lives in the invoking user's home unless PREFIX says otherwise;
# regenerate caches there as that user, so none end up owned by root.
as_app_owner() {
    if [ "$(id -u)" -eq 0 ] && [ -n "${SUDO_USER:-}" ] && [ -z "${PREFIX:-}" ]; then
        sudo -u "$SUDO_USER" "$@"
    else
        "$@"
    fi
}

# Installing the app as root into the user's home would leave root-owned
# files there; it is a per-user install and needs no privileges.
refuse_app_install_as_root() {
    if [ "$(id -u)" -eq 0 ] && [ -n "${SUDO_USER:-}" ] && [ -z "${PREFIX:-}" ]; then
        echo "install is per-user and needs no root: re-run it without sudo" >&2
        exit 1
    fi
}
BINARY="$HERE/wdill"

# The package ships a compiled binary; it needs GTK4 and libadwaita present
# on the system. Check before installing rather than failing at first launch.
check_deps() {
    [ -x "$BINARY" ] || { echo "Missing $BINARY — is this package complete?" >&2; exit 1; }
    command -v ldd >/dev/null 2>&1 || return 0
    local missing
    missing="$(ldd "$BINARY" 2>/dev/null | awk '/not found/ {print "  " $1}')"
    if [ -n "$missing" ]; then
        echo "wdill needs shared libraries that aren't installed:" >&2
        echo "$missing" >&2
        echo >&2
        echo "On Fedora:  sudo dnf install gtk4 libadwaita" >&2
        echo "On Debian/Ubuntu:  sudo apt install libgtk-4-1 libadwaita-1-0" >&2
        exit 1
    fi
}

# wdill is GPLv3-or-later: the licence text and the third-party attribution
# notices have to travel with the program, so every prefix that gets a binary
# gets them too. Mirrors where the Flatpak build puts them (share/licenses/).
install_licences() {
    local dir="$1/share/licenses/wdill"
    install -Dm644 "$HERE/LICENSE" "$dir/LICENSE"
    install -Dm644 "$HERE/THIRD-PARTY.md" "$dir/THIRD-PARTY.md"
}

remove_licences() {
    local dir="$1/share/licenses/wdill"
    rm -f "$dir/LICENSE" "$dir/THIRD-PARTY.md"
    rmdir "$dir" 2>/dev/null || true
}

# shared-mime-info knows no FBX type, so wdill ships one; without it neither
# the desktop entry nor the thumbnailer can claim .fbx files. Optional second
# argument runs the cache update through a wrapper (as_app_owner).
install_mime() {
    install -Dm644 "$HERE/data/$APP_ID.mime.xml" "$1/share/mime/packages/$APP_ID.xml"
    ${2:-} update-mime-database "$1/share/mime" 2>/dev/null || true
}

remove_mime() {
    rm -f "$1/share/mime/packages/$APP_ID.xml"
    ${2:-} update-mime-database "$1/share/mime" 2>/dev/null || true
}

# When a thumbnailer exits non-zero, GNOME records a *permanent* failure for
# that file: a tiny PNG under thumbnails/fail/gnome-thumbnail-factory/ holding
# the file's URI, which it never retries. So every file an older wdill choked
# on stays blank forever, even once an upgrade reads it perfectly well —
# installing the fix is not enough, the markers have to go too. Drop only the
# ones for the types wdill handles: that directory is shared with every other
# thumbnailer on the system. Runs against the invoking user's cache, not
# root's, since this subcommand is normally reached through sudo.
clear_failed_thumbnails() {
    local user home cache fail_dir marker n=0
    if [ "$(id -u)" -eq 0 ] && [ -n "${SUDO_USER:-}" ]; then
        user="$SUDO_USER"
        home="$(getent passwd "$user" 2>/dev/null | cut -d: -f6)"
        [ -n "$home" ] || return 0
        cache="$home/.cache"
    else
        user="$(id -un)"
        cache="${XDG_CACHE_HOME:-$HOME/.cache}"
    fi
    fail_dir="$cache/thumbnails/fail/gnome-thumbnail-factory"
    [ -d "$fail_dir" ] || return 0
    for marker in "$fail_dir"/*.png; do
        [ -f "$marker" ] || continue
        # The URI sits in a PNG tEXt chunk, NUL-separated from its key.
        if LC_ALL=C tr '\0' '\n' < "$marker" \
            | grep -aiqE '^file:.*\.(stl|3mf|obj|step|stp|glb|fbx)'; then
            rm -f "$marker"
            n=$((n + 1))
        fi
    done
    [ "$n" -eq 0 ] || echo "Cleared $n remembered thumbnail failure(s) for $user — GNOME never retries those by itself."
}

install_app() {
    refuse_app_install_as_root
    check_deps
    local bin_dir="$APP_PREFIX/bin"
    local desktop_dir="$APP_PREFIX/share/applications"
    local icon_dir="$APP_PREFIX/share/icons/hicolor/scalable/apps"
    install -Dm755 "$BINARY" "$bin_dir/wdill"
    install -Dm644 "$HERE/data/$APP_ID.svg" "$icon_dir/$APP_ID.svg"
    install -Dm644 "$HERE/data/$APP_ID.metainfo.xml" "$APP_PREFIX/share/metainfo/$APP_ID.metainfo.xml"
    local tmp
    tmp="$(mktemp)"
    sed "s|@BINDIR@|$bin_dir|" "$HERE/data/$APP_ID.desktop.in" > "$tmp"
    install -Dm644 "$tmp" "$desktop_dir/$APP_ID.desktop"
    rm -f "$tmp"
    # A .thumbnailer file outside /usr can never work (see install_thumbnails
    # comment) and, worse, shadows a working /usr one for the same MimeType
    # (XDG per-user dirs take priority) — so make sure none is left behind.
    case "$APP_PREFIX" in
        /usr|/usr/*) ;;
        *) rm -f "$APP_PREFIX/share/thumbnailers/$APP_ID.thumbnailer" \
                 "$APP_PREFIX/share/thumbnailers/$LEGACY_APP_ID.thumbnailer" ;;
    esac
    install_licences "$APP_PREFIX"
    install_mime "$APP_PREFIX"
    update-desktop-database "$desktop_dir" 2>/dev/null || true
    gtk-update-icon-cache -qtf "$APP_PREFIX/share/icons/hicolor" 2>/dev/null || true
    echo "Installed $bin_dir/wdill with desktop entry and icon under $APP_PREFIX/share."
    echo "Licence and third-party notices: $APP_PREFIX/share/licenses/wdill/"
    echo "It should now appear in the GNOME app grid as \"wdill\" (may need Nautilus restarted: nautilus -q)."
    if [ "$APP_PREFIX" = "$HOME/.local" ] && ! printf '%s' ":$PATH:" | grep -q ":$HOME/.local/bin:"; then
        echo "Note: $HOME/.local/bin is not on your PATH, so the 'wdill' command"
        echo "      won't resolve in a terminal. The app grid entry works regardless."
    fi
    echo "For Nautilus preview thumbnails too, also run: sudo ./install.sh thumbnails"
}

install_thumbnails() {
    case "$THUMB_PREFIX" in
        /usr|/usr/*) ;;
        *)
            echo "thumbnails needs a prefix under /usr (GNOME's sandbox can't reach anywhere else); got PREFIX=$THUMB_PREFIX" >&2
            exit 1
            ;;
    esac
    if [ "$(id -u)" -ne 0 ]; then
        echo "thumbnails needs root (GNOME's sandbox only reaches /usr): re-run as e.g. 'sudo ./install.sh thumbnails'" >&2
        exit 1
    fi
    check_deps
    local bin_dir="$THUMB_PREFIX/bin"
    local thumb_dir="$THUMB_PREFIX/share/thumbnailers"
    install -Dm755 "$BINARY" "$bin_dir/wdill"
    local tmp
    tmp="$(mktemp)"
    sed "s|@BINDIR@|$bin_dir|" "$HERE/data/$APP_ID.thumbnailer" > "$tmp"
    install -Dm644 "$tmp" "$thumb_dir/$APP_ID.thumbnailer"
    rm -f "$thumb_dir/$LEGACY_APP_ID.thumbnailer"
    rm -f "$tmp"
    install_licences "$THUMB_PREFIX"
    install_mime "$THUMB_PREFIX"
    clear_failed_thumbnails
    echo "Installed $bin_dir/wdill and Nautilus thumbnailer under $THUMB_PREFIX/share."
    echo "Restart Nautilus to pick it up: nautilus -q"
}

do_uninstall() {
    rm -f "$APP_PREFIX/bin/wdill" \
        "$APP_PREFIX/share/metainfo/$APP_ID.metainfo.xml" \
        "$APP_PREFIX/share/applications/$APP_ID.desktop" \
        "$APP_PREFIX/share/icons/hicolor/scalable/apps/$APP_ID.svg" \
        "$APP_PREFIX/share/applications/$LEGACY_APP_ID.desktop" \
        "$APP_PREFIX/share/icons/hicolor/scalable/apps/$LEGACY_APP_ID.svg" \
        "$APP_PREFIX/share/thumbnailers/$LEGACY_APP_ID.thumbnailer"
    remove_licences "$APP_PREFIX"
    remove_mime "$APP_PREFIX" as_app_owner
    as_app_owner update-desktop-database "$APP_PREFIX/share/applications" 2>/dev/null || true
    as_app_owner gtk-update-icon-cache -qtf "$APP_PREFIX/share/icons/hicolor" 2>/dev/null || true
    echo "Uninstalled app from $APP_PREFIX."

    if [ "$(id -u)" -eq 0 ]; then
        rm -f "$THUMB_PREFIX/bin/wdill" "$THUMB_PREFIX/share/thumbnailers/$APP_ID.thumbnailer" \
            "$THUMB_PREFIX/share/thumbnailers/$LEGACY_APP_ID.thumbnailer"
        remove_licences "$THUMB_PREFIX"
        remove_mime "$THUMB_PREFIX"
        echo "Uninstalled thumbnailer from $THUMB_PREFIX."
    else
        echo "Skipped $THUMB_PREFIX (needs root) — re-run as 'sudo ./install.sh uninstall' to remove thumbnail support too."
    fi
}

# Re-exec this same script under sudo for one subcommand, preserving PREFIX
# if the caller set it. Used by the menu so the user doesn't need to know in
# advance which choice needs root.
run_privileged() {
    exec sudo env ${PREFIX:+"PREFIX=$PREFIX"} "$0" "$1"
}

show_menu() {
    echo "wdill install helper"
    echo
    PS3=$'\nChoose an option: '
    local options=(
        "Install app (GNOME app grid, no root)"
        "Install thumbnails support (Nautilus previews, needs root)"
        "Uninstall everything (needs root to fully clean up)"
        "Quit"
    )
    select _ in "${options[@]}"; do
        case "$REPLY" in
            1) install_app; break ;;
            2)
                if [ "$(id -u)" -eq 0 ]; then install_thumbnails; else run_privileged thumbnails; fi
                break
                ;;
            3)
                if [ "$(id -u)" -eq 0 ]; then do_uninstall; else run_privileged uninstall; fi
                break
                ;;
            4) exit 0 ;;
            *) echo "Invalid choice." ;;
        esac
    done
}

case "${1:-}" in
    install)    install_app ;;
    thumbnails) install_thumbnails ;;
    uninstall)  do_uninstall ;;
    "")         show_menu ;;
    *)
        echo "Usage: $0 {install|thumbnails|uninstall}" >&2
        echo "  (no argument) — interactive menu" >&2
        echo "  install     — app + GNOME app grid entry (per-user, no root; PREFIX default ~/.local)" >&2
        echo "  thumbnails  — Nautilus preview thumbnails (needs root; PREFIX default /usr/local)" >&2
        echo "  uninstall   — removes both (run with sudo to also remove thumbnail support)" >&2
        exit 1
        ;;
esac
