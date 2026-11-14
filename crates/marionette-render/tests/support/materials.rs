//! Textured glTF test models made in code (decision 0270), shared by the render tests and
//! the `make_materials` example that writes the committed copies in `examples/assets/`.
//! Everything here is CC0 1.0 (public domain).

#![allow(dead_code)]

use glam::Vec3;
use marionette_render::assets::encode_png;
use serde_json::json;

/// Triangle mesh: positions, normals, texture coordinates, indices.
#[derive(Default)]
pub struct Mesh {
    pub pos: Vec<[f32; 3]>,
    pub nrm: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    pub idx: Vec<u32>,
}

/// A glTF material: PNG textures and factors.
pub struct Material {
    pub base: Option<Vec<u8>>,
    pub mr: Option<Vec<u8>>,
    pub normal: Option<Vec<u8>>,
    pub base_factor: [f32; 4],
    /// `OPAQUE`, `MASK` or `BLEND`.
    pub alpha_mode: &'static str,
    pub cutoff: f32,
    pub double_sided: bool,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            base: None,
            mr: None,
            normal: None,
            base_factor: [1.0; 4],
            alpha_mode: "OPAQUE",
            cutoff: 0.5,
            double_sided: false,
        }
    }
}

/// A `size` x `size` PNG from `f(u, v)` at texel centers.
pub fn texture(size: u32, f: impl Fn(f32, f32) -> [u8; 4]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            rgba.extend_from_slice(&f((x as f32 + 0.5) / size as f32, (y as f32 + 0.5) / size as f32));
        }
    }
    encode_png(size, size, &rgba)
}

fn pad(buf: &mut Vec<u8>, byte: u8) {
    while !buf.len().is_multiple_of(4) {
        buf.push(byte);
    }
}

/// One mesh with one material as a binary glTF (`.glb`); same input, same bytes.
pub fn glb(name: &str, mesh: &Mesh, mat: &Material) -> Vec<u8> {
    let mut bin = Vec::new();
    let mut views = Vec::new();
    let mut push = |bytes: &[u8], target: Option<u32>| {
        pad(&mut bin, 0);
        let offset = bin.len();
        bin.extend_from_slice(bytes);
        let mut v = json!({ "buffer": 0, "byteOffset": offset, "byteLength": bytes.len() });
        if let Some(t) = target {
            v["target"] = json!(t);
        }
        views.push(v);
        views.len() - 1
    };
    let v_pos = push(bytemuck::cast_slice(&mesh.pos), Some(34962));
    let v_nrm = push(bytemuck::cast_slice(&mesh.nrm), Some(34962));
    let v_uv = push(bytemuck::cast_slice(&mesh.uv), Some(34962));
    let v_idx = push(bytemuck::cast_slice(&mesh.idx), Some(34963));
    let mut images = Vec::new();
    let mut texture_of = |png: &Option<Vec<u8>>| {
        png.as_ref().map(|p| {
            images.push(push(p, None));
            json!({ "index": images.len() - 1 })
        })
    };
    let (base, mr, normal) = (
        texture_of(&mat.base),
        texture_of(&mat.mr),
        texture_of(&mat.normal),
    );
    pad(&mut bin, 0);

    let mut pbr =
        json!({ "baseColorFactor": mat.base_factor, "metallicFactor": 1.0, "roughnessFactor": 1.0 });
    if let Some(t) = base {
        pbr["baseColorTexture"] = t;
    }
    if let Some(t) = mr {
        pbr["metallicRoughnessTexture"] = t;
    }
    let mut material = json!({
        "name": name,
        "pbrMetallicRoughness": pbr,
        "alphaMode": mat.alpha_mode,
        "doubleSided": mat.double_sided,
    });
    if mat.alpha_mode == "MASK" {
        material["alphaCutoff"] = json!(mat.cutoff);
    }
    if let Some(t) = normal {
        material["normalTexture"] = t;
    }
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in &mesh.pos {
        for c in 0..3 {
            lo[c] = lo[c].min(p[c]);
            hi[c] = hi[c].max(p[c]);
        }
    }
    let doc = json!({
        "asset": { "version": "2.0", "generator": "marionette render test models", "copyright": "CC0 1.0 (public domain)" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0, "name": name }],
        "meshes": [{ "name": name, "primitives": [{
            "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 },
            "indices": 3,
            "material": 0
        }]}],
        "materials": [material],
        "samplers": [{ "magFilter": 9729, "minFilter": 9987 }],
        "textures": (0..images.len()).map(|i| json!({ "source": i, "sampler": 0 })).collect::<Vec<_>>(),
        "images": images.iter().map(|&v| json!({ "bufferView": v, "mimeType": "image/png" })).collect::<Vec<_>>(),
        "accessors": [
            { "bufferView": v_pos, "componentType": 5126, "count": mesh.pos.len(), "type": "VEC3", "min": lo, "max": hi },
            { "bufferView": v_nrm, "componentType": 5126, "count": mesh.nrm.len(), "type": "VEC3" },
            { "bufferView": v_uv, "componentType": 5126, "count": mesh.uv.len(), "type": "VEC2" },
            { "bufferView": v_idx, "componentType": 5125, "count": mesh.idx.len(), "type": "SCALAR" }
        ],
        "bufferViews": views,
        "buffers": [{ "byteLength": bin.len() }]
    });
    let mut json_bytes = serde_json::to_vec(&doc).expect("JSON serializes");
    pad(&mut json_bytes, b' ');
    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

/// UV sphere of radius 0.5: u around (+x toward -z), v from the top.
pub fn sphere(segments: u32, rings: u32) -> Mesh {
    let mut m = Mesh::default();
    for r in 0..=rings {
        let v = r as f32 / rings as f32;
        let theta = v * std::f32::consts::PI;
        for s in 0..=segments {
            let u = s as f32 / segments as f32;
            let phi = u * std::f32::consts::TAU;
            let n = Vec3::new(theta.sin() * phi.cos(), theta.cos(), -theta.sin() * phi.sin());
            m.pos.push((n * 0.5).to_array());
            m.nrm.push(n.to_array());
            m.uv.push([u, v]);
        }
    }
    let stride = segments + 1;
    for r in 0..rings {
        for s in 0..segments {
            let a = r * stride + s;
            let b = a + stride;
            m.idx.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    m
}

/// A 1 x 1 square in the XY plane facing +z, `repeat` texture repeats across it.
pub fn quad(repeat: f32) -> Mesh {
    Mesh {
        pos: vec![
            [-0.5, -0.5, 0.0],
            [0.5, -0.5, 0.0],
            [0.5, 0.5, 0.0],
            [-0.5, 0.5, 0.0],
        ],
        nrm: vec![[0.0, 0.0, 1.0]; 4],
        uv: vec![[0.0, repeat], [repeat, repeat], [repeat, 0.0], [0.0, 0.0]],
        idx: vec![0, 1, 2, 0, 2, 3],
    }
}

/// The material ball: a sphere whose metallic-roughness texture puts metal on one half
/// (u < 0.5, blue channel) and alternates glossy and rough bands down it (v, green channel),
/// with a normal map of round bumps lit differently on their upper and lower sides. Swapping
/// the metallic and roughness channels, or flipping the normal map's green channel, changes
/// the picture far beyond the reference tolerance (`swap_mr`, `flip_green` make those
/// broken variants for the test that proves it).
pub fn material_ball(swap_mr: bool, flip_green: bool) -> Vec<u8> {
    let base = texture(128, |u, v| {
        let warm = ((u * 8.0).floor() as i32 + (v * 4.0).floor() as i32) % 2 == 0;
        if warm {
            [235, 190, 140, 255]
        } else {
            [200, 205, 215, 255]
        }
    });
    let mr = texture(128, |u, v| {
        let metallic = if u < 0.5 { 255 } else { 0 };
        let roughness = if ((v * 6.0).floor() as i32) % 2 == 0 {
            40
        } else {
            220
        };
        if swap_mr {
            [0, metallic, roughness, 255]
        } else {
            [0, roughness, metallic, 255]
        }
    });
    let normal = texture(128, |u, v| {
        // Bumps on a 16 x 8 grid: the slope points away from each bump's center.
        let (fx, fy) = ((u * 16.0).fract() - 0.5, (v * 8.0).fract() - 0.5);
        let r = (fx * fx + fy * fy).sqrt();
        let (sx, sy) = if r < 0.4 { (fx * 3.0, fy * 3.0) } else { (0.0, 0.0) };
        let n = Vec3::new(sx, if flip_green { sy } else { -sy }, 1.0).normalize();
        let e = |c: f32| ((c * 0.5 + 0.5) * 255.0).round() as u8;
        [e(n.x), e(n.y), e(n.z), 255]
    });
    glb(
        "material-ball",
        &sphere(64, 32),
        &Material {
            base: Some(base),
            mr: Some(mr),
            normal: Some(normal),
            ..Default::default()
        },
    )
}

/// A double-sided cut-out fence panel: bars on a clear background (alpha mode `MASK`).
pub fn fence() -> Vec<u8> {
    let base = texture(64, |u, v| {
        let bar = (u * 4.0).fract() < 0.25 || (v * 4.0).fract() < 0.25;
        if bar {
            [150, 100, 60, 255]
        } else {
            [150, 100, 60, 0]
        }
    });
    glb(
        "fence",
        &quad(1.0),
        &Material {
            base: Some(base),
            alpha_mode: "MASK",
            double_sided: true,
            ..Default::default()
        },
    )
}

/// A see-through blue glass pane (alpha mode `BLEND`), single-sided.
pub fn glass() -> Vec<u8> {
    glb(
        "glass",
        &quad(1.0),
        &Material {
            base_factor: [0.2, 0.45, 1.0, 0.35],
            alpha_mode: "BLEND",
            ..Default::default()
        },
    )
}

/// A plane with a fine black and white checker repeated `repeat` times (mipmap tests).
pub fn checker(repeat: f32) -> Vec<u8> {
    let base = texture(64, |u, v| {
        let on = ((u * 32.0).floor() as i32 + (v * 32.0).floor() as i32) % 2 == 0;
        if on { [255, 255, 255, 255] } else { [0, 0, 0, 255] }
    });
    glb(
        "checker",
        &quad(repeat),
        &Material {
            base: Some(base),
            mr: Some(texture(4, |_, _| [0, 255, 0, 255])),
            ..Default::default()
        },
    )
}
