#!/usr/bin/env bash
# wdill — build the source distribution zip.
#
#   ./source.sh
#
# Produces dist/wdill-<version>-src.zip: every file needed to build, install
# and modify wdill, plus LICENSE (GPLv3) and THIRD-PARTY.md.
#
# This is not a convenience — it is how wdill meets GPLv3 §6. The .flatpak
# bundle and the wdill-<version>-<arch>.zip both convey object code, so the
# corresponding source has to be offered from the same place they are, at no
# further charge. Publish this zip alongside them, every release.
#
# Deliberately excluded: CLAUDE.md and .claude/ (agent notes, full of paths
# specific to the author's machine), any .cargo/config.toml (it redirects the
# target dir to an absolute path that exists on no other machine), and the
# build outputs — target/, dist/, build-dir/, repo/, *.flatpak.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_ID="no.oyzmo.Wdill"
VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$HERE/Cargo.toml" | head -1)"
NAME="wdill-$VERSION-src"
OUT="$HERE/dist/$NAME.zip"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

command -v zip >/dev/null || { echo "zip is not installed (dnf install zip)" >&2; exit 1; }

# Everything the zip carries, relative to the repo root. Listed explicitly
# rather than filtered with excludes: a new build output or private note in
# the tree then stays out by default instead of leaking in by default.
FILES=(
    LICENSE
    THIRD-PARTY.md
    readme.md
    Cargo.toml
    Cargo.lock
    src/main.rs
    src/mesh.rs
    src/render.rs
    src/viewer.rs
    "data/$APP_ID.desktop.in"
    "data/$APP_ID.metainfo.xml"
    "data/$APP_ID.svg"
    "data/$APP_ID.thumbnailer"
    "data/$APP_ID.mime.xml"
    install.sh
    package.sh
    source.sh
    buildflatpak.sh
    "$APP_ID.yml"
    packaging/install.sh
    packaging/README.txt
)

PKG="$STAGE/wdill-$VERSION"
for f in "${FILES[@]}"; do
    [ -f "$HERE/$f" ] || { echo "Missing source file: $f" >&2; exit 1; }
    mkdir -p "$PKG/$(dirname "$f")"
    cp -p "$HERE/$f" "$PKG/$f"
done
chmod 755 "$PKG"/*.sh "$PKG/packaging/install.sh"

# The licence has to be the real thing, not a stale proprietary LICENSE left
# behind by a bad merge — a source zip shipping the wrong terms is worse than
# no source zip at all.
grep -q "GNU GENERAL PUBLIC LICENSE" "$PKG/LICENSE" \
    || { echo "LICENSE is not the GPL text — refusing to build a source zip" >&2; exit 1; }
grep -q '^license = "GPL-3.0-or-later"' "$PKG/Cargo.toml" \
    || { echo "Cargo.toml does not declare GPL-3.0-or-later" >&2; exit 1; }

mkdir -p "$HERE/dist"
rm -f "$OUT"
(cd "$STAGE" && zip -qr "$OUT" "wdill-$VERSION")

echo "Wrote $OUT"
unzip -l "$OUT" | tail -n +4 | head -n -2
echo
echo "GPLv3 §6: publish this next to the .flatpak and the binary zip."
