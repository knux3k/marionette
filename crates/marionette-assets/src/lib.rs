//! CPU-side assets: meshes, materials and images, from glTF files, PNG files, or built in.
//! Nothing here touches the GPU: the renderer uploads these, and physics builds mesh and
//! convex colliders from the same meshes.

pub use glam;

pub mod font;
pub mod hdr;
pub mod rig;
use glam::{Mat4, Vec2, Vec3};
pub use hdr::{HdrImage, is_hdr_name, load_hdr_image};
pub use rig::{Rig, load_rig};
use std::path::Path;

/// RGBA8 image. `srgb` says how the color bytes are encoded (true for colors, false for
/// data such as normal maps).
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub srgb: bool,
}

impl Image {
    pub fn solid(rgba: [u8; 4], srgb: bool) -> Self {
        Self {
            width: 1,
            height: 1,
            rgba: rgba.to_vec(),
            srgb,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// Up to four joints per vertex (indices into the part's skin). Empty when not skinned.
    pub joints: Vec<[u16; 4]>,
    /// Weights of `joints`, summing to 1. Empty when not skinned.
    pub weights: Vec<[f32; 4]>,
    /// Morph targets (blend shapes): per-vertex deltas added with the node's morph weights.
    pub morph_targets: Vec<MorphTarget>,
}

/// One morph target: position and normal offsets per vertex (normals may be empty).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MorphTarget {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaterialData {
    /// Linear RGBA factor.
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    /// Indices into `ModelData::images`.
    pub base_color_texture: Option<usize>,
    pub metallic_roughness_texture: Option<usize>,
    pub normal_texture: Option<usize>,
    pub normal_scale: f32,
    /// Light given off (linear RGB): glTF `emissiveFactor` times
    /// `KHR_materials_emissive_strength` (decision 0067). Zero for most materials.
    pub emissive: [f32; 3],
    /// sRGB image multiplying `emissive`.
    pub emissive_texture: Option<usize>,
}

impl Default for MaterialData {
    fn default() -> Self {
        Self {
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness: 0.5,
            base_color_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
            normal_scale: 1.0,
            emissive: [0.0; 3],
            emissive_texture: None,
        }
    }
}

/// One drawable piece of a model: a mesh, its material, and its transform within the model.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub mesh: MeshData,
    pub material: usize,
    /// Rest-pose model-space transform of the part's node.
    pub transform: Mat4,
    /// The glTF node this part hangs from (index into `ModelData::rig.nodes`); `None` for
    /// built-in shapes. An animated pose replaces `transform` with that node's matrix.
    pub node: Option<usize>,
    /// Skin (index into `ModelData::rig.skins`) when the mesh is skinned. A skinned part
    /// ignores its node transform: joints place every vertex.
    pub skin: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelData {
    pub parts: Vec<Part>,
    pub materials: Vec<MaterialData>,
    pub images: Vec<Image>,
    /// Nodes, skins and animation clips.
    pub rig: Rig,
}

impl ModelData {
    fn single(mesh: MeshData) -> Self {
        Self {
            parts: vec![Part {
                mesh,
                material: 0,
                transform: Mat4::IDENTITY,
                node: None,
                skin: None,
            }],
            materials: vec![MaterialData::default()],
            images: Vec::new(),
            rig: Rig::default(),
        }
    }
}

/// Loads a model by its scene `asset` string (see `marionette_core::spatial::Model`).
pub fn load_model(asset: &str, base_dir: &Path) -> Result<ModelData, String> {
    if let Some(shape) = asset.strip_prefix("primitive:") {
        return primitive(shape)
            .ok_or_else(|| format!("unknown primitive `{shape}`; built-in shapes are cube, sphere, plane"));
    }
    load_gltf(&base_dir.join(asset))
}

/// Built-in shapes, all centered on the origin with 1 m extent.
pub fn primitive(name: &str) -> Option<ModelData> {
    let mesh = match name {
        "cube" => cube(),
        "sphere" => uv_sphere(32, 16),
        "plane" => plane(),
        _ => return None,
    };
    Some(ModelData::single(mesh))
}

fn cube() -> MeshData {
    let mut m = MeshData::default();
    // (normal, u axis, v axis) for each face.
    let faces = [
        (Vec3::X, Vec3::NEG_Z, Vec3::NEG_Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::NEG_Y),
        (Vec3::Y, Vec3::X, Vec3::Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::NEG_Z),
        (Vec3::Z, Vec3::X, Vec3::NEG_Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::NEG_Y),
    ];
    for (n, u, v) in faces {
        let base = m.positions.len() as u32;
        for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = (n + u * su + v * sv) * 0.5;
            m.positions.push(p.to_array());
            m.normals.push(n.to_array());
            m.uvs.push([(su + 1.0) * 0.5, (sv + 1.0) * 0.5]);
        }
        // Counter-clockwise when seen from outside.
        let (a, b, c) = if u.cross(v).dot(n) > 0.0 {
            (1, 2, 3)
        } else {
            (3, 2, 1)
        };
        m.indices
            .extend_from_slice(&[base, base + a, base + b, base, base + b, base + c]);
    }
    m
}

fn plane() -> MeshData {
    MeshData {
        positions: vec![
            [-0.5, 0.0, 0.5],
            [0.5, 0.0, 0.5],
            [0.5, 0.0, -0.5],
            [-0.5, 0.0, -0.5],
        ],
        normals: vec![[0.0, 1.0, 0.0]; 4],
        uvs: vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        indices: vec![0, 1, 2, 0, 2, 3],
        ..Default::default()
    }
}

fn uv_sphere(segments: u32, rings: u32) -> MeshData {
    let mut m = MeshData::default();
    for r in 0..=rings {
        let v = r as f32 / rings as f32;
        let theta = v * std::f32::consts::PI;
        for s in 0..=segments {
            let u = s as f32 / segments as f32;
            let phi = u * std::f32::consts::TAU;
            let n = Vec3::new(theta.sin() * phi.cos(), theta.cos(), -theta.sin() * phi.sin());
            m.positions.push((n * 0.5).to_array());
            m.normals.push(n.to_array());
            m.uvs.push([u, v]);
        }
    }
    let stride = segments + 1;
    for r in 0..rings {
        for s in 0..segments {
            let a = r * stride + s;
            let b = a + stride;
            m.indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    m
}

/// Loads every mesh of the default scene of a glTF 2.0 file (`.gltf` or `.glb`).
pub fn load_gltf(path: &Path) -> Result<ModelData, String> {
    let (doc, buffers, images) =
        gltf::import(path).map_err(|e| format!("cannot load glTF {}: {e}", path.display()))?;

    // An image can be used as color (sRGB) or as data (linear); record which.
    let mut srgb = vec![false; images.len()];
    for mat in doc.materials() {
        if let Some(info) = mat.pbr_metallic_roughness().base_color_texture() {
            srgb[info.texture().source().index()] = true;
        }
        if let Some(info) = mat.emissive_texture() {
            srgb[info.texture().source().index()] = true;
        }
    }
    let images: Vec<Image> = images.iter().zip(srgb).map(|(d, s)| gltf_image(d, s)).collect();

    let mut model = ModelData {
        images,
        rig: rig::rig_from(&doc, &buffers)?,
        ..Default::default()
    };
    for mat in doc.materials() {
        let pbr = mat.pbr_metallic_roughness();
        model.materials.push(MaterialData {
            base_color: pbr.base_color_factor(),
            metallic: pbr.metallic_factor(),
            roughness: pbr.roughness_factor(),
            base_color_texture: pbr.base_color_texture().map(|t| t.texture().source().index()),
            metallic_roughness_texture: pbr
                .metallic_roughness_texture()
                .map(|t| t.texture().source().index()),
            normal_texture: mat.normal_texture().map(|t| t.texture().source().index()),
            normal_scale: mat.normal_texture().map_or(1.0, |t| t.scale()),
            emissive: {
                let strength = mat.emissive_strength().unwrap_or(1.0).max(0.0);
                mat.emissive_factor().map(|c| c * strength)
            },
            emissive_texture: mat.emissive_texture().map(|t| t.texture().source().index()),
        });
    }
    let default_material = model.materials.len();
    model.materials.push(MaterialData::default());

    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .ok_or("glTF file has no scene")?;
    let mut stack: Vec<(gltf::Node, Mat4)> = scene.nodes().map(|n| (n, Mat4::IDENTITY)).collect();
    while let Some((node, parent)) = stack.pop() {
        let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            for prim in mesh.primitives() {
                if prim.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let reader = prim.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
                let positions: Vec<[f32; 3]> = reader
                    .read_positions()
                    .ok_or("primitive has no positions")?
                    .collect();
                let indices: Vec<u32> = match reader.read_indices() {
                    Some(i) => i.into_u32().collect(),
                    None => (0..positions.len() as u32).collect(),
                };
                let normals = match reader.read_normals() {
                    Some(n) => n.collect(),
                    None => flat_normals(&positions, &indices),
                };
                let uvs = match reader.read_tex_coords(0) {
                    Some(t) => t.into_f32().collect(),
                    None => vec![[0.0, 0.0]; positions.len()],
                };
                let skin = node.skin().map(|s| s.index());
                let (mut joints, mut weights) = (Vec::new(), Vec::new());
                if skin.is_some()
                    && let (Some(j), Some(w)) = (reader.read_joints(0), reader.read_weights(0))
                {
                    joints = j.into_u16().collect();
                    weights = w.into_f32().map(normalize_weights).collect();
                }
                let skin =
                    skin.filter(|_| joints.len() == positions.len() && weights.len() == positions.len());
                if skin.is_none() {
                    joints.clear();
                    weights.clear();
                }
                let n = positions.len();
                let morph_targets = reader
                    .read_morph_targets()
                    .map(|(p, nrm, _)| MorphTarget {
                        positions: p.map_or_else(|| vec![[0.0; 3]; n], Iterator::collect),
                        normals: nrm.map_or_else(Vec::new, Iterator::collect),
                    })
                    .collect();
                model.parts.push(Part {
                    mesh: MeshData {
                        positions,
                        normals,
                        uvs,
                        indices,
                        joints,
                        weights,
                        morph_targets,
                    },
                    material: prim.material().index().unwrap_or(default_material),
                    transform: world,
                    node: Some(node.index()),
                    skin,
                });
            }
        }
        stack.extend(node.children().map(|c| (c, world)));
    }
    if model.parts.is_empty() {
        return Err(format!("glTF {} has no triangle meshes", path.display()));
    }
    Ok(model)
}

/// Scales skin weights to sum to 1 (files are allowed to be slightly off).
fn normalize_weights(w: [f32; 4]) -> [f32; 4] {
    let sum = w[0] + w[1] + w[2] + w[3];
    if sum > 0.0 {
        w.map(|x| x / sum)
    } else {
        [1.0, 0.0, 0.0, 0.0]
    }
}

/// Per-vertex normals averaged from faces (used when a glTF mesh has none).
fn flat_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut acc = vec![Vec3::ZERO; positions.len()];
    for tri in indices.as_chunks::<3>().0.iter() {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| Vec3::from(positions[i as usize]));
        let n = (b - a).cross(c - a);
        for &i in tri {
            acc[i as usize] += n;
        }
    }
    acc.into_iter()
        .map(|n| n.normalize_or(Vec3::Y).to_array())
        .collect()
}

fn gltf_image(d: &gltf::image::Data, srgb: bool) -> Image {
    use gltf::image::Format;
    let n = (d.width * d.height) as usize;
    let mut rgba = Vec::with_capacity(n * 4);
    let px = &d.pixels;
    match d.format {
        Format::R8 => px.iter().for_each(|&r| rgba.extend_from_slice(&[r, r, r, 255])),
        Format::R8G8 => px
            .as_chunks::<2>()
            .0
            .iter()
            .for_each(|c| rgba.extend_from_slice(&[c[0], c[1], 0, 255])),
        Format::R8G8B8 => px
            .as_chunks::<3>()
            .0
            .iter()
            .for_each(|c| rgba.extend_from_slice(&[c[0], c[1], c[2], 255])),
        Format::R8G8B8A8 => rgba.extend_from_slice(px),
        _ => {
            // 16-bit and float formats: not used by our assets yet. Fall back to white so the
            // model still renders; the loader is the place to extend.
            rgba.resize(n * 4, 255);
        }
    }
    Image {
        width: d.width,
        height: d.height,
        rgba,
        srgb,
    }
}

/// Loads a sprite image by its scene `image` string (see `marionette_core::spatial::Sprite`).
pub fn load_image(name: &str, base_dir: &Path) -> Result<Image, String> {
    if let Some(builtin) = name.strip_prefix("builtin:") {
        return builtin_image(builtin).ok_or_else(|| {
            format!("unknown built-in image `{builtin}`; built-ins are white, circle, checker, atlas, font")
        });
    }
    let path = base_dir.join(name);
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    decode_png(&bytes).map_err(|e| format!("cannot decode {}: {e}", path.display()))
}

/// Decodes a PNG into sRGB RGBA8.
pub fn decode_png(bytes: &[u8]) -> Result<Image, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("image too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let buf = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf.to_vec(),
        png::ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed PNG was not expanded".into()),
    };
    Ok(Image {
        width: info.width,
        height: info.height,
        rgba,
        srgb: true,
    })
}

/// Encodes RGBA8 pixels as PNG. Deterministic: same pixels, same bytes.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().expect("PNG header writes to memory");
        writer.write_image_data(rgba).expect("PNG data writes to memory");
    }
    out
}

/// Procedural images, so examples need no binary files.
pub fn builtin_image(name: &str) -> Option<Image> {
    let make = |w: u32, h: u32, f: &dyn Fn(u32, u32) -> [u8; 4]| {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&f(x, y));
            }
        }
        Image {
            width: w,
            height: h,
            rgba,
            srgb: true,
        }
    };
    Some(match name {
        "white" => Image::solid([255; 4], true),
        "circle" => make(64, 64, &|x, y| disc(x, y, 32.0, 31.0, [255, 255, 255])),
        "checker" => make(256, 256, &|x, y| {
            if (x / 32 + y / 32) % 2 == 0 {
                [235, 235, 235, 255]
            } else {
                [60, 60, 70, 255]
            }
        }),
        "font" => font::atlas(),
        // 4x2 atlas of 32x32 cells: row 0 = colored discs, row 1 = framed tiles.
        "atlas" => make(128, 64, &|x, y| {
            const COLORS: [[u8; 3]; 4] = [[230, 80, 70], [90, 190, 90], [80, 140, 230], [240, 200, 60]];
            let (cx, cy) = (x / 32, y / 32);
            let (lx, ly) = (x % 32, y % 32);
            let c = COLORS[cx as usize];
            if cy == 0 {
                disc(lx, ly, 16.0, 14.0, c)
            } else if lx < 2 || ly < 2 || lx > 29 || ly > 29 {
                [20, 20, 30, 255]
            } else {
                [c[0] / 2 + 60, c[1] / 2 + 60, c[2] / 2 + 60, 255]
            }
        }),
        _ => return None,
    })
}

/// Anti-aliased disc of radius `r` centered in a cell of size `2 * half`.
fn disc(x: u32, y: u32, half: f32, r: f32, c: [u8; 3]) -> [u8; 4] {
    let d = Vec2::new(x as f32 + 0.5 - half, y as f32 + 0.5 - half).length();
    let a = (r + 0.5 - d).clamp(0.0, 1.0);
    [c[0], c[1], c[2], (a * 255.0).round() as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_are_well_formed() {
        for name in ["cube", "sphere", "plane"] {
            let m = primitive(name).unwrap();
            let mesh = &m.parts[0].mesh;
            assert_eq!(mesh.positions.len(), mesh.normals.len());
            assert_eq!(mesh.positions.len(), mesh.uvs.len());
            assert_eq!(mesh.indices.len() % 3, 0);
            assert!(mesh.indices.iter().all(|&i| (i as usize) < mesh.positions.len()));
        }
    }

    #[test]
    fn cube_faces_point_outward() {
        let m = cube();
        for tri in m.indices.as_chunks::<3>().0.iter() {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| Vec3::from(m.positions[i as usize]));
            let n = (b - a).cross(c - a);
            assert!(n.dot(a) > 0.0, "inward-facing triangle");
        }
    }

    #[test]
    fn png_round_trip() {
        let img = builtin_image("atlas").unwrap();
        let bytes = encode_png(img.width, img.height, &img.rgba);
        assert_eq!(decode_png(&bytes).unwrap(), img);
        assert_eq!(bytes, encode_png(img.width, img.height, &img.rgba));
    }
}
