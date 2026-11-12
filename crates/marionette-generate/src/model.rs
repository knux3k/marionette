//! Model post-processing: check that the GLB is valid glTF 2.0, measure it (bounds through
//! the node hierarchy, triangles, meshes, materials, textures, skins, animations), and
//! normalize it for the engine by adding one root node that grounds the model (lowest point at
//! y = 0), centers it on x and z, and optionally scales it to a height in meters. Before that,
//! a model over its triangle budget or texture size is slimmed ([`crate::slim`], decision
//! 0048); otherwise mesh data is never rewritten. The result is deterministic.

use crate::codes;
use crate::recipe::ModelOptions;
use glam::{DMat4, DVec3};
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};

/// Name of the root node added by normalization.
pub const ROOT_NAME: &str = "marionette_generated_root";

/// What post-processing found and did.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ModelReport {
    /// Bounds before normalization, meters: `[min, max]`.
    pub source_bounds: [[f64; 3]; 2],
    /// Bounds after normalization.
    pub bounds: [[f64; 3]; 2],
    /// Uniform scale applied (1 when no height was asked for).
    pub scale: f64,
    pub triangles: u64,
    pub meshes: usize,
    pub materials: usize,
    pub textures: usize,
    pub skins: usize,
    pub animations: usize,
    /// Problems worth a person's attention (for example a triangle budget exceeded).
    pub notes: Vec<String>,
}

fn post_err(msg: impl Into<String>) -> Errors {
    Diagnostic::error(codes::POSTPROCESS, msg)
        .fix("regenerate the asset; if it keeps failing, report the provider's output format")
        .into()
}

pub(crate) struct Glb {
    pub json: Value,
    pub bin: Option<Vec<u8>>,
}

pub(crate) fn read_glb(bytes: &[u8]) -> Result<Glb, Errors> {
    let u32_at = |o: usize| -> Option<u32> {
        bytes
            .get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if !bytes.starts_with(b"glTF") || u32_at(4) != Some(2) {
        return Err(post_err("the model is not a GLB (glTF 2.0 binary)"));
    }
    let total = u32_at(8).unwrap_or(0) as usize;
    if total > bytes.len() || total < 20 {
        return Err(post_err("the GLB's length field does not match the file"));
    }
    let mut off = 12;
    let mut json = None;
    let mut bin = None;
    while off + 8 <= total {
        let len = u32_at(off).unwrap() as usize;
        let ty = u32_at(off + 4).unwrap();
        let start = off + 8;
        let end = start
            .checked_add(len)
            .filter(|&e| e <= total)
            .ok_or_else(|| post_err("a GLB chunk runs past the end of the file"))?;
        match ty {
            0x4E4F_534A if json.is_none() => {
                json = Some(
                    serde_json::from_slice::<Value>(&bytes[start..end])
                        .map_err(|e| post_err(format!("the GLB's JSON chunk is invalid: {e}")))?,
                )
            }
            0x004E_4942 if bin.is_none() => bin = Some(bytes[start..end].to_vec()),
            _ => {}
        }
        off = end;
    }
    let json = json.ok_or_else(|| post_err("the GLB has no JSON chunk"))?;
    Ok(Glb { json, bin })
}

pub(crate) fn write_glb(json: &Value, bin: Option<&[u8]>) -> Vec<u8> {
    let mut j = serde_json::to_vec(json).expect("JSON values serialize");
    while !j.len().is_multiple_of(4) {
        j.push(b' ');
    }
    let mut b = bin.map(<[u8]>::to_vec);
    if let Some(b) = &mut b {
        while !b.len().is_multiple_of(4) {
            b.push(0);
        }
    }
    let total = 12 + 8 + j.len() + b.as_ref().map_or(0, |b| 8 + b.len());
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(j.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(&j);
    if let Some(b) = b {
        out.extend_from_slice(&(b.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
        out.extend_from_slice(&b);
    }
    out
}

fn local_matrix(node: &gltf::Node) -> DMat4 {
    let m = node.transform().matrix();
    DMat4::from_cols_array_2d(&m.map(|c| c.map(f64::from)))
}

/// Bounds of every mesh in the default scene, through node transforms, from the POSITION
/// accessors' `min`/`max` (required by glTF 2.0). Also counts triangles.
fn measure(doc: &gltf::Document) -> Result<([DVec3; 2], u64), Errors> {
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .ok_or_else(|| post_err("the model has no scene"))?;
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = DVec3::splat(f64::NEG_INFINITY);
    let mut triangles = 0u64;
    let mut stack: Vec<(gltf::Node, DMat4)> = scene.nodes().map(|n| (n, DMat4::IDENTITY)).collect();
    let mut visited = 0usize;
    while let Some((node, parent)) = stack.pop() {
        visited += 1;
        if visited > 100_000 {
            return Err(post_err("the node hierarchy is too deep or cyclic"));
        }
        let world = parent * local_matrix(&node);
        if let Some(mesh) = node.mesh() {
            for prim in mesh.primitives() {
                let Some(pos) = prim.get(&gltf::Semantic::Positions) else {
                    continue;
                };
                let (Some(min), Some(max)) = (pos.min(), pos.max()) else {
                    return Err(post_err("a POSITION accessor has no min/max"));
                };
                let v = |x: &Value| -> Result<DVec3, Errors> {
                    let a = x
                        .as_array()
                        .filter(|a| a.len() == 3)
                        .ok_or_else(|| post_err("bad POSITION bounds"))?;
                    let f = |i: usize| a[i].as_f64().ok_or_else(|| post_err("bad POSITION bounds"));
                    Ok(DVec3::new(f(0)?, f(1)?, f(2)?))
                };
                let (mn, mx) = (v(&min)?, v(&max)?);
                for i in 0..8 {
                    let c = DVec3::new(
                        if i & 1 == 0 { mn.x } else { mx.x },
                        if i & 2 == 0 { mn.y } else { mx.y },
                        if i & 4 == 0 { mn.z } else { mx.z },
                    );
                    let p = world.transform_point3(c);
                    lo = lo.min(p);
                    hi = hi.max(p);
                }
                if prim.mode() == gltf::mesh::Mode::Triangles {
                    let n = prim.indices().map_or(pos.count(), |i| i.count());
                    triangles += (n / 3) as u64;
                }
            }
        }
        stack.extend(node.children().map(|c| (c, world)));
    }
    if !lo.is_finite() || !hi.is_finite() {
        return Err(post_err("the model has no meshes with positions"));
    }
    Ok(([lo, hi], triangles))
}

/// Rounds to 1e-6 so reports do not carry float noise.
fn r6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

fn arr(v: DVec3) -> [f64; 3] {
    [r6(v.x), r6(v.y), r6(v.z)]
}

/// Validates, measures and normalizes a generated GLB.
pub fn process(glb: &[u8], options: Option<&ModelOptions>) -> Result<(Vec<u8>, ModelReport), Errors> {
    let opts = options.cloned().unwrap_or_default();
    let slimmed = crate::slim::slim(glb, opts.max_triangles, opts.max_texture_px);
    let mut notes = slimmed.notes;
    let glb: &[u8] = slimmed.glb.as_deref().unwrap_or(glb);
    let gltf =
        gltf::Gltf::from_slice(glb).map_err(|e| post_err(format!("the model is not valid glTF: {e}")))?;
    let doc = &gltf.document;
    if doc
        .buffers()
        .any(|b| matches!(b.source(), gltf::buffer::Source::Uri(_)))
        || doc
            .images()
            .any(|i| matches!(i.source(), gltf::image::Source::Uri { .. }))
    {
        return Err(post_err(
            "the model refers to external files; a generated GLB must be self-contained",
        ));
    }
    let ([lo, hi], triangles) = measure(doc)?;
    let size = hi - lo;
    let scale = match opts.height_m {
        Some(h) if size.y > 1e-9 => h / size.y,
        Some(_) => {
            return Err(post_err(
                "the model is flat (zero height), so it cannot be scaled to a height",
            ));
        }
        None => 1.0,
    };
    // Ground and center: lowest point to y = 0, middle of x and z to 0, after scaling.
    let center = (lo + hi) * 0.5;
    let translation = DVec3::new(-center.x * scale, -lo.y * scale, -center.z * scale);
    let new_lo = lo * scale + translation;
    let new_hi = hi * scale + translation;

    if let Some(max) = opts.max_triangles
        && triangles > max
    {
        notes.push(format!("{triangles} triangles is still over the budget of {max}"));
    }

    let Glb { mut json, bin } = read_glb(glb)?;
    let scene_index = json.get("scene").and_then(Value::as_u64).unwrap_or(0) as usize;
    let nodes_len = json.get("nodes").and_then(Value::as_array).map_or(0, Vec::len);
    let roots: Vec<Value> = json
        .pointer(&format!("/scenes/{scene_index}/nodes"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let root = json!({
        "name": ROOT_NAME,
        "children": roots,
        "translation": [r6(translation.x), r6(translation.y), r6(translation.z)],
        "scale": [r6(scale), r6(scale), r6(scale)],
    });
    match json.get_mut("nodes").and_then(Value::as_array_mut) {
        Some(n) => n.push(root),
        None => json["nodes"] = json!([root]),
    }
    json["scenes"][scene_index]["nodes"] = json!([nodes_len]);
    json["scene"] = json!(scene_index);
    let out = write_glb(&json, bin.as_deref());
    // The result must still be valid glTF.
    gltf::Gltf::from_slice(&out)
        .map_err(|e| post_err(format!("normalization produced invalid glTF: {e}")))?;

    Ok((
        out,
        ModelReport {
            source_bounds: [arr(lo), arr(hi)],
            bounds: [arr(new_lo), arr(new_hi)],
            scale: r6(scale),
            triangles,
            meshes: doc.meshes().count(),
            materials: doc.materials().count(),
            textures: doc.textures().count(),
            skins: doc.skins().count(),
            animations: doc.animations().count(),
            notes,
        },
    ))
}
