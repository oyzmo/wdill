// wdill — Copyright (C) 2026 oyzmo. Free software under the GNU General Public
// License, version 3 or later; see the LICENSE file. No warranty.
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use glam::{Affine3A, Vec3, Vec3A};
use quick_xml::events::{BytesStart, Event};

/// Loaders report a 0.0–1.0 fraction through this as they go.
pub type Progress<'a> = &'a dyn Fn(f64);

pub struct Mesh {
    pub triangles: Vec<[Vec3; 3]>,
    pub center: Vec3,
    pub radius: f32,
}

impl Mesh {
    fn from_triangles(triangles: Vec<[Vec3; 3]>) -> Result<Self> {
        if triangles.is_empty() {
            bail!("file contains no triangles");
        }
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for tri in &triangles {
            for v in tri {
                // NaN would slip through min/max silently (f32::min ignores it)
                // and leave the model rendering as an empty frame — which the
                // thumbnailer would then cache as a valid blank PNG. Reject the
                // file instead, so callers can fall back to a generic icon.
                if !v.is_finite() {
                    bail!("file contains invalid (NaN or infinite) vertex coordinates");
                }
                min = min.min(*v);
                max = max.max(*v);
            }
        }
        let center = (min + max) / 2.0;
        let radius = (max - center).length();
        if !radius.is_finite() {
            bail!("model bounds are too large to display");
        }
        let radius = radius.max(f32::EPSILON);
        Ok(Self { triangles, center, radius })
    }
}

pub fn load(path: &Path, progress: Progress) -> Result<Mesh> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let triangles = match ext.as_str() {
        "stl" => load_stl(&fs::read(path)?, progress)?,
        "3mf" => load_3mf(fs::File::open(path)?, progress)?,
        // Both formats are ASCII where it matters, but comments and entity
        // names turn up in Latin-1 often enough that strict UTF-8 is a bug.
        "obj" => load_obj(&String::from_utf8_lossy(&fs::read(path)?), progress)?,
        "step" | "stp" => load_step(&String::from_utf8_lossy(&fs::read(path)?), progress)?,
        "glb" => load_glb(&fs::read(path)?, progress)?,
        "fbx" => load_fbx(&fs::read(path)?, progress)?,
        _ => bail!("unsupported file type \".{ext}\" (supported: .stl, .3mf, .obj, .step, .glb, .fbx)"),
    };
    progress(1.0);
    Mesh::from_triangles(triangles)
}

// ---- STL ----

fn load_stl(bytes: &[u8], progress: Progress) -> Result<Vec<[Vec3; 3]>> {
    // Decide on the bytes, not on UTF-8: the solid name is free text, often in
    // a legacy 8-bit encoding (`solid Überteil` in Latin-1), and the 1 KiB
    // window can split a multi-byte character — either used to make an ASCII
    // file look binary. A length that matches the binary layout exactly settles
    // it the other way, for binary headers that happen to start with "solid".
    let head = &bytes[..bytes.len().min(1024)];
    let binary_len =
        bytes.get(80..84).map(|c| 84 + u32::from_le_bytes(c.try_into().unwrap()) as usize * 50);
    let looks_ascii = binary_len != Some(bytes.len())
        && head.starts_with(b"solid")
        && head.windows(5).any(|w| w == b"facet");
    if looks_ascii {
        // Only the keywords and numbers matter, and those are always ASCII.
        parse_stl_ascii(&String::from_utf8_lossy(bytes), progress)
    } else {
        parse_stl_binary(bytes, progress)
    }
}

fn parse_stl_binary(bytes: &[u8], progress: Progress) -> Result<Vec<[Vec3; 3]>> {
    if bytes.len() < 84 {
        bail!("not a recognizable STL file");
    }
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    if bytes.len() < 84 + count * 50 {
        bail!("binary STL is truncated");
    }
    let mut triangles = Vec::with_capacity(count);
    for i in 0..count {
        // 50 bytes per triangle: normal (skipped), 3 vertices, attribute count.
        let base = 84 + i * 50 + 12;
        let f = |off: usize| {
            f32::from_le_bytes(bytes[base + off..base + off + 4].try_into().unwrap())
        };
        let v = |k: usize| Vec3::new(f(k * 12), f(k * 12 + 4), f(k * 12 + 8));
        triangles.push([v(0), v(1), v(2)]);
        if i.is_multiple_of(8192) {
            progress(i as f64 / count as f64);
        }
    }
    Ok(triangles)
}

fn parse_stl_ascii(text: &str, progress: Progress) -> Result<Vec<[Vec3; 3]>> {
    let total = text.len().max(1) as f64;
    let mut consumed = 0usize;
    let mut vertices = Vec::new();
    for (ln, line) in text.lines().enumerate() {
        consumed += line.len() + 1;
        if ln.is_multiple_of(4096) {
            progress(consumed as f64 / total);
        }
        let mut it = line.split_whitespace();
        if it.next() == Some("vertex") {
            let mut num = || -> Result<f32> {
                it.next()
                    .context("truncated vertex line in ASCII STL")?
                    .parse()
                    .context("invalid number in ASCII STL")
            };
            vertices.push(Vec3::new(num()?, num()?, num()?));
        }
    }
    if vertices.len() % 3 != 0 {
        bail!("ASCII STL has an incomplete facet");
    }
    Ok(vertices.as_chunks::<3>().0.to_vec())
}

// ---- OBJ ----

fn load_obj(text: &str, progress: Progress) -> Result<Vec<[Vec3; 3]>> {
    let total = text.len().max(1) as f64;
    let mut consumed = 0usize;
    let mut vertices: Vec<Vec3> = Vec::new();
    let mut triangles = Vec::new();
    for (ln, line) in text.lines().enumerate() {
        consumed += line.len() + 1;
        if ln.is_multiple_of(4096) {
            progress(consumed as f64 / total);
        }
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let mut num = || -> Result<f32> {
                    it.next()
                        .context("truncated vertex line in OBJ")?
                        .parse()
                        .context("invalid number in OBJ")
                };
                vertices.push(Vec3::new(num()?, num()?, num()?));
            }
            Some("f") => {
                let idx = it
                    .map(|tok| obj_index(tok, vertices.len()))
                    .collect::<Result<Vec<_>>>()?;
                // Fan-triangulate polygons.
                for i in 1..idx.len().saturating_sub(1) {
                    triangles.push([vertices[idx[0]], vertices[idx[i]], vertices[idx[i + 1]]]);
                }
            }
            _ => {}
        }
    }
    Ok(triangles)
}

fn obj_index(token: &str, vertex_count: usize) -> Result<usize> {
    let first = token.split('/').next().unwrap_or_default();
    let i: i64 = first.parse().context("invalid face index in OBJ")?;
    let idx = if i < 0 { vertex_count as i64 + i } else { i - 1 };
    if idx < 0 || idx as usize >= vertex_count {
        bail!("face index out of range in OBJ");
    }
    Ok(idx as usize)
}

// ---- STEP ----

fn load_step(text: &str, progress: Progress) -> Result<Vec<[Vec3; 3]>> {
    use truck_meshalgo::prelude::*;
    use truck_stepio::r#in::{ruststep, Table};

    progress(0.02);
    let exchange =
        ruststep::parser::parse(text).map_err(|e| anyhow!("STEP parse error: {e}"))?;
    let table = Table::from_data_section(
        exchange.data.first().context("STEP file has no data section")?,
    );
    progress(0.15);
    if table.shell.is_empty() {
        bail!("STEP file contains no shells");
    }

    let shell_count = table.shell.len() as f64;
    let mut triangles: Vec<[Vec3; 3]> = Vec::new();
    for (i, shell) in table.shell.values().enumerate() {
        // Shells with geometry truck doesn't support are skipped rather than fatal.
        let Ok(shell) = table.to_compressed_shell(shell) else {
            continue;
        };
        if shell.vertices.is_empty() {
            continue;
        }
        // Mesh tolerance scaled to the model size.
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for p in &shell.vertices {
            for (k, c) in [p.x, p.y, p.z].into_iter().enumerate() {
                min[k] = min[k].min(c);
                max[k] = max[k].max(c);
            }
        }
        let diameter = (0..3).map(|k| (max[k] - min[k]).powi(2)).sum::<f64>().sqrt();
        let tol = (diameter * 1e-3).max(1e-4);
        let poly = shell.robust_triangulation(tol).to_polygon();
        let positions = poly.positions();
        let vec3 = |i: usize| {
            let p = positions[i];
            Vec3::new(p.x as f32, p.y as f32, p.z as f32)
        };
        for tri in poly.faces().triangle_iter() {
            triangles.push([vec3(tri[0].pos), vec3(tri[1].pos), vec3(tri[2].pos)]);
        }
        progress(0.15 + 0.85 * (i as f64 + 1.0) / shell_count);
    }
    if triangles.is_empty() {
        bail!("no triangulatable geometry found in STEP file");
    }
    Ok(triangles)
}

// ---- GLB / FBX ----

/// Ceiling on the triangles a GLB or FBX scene may expand to. Both formats
/// instance one mesh from many nodes, so like a 3MF build (see
/// `MAX_3MF_EXPANSION`) a small file can multiply into far more geometry.
const MAX_SCENE_TRIANGLES: usize = 32_000_000;

fn charge_scene(budget: &mut usize, cost: usize, format: &str) -> Result<()> {
    if cost > *budget {
        bail!("{format} scene expands to more than {} million triangles", MAX_SCENE_TRIANGLES / 1_000_000);
    }
    *budget -= cost;
    Ok(())
}

/// Binary glTF. Only the embedded BIN chunk is read: a `.glb` that points at
/// external buffers can't be followed inside the thumbnailer sandbox anyway.
fn load_glb(bytes: &[u8], progress: Progress) -> Result<Vec<[Vec3; 3]>> {
    use gltf::mesh::Mode;

    let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| anyhow!("GLB parse error: {e}"))?;
    progress(0.1);
    let blob = gltf.blob.as_deref();
    let buffer = |b: gltf::Buffer| match b.source() {
        gltf::buffer::Source::Bin => blob,
        gltf::buffer::Source::Uri(_) => None,
    };

    // glTF is Y-up; wdill views Z-up, like the print formats.
    let y_up_to_z_up = Affine3A::from_cols(Vec3A::X, Vec3A::Z, -Vec3A::Y, Vec3A::ZERO);
    let roots: Vec<gltf::Node> = match gltf.default_scene().or_else(|| gltf.scenes().next()) {
        Some(scene) => scene.nodes().collect(),
        None => gltf.nodes().collect(),
    };
    let total = roots.len().max(1) as f64;
    let mut budget = MAX_SCENE_TRIANGLES;
    let mut out = Vec::new();
    // Nodes form a tree by spec, but nothing stops a file from looping, so
    // the depth is capped rather than trusted.
    let mut stack: Vec<(gltf::Node, Affine3A, usize)> =
        roots.into_iter().map(|n| (n, y_up_to_z_up, 0)).collect();
    let mut done = 0usize;
    while let Some((node, parent, depth)) = stack.pop() {
        if depth == 0 {
            done += 1;
            progress(0.1 + 0.9 * done as f64 / total);
        }
        charge_scene(&mut budget, 1, "GLB")?;
        let t = parent * Affine3A::from_mat4(glam::Mat4::from_cols_array_2d(&node.transform().matrix()));
        if let Some(mesh) = node.mesh() {
            for prim in mesh.primitives() {
                let reader = prim.reader(buffer);
                // Draco/meshopt-compressed primitives have no plain positions.
                let Some(positions) = reader.read_positions() else { continue };
                let positions: Vec<Vec3> =
                    positions.map(|p| t.transform_point3(Vec3::from_array(p))).collect();
                let indices: Vec<usize> = match reader.read_indices() {
                    Some(ix) => ix.into_u32().map(|i| i as usize).collect(),
                    None => (0..positions.len()).collect(),
                };
                let tri_of = |a: usize, b: usize, c: usize| -> Option<[Vec3; 3]> {
                    Some([*positions.get(a)?, *positions.get(b)?, *positions.get(c)?])
                };
                let before = out.len();
                match prim.mode() {
                    Mode::Triangles => out.extend(
                        indices.as_chunks::<3>().0.iter().filter_map(|&[a, b, c]| tri_of(a, b, c)),
                    ),
                    Mode::TriangleStrip => out.extend(
                        indices.windows(3).filter_map(|w| tri_of(w[0], w[1], w[2])),
                    ),
                    Mode::TriangleFan if !indices.is_empty() => out.extend(
                        indices[1..].windows(2).filter_map(|w| tri_of(indices[0], w[0], w[1])),
                    ),
                    _ => {} // points and lines have no surface
                }
                charge_scene(&mut budget, out.len() - before, "GLB")?;
            }
        }
        if depth < 64 {
            stack.extend(node.children().map(|c| (c, t, depth + 1)));
        }
    }
    if out.is_empty() && gltf.extensions_required().any(|e| e.contains("compression")) {
        bail!("compressed GLB meshes are not supported");
    }
    Ok(out)
}

/// FBX, binary or ASCII, any version ufbx reads (6100 and up).
fn load_fbx(bytes: &[u8], progress: Progress) -> Result<Vec<[Vec3; 3]>> {
    let on_progress = |p: &ufbx::Progress| {
        if p.bytes_total > 0 {
            progress(0.8 * p.bytes_read as f64 / p.bytes_total as f64);
        }
        ufbx::ProgressResult::Continue
    };
    let opts = ufbx::LoadOpts {
        target_axes: ufbx::CoordinateAxes::right_handed_z_up(),
        progress_cb: ufbx::ProgressCb::Ref(&on_progress),
        ignore_animation: true,
        ignore_embedded: true,
        ..Default::default()
    };
    let scene = ufbx::load_memory(bytes, opts)
        .map_err(|e| anyhow!("FBX parse error: {}", e.description))?;

    let mut budget = MAX_SCENE_TRIANGLES;
    let mut out = Vec::new();
    let mut indices = Vec::new();
    let total = scene.nodes.len().max(1) as f64;
    for (n, node) in scene.nodes.iter().enumerate() {
        progress(0.8 + 0.2 * n as f64 / total);
        let Some(mesh) = &node.mesh else { continue };
        charge_scene(&mut budget, 1 + mesh.num_triangles, "FBX")?;
        let m = &node.geometry_to_world;
        let t = Affine3A::from_cols(
            Vec3A::new(m.m00 as f32, m.m10 as f32, m.m20 as f32),
            Vec3A::new(m.m01 as f32, m.m11 as f32, m.m21 as f32),
            Vec3A::new(m.m02 as f32, m.m12 as f32, m.m22 as f32),
            Vec3A::new(m.m03 as f32, m.m13 as f32, m.m23 as f32),
        );
        let pos = &mesh.vertex_position;
        let at = |ix: u32| {
            let v = pos.values[pos.indices[ix as usize] as usize];
            t.transform_point3(Vec3::new(v.x as f32, v.y as f32, v.z as f32))
        };
        for &face in mesh.faces.iter() {
            let count = ufbx::triangulate_face_vec(&mut indices, mesh, face) as usize;
            out.extend(indices[..count * 3].as_chunks::<3>().0.iter().map(|c| c.map(at)));
        }
    }
    Ok(out)
}

// ---- 3MF ----

/// Ceiling on the decompressed `.model` XML of a 3MF (see `read_model_part`),
/// summed over every part the archive pulls in.
const MAX_MODEL_XML: u64 = 256 * 1024 * 1024;

/// Ceiling on how many `.model` parts one archive may chain together.
const MAX_MODEL_PARTS: usize = 4096;

/// Ceiling on what a 3MF build may expand to, counted as triangles emitted
/// plus object references followed. Components instance other objects, so a
/// few KB of nested fan-out (ten copies of ten copies of …) would otherwise
/// multiply into billions of triangles — the XML cap can't see that coming.
const MAX_3MF_EXPANSION: usize = 32_000_000;

/// A reference to an object. `part` is `None` for the part the reference itself
/// lives in, and `Some` for the production extension's cross-part `p:path`.
struct Ref3mf {
    part: Option<String>,
    id: String,
    transform: Affine3A,
}

#[derive(Default)]
struct Object3mf {
    triangles: Vec<[Vec3; 3]>,
    components: Vec<Ref3mf>,
}

#[derive(Default)]
struct Part3mf {
    objects: HashMap<String, Object3mf>,
    items: Vec<Ref3mf>,
}

fn load_3mf<R: Read + Seek>(reader: R, progress: Progress) -> Result<Vec<[Vec3; 3]>> {
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(reader))
        .context("not a valid 3MF (zip) archive")?;

    // Entry names and the total size of the `.model` parts, taken once from the
    // central directory: the walk below holds `&mut archive`, and progress has
    // to be spread over parts whose count isn't known until they're followed.
    let mut names = Vec::with_capacity(archive.len());
    let mut model_bytes = 0u64;
    for i in 0..archive.len() {
        let f = archive.by_index_raw(i).context("damaged 3MF (zip) archive")?;
        let name = f.name().to_string();
        if is_model_part(&name) {
            model_bytes += f.size();
        }
        names.push(name);
    }
    let total = model_bytes.max(1) as f64;

    // Bambu Studio, Orca and PrusaSlicer all write the 3MF production
    // extension: the root part carries only the build and components pointing,
    // by `p:path`, at geometry that lives in *other* `.model` parts. Reading
    // one part — and picking it as "first entry whose name ends in .model" —
    // yields an empty mesh for every such file, so follow the references.
    let root = root_model_part(&mut archive, &names)?;
    let mut parts: HashMap<String, Part3mf> = HashMap::new();
    let mut queue = vec![root.clone()];
    let mut budget = MAX_MODEL_XML;
    let mut done = 0u64;
    while let Some(name) = queue.pop() {
        if parts.contains_key(&name) {
            continue;
        }
        // A `p:path` naming a part the archive doesn't hold, or one that is
        // damaged, resolves to no geometry rather than failing the load: one
        // bad part in a multi-object plate shouldn't cost the intact objects.
        // Running out of XML budget is fatal wherever it happens, though.
        let parsed = read_model_part(&mut archive, &name, &mut budget).and_then(|xml| {
            let (base, span) = (done as f64, xml.len() as f64);
            done += xml.len() as u64;
            parse_3mf_part(&xml, &|f| progress((base + f * span) / total))
        });
        let mut part = match parsed {
            Ok(part) => part,
            Err(e) if name == root || e.is::<ModelTooLarge>() => return Err(e),
            Err(_) => Part3mf::default(),
        };
        for r in part.items.iter_mut().chain(part.objects.values_mut().flat_map(|o| &mut o.components))
        {
            if let Some(p) = r.part.take() {
                r.part = Some(resolve_part(&names, &p).unwrap_or(p));
            }
        }
        for r in part.items.iter().chain(part.objects.values().flat_map(|o| &o.components)) {
            if let Some(p) = &r.part
                && !parts.contains_key(p)
                && parts.len() + queue.len() < MAX_MODEL_PARTS
            {
                queue.push(p.clone());
            }
        }
        parts.insert(name, part);
    }

    let mut out = collect_3mf_build(&parts, &root, MAX_3MF_EXPANSION)?;
    if out.is_empty() {
        // No usable build section, or one that resolved to nothing: fall back
        // to showing every mesh the archive holds, untransformed — including
        // `.model` parts that nothing referenced.
        for name in &names {
            if !is_model_part(name) || parts.contains_key(name) {
                continue;
            }
            match read_model_part(&mut archive, name, &mut budget)
                .and_then(|xml| parse_3mf_part(&xml, &|_| {}))
            {
                Ok(part) => {
                    parts.insert(name.clone(), part);
                }
                Err(e) if e.is::<ModelTooLarge>() => return Err(e),
                Err(_) => {}
            }
        }
        for part in parts.values() {
            for obj in part.objects.values() {
                out.extend_from_slice(&obj.triangles);
            }
        }
    }
    Ok(out)
}

fn is_model_part(name: &str) -> bool {
    Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("model"))
}

fn read_model_part<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    budget: &mut u64,
) -> Result<String> {
    // A .3mf is a zip, so the file's own size says nothing about what it
    // expands to: a few KB of deflate can hold gigabytes of repetitive XML.
    // Cap the decompressed total — this runs unattended under GNOME's
    // thumbnailer, where exhausting memory is the caller's problem, not ours.
    let mut xml = String::new();
    archive
        .by_name(name)
        .with_context(|| format!("3MF part \"{name}\" is missing from the archive"))?
        .take(*budget + 1)
        .read_to_string(&mut xml)?;
    if xml.len() as u64 > *budget {
        return Err(ModelTooLarge.into());
    }
    *budget -= xml.len() as u64;
    Ok(xml)
}

/// The `MAX_MODEL_XML` budget ran out — kept apart from other part errors
/// because, unlike a damaged part, it must fail the whole load.
#[derive(Debug)]
struct ModelTooLarge;

impl std::fmt::Display for ModelTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "3MF model data exceeds {} MiB", MAX_MODEL_XML / (1024 * 1024))
    }
}

impl std::error::Error for ModelTooLarge {}

/// The archive's start part, which `_rels/.rels` names. Guessing it instead
/// ("first entry ending in .model") is a coin toss in a production-extension
/// archive, where the geometry parts outnumber the root and the zip's own
/// ordering carries no meaning.
fn root_model_part<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    names: &[String],
) -> Result<String> {
    const MODEL_REL: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";

    let mut rels = String::new();
    if let Ok(f) = archive.by_name("_rels/.rels") {
        f.take(4 * 1024 * 1024).read_to_string(&mut rels).ok();
    }
    let mut reader = quick_xml::Reader::from_str(&rels);
    while let Ok(ev) = reader.read_event() {
        match ev {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"Relationship" => {
                if attr(&e, "Type")?.as_deref() == Some(MODEL_REL)
                    && let Some(t) = attr(&e, "Target")?
                    && let Some(name) = resolve_part(names, &t)
                {
                    return Ok(name);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    // No usable `.rels`: try the conventional name, then the shallowest
    // `.model` entry, so at least the choice is deterministic.
    resolve_part(names, "/3D/3dmodel.model")
        .or_else(|| {
            names
                .iter()
                .filter(|n| is_model_part(n))
                .min_by(|a, b| depth_key(a).cmp(&depth_key(b)))
                .cloned()
        })
        .context("no 3D model part found in 3MF archive")
}

fn depth_key(name: &str) -> (usize, usize, &str) {
    (name.matches('/').count(), name.len(), name)
}

/// Map an OPC part name (`/3D/Objects/thing.model`) onto the zip entry that
/// holds it. Part names are URI references, so a producer may percent-encode
/// what the entry spells literally; a few write the path in another case.
fn resolve_part(names: &[String], target: &str) -> Option<String> {
    let want = target.trim_start_matches('/');
    let mut candidates = vec![want.to_string()];
    if let Some(decoded) = percent_decode(want) {
        candidates.push(decoded);
    }
    let exact = names.iter().find(|n| candidates.iter().any(|c| *n == c));
    exact
        .or_else(|| {
            names.iter().find(|n| candidates.iter().any(|c| n.eq_ignore_ascii_case(c)))
        })
        .cloned()
}

fn percent_decode(s: &str) -> Option<String> {
    if !s.contains('%') {
        return None;
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(hex) = std::str::from_utf8(&b[i + 1..i + 3])
            && let Ok(v) = u8::from_str_radix(hex, 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

fn attr(e: &BytesStart, name: &str) -> Result<Option<String>> {
    for a in e.attributes() {
        let a = a?;
        if a.key.local_name().as_ref() == name.as_bytes() {
            return Ok(Some(a.normalized_value(quick_xml::XmlVersion::Implicit1_0)?.into_owned()));
        }
    }
    Ok(None)
}

fn fattr(e: &BytesStart, name: &str) -> Result<f32> {
    attr(e, name)?
        .with_context(|| format!("3MF element is missing attribute \"{name}\""))?
        .parse()
        .with_context(|| format!("invalid number in 3MF attribute \"{name}\""))
}

/// The `objectid` + optional `p:path` + optional `transform` an `<item>` or
/// `<component>` carries.
fn ref_of(e: &BytesStart) -> Result<Option<Ref3mf>> {
    let Some(id) = attr(e, "objectid")? else {
        return Ok(None);
    };
    Ok(Some(Ref3mf {
        part: attr(e, "path")?.filter(|p| !p.is_empty()),
        id,
        transform: match attr(e, "transform")? {
            Some(s) => parse_3mf_transform(&s)?,
            None => Affine3A::IDENTITY,
        },
    }))
}

fn parse_3mf_part(xml: &str, progress: Progress) -> Result<Part3mf> {
    let total = xml.len().max(1) as f64;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut objects: HashMap<String, Object3mf> = HashMap::new();
    let mut current_id: Option<String> = None;
    let mut vertices: Vec<Vec3> = Vec::new();
    let mut items: Vec<Ref3mf> = Vec::new();
    let mut events = 0usize;
    // Other extensions reuse the element names — the slice extension's
    // `<s:vertex>` is 2D, with no `z` — so geometry is only read in `<mesh>`.
    let mut in_mesh = false;

    loop {
        events += 1;
        if events.is_multiple_of(8192) {
            progress(reader.buffer_position() as f64 / total);
        }
        match reader.read_event()? {
            Event::Start(e) if e.local_name().as_ref() == b"mesh" => in_mesh = true,
            Event::End(e) if e.local_name().as_ref() == b"mesh" => in_mesh = false,
            Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                b"object" => {
                    let id = attr(&e, "id")?.unwrap_or_default();
                    objects.entry(id.clone()).or_default();
                    current_id = Some(id);
                    vertices.clear();
                }
                b"vertex" if in_mesh => {
                    vertices.push(Vec3::new(fattr(&e, "x")?, fattr(&e, "y")?, fattr(&e, "z")?));
                }
                b"triangle" if in_mesh => {
                    let idx = |name| -> Result<usize> {
                        let i: usize = attr(&e, name)?
                            .with_context(|| format!("3MF triangle is missing attribute \"{name}\""))?
                            .parse()
                            .context("invalid triangle index in 3MF")?;
                        if i >= vertices.len() {
                            bail!("triangle index out of range in 3MF");
                        }
                        Ok(i)
                    };
                    let tri = [vertices[idx("v1")?], vertices[idx("v2")?], vertices[idx("v3")?]];
                    if let Some(id) = &current_id {
                        objects.get_mut(id).unwrap().triangles.push(tri);
                    }
                }
                b"component" => {
                    if let (Some(cur), Some(r)) = (&current_id, ref_of(&e)?) {
                        objects.get_mut(cur).unwrap().components.push(r);
                    }
                }
                b"item" => {
                    if let Some(r) = ref_of(&e)? {
                        items.push(r);
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(Part3mf { objects, items })
}

/// Walk the root part's build items, pulling in objects from whichever part
/// each reference names.
/// `limit` bounds the expansion (see `MAX_3MF_EXPANSION`).
fn collect_3mf_build(
    parts: &HashMap<String, Part3mf>,
    root: &str,
    limit: usize,
) -> Result<Vec<[Vec3; 3]>> {
    let mut out = Vec::new();
    let mut budget = limit;
    let mut stack = Vec::new();
    if let Some(part) = parts.get(root) {
        for item in &part.items {
            collect_3mf_ref(parts, root, item, Affine3A::IDENTITY, &mut stack, &mut budget, &mut out)?;
        }
    }
    Ok(out)
}

fn collect_3mf_ref<'a>(
    parts: &'a HashMap<String, Part3mf>,
    from: &'a str,
    r: &'a Ref3mf,
    t: Affine3A,
    stack: &mut Vec<(&'a str, &'a str)>,
    budget: &mut usize,
    out: &mut Vec<[Vec3; 3]>,
) -> Result<()> {
    let part = r.part.as_deref().unwrap_or(from);
    let key = (part, r.id.as_str());
    // A component cycle is invalid 3MF; don't go round it again.
    if stack.len() > 8 || stack.contains(&key) {
        return Ok(());
    }
    let Some(obj) = parts.get(part).and_then(|p| p.objects.get(&r.id)) else { return Ok(()) };
    // Charge the reference itself too, so a fan-out of geometry-less
    // objects can't spin for ever without emitting a triangle.
    let cost = 1 + obj.triangles.len();
    if cost > *budget {
        bail!("3MF build expands to more than {} million triangles", MAX_3MF_EXPANSION / 1_000_000);
    }
    *budget -= cost;
    let t = t * r.transform;
    out.extend(obj.triangles.iter().map(|tri| tri.map(|v| t.transform_point3(v))));
    stack.push(key);
    for c in &obj.components {
        collect_3mf_ref(parts, part, c, t, stack, budget, out)?;
    }
    stack.pop();
    Ok(())
}

fn parse_3mf_transform(s: &str) -> Result<Affine3A> {
    let n = s
        .split_whitespace()
        .map(|t| t.parse::<f32>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("invalid number in 3MF transform")?;
    if n.len() != 12 {
        bail!("3MF transform must have 12 values");
    }
    // 3MF matrices are row-major 4x3; consecutive triples are the basis columns.
    Ok(Affine3A::from_cols(
        Vec3A::new(n[0], n[1], n[2]),
        Vec3A::new(n[3], n[4], n[5]),
        Vec3A::new(n[6], n[7], n[8]),
        Vec3A::new(n[9], n[10], n[11]),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    const NOP: Progress = &|_| {};

    /// A single `.model` part standing alone, as it is before the production
    /// extension splits geometry across several of them.
    fn parse_3mf_model(xml: &str) -> Result<Vec<[Vec3; 3]>> {
        let mut parts = HashMap::new();
        parts.insert(String::new(), parse_3mf_part(xml, NOP)?);
        collect_3mf_build(&parts, "", MAX_3MF_EXPANSION)
    }

    fn zip_of(entries: &[(&str, &str)]) -> Cursor<Vec<u8>> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            for (name, body) in entries {
                zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
                zip.write_all(body.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        cursor.set_position(0);
        cursor
    }

    fn binary_stl(tris: &[[Vec3; 3]]) -> Vec<u8> {
        let mut b = vec![0u8; 80];
        b.extend_from_slice(&(tris.len() as u32).to_le_bytes());
        for tri in tris {
            b.extend_from_slice(&[0u8; 12]); // normal
            for v in tri {
                for c in [v.x, v.y, v.z] {
                    b.extend_from_slice(&c.to_le_bytes());
                }
            }
            b.extend_from_slice(&[0u8; 2]); // attribute byte count
        }
        b
    }

    #[test]
    fn parses_binary_stl() {
        let tri = [Vec3::ZERO, Vec3::X, Vec3::Y];
        let tris = load_stl(&binary_stl(&[tri]), NOP).unwrap();
        assert_eq!(tris, vec![tri]);
    }

    #[test]
    fn parses_ascii_stl() {
        let text = "solid demo\n facet normal 0 0 1\n  outer loop\n   vertex 0 0 0\n   vertex 1 0 0\n   vertex 0 1 0\n  endloop\n endfacet\nendsolid demo\n";
        let tris = load_stl(text.as_bytes(), NOP).unwrap();
        assert_eq!(tris, vec![[Vec3::ZERO, Vec3::X, Vec3::Y]]);
    }

    /// A Latin-1 solid name, and a UTF-8 one split by the 1 KiB sniff window,
    /// both used to send an ASCII file down the binary path.
    #[test]
    fn parses_ascii_stl_that_is_not_utf8() {
        let body = b" facet normal 0 0 1\n  outer loop\n   vertex 0 0 0\n   vertex 1 0 0\n   vertex 0 1 0\n  endloop\n endfacet\nendsolid\n";
        let mut latin1 = b"solid \xDCberteil\n".to_vec();
        latin1.extend_from_slice(body);
        assert_eq!(load_stl(&latin1, NOP).unwrap().len(), 1);

        // Two solids; the second one's name puts an 'Ü' across byte 1024.
        let mut split = b"solid a\n".to_vec();
        split.extend_from_slice(body);
        let pad = 1023 - split.len() - "solid ".len();
        split.extend_from_slice(format!("solid {}Ü\n", "x".repeat(pad)).as_bytes());
        split.extend_from_slice(body);
        assert!(std::str::from_utf8(&split[..1024]).is_err());
        assert_eq!(load_stl(&split, NOP).unwrap().len(), 2);
    }

    /// A binary header may start with "solid"; the exact binary length wins.
    #[test]
    fn binary_stl_with_solid_header_stays_binary() {
        let tri = [Vec3::ZERO, Vec3::X, Vec3::Y];
        let mut b = binary_stl(&[tri]);
        b[..14].copy_from_slice(b"solid facet ab");
        assert_eq!(load_stl(&b, NOP).unwrap(), vec![tri]);
    }

    #[test]
    fn parses_obj_with_quads_and_negative_indices() {
        let text = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1/1/1 2/2/2 3/3/3 -1\n";
        let tris = load_obj(text, NOP).unwrap();
        assert_eq!(tris.len(), 2);
        assert_eq!(tris[1][2], Vec3::new(0.0, 1.0, 0.0));
    }

    #[test]
    fn parses_step_cube_roundtrip() {
        use truck_modeling::{builder, Point3, Vector3};
        use truck_stepio::out;

        let v = builder::vertex(Point3::new(0.0, 0.0, 0.0));
        let e = builder::tsweep(&v, Vector3::unit_x());
        let f = builder::tsweep(&e, Vector3::unit_y());
        let cube = builder::tsweep(&f, Vector3::unit_z());
        let step = out::CompleteStepDisplay::new(
            out::StepModel::from(&cube.compress()),
            Default::default(),
        )
        .to_string();

        let mesh = Mesh::from_triangles(load_step(&step, NOP).unwrap()).unwrap();
        assert!(mesh.triangles.len() >= 12);
        assert!((mesh.center - Vec3::splat(0.5)).length() < 1e-3);
        assert!((mesh.radius - 0.75f32.sqrt()).abs() < 1e-3);

        // A Latin-1 byte in a header string, read the way `load` reads it.
        let (head, tail) = step.split_once("FILE_NAME('").unwrap();
        let mut latin1 = format!("{head}FILE_NAME('").into_bytes();
        latin1.push(0xDC);
        latin1.extend_from_slice(tail.as_bytes());
        assert!(std::str::from_utf8(&latin1).is_err());
        assert!(load_step(&String::from_utf8_lossy(&latin1), NOP).unwrap().len() >= 12);
    }

    #[test]
    fn parses_3mf_model_with_build_transform() {
        let xml = r#"<?xml version="1.0"?>
            <model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
             <resources>
              <object id="1" type="model">
               <mesh>
                <vertices>
                 <vertex x="0" y="0" z="0"/>
                 <vertex x="1" y="0" z="0"/>
                 <vertex x="0" y="1" z="0"/>
                </vertices>
                <triangles><triangle v1="0" v2="1" v3="2"/></triangles>
               </mesh>
              </object>
             </resources>
             <build><item objectid="1" transform="1 0 0 0 1 0 0 0 1 10 20 30"/></build>
            </model>"#;
        let tris = parse_3mf_model(xml).unwrap();
        assert_eq!(tris, vec![[
            Vec3::new(10.0, 20.0, 30.0),
            Vec3::new(11.0, 20.0, 30.0),
            Vec3::new(10.0, 21.0, 30.0),
        ]]);
    }

    const TRIANGLE_OBJECT: &str = r#"<model><resources><object id="1"><mesh>
        <vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="0" z="1"/></vertices>
        <triangles><triangle v1="0" v2="1" v3="2"/></triangles>
        </mesh></object></resources>"#;

    #[test]
    fn parses_3mf_archive() {
        let xml = format!("{TRIANGLE_OBJECT}<build><item objectid=\"1\"/></build></model>");
        let tris = load_3mf(zip_of(&[("3D/3dmodel.model", &xml)]), NOP).unwrap();
        assert_eq!(tris.len(), 1);
        assert_eq!(tris[0][2], Vec3::new(0.0, 0.0, 1.0));
    }

    /// The production extension (Bambu Studio, Orca, PrusaSlicer): the root
    /// part holds no geometry at all, only a component pointing at another
    /// part by `p:path` — which is spelled with a space the entry name has
    /// literally and `p:path` percent-encodes.
    #[test]
    fn parses_3mf_production_extension() {
        let rels = r#"<Relationships><Relationship Id="rel-1" Target="/3D/3dmodel.model"
            Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#;
        let root = r#"<model xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06">
            <resources><object id="2" type="model"><components>
              <component p:path="/3D/Objects/a%20part.model" objectid="1"/>
            </components></object></resources>
            <build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 10 20 30"/></build></model>"#;
        let object = format!("{TRIANGLE_OBJECT}</model>");
        let tris = load_3mf(
            zip_of(&[
                ("_rels/.rels", rels),
                // Deliberately ahead of the root part in the archive, since
                // picking the first `.model` entry used to decide the load.
                ("3D/Objects/a part.model", &object),
                ("3D/3dmodel.model", root),
            ]),
            NOP,
        )
        .unwrap();
        assert_eq!(tris, vec![[
            Vec3::new(10.0, 20.0, 30.0),
            Vec3::new(11.0, 20.0, 30.0),
            Vec3::new(10.0, 20.0, 31.0),
        ]]);
    }

    /// Object ids are unique only within a part, so two parts may both hold an
    /// object 1 — the build must not conflate them.
    #[test]
    fn keeps_3mf_object_ids_per_part() {
        let root = r#"<model><resources>
            <object id="1"><mesh>
              <vertices><vertex x="0" y="0" z="0"/><vertex x="5" y="0" z="0"/><vertex x="0" y="5" z="0"/></vertices>
              <triangles><triangle v1="0" v2="1" v3="2"/></triangles>
            </mesh></object>
            </resources><build>
              <item objectid="1"/>
              <item objectid="1" path="/3D/Objects/other.model"/>
            </build></model>"#;
        let other = format!("{TRIANGLE_OBJECT}</model>");
        let tris = load_3mf(
            zip_of(&[("3D/3dmodel.model", root), ("3D/Objects/other.model", &other)]),
            NOP,
        )
        .unwrap();
        assert_eq!(tris.len(), 2);
        assert_eq!(tris[0][1], Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(tris[1][1], Vec3::new(1.0, 0.0, 0.0));
    }

    /// A `p:path` the archive doesn't hold costs that object, not the file.
    #[test]
    fn survives_dangling_3mf_part_reference() {
        let root = format!(
            "{TRIANGLE_OBJECT}<build><item objectid=\"1\"/>\
             <item objectid=\"1\" path=\"/3D/Objects/gone.model\"/></build></model>"
        );
        let tris = load_3mf(zip_of(&[("3D/3dmodel.model", &root)]), NOP).unwrap();
        assert_eq!(tris.len(), 1);
    }

    /// Ten copies of ten copies of … one triangle, eight levels deep: 10⁸
    /// triangles from a couple of KB. It must fail fast, not expand.
    #[test]
    fn rejects_3mf_component_fan_out_bomb() {
        let mut xml = TRIANGLE_OBJECT.replace("</resources>", "");
        for id in 2..=9 {
            xml += &format!("<object id=\"{id}\"><components>");
            xml += &format!("<component objectid=\"{}\"/>", id - 1).repeat(10);
            xml += "</components></object>";
        }
        xml += "</resources><build><item objectid=\"9\"/></build></model>";
        let mut parts = HashMap::new();
        parts.insert(String::new(), parse_3mf_part(&xml, NOP).unwrap());
        assert!(collect_3mf_build(&parts, "", 100_000).is_err());
    }

    /// The slice extension's 2D `<s:vertex>` must not be read as mesh geometry.
    #[test]
    fn ignores_3mf_slice_vertices() {
        let xml = TRIANGLE_OBJECT.replace(
            "</resources>",
            r#"<s:slicestack id="9"><s:slice ztop="1"><s:vertices>
                <s:vertex x="0" y="0"/></s:vertices></s:slice></s:slicestack></resources>"#,
        ) + "<build><item objectid=\"1\"/></build></model>";
        assert_eq!(parse_3mf_model(&xml).unwrap().len(), 1);
    }

    #[test]
    fn rejects_non_integer_3mf_triangle_index() {
        for bad in ["-1", "1.5"] {
            let xml = TRIANGLE_OBJECT.replace("v1=\"0\"", &format!("v1=\"{bad}\""));
            assert!(parse_3mf_part(&xml, NOP).is_err(), "accepted {bad}");
        }
    }

    /// A malformed part other than the root costs its objects, not the file —
    /// the same as a missing one.
    #[test]
    fn survives_damaged_3mf_part() {
        let root = format!(
            "{TRIANGLE_OBJECT}<build><item objectid=\"1\"/>\
             <item objectid=\"1\" path=\"/3D/Objects/bad.model\"/></build></model>"
        );
        let bad = r#"<model><resources><object id="1"><mesh><vertices><vertex x="oops"/>"#;
        let tris =
            load_3mf(zip_of(&[("3D/3dmodel.model", &root), ("3D/Objects/bad.model", bad)]), NOP)
                .unwrap();
        assert_eq!(tris.len(), 1);
    }

    /// A component cycle is invalid, but must not repeat the geometry.
    #[test]
    fn ignores_3mf_component_cycle() {
        let xml = TRIANGLE_OBJECT.replace(
            "</mesh></object>",
            "</mesh><components><component objectid=\"1\"/></components></object>",
        ) + "<build><item objectid=\"1\"/></build></model>";
        assert_eq!(parse_3mf_model(&xml).unwrap().len(), 1);
    }

    /// A GLB holding `verts` as one POSITION accessor, drawn with `mode`
    /// from a child node under a parent translated by `parent_t`.
    fn glb_of(verts: &[[f32; 3]], mode: u32, parent_t: [f32; 3]) -> Vec<u8> {
        let bin: Vec<u8> = verts.iter().flatten().flat_map(|c| c.to_le_bytes()).collect();
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[{{"translation":{parent_t:?},"children":[1]}},{{"mesh":0}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"mode":{mode}}}]}}],
            "accessors":[{{"bufferView":0,"componentType":5126,"count":{},"type":"VEC3",
                "min":[-9,-9,-9],"max":[9,9,9]}}],
            "bufferViews":[{{"buffer":0,"byteLength":{}}}],
            "buffers":[{{"byteLength":{}}}]}}"#,
            verts.len(),
            bin.len(),
            bin.len()
        );
        let mut json = json.into_bytes();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        let mut b = Vec::new();
        b.extend_from_slice(b"glTF");
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        b.extend_from_slice(&(json.len() as u32).to_le_bytes());
        b.extend_from_slice(b"JSON");
        b.extend_from_slice(&json);
        b.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        b.extend_from_slice(b"BIN\0");
        b.extend_from_slice(&bin);
        b
    }

    #[test]
    fn parses_glb_with_node_transform_as_z_up() {
        let tri = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let tris = load_glb(&glb_of(&tri, 4, [0.0, 5.0, 0.0]), NOP).unwrap();
        assert_eq!(tris.len(), 1);
        // glTF +Y (up) lands on wdill's +Z, glTF +Z on -Y.
        assert_eq!(tris[0], [Vec3::new(0.0, 0.0, 5.0), Vec3::new(1.0, 0.0, 5.0), Vec3::new(0.0, 0.0, 6.0)]);
    }

    #[test]
    fn parses_glb_triangle_fan() {
        let quad = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        assert_eq!(load_glb(&glb_of(&quad, 6, [0.0; 3]), NOP).unwrap().len(), 2);
    }

    #[test]
    fn rejects_garbage_glb() {
        assert!(load_glb(b"glTF not really", NOP).is_err());
    }

    const ASCII_FBX: &str = r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {
	FBXHeaderVersion: 1003
	FBXVersion: 7400
}
GlobalSettings:  {
	Version: 1000
	Properties70:  {
		P: "UpAxis", "int", "Integer", "",1
		P: "UpAxisSign", "int", "Integer", "",1
		P: "FrontAxis", "int", "Integer", "",2
		P: "FrontAxisSign", "int", "Integer", "",1
		P: "CoordAxis", "int", "Integer", "",0
		P: "CoordAxisSign", "int", "Integer", "",1
		P: "UnitScaleFactor", "double", "Number", "",1
	}
}
Objects:  {
	Geometry: 100, "Geometry::quad", "Mesh" {
		Vertices: *12 {
			a: 0,0,0,1,0,0,1,1,0,0,1,0
		}
		PolygonVertexIndex: *4 {
			a: 0,1,2,-4
		}
	}
	Model: 200, "Model::quad", "Mesh" {
		Version: 232
		Properties70:  {
			P: "Lcl Translation", "Lcl Translation", "", "A",0,5,0
		}
	}
}
Connections:  {
	C: "OO",100,200
	C: "OO",200,0
}
"#;

    #[test]
    fn parses_ascii_fbx_quad_as_z_up() {
        let tris = load_fbx(ASCII_FBX.as_bytes(), NOP).unwrap();
        assert_eq!(tris.len(), 2);
        // Y-up file: the +Y translation ends up on +Z.
        for v in tris.iter().flatten() {
            assert!((v.z - 5.0).abs() < 1e-4 || (v.z - 6.0).abs() < 1e-4, "{v}");
        }
    }

    #[test]
    fn rejects_garbage_fbx() {
        assert!(load_fbx(b"Kaydara FBX Binary  \0garbage", NOP).is_err());
    }

    #[test]
    fn rejects_non_finite_vertices() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let tri = [Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, bad, 0.0)];
            assert!(Mesh::from_triangles(vec![tri]).is_err(), "accepted {bad}");
        }
        // Finite but huge: the bounds themselves overflow to infinity.
        let huge = [
            Vec3::splat(-3.0e38),
            Vec3::splat(3.0e38),
            Vec3::new(3.0e38, -3.0e38, 0.0),
        ];
        assert!(Mesh::from_triangles(vec![huge]).is_err());
    }

    #[test]
    fn mesh_bounds() {
        let mesh = Mesh::from_triangles(vec![[Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0), Vec3::new(0.0, 2.0, 0.0)]]).unwrap();
        assert_eq!(mesh.center, Vec3::new(1.0, 1.0, 0.0));
        assert!((mesh.radius - 2.0f32.sqrt()).abs() < 1e-6);
    }
}
