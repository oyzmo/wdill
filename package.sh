#!/usr/bin/env bash
# wdill — build the distributable manual-install zip.
#
#   ./package.sh
#
# Produces wdill-<version>-<arch>.zip in dist/, containing a prebuilt binary,
# the desktop/icon/thumbnailer data files, a standalone install.sh (which does
# no compiling) and the licences. That zip is what gets handed to users who
# want Nautilus thumbnails, which the Flatpak build cannot provide.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_ID="no.oyzmo.Wdill"
ARCH="$(uname -m)"
VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$HERE/Cargo.toml" | head -1)"
NAME="wdill-$VERSION-$ARCH"
OUT="$HERE/dist/$NAME.zip"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

command -v zip >/dev/null || { echo "zip is not installed (dnf install zip)" >&2; exit 1; }

echo "Building wdill $VERSION ($ARCH)..."
cargo build --release --manifest-path "$HERE/Cargo.toml"
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps --manifest-path "$HERE/Cargo.toml" \
    | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
BIN="$TARGET_DIR/release/wdill"
[ -x "$BIN" ] || { echo "No binary at $BIN" >&2; exit 1; }

# Sanity-check what the shipped binary actually demands of a user's machine,
# so the README's stated floor can't silently drift away from reality.
GLIBC_MAX="$(objdump -T "$BIN" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)"
echo "Highest glibc symbol required: $GLIBC_MAX"

PKG="$STAGE/$NAME"
mkdir -p "$PKG/data"
install -Dm755 "$BIN" "$PKG/wdill"
strip "$PKG/wdill" 2>/dev/null || true
install -Dm755 "$HERE/packaging/install.sh" "$PKG/install.sh"
install -Dm644 "$HERE/data/$APP_ID.desktop.in" "$PKG/data/$APP_ID.desktop.in"
install -Dm644 "$HERE/data/$APP_ID.svg"        "$PKG/data/$APP_ID.svg"
install -Dm644 "$HERE/data/$APP_ID.metainfo.xml" "$PKG/data/$APP_ID.metainfo.xml"
install -Dm644 "$HERE/data/$APP_ID.thumbnailer" "$PKG/data/$APP_ID.thumbnailer"
install -Dm644 "$HERE/data/$APP_ID.mime.xml"    "$PKG/data/$APP_ID.mime.xml"
install -Dm644 "$HERE/LICENSE"        "$PKG/LICENSE"
install -Dm644 "$HERE/THIRD-PARTY.md" "$PKG/THIRD-PARTY.md"
sed -e "s|@VERSION@|$VERSION|g" -e "s|@ARCH@|$ARCH|g" \
    "$HERE/packaging/README.txt" > "$PKG/README.txt"
chmod 644 "$PKG/README.txt"

mkdir -p "$HERE/dist"
rm -f "$OUT"
(cd "$STAGE" && zip -qr "$OUT" "$NAME")

echo
echo "Wrote $OUT"
unzip -l "$OUT" | tail -n +4 | head -n -2

# This zip conveys object code, so GPLv3 §6 requires the corresponding source
# to be offered from the same place. Build it here so the two can never be
# published out of step.
echo
"$HERE/source.sh"
