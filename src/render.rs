// wdill — Copyright (C) 2026 oyzmo. Free software under the GNU General Public
// License, version 3 or later; see the LICENSE file. No warranty.
// SPDX-License-Identifier: GPL-3.0-or-later

//! Multithreaded software rasterizer: triangles go through an orthographic
//! transform, then bands of rows are rasterized in parallel with a z-buffer.
//! Output is a `0x00RRGGBB` pixel buffer matching cairo's `Format::Rgb24`.

use glam::{Mat3, Vec3};
use rayon::prelude::*;

pub const BACKGROUND: u32 = 0x00FB_F5EA; // Cream page bg
const BASE_COLOR: Vec3 = Vec3::new(0.498, 0.627, 0.478); // Sage #7FA07A
const BAND_ROWS: usize = 64;

pub struct Params {
    pub rot: Mat3,
    pub center: Vec3,
    /// Pixels per model unit (includes zoom and HiDPI scale).
    pub scale: f32,
    pub width: usize,
    pub height: usize,
}

struct ScreenTri {
    v: [Vec3; 3],
    ymin: i32,
    ymax: i32, // inclusive
    xmin: i32,
    xmax: i32, // inclusive
    color: u32,
}

pub fn render(triangles: &[[Vec3; 3]], p: &Params, pixels: &mut Vec<u32>, zbuf: &mut Vec<f32>) {
    let (w, h) = (p.width, p.height);
    pixels.clear();
    pixels.resize(w * h, BACKGROUND);
    zbuf.clear();
    zbuf.resize(w * h, f32::NEG_INFINITY);
    if w == 0 || h == 0 {
        return;
    }

    // Screen space: y points down, so negative y is light from above.
    let light = Vec3::new(0.3, -0.45, 0.85).normalize();
    let (wf, hf) = (w as f32, h as f32);

    let tris: Vec<ScreenTri> = triangles
        .par_iter()
        .filter_map(|t| {
            let v = t.map(|q| {
                let r = p.rot * (q - p.center) * p.scale;
                Vec3::new(wf / 2.0 + r.x, hf / 2.0 - r.y, r.z)
            });
            let mut normal = (v[1] - v[0]).cross(v[2] - v[0]);
            let len = normal.length();
            if len <= f32::EPSILON {
                return None;
            }
            if normal.z < 0.0 {
                normal = -normal; // shade double-sided, winding order varies in the wild
            }
            // The flip above already makes shading double-sided; clamping (not
            // abs) keeps faces turned away from the light in shadow.
            let shade = 0.18 + 0.82 * (normal / len).dot(light).max(0.0);
            let c = (BASE_COLOR * shade).clamp(Vec3::ZERO, Vec3::ONE) * 255.0;
            let color = ((c.x as u32) << 16) | ((c.y as u32) << 8) | c.z as u32;

            let xmin = v[0].x.min(v[1].x).min(v[2].x).floor() as i32;
            let xmax = v[0].x.max(v[1].x).max(v[2].x).ceil() as i32;
            let ymin = v[0].y.min(v[1].y).min(v[2].y).floor() as i32;
            let ymax = v[0].y.max(v[1].y).max(v[2].y).ceil() as i32;
            if xmax < 0 || ymax < 0 || xmin >= w as i32 || ymin >= h as i32 {
                return None;
            }
            Some(ScreenTri {
                v,
                ymin: ymin.max(0),
                ymax: ymax.min(h as i32 - 1),
                xmin: xmin.max(0),
                xmax: xmax.min(w as i32 - 1),
                color,
            })
        })
        .collect();

    pixels
        .par_chunks_mut(w * BAND_ROWS)
        .zip(zbuf.par_chunks_mut(w * BAND_ROWS))
        .enumerate()
        .for_each(|(band, (pix, zb))| {
            let y0 = (band * BAND_ROWS) as i32;
            let y1 = y0 + (pix.len() / w) as i32; // exclusive
            for t in &tris {
                if t.ymax < y0 || t.ymin >= y1 {
                    continue;
                }
                raster(t, pix, zb, w, y0, y1);
            }
        });
}

fn raster(t: &ScreenTri, pix: &mut [u32], zb: &mut [f32], w: usize, y0: i32, y1: i32) {
    let [a, b, c] = t.v;
    let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    if area.abs() < f32::EPSILON {
        return;
    }
    let inv = 1.0 / area;
    for y in t.ymin.max(y0)..=t.ymax.min(y1 - 1) {
        let py = y as f32 + 0.5;
        let row = (y - y0) as usize * w;
        for x in t.xmin..=t.xmax {
            let px = x as f32 + 0.5;
            // Barycentric coordinates; `inv` folds the winding sign away.
            let l0 = ((c.x - b.x) * (py - b.y) - (c.y - b.y) * (px - b.x)) * inv;
            let l1 = ((a.x - c.x) * (py - c.y) - (a.y - c.y) * (px - c.x)) * inv;
            let l2 = 1.0 - l0 - l1;
            if l0 < 0.0 || l1 < 0.0 || l2 < 0.0 {
                continue;
            }
            let z = l0 * a.z + l1 * b.z + l2 * c.z;
            let idx = row + x as usize;
            if z > zb[idx] {
                zb[idx] = z;
                pix[idx] = t.color;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_scene(tris: &[[Vec3; 3]]) -> Vec<u32> {
        let p = Params {
            rot: Mat3::IDENTITY,
            center: Vec3::ZERO,
            scale: 1.0,
            width: 64,
            height: 64,
        };
        let (mut pix, mut z) = (Vec::new(), Vec::new());
        render(tris, &p, &mut pix, &mut z);
        pix
    }

    // Flat triangle around the origin (screen center).
    const FLAT: [Vec3; 3] = [
        Vec3::new(-20.0, -20.0, 0.0),
        Vec3::new(20.0, -20.0, 0.0),
        Vec3::new(0.0, 25.0, 0.0),
    ];
    // Tilted triangle, nearer to the viewer at the center (different shade than FLAT).
    const TILTED: [Vec3; 3] = [
        Vec3::new(-15.0, -15.0, 10.0),
        Vec3::new(15.0, -15.0, 2.0),
        Vec3::new(0.0, 20.0, 2.0),
    ];

    #[test]
    fn rasterizes_triangle() {
        let pix = render_scene(&[FLAT]);
        assert_ne!(pix[32 * 64 + 32], BACKGROUND);
        assert_eq!(pix[0], BACKGROUND); // corner stays background
    }

    #[test]
    #[ignore = "timing benchmark; run with: cargo test --release -- --ignored"]
    fn bench_300k_triangles() {
        // Pseudo-random triangle soup on a sphere surface, ~300k triangles.
        let mut seed = 1u64;
        let mut rnd = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 40) as f32 / (1 << 24) as f32 - 0.5
        };
        let tris: Vec<[Vec3; 3]> = (0..300_000)
            .map(|_| {
                let base = Vec3::new(rnd(), rnd(), rnd()).normalize() * 200.0;
                [base, base + Vec3::new(rnd(), rnd(), 0.0), base + Vec3::new(0.0, rnd(), rnd())]
            })
            .collect();
        let p = Params {
            rot: Mat3::IDENTITY,
            center: Vec3::ZERO,
            scale: 2.0,
            width: 1920,
            height: 1080,
        };
        let (mut pix, mut z) = (Vec::new(), Vec::new());
        render(&tris, &p, &mut pix, &mut z); // warm-up
        let t = std::time::Instant::now();
        for _ in 0..10 {
            render(&tris, &p, &mut pix, &mut z);
        }
        println!("300k triangles @1080p: {:.1?}/frame", t.elapsed() / 10);
    }

    #[test]
    fn zbuffer_is_order_independent() {
        let flat_only = render_scene(&[FLAT]);
        let ab = render_scene(&[FLAT, TILTED]);
        let ba = render_scene(&[TILTED, FLAT]);
        let center = 32 * 64 + 32;
        assert_eq!(ab[center], ba[center]);
        assert_ne!(ab[center], flat_only[center]); // nearer tilted triangle wins
    }
}
