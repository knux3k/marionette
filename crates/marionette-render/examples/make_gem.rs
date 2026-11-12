//! Writes `examples/assets/gem.glb`: a small textured glTF 2.0 model made entirely in code
//! (base color, metallic-roughness and normal textures embedded as PNG). The output is
//! dedicated to the public domain (CC0 1.0) like everything this generator makes.
//!
//! Run from the repository root: `cargo run -p marionette-render --example make_gem`

use glam::Vec3;
use marionette_render::assets::encode_png;
use serde_json::json;

fn texture(size: u32, f: impl Fn(f32, f32) -> [u8; 4]) -> Vec<u8> {
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

fn main() {
    // Octahedron with flat faces; each face maps to the same UV triangle.
    let top = Vec3::new(0.0, 0.7, 0.0);
    let bottom = Vec3::new(0.0, -0.7, 0.0);
    let ring = [
        Vec3::new(0.5, 0.0, 0.0),
        Vec3::new(0.0, 0.0, -0.5),
        Vec3::new(-0.5, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.5),
    ];
    let (mut pos, mut nrm, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::<u16>::new());
    for i in 0..4 {
        let (a, b) = (ring[i], ring[(i + 1) % 4]);
        for tri in [[a, b, top], [b, a, bottom]] {
            let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize();
            for (p, t) in tri.iter().zip([[0.0, 1.0], [1.0, 1.0], [0.5, 0.0]]) {
                idx.push(pos.len() as u16);
                pos.push(p.to_array());
                nrm.push(n.to_array());
                uv.push(t);
            }
        }
    }

    // Gold and teal bands; gold is metal, teal is glossy dielectric; ridged normal map.
    let band = |v: f32| ((v * 6.0).floor() as i32) % 2 == 0;
    let base = texture(64, |_, v| {
        if band(v) {
            [255, 196, 80, 255]
        } else {
            [40, 170, 160, 255]
        }
    });
    let mr = texture(64, |_, v| {
        if band(v) {
            [0, 90, 255, 255]
        } else {
            [0, 40, 0, 255]
        }
    });
    let normal = texture(64, |_, v| {
        let s = (v * 12.0 * std::f32::consts::TAU).sin() * 0.5;
        let n = Vec3::new(0.0, s, 1.0).normalize();
        let e = |c: f32| ((c * 0.5 + 0.5) * 255.0).round() as u8;
        [e(n.x), e(n.y), e(n.z), 255]
    });

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
    let v_pos = push(bytemuck::cast_slice(&pos), Some(34962));
    let v_nrm = push(bytemuck::cast_slice(&nrm), Some(34962));
    let v_uv = push(bytemuck::cast_slice(&uv), Some(34962));
    let v_idx = push(bytemuck::cast_slice(&idx), Some(34963));
    let images: Vec<usize> = [&base, &mr, &normal].iter().map(|png| push(png, None)).collect();
    pad(&mut bin, 0);

    let doc = json!({
        "asset": { "version": "2.0", "generator": "marionette make_gem example", "copyright": "CC0 1.0 (public domain)" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0, "name": "gem" }],
        "meshes": [{ "name": "gem", "primitives": [{
            "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 },
            "indices": 3,
            "material": 0
        }]}],
        "materials": [{
            "name": "gem",
            "pbrMetallicRoughness": {
                "baseColorTexture": { "index": 0 },
                "metallicRoughnessTexture": { "index": 1 },
                "metallicFactor": 1.0,
                "roughnessFactor": 1.0
            },
            "normalTexture": { "index": 2 }
        }],
        "samplers": [{ "magFilter": 9729, "minFilter": 9729 }],
        "textures": [{ "source": 0, "sampler": 0 }, { "source": 1, "sampler": 0 }, { "source": 2, "sampler": 0 }],
        "images": images.iter().map(|&v| json!({ "bufferView": v, "mimeType": "image/png" })).collect::<Vec<_>>(),
        "accessors": [
            { "bufferView": v_pos, "componentType": 5126, "count": pos.len(), "type": "VEC3",
              "min": [-0.5, -0.7, -0.5], "max": [0.5, 0.7, 0.5] },
            { "bufferView": v_nrm, "componentType": 5126, "count": nrm.len(), "type": "VEC3" },
            { "bufferView": v_uv, "componentType": 5126, "count": uv.len(), "type": "VEC2" },
            { "bufferView": v_idx, "componentType": 5123, "count": idx.len(), "type": "SCALAR" }
        ],
        "bufferViews": views,
        "buffers": [{ "byteLength": bin.len() }]
    });

    let mut json_bytes = serde_json::to_vec(&doc).expect("JSON serializes");
    pad(&mut json_bytes, b' ');
    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json_bytes);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin);

    std::fs::create_dir_all("examples/assets").expect("create examples/assets");
    std::fs::write("examples/assets/gem.glb", &glb).expect("write gem.glb");
    println!("wrote examples/assets/gem.glb ({} bytes)", glb.len());
}
