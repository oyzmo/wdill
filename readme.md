# wdill (WhatDoesItLookLike)

A tiny 3D printfile viewer for GNOME, written in Rust with GTK4 + Libadwaita.
Free software under the [GPL-3.0-or-later](LICENSE).

Opens and displays .stl (binary and ASCII), .3mf, .obj, .step/.stp, .glb and .fbx files.
Files load on a background thread with a progress bar, so big models don't freeze the UI.
Rendering is a multithreaded software rasterizer — meshes with hundreds of thousands of triangles rotate smoothly.

## Usage

- Open a file with the header-bar button (Ctrl+O), by dropping it on the window, or `wdill model.stl`
- Drag to rotate, scroll to zoom

## Installing (Flatpak)

```sh
flatpak install https://flatpak.oyzmo.no/wdill.flatpakref
```

or open that link in GNOME Software. New versions then show up in Software's
Updates (or with `flatpak update`). The Flatpak can't provide Nautilus
thumbnails; for those, use the manual-install zip from the GitHub releases.

## Building and installing

```sh
./install.sh compile    # cargo build --release
./install.sh install    # build + install to ~/.local (binary, GNOME desktop entry, icon)
./install.sh uninstall  # remove everything again
```

`install.sh install` needs no root; wdill then shows up in the GNOME app grid.
Set `PREFIX` for a system-wide install: `sudo env PREFIX=/usr/local ./install.sh install`
(sudo drops a `PREFIX` set in front of it, and without one `install` refuses
to run as root).

## Distributing

```sh
./package.sh   # -> dist/wdill-<version>-<arch>.zip  (+ the source zip below)
./source.sh    # -> dist/wdill-<version>-src.zip
./buildflatpak.sh  # -> dist/wdill-<version>-<arch>.flatpak  (+ the source zip)
./install.sh       # menu; its Release section bumps, builds all of the above,
                   # publishes to flatpak.oyzmo.no and makes the GitHub release
```

`package.sh` builds a release binary and packs it with the data files, a
standalone `install.sh` (install / thumbnails / uninstall, no compiler needed)
and the licences. This is the manual-install package for users who want
Nautilus thumbnails — the Flatpak build cannot provide those. It needs
GTK >= 4.10, libadwaita >= 1.5 and glibc >= 2.35 on the target machine.

`source.sh` builds the source release: everything needed to build, install and
modify wdill, plus `LICENSE` and `THIRD-PARTY.md`. `package.sh` runs it too, so
the binary and source zips can never be published out of step. **Both the
`.flatpak` bundle and the binary zip convey object code, so this source zip has
to be published in the same place, at no further charge** — that is what GPLv3
section 6 requires, and it is why `source.sh` exists.

## Development

```sh
cargo run -- path/to/model.stl
cargo test
```

## Licence

wdill is free software: you can redistribute it and/or modify it under the
terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version. It is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
PARTICULAR PURPOSE. See [LICENSE](LICENSE) for the full text.

Copyright (C) 2026 oyzmo.

wdill links GTK 4, libadwaita and GLib (LGPL-2.1-or-later) and compiles in
crates from crates.io under their own permissive licences; all of it is
compatible with the GPLv3 above. The attribution those licences require is
reproduced in [THIRD-PARTY.md](THIRD-PARTY.md), which is installed next to the
licence text at `<prefix>/share/licenses/wdill/` and, in the Flatpak build, at
`/app/share/licenses/no.oyzmo.Wdill/`.
