wdill @VERSION@ — WhatDoesItLookLike
A tiny 3D print file viewer for GNOME. Opens .stl (binary and ASCII), .3mf,
.obj, .step/.stp, .glb and .fbx files.

This package contains a prebuilt @ARCH@ Linux binary — no compiler, no Rust
toolchain, no build step. Unlike the Flatpak build, this one CAN provide
Nautilus file-manager thumbnails for 3D model files.


REQUIREMENTS
------------
  * Linux, @ARCH@
  * GTK >= 4.10 and libadwaita >= 1.5, i.e. Fedora 40+, Ubuntu 24.04+,
    Debian 13+ or newer. (Older releases such as Ubuntu 22.04, Debian 12 or
    Fedora 39 ship libraries that are too old and will not run this build.)
        Fedora:         sudo dnf install gtk4 libadwaita
        Debian/Ubuntu:  sudo apt install libgtk-4-1 libadwaita-1-0
    The installer checks that they are present. It cannot tell whether they
    are new enough — on a too-old distro wdill starts and immediately exits
    with a "symbol lookup error". Use the Flatpak build there instead; it
    bundles its own GTK and runs anywhere, but has no thumbnail support.
  * glibc >= 2.35 (any distro new enough for the GTK version above has this).
  * GNOME/Nautilus, for the optional thumbnail support.


INSTALLING
----------
Unpack the zip, open a terminal in the unpacked folder, and run:

    ./install.sh

That gives you a menu:

  1) Install app                — the viewer plus its GNOME app-grid entry.
                                  Installs to ~/.local, no root needed.
  2) Install thumbnails support — Nautilus preview thumbnails for 3D files.
                                  Installs to /usr/local, so it asks for your
                                  password (the menu re-runs itself with sudo).
  3) Uninstall everything       — removes both.

Pick 1, then pick 2 as well if you want thumbnails. You can also skip the
menu and run the steps directly:

    ./install.sh install
    sudo ./install.sh thumbnails
    sudo ./install.sh uninstall

Set PREFIX to install somewhere else, e.g. PREFIX=/usr ./install.sh install.


WHY THUMBNAILS NEED ROOT
------------------------
GNOME runs every thumbnailer inside a bwrap sandbox that bind-mounts only
/usr — never your home directory. A thumbnailer installed under ~/.local is
therefore unreachable from inside that sandbox and can never run. So the
thumbnail half has to go to a system prefix (/usr/local by default), which
needs root. The app itself has no such restriction and installs per-user.

If some files still show no preview after installing, GNOME may have recorded
an earlier failure as permanent. Clear the cache and restart Nautilus:

    rm -rf ~/.cache/thumbnails
    nautilus -q


USING IT
--------
  * Open a file from the header-bar button (Ctrl+O), by dropping it on the
    window, or from a terminal: wdill model.stl
  * Drag to rotate, scroll to zoom.

Large meshes load on a background thread with a progress bar, and rendering
is a multithreaded software rasterizer — no GPU driver requirements.


LICENCE
-------
wdill is free software: you may use, study, share and modify it under the
terms of the GNU General Public License, version 3 or later. The full text is
in the LICENSE file, and the installer copies it — together with the
third-party attribution notices in THIRD-PARTY.md — to

    <prefix>/share/licenses/wdill/

i.e. ~/.local/share/licenses/wdill/ for the per-user app install, and
/usr/local/share/licenses/wdill/ for the thumbnail install.

This package ships a prebuilt binary. Its complete corresponding source — all
of what is needed to rebuild and modify exactly this program — is published as

    wdill-@VERSION@-src.zip

from the same place you downloaded this package (http://oyzmo.no/), at no
further charge, as GPLv3 section 6 requires.

wdill comes with ABSOLUTELY NO WARRANTY, to the extent permitted by law.
