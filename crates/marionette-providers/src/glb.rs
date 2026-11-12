//! A minimal GLB writer: one box mesh with one PBR material. Used by the mock 3D provider so
//! its output loads in the engine like any generated model. Deterministic: same input, same
//! bytes.

use serde_json::json;

/// A box centered on the origin with the given half extents and linear RGBA base color.
pub fn box_glb(half: [f32; 3], color: [f32; 4], name: &str) -> Vec<u8> {
    let [hx, hy, hz] = half;
    // Faces: normal, then four corners counter-clockwise seen from outside.
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [1.0, 0.0, 0.0],
            [[hx, -hy, hz], [hx, -hy, -hz], [hx, hy, -hz], [hx, hy, hz]],
        ),
        (
            [-1.0, 0.0, 0.0],
            [[-hx, -hy, -hz], [-hx, -hy, hz], [-hx, hy, hz], [-hx, hy, -hz]],
        ),
        (
            [0.0, 1.0, 0.0],
            [[-hx, hy, hz], [hx, hy, hz], [hx, hy, -hz], [-hx, hy, -hz]],
        ),
        (
            [0.0, -1.0, 0.0],
            [[-hx, -hy, -hz], [hx, -hy, -hz], [hx, -hy, hz], [-hx, -hy, hz]],
        ),
        (
            [0.0, 0.0, 1.0],
            [[-hx, -hy, hz], [hx, -hy, hz], [hx, hy, hz], [-hx, hy, hz]],
        ),
        (
            [0.0, 0.0, -1.0],
            [[hx, -hy, -hz], [-hx, -hy, -hz], [-hx, hy, -hz], [hx, hy, -hz]],
        ),
    ];
    let mut positions = Vec::with_capacity(24 * 12);
    let mut normals = Vec::with_capacity(24 * 12);
    let mut indices = Vec::with_capacity(36 * 2);
    for (f, (n, corners)) in faces.iter().enumerate() {
        for c in corners {
            for v in c {
                positions.extend_from_slice(&v.to_le_bytes());
            }
            for v in n {
                normals.extend_from_slice(&v.to_le_bytes());
            }
        }
        let b = (f * 4) as u16;
        for i in [b, b + 1, b + 2, b, b + 2, b + 3] {
            indices.extend_from_slice(&i.to_le_bytes());
        }
    }
    let mut bin = Vec::new();
    bin.extend_from_slice(&positions);
    bin.extend_from_slice(&normals);
    bin.extend_from_slice(&indices);
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let (p_len, n_len, i_len) = (positions.len(), normals.len(), indices.len());
    let doc = json!({
        "asset": { "version": "2.0", "generator": "Marionette mock provider" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0, "name": name }],
        "meshes": [{
            "name": name,
            "primitives": [{
                "attributes": { "POSITION": 0, "NORMAL": 1 },
                "indices": 2,
                "material": 0
            }]
        }],
        "materials": [{
            "name": name,
            "pbrMetallicRoughness": {
                "baseColorFactor": color,
                "metallicFactor": 0.0,
                "roughnessFactor": 0.8
            }
        }],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": p_len, "target": 34962 },
            { "buffer": 0, "byteOffset": p_len, "byteLength": n_len, "target": 34962 },
            { "buffer": 0, "byteOffset": p_len + n_len, "byteLength": i_len, "target": 34963 }
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 24, "type": "VEC3",
              "min": [-hx, -hy, -hz], "max": [hx, hy, hz] },
            { "bufferView": 1, "componentType": 5126, "count": 24, "type": "VEC3" },
            { "bufferView": 2, "componentType": 5123, "count": 36, "type": "SCALAR" }
        ]
    });
    let mut json_chunk = serde_json::to_vec(&doc).expect("glTF JSON serializes");
    while !json_chunk.len().is_multiple_of(4) {
        json_chunk.push(b' ');
    }
    let total = 12 + 8 + json_chunk.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_chunk.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_chunk);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}
