#!/bin/bash
# wdill — build the Flatpak bundle: dist/wdill-<version>-<arch>.flatpak
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$HERE/Cargo.toml" | head -1)"
OUT="$HERE/dist/wdill-$VERSION-$(uname -m).flatpak"

cd "$HERE"
flatpak-builder --force-clean --repo=repo build-dir no.oyzmo.Wdill.yml
mkdir -p dist
flatpak build-bundle repo "$OUT" no.oyzmo.Wdill
echo "Wrote $OUT"

# The bundle conveys object code, so GPLv3 §6 requires the corresponding
# source beside it — build that too, as package.sh does for the zip.
echo
"$HERE/source.sh"
