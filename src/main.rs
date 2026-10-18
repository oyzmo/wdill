// wdill (WhatDoesItLookLike) — a tiny 3D printfile viewer for GNOME.
// Copyright (C) 2026 oyzmo
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option)
// any later version.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or
// FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for
// more details.
//
// You should have received a copy of the GNU General Public License along
// with this program. If not, see <https://www.gnu.org/licenses/>.
//
// SPDX-License-Identifier: GPL-3.0-or-later

mod mesh;
mod render;
mod viewer;

use std::f32::consts::FRAC_PI_2;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use anyhow::Context;
use glam::Mat3;
use gtk::{cairo, gdk, gio, glib};

use crate::viewer::Viewer;

const APP_ID: &str = "no.oyzmo.Wdill";

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--thumbnail") {
        return match run_thumbnail(&args[2..]) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("wdill --thumbnail: {err:#}");
                glib::ExitCode::FAILURE
            }
        };
    }

    glib::set_application_name("wdill");
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.connect_activate(|app| build_window(app, None));
    app.connect_open(|app, files, _| {
        let paths: Vec<PathBuf> = files.iter().filter_map(|f| f.path()).collect();
        if paths.is_empty() {
            build_window(app, None);
        } else {
            for path in paths {
                build_window(app, Some(path));
            }
        }
    });
    app.run()
}

fn build_window(app: &adw::Application, path: Option<PathBuf>) {
    let viewer = Rc::new(Viewer::new());

    let title = adw::WindowTitle::new("wdill", "What does it look like?");
    let header = adw::HeaderBar::builder().title_widget(&title).build();

    let open_btn = gtk::Button::from_icon_name("document-open-symbolic");
    open_btn.set_tooltip_text(Some("Open a 3D model (Ctrl+O)"));
    open_btn.set_action_name(Some("win.open"));
    header.pack_start(&open_btn);

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&viewer.widget));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&toasts);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("wdill")
        .icon_name(APP_ID)
        .default_width(960)
        .default_height(720)
        .content(&content)
        .build();

    let open_action = gio::SimpleAction::new("open", None);
    {
        let window = window.clone();
        let viewer = viewer.clone();
        let title = title.clone();
        let toasts = toasts.clone();
        open_action.connect_activate(move |_, _| {
            show_open_dialog(&window, &viewer, &title, &toasts);
        });
    }
    window.add_action(&open_action);
    app.set_accels_for_action("win.open", &["<Primary>o"]);

    let drop = gtk::DropTarget::new(gio::File::static_type(), gdk::DragAction::COPY);
    {
        let viewer = viewer.clone();
        let title = title.clone();
        let toasts = toasts.clone();
        drop.connect_drop(move |_, value, _, _| {
            if let Ok(file) = value.get::<gio::File>()
                && let Some(path) = file.path()
            {
                open_path(&viewer, &title, &toasts, &path);
                return true;
            }
            false
        });
    }
    viewer.widget.add_controller(drop);

    if let Some(path) = path {
        open_path(&viewer, &title, &toasts, &path);
    }
    window.present();
}

fn show_open_dialog(
    window: &adw::ApplicationWindow,
    viewer: &Rc<Viewer>,
    title: &adw::WindowTitle,
    toasts: &adw::ToastOverlay,
) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("3D models (STL, 3MF, OBJ, STEP, GLB, FBX)"));
    for suffix in ["stl", "3mf", "obj", "step", "stp", "glb", "fbx"] {
        filter.add_suffix(suffix);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);

    let dialog = gtk::FileDialog::builder().title("Open 3D model").modal(true).build();
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&filter));

    let viewer = viewer.clone();
    let title = title.clone();
    let toasts = toasts.clone();
    dialog.open(Some(window), gio::Cancellable::NONE, move |result| {
        if let Ok(file) = result
            && let Some(path) = file.path()
        {
            open_path(&viewer, &title, &toasts, &path);
        }
    });
}

/// Headless mode for GNOME thumbnailers (see data/no.oyzmo.Wdill.thumbnailer):
/// loads the mesh, rasterizes one fixed view with `render::render`, and writes
/// a PNG — no display connection, no GTK application, no window.
fn run_thumbnail(args: &[String]) -> anyhow::Result<()> {
    let [input, output, size] = args else {
        anyhow::bail!("usage: wdill --thumbnail <input> <output> <size>");
    };
    let size: usize = size.parse().context("size must be a positive integer")?;
    anyhow::ensure!(size > 0, "size must be a positive integer");

    let mesh = mesh::load(Path::new(input), &|_| {})?;

    // Same fixed angle as the viewer's initial yaw/tilt (viewer.rs State::new).
    let params = render::Params {
        rot: Mat3::from_rotation_x(-FRAC_PI_2 + 0.42) * Mat3::from_rotation_z(0.7),
        center: mesh.center,
        scale: 0.45 * size as f32 / mesh.radius,
        width: size,
        height: size,
    };
    let (mut pixels, mut zbuf) = (Vec::new(), Vec::new());
    render::render(&mesh.triangles, &params, &mut pixels, &mut zbuf);

    // Untouched background pixels become transparent; rendered pixels stay
    // opaque (premultiplied alpha, so alpha=0xFF leaves the RGB unchanged).
    for p in pixels.iter_mut() {
        *p = if *p == render::BACKGROUND { 0 } else { *p | 0xFF00_0000 };
    }

    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, size as i32, size as i32)
        .context("failed to create image surface")?;
    {
        let stride = surface.stride() as usize;
        let mut data = surface.data().context("failed to access surface data")?;
        let bytes: &[u8] = bytemuck::cast_slice(&pixels);
        if stride == size * 4 {
            data.copy_from_slice(bytes);
        } else {
            for y in 0..size {
                data[y * stride..y * stride + size * 4]
                    .copy_from_slice(&bytes[y * size * 4..(y + 1) * size * 4]);
            }
        }
    }
    let mut file = std::fs::File::create(output).context("failed to create output file")?;
    surface.write_to_png(&mut file).context("failed to encode PNG")?;
    Ok(())
}

fn open_path(viewer: &Viewer, title: &adw::WindowTitle, toasts: &adw::ToastOverlay, path: &Path) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let title = title.clone();
    let toasts = toasts.clone();
    viewer.open(path, move |result| match result {
        Ok(count) => {
            title.set_title(&name);
            title.set_subtitle(&format!("{count} triangles"));
        }
        Err(err) => {
            let toast = adw::Toast::builder()
                .title(format!("Could not open {name}: {err:#}"))
                .use_markup(false)
                .build();
            toasts.add_toast(toast);
        }
    });
}
