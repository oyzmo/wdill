#!/bin/bash
# wdill — build the Flatpak bundle: dist/wdill-<version>-<arch>.flatpak
#
# With the oyzmo Flatpak site checked out (FLATPAK_SITE) and its signing key
# present, the build is also published there through the site's
# tools/publish.sh, and the bundle carries flatpak.oyzmo.no as its update
# address — so GNOME Software offers new versions as updates. Pushing the site
# live is a separate step (install.sh menu). Where there is no site, a plain
# bundle; where there is one but no key, it stops before building.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$HERE/Cargo.toml" | head -1)"
OUT="$HERE/dist/wdill-$VERSION-$(uname -m).flatpak"
# The oyzmo Flatpak site lives in the root of the shared code folder: that is
# /mnt/mycode on the VM and ~/Code on the desktop (the same share), told apart
# by where this script is. FLATPAK_SITE overrides. Anywhere else — someone
# else's checkout — there is no site and the bundle is a plain one.
case "$HERE/" in
    /mnt/mycode/*)   FLATPAK_SITE="${FLATPAK_SITE:-/mnt/mycode/flatpak}" ;;
    "$HOME"/Code/*)  FLATPAK_SITE="${FLATPAK_SITE:-$HOME/Code/flatpak}" ;;
    *)               FLATPAK_SITE="${FLATPAK_SITE:-}" ;;
esac

cd "$HERE"
# With the site present, publishing is not optional: check the key and tools
# before the long build rather than quietly making a bundle without updates.
publish=0
if [ -n "$FLATPAK_SITE" ] && [ -d "$FLATPAK_SITE" ]; then
    "$FLATPAK_SITE/tools/publish.sh" --check || exit 1
    publish=1
fi

flatpak-builder --force-clean --repo=repo build-dir no.oyzmo.Wdill.yml
mkdir -p dist
if [ "$publish" = 1 ]; then
    "$FLATPAK_SITE/tools/publish.sh" repo no.oyzmo.Wdill wdill "$VERSION" "$OUT"
else
    echo "No oyzmo Flatpak site here — building an unpublished bundle."
    flatpak build-bundle repo "$OUT" no.oyzmo.Wdill
    echo "Wrote $OUT"
fi

# The bundle conveys object code, so GPLv3 §6 requires the corresponding
# source beside it — build that too, as package.sh does for the zip.
echo
"$HERE/source.sh"
