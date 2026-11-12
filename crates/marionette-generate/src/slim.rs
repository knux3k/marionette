//! Slimming a generated GLB (decision 0048): fewer triangles and smaller textures, so a
//! model fits the budget its recipe sets.
//!
//! - **Triangles.** When a model is over `max_triangles`, every triangle primitive is
//!   simplified by the same ratio with meshoptimizer's edge-collapse simplifier (the `meshopt`
//!   bindings), which keeps UV and normal seams. A primitive the careful pass cannot
//!   bring near its share falls back to the grid-based "sloppy" pass. Unused vertices are then
//!   dropped and the rest reordered for the vertex cache.
//! - **Textures.** Images larger than `max_texture_px` on a side are halved until they fit, then
//!   re-encoded: JPEG (quality 85) when fully opaque, PNG otherwise.
//!
//! The whole binary chunk is rebuilt (accessors and buffer views are renumbered); materials,
//! textures, samplers and nodes are kept. Models with skins, animations, morph targets, sparse
//! accessors or more than one buffer are left as they are, with a note. Everything is integer
//! or single-threaded float work, so the same input gives the same bytes.

use crate::texture;
use gltf::mesh::Mode;
use serde_json::{Value, json};

/// The outcome: the new GLB (when anything changed) and notes for the provenance.
pub struct Slimmed {
    pub glb: Option<Vec<u8>>,
    pub notes: Vec<String>,
}

const JPEG_QUALITY: u8 = 85;

struct Bin {
    data: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Bin {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
        let mut v = json!({"buffer": 0, "byteOffset": self.data.len(), "byteLength": bytes.len()});
        if let Some(t) = target {
            v["target"] = json!(t);
        }
        self.data.extend_from_slice(bytes);
        self.views.push(v);
        self.views.len() - 1
    }

    fn floats(&mut self, values: &[f32], width: usize, ty: &str, bounds: bool) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|f| f.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(34962));
        let mut a = json!({
            "bufferView": view, "componentType": 5126, "count": values.len() / width, "type": ty,
        });
        if bounds && !values.is_empty() {
            let mut lo = vec![f32::INFINITY; width];
            let mut hi = vec![f32::NEG_INFINITY; width];
            for c in values.chunks_exact(width) {
                for i in 0..width {
                    lo[i] = lo[i].min(c[i]);
                    hi[i] = hi[i].max(c[i]);
                }
            }
            a["min"] = json!(lo);
            a["max"] = json!(hi);
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }

    fn indices(&mut self, idx: &[u32]) -> usize {
        let bytes: Vec<u8> = idx.iter().flat_map(|i| i.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(34963));
        self.accessors.push(json!({
            "bufferView": view, "componentType": 5125, "count": idx.len(), "type": "SCALAR",
        }));
        self.accessors.len() - 1
    }
}

/// One vertex attribute as flat floats.
struct Attr {
    name: String,
    width: usize,
    ty: &'static str,
    values: Vec<f32>,
}

impl Attr {
    fn compact(&self, remap: &[u32], count: usize) -> Vec<f32> {
        let mut out = vec![0.0; count * self.width];
        for (old, &new) in remap.iter().enumerate() {
            if new != u32::MAX {
                let n = new as usize;
                out[n * self.width..(n + 1) * self.width]
                    .copy_from_slice(&self.values[old * self.width..(old + 1) * self.width]);
            }
        }
        out
    }
}

fn read_attrs(prim: &gltf::Primitive, blob: &[u8]) -> Option<Vec<Attr>> {
    let r = prim.reader(|b| (b.index() == 0).then_some(blob));
    let mut out = Vec::new();
    let pos: Vec<f32> = r.read_positions()?.flatten().collect();
    out.push(Attr {
        name: "POSITION".into(),
        width: 3,
        ty: "VEC3",
        values: pos,
    });
    if let Some(n) = r.read_normals() {
        out.push(Attr {
            name: "NORMAL".into(),
            width: 3,
            ty: "VEC3",
            values: n.flatten().collect(),
        });
    }
    if let Some(t) = r.read_tangents() {
        out.push(Attr {
            name: "TANGENT".into(),
            width: 4,
            ty: "VEC4",
            values: t.flatten().collect(),
        });
    }
    for set in 0..2 {
        if let Some(uv) = r.read_tex_coords(set) {
            out.push(Attr {
                name: format!("TEXCOORD_{set}"),
                width: 2,
                ty: "VEC2",
                values: uv.into_f32().flatten().collect(),
            });
        }
    }
    if let Some(c) = r.read_colors(0) {
        out.push(Attr {
            name: "COLOR_0".into(),
            width: 4,
            ty: "VEC4",
            values: c.into_rgba_f32().flatten().collect(),
        });
    }
    // Every attribute the primitive names must be one we carry, or the rebuild would drop it.
    (prim.attributes().count() == out.len()).then_some(out)
}

/// Why a model cannot be rebuilt, if it cannot.
fn unsupported(doc: &gltf::Document) -> Option<&'static str> {
    if doc.skins().count() > 0 {
        return Some("it has skins");
    }
    if doc.animations().count() > 0 {
        return Some("it has animations");
    }
    if doc.buffers().count() != 1 {
        return Some("it has more than one buffer");
    }
    if doc.accessors().any(|a| a.sparse().is_some()) {
        return Some("it has sparse accessors");
    }
    if doc
        .meshes()
        .flat_map(|m| m.primitives())
        .any(|p| p.morph_targets().len() > 0)
    {
        return Some("it has morph targets");
    }
    None
}

/// Simplifies one triangle list towards `target` indices. Returns the kept indices into the
/// original vertices. Tries a small error first, then a larger one, then the grid pass.
fn simplify(indices: &[u32], positions: &[[f32; 3]], target: usize) -> Vec<u32> {
    use meshopt::SimplifyOptions;
    if target >= indices.len() {
        return indices.to_vec();
    }
    let near = |n: usize| n <= target + target / 4;
    let mut best = indices.to_vec();
    for error in [0.01, 0.05] {
        let r = meshopt::simplify_decoder(indices, positions, target, error, SimplifyOptions::Prune, None);
        if r.len() >= 3 && r.len() < best.len() {
            best = r;
        }
        if near(best.len()) {
            return best;
        }
    }
    let r = meshopt::simplify_sloppy_decoder(indices, positions, target, 1.0, None);
    if r.len() >= 3 && r.len() < best.len() {
        best = r;
    }
    best
}

fn is_opaque(rgba: &[u8]) -> bool {
    rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255)
}

/// Shrinks one image so its longer side is at most `max_px`, re-encoding it. None when the
/// image is already small enough or cannot be decoded.
struct Shrunk {
    bytes: Vec<u8>,
    mime: &'static str,
    from: [u32; 2],
    to: [u32; 2],
}

fn shrink_image(bytes: &[u8], max_px: u32) -> Option<Shrunk> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    if w.max(h) <= max_px {
        return None;
    }
    let (mut tw, mut th) = (w, h);
    while tw.max(th) > max_px {
        tw = (tw / 2).max(1);
        th = (th / 2).max(1);
    }
    let rgba = texture::resample(img.as_raw(), w, h, tw, th);
    let mut out = Vec::new();
    let mime = if is_opaque(&rgba) {
        let rgb: Vec<u8> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
            .encode(&rgb, tw, th, image::ExtendedColorType::Rgb8)
            .ok()?;
        "image/jpeg"
    } else {
        out = marionette_assets::encode_png(tw, th, &rgba);
        "image/png"
    };
    Some(Shrunk {
        bytes: out,
        mime,
        from: [w, h],
        to: [tw, th],
    })
}

/// Slims `glb` to `max_triangles` and `max_texture_px` (either may be absent).
pub fn slim(glb: &[u8], max_triangles: Option<u64>, max_texture_px: Option<u32>) -> Slimmed {
    let mut notes = Vec::new();
    let Ok(g) = gltf::Gltf::from_slice(glb) else {
        return Slimmed { glb: None, notes };
    };
    let doc = &g.document;
    let blob: &[u8] = g.blob.as_deref().unwrap_or(&[]);
    let total: u64 = doc
        .meshes()
        .flat_map(|m| m.primitives())
        .filter(|p| p.mode() == Mode::Triangles)
        .map(|p| {
            let n = p.indices().map_or_else(
                || p.get(&gltf::Semantic::Positions).map_or(0, |a| a.count()),
                |i| i.count(),
            );
            (n / 3) as u64
        })
        .sum();
    let decimate = max_triangles.is_some_and(|m| total > m);
    let shrink = max_texture_px.is_some();
    if !decimate && !shrink {
        return Slimmed { glb: None, notes };
    }
    if let Some(why) = unsupported(doc) {
        notes.push(format!("not slimmed because {why}"));
        return Slimmed { glb: None, notes };
    }
    let ratio = match max_triangles {
        Some(m) if decimate => m as f64 / total as f64,
        _ => 1.0,
    };

    // Edit the file's own JSON, so extensions and extras this step does not know survive.
    let Ok(crate::model::Glb { mut json, .. }) = crate::model::read_glb(glb) else {
        return Slimmed { glb: None, notes };
    };
    let mut bin = Bin {
        data: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
    };
    let mut kept = 0u64;
    for mesh in doc.meshes() {
        for prim in mesh.primitives() {
            let Some(attrs) = read_attrs(&prim, blob) else {
                notes.push(format!(
                    "not slimmed because mesh {} has an attribute this step does not carry",
                    mesh.index()
                ));
                return Slimmed { glb: None, notes };
            };
            let count = attrs[0].values.len() / 3;
            let r = prim.reader(|b| (b.index() == 0).then_some(blob));
            let indices: Vec<u32> = match r.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..count as u32).collect(),
            };
            let positions: Vec<[f32; 3]> = attrs[0].values.as_chunks::<3>().0.to_vec();
            let tri = prim.mode() == Mode::Triangles && indices.len() >= 3;
            let mut idx = if tri && ratio < 1.0 {
                let target = ((indices.len() / 3) as f64 * ratio).floor().max(1.0) as usize * 3;
                simplify(&indices, &positions, target)
            } else {
                indices
            };
            if tri {
                kept += (idx.len() / 3) as u64;
                idx = meshopt::optimize_vertex_cache(&idx, count);
            }
            // Drop vertices nothing uses any more, keeping first-use order.
            let mut remap = vec![u32::MAX; count];
            let mut next = 0u32;
            for i in idx.iter_mut() {
                let slot = &mut remap[*i as usize];
                if *slot == u32::MAX {
                    *slot = next;
                    next += 1;
                }
                *i = *slot;
            }
            let mut attributes = serde_json::Map::new();
            for a in &attrs {
                let values = a.compact(&remap, next as usize);
                let acc = bin.floats(&values, a.width, a.ty, a.name == "POSITION");
                attributes.insert(a.name.clone(), json!(acc));
            }
            let ind = bin.indices(&idx);
            let p = &mut json["meshes"][mesh.index()]["primitives"][prim.index()];
            p["attributes"] = Value::Object(attributes);
            p["indices"] = json!(ind);
        }
    }
    if decimate {
        notes.push(format!(
            "decimated from {total} to {kept} triangles (budget {})",
            max_triangles.unwrap_or_default()
        ));
    }
    for img in doc.images() {
        let gltf::image::Source::View { view, mime_type } = img.source() else {
            continue;
        };
        let start = view.offset();
        let Some(bytes) = blob.get(start..start + view.length()) else {
            continue;
        };
        let (bytes, mime) = match max_texture_px.and_then(|m| shrink_image(bytes, m)) {
            Some(s) => {
                notes.push(format!(
                    "image {} resized from {}x{} to {}x{} ({})",
                    img.index(),
                    s.from[0],
                    s.from[1],
                    s.to[0],
                    s.to[1],
                    s.mime
                ));
                (s.bytes, s.mime.to_string())
            }
            None => (bytes.to_vec(), mime_type.to_string()),
        };
        let v = bin.view(&bytes, None);
        json["images"][img.index()]["bufferView"] = json!(v);
        json["images"][img.index()]["mimeType"] = json!(mime);
    }
    while !bin.data.len().is_multiple_of(4) {
        bin.data.push(0);
    }
    json["accessors"] = Value::Array(bin.accessors);
    json["bufferViews"] = Value::Array(bin.views);
    json["buffers"] = json!([{"byteLength": bin.data.len()}]);
    Slimmed {
        glb: Some(crate::model::write_glb(&json, Some(&bin.data))),
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wavy `n` x `n` grid (2 * (n-1)^2 triangles) with normals, UVs and one opaque
    /// `tex` x `tex` texture.
    fn grid_glb(n: u32, tex: u32) -> Vec<u8> {
        let mut pos = Vec::new();
        let mut nor = Vec::new();
        let mut uv = Vec::new();
        for y in 0..n {
            for x in 0..n {
                let (u, v) = (x as f32 / (n - 1) as f32, y as f32 / (n - 1) as f32);
                pos.extend([u, (u * 6.0).sin() * 0.05 + (v * 4.0).cos() * 0.05, v]);
                nor.extend([0.0f32, 1.0, 0.0]);
                uv.extend([u, v]);
            }
        }
        let mut idx = Vec::new();
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let i = y * n + x;
                idx.extend([i, i + n, i + 1, i + 1, i + n, i + n + 1]);
            }
        }
        let rgba: Vec<u8> = (0..tex * tex)
            .flat_map(|i| [(i % 251) as u8, 90, 40, 255])
            .collect();
        let png = marionette_assets::encode_png(tex, tex, &rgba);
        let mut bin = Bin {
            data: Vec::new(),
            views: Vec::new(),
            accessors: Vec::new(),
        };
        let p = bin.floats(&pos, 3, "VEC3", true);
        let nn = bin.floats(&nor, 3, "VEC3", false);
        let t = bin.floats(&uv, 2, "VEC2", false);
        let i = bin.indices(&idx);
        let img = bin.view(&png, None);
        while !bin.data.len().is_multiple_of(4) {
            bin.data.push(0);
        }
        let json = json!({
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": p, "NORMAL": nn, "TEXCOORD_0": t}, "indices": i, "material": 0}]}],
            "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}],
            "textures": [{"source": 0}],
            "images": [{"bufferView": img, "mimeType": "image/png"}],
            "accessors": bin.accessors,
            "bufferViews": bin.views,
            "buffers": [{"byteLength": bin.data.len()}],
        });
        crate::model::write_glb(&json, Some(&bin.data))
    }

    fn triangles(glb: &[u8]) -> usize {
        let g = gltf::Gltf::from_slice(glb).unwrap();
        g.document
            .meshes()
            .flat_map(|m| m.primitives())
            .map(|p| p.indices().unwrap().count() / 3)
            .sum()
    }

    #[test]
    fn a_heavy_model_is_brought_to_its_budget_and_its_texture_shrunk() {
        let glb = grid_glb(80, 256);
        assert_eq!(triangles(&glb), 2 * 79 * 79);
        let out = slim(&glb, Some(1000), Some(64));
        let slim_glb = out.glb.expect("slimmed");
        let tris = triangles(&slim_glb);
        assert!((100..=1250).contains(&tris), "{tris} triangles");
        assert!(
            slim_glb.len() < glb.len() / 4,
            "{} -> {}",
            glb.len(),
            slim_glb.len()
        );
        assert!(
            out.notes.iter().any(|n| n.starts_with("decimated from 12482 to")),
            "{:?}",
            out.notes
        );
        assert!(
            out.notes
                .iter()
                .any(|n| n.contains("256x256 to 64x64 (image/jpeg)")),
            "{:?}",
            out.notes
        );
        // Still valid glTF with every attribute, and the texture decodes at the new size.
        let g = gltf::Gltf::from_slice(&slim_glb).unwrap();
        let prim = g.document.meshes().next().unwrap().primitives().next().unwrap();
        assert_eq!(prim.attributes().count(), 3);
        let blob = g.blob.as_deref().unwrap();
        let gltf::image::Source::View { view, mime_type } = g.document.images().next().unwrap().source()
        else {
            panic!("image moved out of the buffer")
        };
        assert_eq!(mime_type, "image/jpeg");
        let img = image::load_from_memory(&blob[view.offset()..view.offset() + view.length()]).unwrap();
        assert_eq!((img.width(), img.height()), (64, 64));
        // The full pipeline accepts it, and the same input gives the same bytes.
        crate::model::process(&slim_glb, None).unwrap();
        assert_eq!(slim(&glb, Some(1000), Some(64)).glb.unwrap(), slim_glb);
    }

    #[test]
    fn a_model_within_budget_is_left_alone() {
        let glb = grid_glb(10, 32);
        assert!(slim(&glb, Some(1000), None).glb.is_none());
        let out = slim(&glb, Some(1000), Some(64));
        // Textures are rewritten into the new buffer but not resized.
        assert_eq!(triangles(&out.glb.unwrap()), 162);
        assert!(out.notes.is_empty(), "{:?}", out.notes);
    }
}
