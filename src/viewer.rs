// wdill — Copyright (C) 2026 oyzmo. Free software under the GNU General Public
// License, version 3 or later; see the LICENSE file. No warranty.
// SPDX-License-Identifier: GPL-3.0-or-later

use std::cell::{Cell, RefCell};
use std::f32::consts::{FRAC_PI_2, PI};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use anyhow::Result;
use glam::{Mat3, Vec3};
use gtk::{cairo, glib};

use crate::mesh::{self, Mesh};
use crate::render;

/// While rotating/zooming, meshes bigger than this are drawn as a subsample
/// of about this many triangles; the full mesh is rendered when input rests.
const LOD_TARGET: usize = 60_000;
/// How long after the last drag/scroll event the full-quality redraw happens.
const SETTLE: Duration = Duration::from_millis(160);

struct State {
    mesh: Option<Mesh>,
    lod: Vec<[Vec3; 3]>,
    interactive: bool,
    yaw: f32,
    tilt: f32,
    zoom: f32,
    drag_from: (f32, f32),
}

#[derive(Default)]
struct Scratch {
    pixels: Vec<u32>,
    zbuf: Vec<f32>,
}

enum Msg {
    Progress(f64),
    Done(Result<Mesh>),
}

pub struct Viewer {
    pub widget: gtk::Stack,
    area: gtk::DrawingArea,
    loading_page: adw::StatusPage,
    progress: gtk::ProgressBar,
    load_gen: Rc<Cell<u64>>,
    state: Rc<RefCell<State>>,
}

impl Viewer {
    pub fn new() -> Self {
        let state = Rc::new(RefCell::new(State {
            mesh: None,
            lod: Vec::new(),
            interactive: false,
            yaw: 0.7,
            tilt: -FRAC_PI_2 + 0.42,
            zoom: 1.0,
            drag_from: (0.0, 0.0),
        }));
        let settle_timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

        let area = gtk::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_cursor_from_name(Some("grab"));
        {
            let state = state.clone();
            let scratch = RefCell::new(Scratch::default());
            area.set_draw_func(move |area, cr, w, h| {
                draw(&state.borrow(), &mut scratch.borrow_mut(), cr, w, h, area.scale_factor());
            });
        }

        let drag = gtk::GestureDrag::new();
        {
            let state = state.clone();
            drag.connect_drag_begin(move |_, _, _| {
                let mut s = state.borrow_mut();
                s.drag_from = (s.yaw, s.tilt);
            });
        }
        {
            let state = state.clone();
            let area = area.clone();
            let settle_timer = settle_timer.clone();
            drag.connect_drag_update(move |_, dx, dy| {
                {
                    let mut s = state.borrow_mut();
                    s.yaw = s.drag_from.0 + dx as f32 * 0.008;
                    s.tilt = (s.drag_from.1 + dy as f32 * 0.008).clamp(-PI, 0.0);
                }
                touch_interaction(&state, &area, &settle_timer);
                area.queue_draw();
            });
        }
        {
            let state = state.clone();
            let area = area.clone();
            let settle_timer = settle_timer.clone();
            drag.connect_drag_end(move |_, _, _| {
                if let Some(id) = settle_timer.borrow_mut().take() {
                    id.remove();
                }
                state.borrow_mut().interactive = false;
                area.queue_draw();
            });
        }
        area.add_controller(drag);

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        {
            let state = state.clone();
            let area = area.clone();
            let settle_timer = settle_timer.clone();
            scroll.connect_scroll(move |_, _, dy| {
                {
                    let mut s = state.borrow_mut();
                    s.zoom = (s.zoom * 1.1f32.powf(-dy as f32)).clamp(0.05, 50.0);
                }
                touch_interaction(&state, &area, &settle_timer);
                area.queue_draw();
                glib::Propagation::Stop
            });
        }
        area.add_controller(scroll);

        let empty = adw::StatusPage::builder()
            .icon_name("image-x-generic-symbolic")
            .title("No model open")
            .description("Open or drop an STL, 3MF, OBJ or STEP file")
            .build();

        let progress = gtk::ProgressBar::builder()
            .width_request(280)
            .halign(gtk::Align::Center)
            .build();
        let loading_page = adw::StatusPage::builder().title("Loading…").build();
        loading_page.set_child(Some(&progress));

        let widget = gtk::Stack::new();
        widget.add_named(&empty, Some("empty"));
        widget.add_named(&loading_page, Some("loading"));
        widget.add_named(&area, Some("viewer"));

        Self {
            widget,
            area,
            loading_page,
            progress,
            load_gen: Rc::new(Cell::new(0)),
            state,
        }
    }

    /// Load a file on a worker thread; `on_done` runs on the main thread with
    /// the triangle count or the error. Starting a new load supersedes any
    /// load still in flight.
    pub fn open(&self, path: &Path, on_done: impl FnOnce(Result<usize>) + 'static) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());

        let generation = self.load_gen.get().wrapping_add(1);
        self.load_gen.set(generation);
        self.progress.set_fraction(0.0);
        self.loading_page.set_description(Some(&name));
        self.widget.set_visible_child_name("loading");

        let (tx, rx) = async_channel::bounded::<Msg>(16);
        let path = path.to_owned();
        std::thread::spawn(move || {
            let last = Cell::new(0.0f64);
            let progress_tx = tx.clone();
            let report = move |f: f64| {
                // Throttle to full percents so the UI isn't flooded.
                if f - last.get() >= 0.01 {
                    last.set(f);
                    let _ = progress_tx.send_blocking(Msg::Progress(f.clamp(0.0, 1.0)));
                }
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                mesh::load(&path, &report)
            }))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("internal error while reading the file")));
            let _ = tx.send_blocking(Msg::Done(result));
        });

        let state = self.state.clone();
        let widget = self.widget.clone();
        let area = self.area.clone();
        let progress = self.progress.clone();
        let load_gen = self.load_gen.clone();
        glib::spawn_future_local(async move {
            let mut on_done = Some(on_done);
            while let Ok(msg) = rx.recv().await {
                if load_gen.get() != generation {
                    break; // a newer load took over this Viewer
                }
                match msg {
                    Msg::Progress(f) => progress.set_fraction(f),
                    Msg::Done(result) => {
                        let on_done = on_done.take().unwrap();
                        match result {
                            Ok(mesh) => {
                                let count = mesh.triangles.len();
                                let mut s = state.borrow_mut();
                                s.lod = build_lod(&mesh.triangles);
                                s.mesh = Some(mesh);
                                drop(s);
                                widget.set_visible_child_name("viewer");
                                area.queue_draw();
                                on_done(Ok(count));
                            }
                            Err(err) => {
                                let page =
                                    if state.borrow().mesh.is_some() { "viewer" } else { "empty" };
                                widget.set_visible_child_name(page);
                                on_done(Err(err));
                            }
                        }
                        break;
                    }
                }
            }
        });
    }
}

/// Uniform subsample used while the model is being rotated/zoomed.
fn build_lod(triangles: &[[Vec3; 3]]) -> Vec<[Vec3; 3]> {
    if triangles.len() <= LOD_TARGET * 4 / 3 {
        return Vec::new(); // full mesh is cheap enough
    }
    let step = triangles.len().div_ceil(LOD_TARGET);
    triangles.iter().step_by(step).copied().collect()
}

/// Mark the view as being interacted with and (re)start the timer that
/// switches back to full quality once input rests.
fn touch_interaction(
    state: &Rc<RefCell<State>>,
    area: &gtk::DrawingArea,
    settle_timer: &Rc<RefCell<Option<glib::SourceId>>>,
) {
    state.borrow_mut().interactive = true;
    if let Some(id) = settle_timer.borrow_mut().take() {
        id.remove();
    }
    let state = state.clone();
    let area = area.clone();
    let timer = settle_timer.clone();
    let id = glib::timeout_add_local_once(SETTLE, move || {
        timer.borrow_mut().take();
        state.borrow_mut().interactive = false;
        area.queue_draw();
    });
    *settle_timer.borrow_mut() = Some(id);
}

fn draw(state: &State, scratch: &mut Scratch, cr: &cairo::Context, w: i32, h: i32, sf: i32) {
    if w <= 0 || h <= 0 {
        return;
    }
    let Some(mesh) = &state.mesh else {
        let bg = render::BACKGROUND;
        cr.set_source_rgb(
            f64::from(bg >> 16 & 0xff) / 255.0,
            f64::from(bg >> 8 & 0xff) / 255.0,
            f64::from(bg & 0xff) / 255.0,
        );
        let _ = cr.paint();
        return;
    };

    let sf = sf.max(1);
    let (pw, ph) = ((w * sf) as usize, (h * sf) as usize);
    let triangles: &[[Vec3; 3]] = if state.interactive && !state.lod.is_empty() {
        &state.lod
    } else {
        &mesh.triangles
    };
    let params = render::Params {
        // Models are Z-up; tilt maps that onto the screen (0 = top view, -PI/2 = front).
        rot: Mat3::from_rotation_x(state.tilt) * Mat3::from_rotation_z(state.yaw),
        center: mesh.center,
        scale: 0.45 * pw.min(ph) as f32 * state.zoom / mesh.radius,
        width: pw,
        height: ph,
    };
    render::render(triangles, &params, &mut scratch.pixels, &mut scratch.zbuf);

    let Ok(mut surface) = cairo::ImageSurface::create(cairo::Format::Rgb24, pw as i32, ph as i32)
    else {
        return;
    };
    surface.set_device_scale(f64::from(sf), f64::from(sf));
    {
        let stride = surface.stride() as usize;
        let Ok(mut data) = surface.data() else { return };
        let bytes: &[u8] = bytemuck::cast_slice(&scratch.pixels);
        if stride == pw * 4 {
            data.copy_from_slice(bytes);
        } else {
            for y in 0..ph {
                data[y * stride..y * stride + pw * 4]
                    .copy_from_slice(&bytes[y * pw * 4..(y + 1) * pw * 4]);
            }
        }
    }
    let _ = cr.set_source_surface(&surface, 0.0, 0.0);
    let _ = cr.paint();
}
