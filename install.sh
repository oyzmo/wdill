#!/usr/bin/env bash
# wdill — compile / install / uninstall helper.
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
#       executable has to live under a system prefix to be reachable at all
#       (see CLAUDE.md). Set PREFIX to change it (default /usr/local).
#
#   ./install.sh uninstall
#       Removes the app install; also removes the thumbnailer install if run
#       with sudo (skipped with a note otherwise).
#
# The menu re-runs itself under sudo for whichever single action needs root,
# so you never have to know in advance which one that is. Its Release section
# (bump, distribution, flatpak.oyzmo.no, GitHub) has subcommands too; run
# without arguments to see them.
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

# Runs as the original user when invoked via sudo, so cargo (~/.cargo/bin,
# not on root's PATH) resolves and build artifacts stay owned by that user.
as_builder() {
    if [ "$(id -u)" -eq 0 ] && [ -n "${SUDO_USER:-}" ]; then
        sudo -u "$SUDO_USER" bash -lc "$*"
    else
        bash -c "$*"
    fi
}

target_dir() {
    as_builder "cargo metadata --format-version 1 --no-deps --manifest-path '$HERE/Cargo.toml'" \
        | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p'
}

compile() {
    as_builder "cargo build --release --manifest-path '$HERE/Cargo.toml'"
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
    compile
    local bin_dir="$APP_PREFIX/bin"
    local desktop_dir="$APP_PREFIX/share/applications"
    local icon_dir="$APP_PREFIX/share/icons/hicolor/scalable/apps"
    install -Dm755 "$(target_dir)/release/wdill" "$bin_dir/wdill"
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
    compile
    local bin_dir="$THUMB_PREFIX/bin"
    local thumb_dir="$THUMB_PREFIX/share/thumbnailers"
    install -Dm755 "$(target_dir)/release/wdill" "$bin_dir/wdill"
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

# ---------------------------------------------------------------- release
# The same release steps, numbers and order as Bindle's bindle.sh.

# The oyzmo Flatpak site lives in the root of the shared code folder: that is
# /mnt/mycode on the VM and ~/Code on the desktop (the same share), told apart
# by where this script is. FLATPAK_SITE overrides.
case "$HERE/" in
    /mnt/mycode/*)   FLATPAK_SITE="${FLATPAK_SITE:-/mnt/mycode/flatpak}" ;;
    "$HOME"/Code/*)  FLATPAK_SITE="${FLATPAK_SITE:-$HOME/Code/flatpak}" ;;
    *)               FLATPAK_SITE="${FLATPAK_SITE:-}" ;;
esac

current_version() {
    sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$HERE/Cargo.toml" | head -1
}

# bump.sh plus a commit of just the files it touches, so the release steps
# after it (which clone main) see the new version.
bump_version() {
    local level notes=() note
    echo "Now at $(current_version)."
    read -rp "Bump which part? [p]atch (fixes) / [m]inor (features) / ma[j]or: " level
    case "$level" in
        p|P|patch) level=patch ;;
        m|M|minor) level=minor ;;
        j|J|major) level=major ;;
        *) echo "Cancelled."; return 0 ;;
    esac
    echo "What changed? One short line per note; an empty line ends."
    while read -rp "  note: " note && [ -n "$note" ]; do
        notes+=("$note")
    done
    [ ${#notes[@]} -gt 0 ] || { echo "At least one note is needed." >&2; return 1; }
    "$HERE/bump.sh" "$level" "${notes[@]}"
    git -C "$HERE" commit -q -m "wdill $(current_version)" -- \
        Cargo.toml Cargo.lock "data/$APP_ID.metainfo.xml"
    echo "Committed wdill $(current_version)."
}

# Everything a release consists of, in dist/: the manual-install zip and the
# Flatpak bundle (each also builds the source zip; buildflatpak.sh publishes
# into the Flatpak site).
make_distribution() {
    "$HERE/package.sh"
    "$HERE/buildflatpak.sh"
    echo
    echo "dist/ now holds, for $(current_version):"
    ls -lh "$HERE/dist" | grep -F -- "-$(current_version)-" | sed 's/^/  /'
}

publish_flatpak() {
    [ -x "$FLATPAK_SITE/tools/push.sh" ] || { echo "No oyzmo Flatpak site here (looked for /mnt/mycode/flatpak or ~/Code/flatpak)." >&2; return 1; }
    "$FLATPAK_SITE/tools/push.sh"
}

release_github() {
    echo "Pushing main to GitHub…"
    "$HERE/dev/push-github.sh"
    "$HERE/dev/release-github.sh"
}

# Everything after the bump: dist/, NAS backup, flatpak.oyzmo.no, GitHub.
full_release() {
    if ! git -C "$HERE" diff --quiet HEAD; then
        echo "There are uncommitted changes — commit them first, a release is built from main." >&2
        return 1
    fi
    echo "Full release of wdill $(current_version)"
    make_distribution
    echo "Backing up to the NAS…"
    git -C "$HERE" push -q nas main || echo "NAS push failed — continuing." >&2
    publish_flatpak || echo "flatpak.oyzmo.no was not updated." >&2
    release_github || echo "The GitHub release failed (see above)." >&2
}

backup_key() {
    [ -x "$FLATPAK_SITE/tools/backup-key.sh" ] || { echo "No oyzmo Flatpak site here." >&2; return 1; }
    "$FLATPAK_SITE/tools/backup-key.sh"
}

# Swap an installed wdill flatpak for the newest bundle in dist/. GNOME
# Software only offers "Open" for a bundle of an app that is already
# installed. Uninstalling keeps the app's data (~/.var/app/$APP_ID).
reinstall_flatpak() {
    local bundle scope found=() version
    bundle="$(ls "$HERE"/dist/wdill-*-"$(uname -m)".flatpak 2>/dev/null | sort -V | tail -1)"
    [ -n "$bundle" ] || { echo "No wdill-*.flatpak in dist/ — make one with option 7 first." >&2; return 1; }
    for scope in user system; do
        if flatpak info --"$scope" "$APP_ID" >/dev/null 2>&1; then
            version="$(flatpak info --"$scope" "$APP_ID" | sed -n 's/^ *Version: //p')"
            found+=("$scope")
            echo "  Installed ($scope): ${version:-unknown version}"
        fi
    done
    [ ${#found[@]} -eq 0 ] && echo "  Not installed yet."
    echo "  New:  $(basename "$bundle")"
    read -rp "Uninstall the old and install this one? [y/N] " answer
    [[ "$answer" == [yY]* ]] || { echo "Cancelled."; return 0; }
    for scope in ${found[@]+"${found[@]}"}; do
        flatpak uninstall --"$scope" -y --noninteractive "$APP_ID"
    done
    scope=user
    [ ${#found[@]} -eq 1 ] && scope="${found[0]}"
    flatpak install --"$scope" -y --noninteractive "$bundle"
    echo "Installed $(flatpak info --"$scope" "$APP_ID" | sed -n 's/^ *Version: //p')."
}

cleanup_build() {
    rm -rf "$HERE/build-dir" "$HERE/repo" "$HERE/.flatpak-builder"
    as_builder "cargo clean --manifest-path '$HERE/Cargo.toml'"
    echo "Removed build-dir/, repo/, .flatpak-builder/ and the cargo build. dist/ is kept."
}

# ------------------------------------------------------------------ menu
bold() { printf '\033[1m%s\033[0m\n' "$*"; }

# Each choice runs this script again with its subcommand, so a failing step
# (set -e) ends that step only, not the menu. Root-only steps go through sudo,
# keeping PREFIX if the caller set it.
run_step() {
    "$0" "$1" || printf '\033[31m✘ That step failed (see above).\033[0m\n'
}
run_privileged() {
    sudo env ${PREFIX:+"PREFIX=$PREFIX"} "$0" "$1" || printf '\033[31m✘ That step failed (see above).\033[0m\n'
}

show_menu() {
    while true; do
        echo
        bold "── wdill $(current_version) ─────────────────────────"
        echo "  1) Compile"
        echo "  2) Install on this GNOME desktop (no root)"
        echo "  3) Install thumbnail support (Nautilus previews, needs root)"
        echo "  4) Uninstall everything (needs root to fully clean up)"
        echo "  8) Cleanup (delete temporary build files)"
        echo " 10) Replace installed flatpak with the newest one (from dist/)"
        bold "── Release (in this order) ──────────────"
        echo " 11) Bump version (asks for notes, commits)"
        echo " 13) Full release: 7 → NAS → 12 → 9 in one go"
        echo "     or step by step:"
        echo "  7)   Make distribution (zip + flatpak + source → dist/ and the Flatpak site)"
        echo " 12)   Publish flatpak updates (flatpak.oyzmo.no → GNOME Software)"
        echo "  9)   Release newest version on GitHub (from dist/)"
        echo " 14) Back up the flatpak signing key to the NAS (after the key changes)"
        echo "  q) Quit"
        read -rp "Choose: " choice
        case "$choice" in
            1) run_step compile ;;
            2) run_step install ;;
            3) if [ "$(id -u)" -eq 0 ]; then run_step thumbnails; else run_privileged thumbnails; fi ;;
            4) if [ "$(id -u)" -eq 0 ]; then run_step uninstall; else run_privileged uninstall; fi ;;
            7) run_step dist ;;
            8) run_step cleanup ;;
            9) run_step release-github ;;
            10) run_step reinstall-flatpak ;;
            11) run_step bump ;;
            12) run_step publish-flatpak ;;
            13) run_step full-release ;;
            14) run_step backup-key ;;
            q|Q) exit 0 ;;
            *) echo "Unknown choice." ;;
        esac
    done
}

case "${1:-}" in
    compile)    compile ;;
    install)    install_app ;;
    thumbnails) install_thumbnails ;;
    uninstall)  do_uninstall ;;
    dist)               make_distribution ;;
    cleanup)            cleanup_build ;;
    release-github)     release_github ;;
    reinstall-flatpak)  reinstall_flatpak ;;
    bump)               bump_version ;;
    publish-flatpak)    publish_flatpak ;;
    full-release)       full_release ;;
    backup-key)         backup_key ;;
    "")         show_menu ;;
    *)
        echo "Usage: $0 {install|thumbnails|uninstall|compile|cleanup|reinstall-flatpak}" >&2
        echo "       $0 {bump|dist|publish-flatpak|release-github|full-release|backup-key}" >&2
        echo "  (no argument) — interactive menu" >&2
        echo "  install     — app + GNOME app grid entry (per-user, no root; PREFIX default ~/.local)" >&2
        echo "  thumbnails  — Nautilus preview thumbnails (needs root; PREFIX default /usr/local)" >&2
        echo "  uninstall   — removes both (run with sudo to also remove thumbnail support)" >&2
        exit 1
        ;;
esac
