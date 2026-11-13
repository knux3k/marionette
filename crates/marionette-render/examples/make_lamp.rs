//! Writes `examples/assets/lamp.glb`: a street lamp made in code, for glTF emissive import
//! (decision 0067). A dark metal post and a lantern whose material glows: `emissiveFactor`
//! times an emissive texture (four lit panes in a frame), with
//! `KHR_materials_emissive_strength` raising it above 1 so bloom picks it up. The output is
//! dedicated to the public domain (CC0 1.0) like everything this generator makes.
//!
//! Run from the repository root: `cargo run -p marionette-render --example make_lamp`

use glam::Vec3;
use marionette_render::assets::encode_png;
use serde_json::json;

fn pad(buf: &mut Vec<u8>, byte: u8) {
    while !buf.len().is_multiple_of(4) {
        buf.push(byte);
    }
}

/// Axis-aligned box: positions, normals, uvs (each face the whole 0..1 square), indices.
type Mesh = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u16>);

fn cuboid(min: Vec3, max: Vec3) -> Mesh {
    let (mut p, mut n, mut t, mut i) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let c = (min + max) * 0.5;
    let h = (max - min) * 0.5;
    for axis in 0..3 {
        for sign in [1.0f32, -1.0] {
            let mut normal = Vec3::ZERO;
            normal[axis] = sign;
            // Two axes spanning the face with u x v = normal, so the winding faces out.
            let u = if axis == 1 { Vec3::X } else { Vec3::Y.cross(normal) };
            let v = normal.cross(u);
            let base = p.len() as u16;
            for (a, b, uv) in [
                (-1.0, -1.0, [0.0, 1.0]),
                (1.0, -1.0, [1.0, 1.0]),
                (1.0, 1.0, [1.0, 0.0]),
                (-1.0, 1.0, [0.0, 0.0]),
            ] {
                let q = c + normal * h + (u * a + v * b) * h;
                p.push(q.to_array());
                n.push(normal.to_array());
                t.push(uv);
            }
            i.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    (p, n, t, i)
}

fn main() {
    let post = cuboid(Vec3::new(-0.06, 0.0, -0.06), Vec3::new(0.06, 2.2, 0.06));
    let head = cuboid(Vec3::new(-0.22, 2.2, -0.22), Vec3::new(0.22, 2.7, 0.22));

    // Emissive texture: four bright panes inside a dark frame (white = full emissive).
    let size = 32u32;
    let mut rgba = Vec::new();
    for y in 0..size {
        for x in 0..size {
            let f = |c: u32| c < 3 || c >= size - 3 || (size / 2 - 1..=size / 2).contains(&c);
            let lit = !(f(x) || f(y));
            let g = if lit { 255 } else { 20 };
            rgba.extend_from_slice(&[g, g, g, 255]);
        }
    }
    let png = encode_png(size, size, &rgba);

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
    let mut accessors = Vec::new();
    let mut primitive = |mesh: &Mesh, material: usize, min: [f32; 3], max: [f32; 3]| {
        let first = accessors.len();
        let vp = push(bytemuck::cast_slice(&mesh.0), Some(34962));
        let vn = push(bytemuck::cast_slice(&mesh.1), Some(34962));
        let vt = push(bytemuck::cast_slice(&mesh.2), Some(34962));
        let vi = push(bytemuck::cast_slice(&mesh.3), Some(34963));
        let count = mesh.0.len();
        accessors.push(json!({ "bufferView": vp, "componentType": 5126, "count": count, "type": "VEC3", "min": min, "max": max }));
        accessors.push(json!({ "bufferView": vn, "componentType": 5126, "count": count, "type": "VEC3" }));
        accessors.push(json!({ "bufferView": vt, "componentType": 5126, "count": count, "type": "VEC2" }));
        accessors.push(
            json!({ "bufferView": vi, "componentType": 5123, "count": mesh.3.len(), "type": "SCALAR" }),
        );
        json!({
            "attributes": { "POSITION": first, "NORMAL": first + 1, "TEXCOORD_0": first + 2 },
            "indices": first + 3,
            "material": material
        })
    };
    let prims = vec![
        primitive(&post, 0, [-0.06, 0.0, -0.06], [0.06, 2.2, 0.06]),
        primitive(&head, 1, [-0.22, 2.2, -0.22], [0.22, 2.7, 0.22]),
    ];
    let image = push(&png, None);
    pad(&mut bin, 0);

    let doc = json!({
        "asset": { "version": "2.0", "generator": "marionette make_lamp example", "copyright": "CC0 1.0 (public domain)" },
        "extensionsUsed": ["KHR_materials_emissive_strength"],
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0, "name": "lamp" }],
        "meshes": [{ "name": "lamp", "primitives": prims }],
        "materials": [
            {
                "name": "iron",
                "pbrMetallicRoughness": { "baseColorFactor": [0.08, 0.08, 0.09, 1.0], "metallicFactor": 1.0, "roughnessFactor": 0.45 }
            },
            {
                "name": "lantern",
                "pbrMetallicRoughness": { "baseColorFactor": [0.9, 0.85, 0.7, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.3 },
                "emissiveFactor": [1.0, 0.72, 0.38],
                "emissiveTexture": { "index": 0 },
                "extensions": { "KHR_materials_emissive_strength": { "emissiveStrength": 4.0 } }
            }
        ],
        "samplers": [{ "magFilter": 9729, "minFilter": 9729 }],
        "textures": [{ "source": 0, "sampler": 0 }],
        "images": [{ "bufferView": image, "mimeType": "image/png" }],
        "accessors": accessors,
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
    std::fs::write("examples/assets/lamp.glb", &glb).expect("write lamp.glb");
    println!("wrote examples/assets/lamp.glb ({} bytes)", glb.len());
}
