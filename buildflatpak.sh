#!/bin/bash
# wdill — build the Flatpak bundle: dist/wdill-<version>-<arch>.flatpak
#
# With the oyzmo Flatpak site checked out (FLATPAK_SITE) and its signing key
# present, the build is also published there through the site's
# tools/publish.sh, and the bundle carries flatpak.oyzmo.no as its update
# address — so GNOME Software offers new versions as updates. Pushing the site
# live is a separate step (install.sh menu). Without either, a plain bundle.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$HERE/Cargo.toml" | head -1)"
OUT="$HERE/dist/wdill-$VERSION-$(uname -m).flatpak"
FLATPAK_SITE="${FLATPAK_SITE:-/mnt/mycode/flatpak}"
FLATPAK_GNUPG="${FLATPAK_GNUPG:-$HOME/.local/share/oyzmo-flatpak/gnupg}"

cd "$HERE"
flatpak-builder --force-clean --repo=repo build-dir no.oyzmo.Wdill.yml
mkdir -p dist
if [ -x "$FLATPAK_SITE/tools/publish.sh" ] && [ -d "$FLATPAK_GNUPG" ]; then
    "$FLATPAK_SITE/tools/publish.sh" repo no.oyzmo.Wdill wdill "$VERSION" "$OUT"
else
    echo "No $FLATPAK_SITE or signing key — building an unpublished bundle."
    flatpak build-bundle repo "$OUT" no.oyzmo.Wdill
    echo "Wrote $OUT"
fi

# The bundle conveys object code, so GPLv3 §6 requires the corresponding
# source beside it — build that too, as package.sh does for the zip.
echo
"$HERE/source.sh"
