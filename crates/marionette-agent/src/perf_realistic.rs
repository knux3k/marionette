//! The realistic render benchmark (decision 0284): a generated scene with millions of
//! triangles, hundreds of materials, 2K textures and thousands of moving objects, written in
//! formats both Marionette and another engine can load, so the same scene can be timed in
//! each. Everything is made from a seed: no downloads, no binary files in the repository.
//!
//! Output folder, next to the scene file `<stem>.scene.json`:
//! - `<stem>/textures/set-NN-{color,normal,mr,ms}.png`: tileable color, normal (+Y up), glTF
//!   metal-roughness (G roughness, B metal) and metal-smoothness (RGB metal, A smoothness)
//! - `<stem>/models/model-NN.glb`: 8 parts per model, one material each, textures by file
//! - `<stem>/meshes/model-NN.bin`: the same meshes as plain arrays (see [`write_bin`])
//! - `<stem>/manifest.json`: materials, instances, movers, camera, light and settings

use crate::{Agent, Op, op, write_atomic};
use marionette_core::{Diagnostic, Errors, scene};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

pub(crate) fn ops() -> Vec<Op> {
    vec![op(
        "perf_realistic_scene",
        "Write the realistic render benchmark: 32 models with 8 materials each and their own \
         2K texture set, a textured ground, about 380 large static models and 3,200 moving ones \
         (about 18 million triangles at scale 1), sun with cascaded shadows, BC7 textures. Also \
         writes a manifest and plain mesh files so another engine can build the same scene.",
        true,
        realistic_scene,
    )]
}

/// Texture sets of the 32 models (the ground has one more).
pub const MODELS: usize = 32;
/// Models that stand still; the rest move.
pub const STATIC_MODELS: usize = 24;
/// Parts (and materials) per model.
pub const PARTS: usize = 8;
/// Static and moving instances at scale 1.
pub const STATIC_INSTANCES: u32 = 384;
pub const MOVING_INSTANCES: u32 = 3200;
/// Points the movers orbit.
pub const ANCHORS: usize = 32;
/// Simulation step of the movers (the engine default), seconds.
pub const DT: f32 = 1.0 / 60.0;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RealisticInput {
    /// Output scene file, `<stem>.scene.json` (relative to the project root). Assets go in
    /// the folder `<stem>` beside it.
    pub out: String,
    /// Multiplies the instance counts (0.01 to 8; default 1).
    #[serde(default = "one")]
    pub scale: f32,
    /// Texture width and height in texels, a power of two from 16 to 4096 (default 2048).
    #[serde(default = "texture_default")]
    pub texture_size: u32,
}

fn one() -> f32 {
    1.0
}
fn texture_default() -> u32 {
    2048
}

#[derive(Serialize, JsonSchema)]
pub struct RealisticOutput {
    pub scene: String,
    /// Folder with textures, models, meshes and the manifest.
    pub folder: String,
    pub models: u32,
    pub materials: u32,
    pub textures: u32,
    pub texture_size: u32,
    pub static_instances: u32,
    pub moving_instances: u32,
    /// Triangles of every instance together (before culling).
    pub triangles: u64,
    pub entities: u32,
}

/// Splitmix64: small, seedable, the same everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f()
    }
}

fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8DA6_B343)
        ^ (y as u32).wrapping_mul(0xD816_3841)
        ^ seed.wrapping_mul(0xCB1A_B31F);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xFF_FFFF) as f32 / 16_777_216.0
}

/// Value noise that repeats every `period` lattice cells, smooth (quintic) between them.
fn noise(x: f32, y: f32, period: i32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - xi as f32, y - yi as f32);
    let s = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (sx, sy) = (s(fx), s(fy));
    let w = |v: i32| v.rem_euclid(period);
    let a = hash2(w(xi), w(yi), seed);
    let b = hash2(w(xi + 1), w(yi), seed);
    let c = hash2(w(xi), w(yi + 1), seed);
    let d = hash2(w(xi + 1), w(yi + 1), seed);
    let top = a + (b - a) * sx;
    let bottom = c + (d - c) * sx;
    top + (bottom - top) * sy
}

/// Tileable fractal noise over [0, 1) x [0, 1), `octaves` from `base` cells across.
fn fbm(u: f32, v: f32, base: i32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut cells) = (0.0, 1.0, 0.0, base);
    for o in 0..octaves {
        sum += amp
            * noise(
                u * cells as f32,
                v * cells as f32,
                cells,
                seed.wrapping_add(o * 977),
            );
        norm += amp;
        amp *= 0.5;
        cells *= 2;
    }
    sum / norm
}

/// What a texture set looks like.
#[derive(Clone, Copy)]
struct Look {
    kind: u32,
    a: [f32; 3],
    b: [f32; 3],
    metal: f32,
    rough: (f32, f32),
}

fn look(set: usize) -> Look {
    if set == MODELS {
        // The ground: grey-brown paving.
        return Look {
            kind: 0,
            a: [0.22, 0.2, 0.18],
            b: [0.5, 0.47, 0.42],
            metal: 0.0,
            rough: (0.6, 0.95),
        };
    }
    let mut r = Rng(0x5EED_0000 + set as u64);
    let hue = (set as f32 * 0.618_034).fract();
    let rgb = |h: f32, s: f32, v: f32| {
        let k = |n: f32| {
            let k = (n + h * 6.0) % 6.0;
            v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)
        };
        [k(5.0), k(3.0), k(1.0)]
    };
    let kind = (set % 4) as u32;
    let metal = if kind == 3 { 1.0 } else { 0.0 };
    Look {
        kind,
        a: rgb(hue, r.range(0.15, 0.45), r.range(0.15, 0.35)),
        b: rgb((hue + 0.05) % 1.0, r.range(0.1, 0.35), r.range(0.55, 0.9)),
        metal,
        rough: if metal > 0.0 { (0.25, 0.6) } else { (0.45, 0.95) },
    }
}

/// Height in [0, 1] at (u, v) of a set's surface: tiles, planks, rock or riveted panels.
fn height(l: &Look, u: f32, v: f32, seed: u32) -> f32 {
    let fine = fbm(u, v, 64, 3, seed ^ 0xF1);
    match l.kind {
        0 => {
            // Tiles with grout.
            let (gu, gv) = ((u * 8.0).fract(), (v * 8.0).fract());
            let edge = gu.min(1.0 - gu).min(gv.min(1.0 - gv));
            let tile = hash2((u * 8.0) as i32, (v * 8.0) as i32, seed);
            let bevel = (edge / 0.04).clamp(0.0, 1.0);
            (0.55 + 0.25 * tile) * bevel + 0.15 * fine
        }
        1 => {
            // Planks with grain.
            let row = (v * 16.0) as i32;
            let shift = hash2(row, 0, seed);
            let pu = (u * 2.0 + shift).fract();
            let gap = ((v * 16.0).fract().min(1.0 - (v * 16.0).fract()) / 0.06).clamp(0.0, 1.0)
                * (pu.min(1.0 - pu) / 0.01).clamp(0.0, 1.0);
            let grain = 0.5 + 0.5 * (u * 300.0 + 8.0 * fbm(u, v, 8, 3, seed ^ row as u32)).sin();
            (0.5 + 0.3 * grain + 0.1 * fine) * gap
        }
        2 => fbm(u, v, 4, 7, seed),
        _ => {
            // Panels with bevels and rivets.
            let (gu, gv) = ((u * 4.0).fract(), (v * 2.0).fract());
            let edge = gu.min(1.0 - gu).min(gv.min(1.0 - gv) * 0.5);
            let bevel = (edge / 0.02).clamp(0.0, 1.0);
            let (ru, rv) = ((u * 32.0).fract() - 0.5, (v * 32.0).fract() - 0.5);
            let rivet = if edge < 0.06 {
                (1.0 - (ru * ru + rv * rv).sqrt() / 0.2).max(0.0)
            } else {
                0.0
            };
            0.6 * bevel + 0.3 * rivet + 0.08 * fine
        }
    }
}

/// The four images of one texture set: color, normal, glTF metal-roughness, metal-smoothness.
fn texture_set(set: usize, size: u32) -> [Vec<u8>; 4] {
    let l = look(set);
    let seed = 1000 + set as u32;
    let n = size as usize;
    let mut h = vec![0f32; n * n];
    for y in 0..n {
        for x in 0..n {
            h[y * n + x] = height(
                &l,
                (x as f32 + 0.5) / size as f32,
                (y as f32 + 0.5) / size as f32,
                seed,
            );
        }
    }
    let at =
        |x: isize, y: isize| h[(y.rem_euclid(n as isize) as usize) * n + x.rem_euclid(n as isize) as usize];
    let mut color = vec![0u8; n * n * 4];
    let mut normal = vec![0u8; n * n * 4];
    let mut mr = vec![0u8; n * n * 4];
    let mut ms = vec![0u8; n * n * 4];
    let strength = size as f32 / 256.0;
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    for y in 0..n {
        for x in 0..n {
            let i = y * n + x;
            let t = h[i];
            let tint = 0.85 + 0.3 * fbm(x as f32 / size as f32, y as f32 / size as f32, 8, 2, seed ^ 0xC0);
            for c in 0..3 {
                color[i * 4 + c] = byte((l.a[c] + (l.b[c] - l.a[c]) * t) * tint);
            }
            color[i * 4 + 3] = 255;
            let (xi, yi) = (x as isize, y as isize);
            let dx = (at(xi + 1, yi) - at(xi - 1, yi)) * 0.5 * strength;
            // Image rows go down, +V (and the normal's +Y) goes up.
            let dy = (at(xi, yi - 1) - at(xi, yi + 1)) * 0.5 * strength;
            let len = (dx * dx + dy * dy + 1.0).sqrt();
            let nrm = [-dx / len, -dy / len, 1.0 / len];
            for c in 0..3 {
                normal[i * 4 + c] = byte(nrm[c] * 0.5 + 0.5);
            }
            normal[i * 4 + 3] = 255;
            let rough = l.rough.1 + (l.rough.0 - l.rough.1) * t;
            mr[i * 4..i * 4 + 4].copy_from_slice(&[255, byte(rough), byte(l.metal), 255]);
            let m = byte(l.metal);
            ms[i * 4..i * 4 + 4].copy_from_slice(&[m, m, m, byte(1.0 - rough)]);
        }
    }
    [color, normal, mr, ms]
}

/// One part's mesh: positions, normals, texture coordinates, triangle indices.
#[derive(Default, Clone)]
pub struct Part {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// A lumpy closed shape: a sphere of `seg` x `rings` quads stretched by `size`, pushed out
/// by smooth noise, centered at `at`. UVs repeat `repeat` times around.
fn lump(seg: u32, rings: u32, size: [f32; 3], at: [f32; 3], bump: f32, seed: u32, repeat: f32) -> Part {
    let mut p = Part::default();
    for r in 0..=rings {
        let theta = std::f32::consts::PI * r as f32 / rings as f32;
        for s in 0..=seg {
            let phi = std::f32::consts::TAU * (s % seg) as f32 / seg as f32;
            let dir = [theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin()];
            let k =
                1.0 + bump * (fbm(s as f32 / seg as f32, r as f32 / rings as f32 * 0.5, 4, 3, seed) - 0.5);
            p.positions.push([
                at[0] + dir[0] * size[0] * k,
                at[1] + dir[1] * size[1] * k,
                at[2] + dir[2] * size[2] * k,
            ]);
            p.uvs.push([
                repeat * s as f32 / seg as f32,
                repeat * 0.5 * r as f32 / rings as f32,
            ]);
        }
    }
    let row = seg + 1;
    for r in 0..rings {
        for s in 0..seg {
            let (a, b, c, d) = (
                r * row + s,
                r * row + s + 1,
                (r + 1) * row + s,
                (r + 1) * row + s + 1,
            );
            // Counter-clockwise seen from outside; the caps drop their empty triangle.
            if r != 0 {
                p.indices.extend([a, b, c]);
            }
            if r != rings - 1 {
                p.indices.extend([b, d, c]);
            }
        }
    }
    p.normals = smooth_normals(&p.positions, &p.indices);
    p
}

/// Area-weighted vertex normals; vertices on a UV seam share a position, so they are
/// averaged by position to hide the seam.
fn smooth_normals(pos: &[[f32; 3]], idx: &[u32]) -> Vec<[f32; 3]> {
    let key = |p: [f32; 3]| (p[0].to_bits(), p[1].to_bits(), p[2].to_bits());
    let mut sums: std::collections::HashMap<(u32, u32, u32), [f32; 3]> = std::collections::HashMap::new();
    for t in idx.chunks_exact(3) {
        let [a, b, c] = [pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]];
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        for &v in t {
            let s = sums.entry(key(pos[v as usize])).or_insert([0.0; 3]);
            for k in 0..3 {
                s[k] += n[k];
            }
        }
    }
    pos.iter()
        .map(|&p| {
            let s = sums.get(&key(p)).copied().unwrap_or([0.0, 1.0, 0.0]);
            let l = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt().max(1e-12);
            [s[0] / l, s[1] / l, s[2] / l]
        })
        .collect()
}

/// The 8 parts of model `m`: large clusters (about 32,000 triangles) for static models,
/// small ones (about 1,800) for moving models.
fn model_parts(m: usize) -> Vec<Part> {
    let mut r = Rng(0xA11CE + m as u64);
    let moving = m >= STATIC_MODELS;
    let (seg, rings) = if moving { (16, 8) } else { (64, 32) };
    (0..PARTS)
        .map(|k| {
            let size = if moving {
                [r.range(0.15, 0.4), r.range(0.1, 0.3), r.range(0.15, 0.4)]
            } else {
                [r.range(0.5, 1.4), r.range(0.6, 2.2), r.range(0.5, 1.4)]
            };
            let ring = if moving { 0.35 } else { 1.6 };
            let a = std::f32::consts::TAU * k as f32 / PARTS as f32;
            let at = if k == 0 {
                [0.0, size[1], 0.0]
            } else {
                [ring * a.cos(), size[1] * r.range(0.6, 1.4), ring * a.sin()]
            };
            lump(
                seg,
                rings,
                size,
                at,
                if moving { 0.3 } else { 0.5 },
                (m * 31 + k) as u32,
                if moving { 1.0 } else { 2.0 },
            )
        })
        .collect()
}

/// The ground: a flat square of `side` meters, `cells` quads across, UVs repeating every 8 m.
fn ground_part(side: f32, cells: u32) -> Part {
    let mut p = Part::default();
    for z in 0..=cells {
        for x in 0..=cells {
            let (u, v) = (x as f32 / cells as f32, z as f32 / cells as f32);
            p.positions.push([(u - 0.5) * side, 0.0, (v - 0.5) * side]);
            p.normals.push([0.0, 1.0, 0.0]);
            p.uvs.push([u * side / 8.0, v * side / 8.0]);
        }
    }
    let row = cells + 1;
    for z in 0..cells {
        for x in 0..cells {
            let (a, b, c, d) = (
                z * row + x,
                z * row + x + 1,
                (z + 1) * row + x,
                (z + 1) * row + x + 1,
            );
            p.indices.extend([a, c, b, b, c, d]);
        }
    }
    p
}

/// Material `k` of a model: the set's textures with its own tint. Metal and roughness come
/// from the textures alone (factors 1), so engines that cannot multiply a roughness factor
/// into a smoothness map draw the same surface.
fn material(set: usize, k: usize) -> (String, [f32; 4], f32, f32) {
    let mut r = Rng(0x3A7 + (set * PARTS + k) as u64);
    let tint = [r.range(0.7, 1.0), r.range(0.7, 1.0), r.range(0.7, 1.0), 1.0];
    (format!("set-{set:02}-mat-{k}"), tint, 1.0, 1.0)
}

fn texture_names(set: usize) -> [String; 4] {
    ["color", "normal", "mr", "ms"].map(|k| format!("set-{set:02}-{k}.png"))
}

/// A binary glTF of `parts`, part `k` with material `k`, textures referenced by file.
fn glb(parts: &[Part], set: usize, materials: &[(String, [f32; 4], f32, f32)]) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut prims = Vec::new();
    let mut push = |bytes: &[u8], target: u32| {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        views.push(
            json!({ "buffer": 0, "byteOffset": bin.len(), "byteLength": bytes.len(), "target": target }),
        );
        bin.extend_from_slice(bytes);
        views.len() - 1
    };
    for (k, p) in parts.iter().enumerate() {
        let f = |v: &[[f32; 3]]| {
            v.iter()
                .flatten()
                .flat_map(|x| x.to_le_bytes())
                .collect::<Vec<u8>>()
        };
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for q in &p.positions {
            for c in 0..3 {
                lo[c] = lo[c].min(q[c]);
                hi[c] = hi[c].max(q[c]);
            }
        }
        let vp = push(&f(&p.positions), 34962);
        let vn = push(&f(&p.normals), 34962);
        let uv: Vec<u8> = p.uvs.iter().flatten().flat_map(|x| x.to_le_bytes()).collect();
        let vt = push(&uv, 34962);
        let ix: Vec<u8> = p.indices.iter().flat_map(|x| x.to_le_bytes()).collect();
        let vi = push(&ix, 34963);
        let n = p.positions.len();
        let base = accessors.len();
        accessors.push(json!({ "bufferView": vp, "componentType": 5126, "count": n, "type": "VEC3", "min": lo, "max": hi }));
        accessors.push(json!({ "bufferView": vn, "componentType": 5126, "count": n, "type": "VEC3" }));
        accessors.push(json!({ "bufferView": vt, "componentType": 5126, "count": n, "type": "VEC2" }));
        accessors.push(
            json!({ "bufferView": vi, "componentType": 5125, "count": p.indices.len(), "type": "SCALAR" }),
        );
        prims.push(json!({
            "attributes": { "POSITION": base, "NORMAL": base + 1, "TEXCOORD_0": base + 2 },
            "indices": base + 3, "material": k,
        }));
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let names = texture_names(set);
    let mats: Vec<Value> = materials
        .iter()
        .map(|(name, tint, metal, rough)| {
            json!({
                "name": name,
                "pbrMetallicRoughness": {
                    "baseColorTexture": { "index": 0 }, "baseColorFactor": tint,
                    "metallicRoughnessTexture": { "index": 2 },
                    "metallicFactor": metal, "roughnessFactor": rough,
                },
                "normalTexture": { "index": 1 },
            })
        })
        .collect();
    let doc = json!({
        "asset": { "version": "2.0", "generator": "marionette perf_realistic_scene" },
        "scene": 0, "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0 }],
        "meshes": [{ "primitives": prims }],
        "materials": mats,
        "textures": [{ "source": 0, "sampler": 0 }, { "source": 1, "sampler": 0 }, { "source": 2, "sampler": 0 }],
        "samplers": [{ "magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497 }],
        "images": [
            { "uri": format!("../textures/{}", names[0]) },
            { "uri": format!("../textures/{}", names[1]) },
            { "uri": format!("../textures/{}", names[2]) },
        ],
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{ "byteLength": bin.len() }],
    });
    let mut text = doc.to_string().into_bytes();
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    let total = 12 + 8 + text.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&text);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

/// The plain mesh file read by other engines' importers: `MBM1`, part count (u32), then per
/// part vertex count and index count (u32), positions (3 f32), normals (3 f32), UVs (2 f32,
/// glTF convention: (0, 0) is the image's top-left) and counter-clockwise triangles (u32),
/// all little-endian, glTF axes (right-handed, +Y up).
pub fn write_bin(parts: &[Part]) -> Vec<u8> {
    let mut out = b"MBM1".to_vec();
    out.extend((parts.len() as u32).to_le_bytes());
    for p in parts {
        out.extend((p.positions.len() as u32).to_le_bytes());
        out.extend((p.indices.len() as u32).to_le_bytes());
        out.extend(p.positions.iter().flatten().flat_map(|x| x.to_le_bytes()));
        out.extend(p.normals.iter().flatten().flat_map(|x| x.to_le_bytes()));
        out.extend(p.uvs.iter().flatten().flat_map(|x| x.to_le_bytes()));
        out.extend(p.indices.iter().flat_map(|x| x.to_le_bytes()));
    }
    out
}

fn color(r: f32, g: f32, b: f32) -> Value {
    json!({ "r": r, "g": g, "b": b, "a": 1.0 })
}

fn v3(v: [f32; 3]) -> Value {
    json!({ "x": v[0], "y": v[1], "z": v[2] })
}

fn yaw(a: f32) -> [f32; 4] {
    [0.0, (a * 0.5).sin(), 0.0, (a * 0.5).cos()]
}

fn quat(q: [f32; 4]) -> Value {
    json!({ "x": q[0], "y": q[1], "z": q[2], "w": q[3] })
}

fn transform(t: [f32; 3], q: [f32; 4], s: f32) -> Value {
    json!({ "translation": v3(t), "rotation": quat(q), "scale": v3([s, s, s]) })
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(a: [f32; 3]) -> [f32; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn bad(msg: &str, fix: &str) -> Errors {
    Diagnostic::error(crate::codes::BAD_INPUT, msg).fix(fix).into()
}

fn realistic_scene(agent: &mut Agent, input: RealisticInput) -> Result<RealisticOutput, Errors> {
    if !(0.01..=8.0).contains(&input.scale) {
        return Err(bad("scale must be 0.01 to 8", "use 1 for the comparison scene"));
    }
    let size = input.texture_size;
    if !(16..=4096).contains(&size) || !size.is_power_of_two() {
        return Err(bad(
            "texture_size must be a power of two from 16 to 4096",
            "use 2048 for the comparison scene, 64 for a quick test",
        ));
    }
    let Some(stem) = input.out.strip_suffix(".scene.json") else {
        return Err(bad(
            "out must end in .scene.json",
            "for example bench/realistic.scene.json",
        ));
    };
    let path = agent.resolve(&input.out)?;
    let dir_name = Path::new(stem)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let root = path.with_file_name(&dir_name);
    let write = |rel: &str, bytes: &[u8]| write_atomic(&root.join(rel), &format!("{stem}/{rel}"), bytes);

    // Textures: one set per model plus the ground's, made in parallel.
    let sets = MODELS + 1;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let made: Vec<(usize, [Vec<u8>; 4])> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                scope.spawn(move || {
                    (t..sets)
                        .step_by(threads)
                        .map(|s| {
                            let imgs = texture_set(s, size);
                            (s, imgs.map(|px| marionette_assets::encode_png(size, size, &px)))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    if made.len() != sets {
        return Err(bad(
            "texture generation failed",
            "run again; report if it repeats",
        ));
    }
    for (s, pngs) in &made {
        for (name, png) in texture_names(*s).iter().zip(pngs) {
            write(&format!("textures/{name}"), png)?;
        }
    }

    // Models.
    let mut model_tris = Vec::new();
    let mut manifest_models = Vec::new();
    for m in 0..=MODELS {
        let (parts, mats) = if m < MODELS {
            (
                model_parts(m),
                (0..PARTS).map(|k| material(m, k)).collect::<Vec<_>>(),
            )
        } else {
            (
                vec![ground_part(400.0, 64)],
                vec![(format!("set-{m:02}-mat-0"), [1.0; 4], 1.0, 1.0)],
            )
        };
        let name = if m < MODELS {
            format!("model-{m:02}")
        } else {
            "ground".to_string()
        };
        write(&format!("models/{name}.glb"), &glb(&parts, m, &mats))?;
        write(&format!("meshes/{name}.bin"), &write_bin(&parts))?;
        let tris: u64 = parts.iter().map(|p| p.indices.len() as u64 / 3).sum();
        model_tris.push(tris);
        let names = texture_names(m);
        manifest_models.push(json!({
            "name": name, "mesh": format!("meshes/{name}.bin"), "glb": format!("models/{name}.glb"),
            "triangles": tris, "moving": (STATIC_MODELS..MODELS).contains(&m),
            "textures": { "color": format!("textures/{}", names[0]), "normal": format!("textures/{}", names[1]),
                          "metal_roughness": format!("textures/{}", names[2]), "metal_smoothness": format!("textures/{}", names[3]) },
            "materials": mats.iter().map(|(n, tint, metal, rough)| json!({
                "name": n, "tint": tint, "metallic": metal, "roughness": rough })).collect::<Vec<_>>(),
        }));
    }

    // Entities.
    let mut r = Rng(0xBE7C);
    let model_asset = |m: usize| {
        if m < MODELS {
            format!("{dir_name}/models/model-{m:02}.glb")
        } else {
            format!("{dir_name}/models/ground.glb")
        }
    };
    let field = 300.0f32;
    let cam = ([0.0, 22.0, field * 0.5 + 10.0], [0.0, 2.0, 0.0]);
    let mut entities = vec![
        json!({ "id": 1, "name": "camera", "components": {
            "Camera": { "clear_color": color(0.55, 0.68, 0.85),
                "projection": { "Perspective": { "fov_y_degrees": 60.0, "near": 0.3, "far": 600.0 } } },
            "Transform": transform(cam.0, [0.0, 0.0, 0.0, 1.0], 1.0),
            "LookAt": { "target": v3(cam.1), "up": v3([0.0, 1.0, 0.0]) },
        }}),
        json!({ "id": 2, "name": "sun", "components": {
            "DirectionalLight": { "color": color(1.0, 0.96, 0.88), "direction": v3([-0.45, -1.0, -0.35]), "illuminance": 3.0 },
        }}),
        json!({ "id": 3, "name": "ground", "components": {
            "Model": { "asset": model_asset(MODELS) },
            "Transform": transform([0.0; 3], [0.0, 0.0, 0.0, 1.0], 1.0),
        }}),
    ];
    let mut instances = Vec::new();
    let mut triangles = model_tris[MODELS];
    let n_static = ((STATIC_INSTANCES as f32 * input.scale).round() as u32).max(1);
    let n_moving = ((MOVING_INSTANCES as f32 * input.scale).round() as u32).max(1);
    let mut id = 10u64;
    for i in 0..n_static {
        let m = i as usize % STATIC_MODELS;
        let t = [r.range(-0.5, 0.5) * field, 0.0, r.range(-0.5, 0.5) * field];
        let q = yaw(r.range(0.0, std::f32::consts::TAU));
        let s = r.range(0.8, 1.6);
        entities.push(json!({ "id": id, "components": {
            "Model": { "asset": model_asset(m) }, "Transform": transform(t, q, s) }}));
        instances.push(json!({ "model": m, "position": t, "rotation": q, "scale": s }));
        triangles += model_tris[m];
        id += 1;
    }
    let mut anchors = Vec::new();
    let side = (ANCHORS as f32).sqrt().ceil() as usize;
    for a in 0..ANCHORS {
        let (gx, gz) = ((a % side) as f32 + 0.5, (a / side) as f32 + 0.5);
        let p = [
            (gx / side as f32 - 0.5) * field * 0.9,
            r.range(6.0, 14.0),
            (gz / side as f32 - 0.5) * field * 0.9,
        ];
        entities.push(json!({ "id": id, "name": format!("anchor-{a}"), "components": { "Position": v3(p) }}));
        anchors.push((id, p));
        id += 1;
    }
    let mut movers = Vec::new();
    for i in 0..n_moving {
        let m = STATIC_MODELS + i as usize % (MODELS - STATIC_MODELS);
        let a = i as usize % ANCHORS;
        let (anchor_id, c) = anchors[a];
        let n = unit([r.range(-1.0, 1.0), r.range(0.3, 1.0), r.range(-1.0, 1.0)]);
        let side = unit(cross(
            n,
            if n[1].abs() < 0.9 {
                [0.0, 1.0, 0.0]
            } else {
                [1.0, 0.0, 0.0]
            },
        ));
        let radius = r.range(2.0, 10.0);
        let gain = r.range(0.2, 1.0);
        let speed = radius * gain.sqrt();
        let tangent = cross(n, side);
        let p = [
            c[0] + side[0] * radius,
            c[1] + side[1] * radius,
            c[2] + side[2] * radius,
        ];
        let v = [tangent[0] * speed, tangent[1] * speed, tangent[2] * speed];
        let q = yaw(r.range(0.0, std::f32::consts::TAU));
        let s = r.range(0.5, 1.0);
        entities.push(json!({ "id": id, "components": {
            "Model": { "asset": model_asset(m) },
            "Transform": transform(p, q, s),
            "Position": v3(p), "Velocity": v3(v),
            "Follow": { "target": anchor_id, "gain": gain },
        }}));
        movers.push(json!({ "model": m, "anchor": a, "position": p, "velocity": v, "gain": gain, "rotation": q, "scale": s }));
        triangles += model_tris[m];
        id += 1;
    }
    let resources = json!({
        "AmbientLight": { "color": color(0.6, 0.65, 0.75), "intensity": 0.25 },
        "SkyLight": { "sky": color(0.55, 0.68, 0.85), "ground": color(0.3, 0.27, 0.22), "intensity": 0.35 },
        "Shadows": { "distance": 60.0, "resolution": 2048, "softness": 1.0 },
        "ShadowCascades": { "count": 4, "far": 150.0 },
        "ToneMapping": { "exposure": 1.0 },
        "TextureCompression": { "normal_maps": true },
        "FixedStep": { "dt": DT },
    });
    let value =
        json!({ "format": "marionette.scene", "version": 1, "resources": resources, "entities": entities });
    let entity_count = value["entities"].as_array().map_or(0, |e| e.len() as u32);
    let text = scene::format_text(&value.to_string(), agent.registry())?;
    let manifest = json!({
        "format": "marionette.realistic-bench", "version": 1,
        "axes": "glTF: right-handed, +Y up, +Z toward the default camera; meters; quaternions x, y, z, w",
        "texture_size": size, "dt": DT,
        "camera": { "position": cam.0, "target": cam.1, "fov_y_degrees": 60.0, "near": 0.3, "far": 600.0,
                    "clear_color": [0.55, 0.68, 0.85] },
        "sun": { "direction": [-0.45, -1.0, -0.35], "color": [1.0, 0.96, 0.88], "intensity": 3.0,
                 "shadow_cascades": 4, "shadow_far": 150.0, "shadow_resolution": 2048 },
        "ambient": { "sky": [0.55, 0.68, 0.85], "ground": [0.3, 0.27, 0.22] },
        "models": manifest_models,
        "ground": MODELS,
        "instances": instances,
        "anchors": anchors.iter().map(|(_, p)| json!({ "position": p })).collect::<Vec<_>>(),
        "movers": movers,
        "motion": "each tick: velocity += (anchor - position) * gain * dt; position += velocity * dt",
        "triangles": triangles,
    });
    write(
        "manifest.json",
        serde_json::to_string_pretty(&manifest)
            .unwrap_or_default()
            .as_bytes(),
    )?;
    write_atomic(&path, &input.out, text.as_bytes())?;
    Ok(RealisticOutput {
        scene: input.out.clone(),
        folder: stem.to_string(),
        models: MODELS as u32 + 1,
        materials: (MODELS * PARTS + 1) as u32,
        textures: sets as u32 * 3,
        texture_size: size,
        static_instances: n_static,
        moving_instances: n_moving,
        triangles,
        entities: entity_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_have_the_planned_size() {
        let tris = |m| model_parts(m).iter().map(|p| p.indices.len() / 3).sum::<usize>();
        assert!((25_000..40_000).contains(&tris(0)), "{}", tris(0));
        assert!(
            (1_000..3_000).contains(&tris(STATIC_MODELS)),
            "{}",
            tris(STATIC_MODELS)
        );
        for p in model_parts(3) {
            assert_eq!(p.positions.len(), p.normals.len());
            assert!(p.indices.iter().all(|&i| (i as usize) < p.positions.len()));
        }
    }

    #[test]
    fn textures_tile_and_differ_per_set() {
        let a = texture_set(0, 32);
        let b = texture_set(1, 32);
        assert_ne!(a[0], b[0]);
        // Normals point out of the surface.
        assert!(a[1].chunks_exact(4).all(|t| t[2] >= 128));
        // Tileable: the noise repeats over one period.
        let l = look(2);
        let (h0, h1) = (height(&l, 0.1, 0.3, 7), height(&l, 1.1, 0.3, 7));
        assert!((h0 - h1).abs() < 1e-4, "{h0} {h1}");
    }

    #[test]
    fn mesh_file_round_trips_its_counts() {
        let parts = model_parts(STATIC_MODELS);
        let bin = write_bin(&parts);
        assert_eq!(&bin[..4], b"MBM1");
        assert_eq!(u32::from_le_bytes(bin[4..8].try_into().unwrap()), PARTS as u32);
        let expected: usize = 8 + parts
            .iter()
            .map(|p| 8 + p.positions.len() * 32 + p.indices.len() * 4)
            .sum::<usize>();
        assert_eq!(bin.len(), expected);
    }
}
