//! What a model's normal map and its metallic-roughness texture do to the picture, checked on
//! pixels: a normal tilted toward the light is brighter and one tilted away is darker, on both
//! texture axes; roughness is read from green and metallic from blue. The model and its
//! textures are written by the test. Skips (passes with a note) when no GPU adapter exists,
//! unless MARIONETTE_REQUIRE_GPU=1.

use marionette_core::{Sim, default_registry};
use marionette_render::{Gpu, Offscreen, extract};
use std::path::{Path, PathBuf};

const W: u32 = 160;
const H: u32 = 90;

fn gpu() -> Option<Gpu> {
    match Gpu::headless() {
        Ok(g) => Some(g),
        Err(e) if std::env::var("MARIONETTE_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipping GPU test: {e}");
            None
        }
        Err(e) => panic!("GPU required but unavailable: {e}"),
    }
}

/// Writes a 64 x 64 PNG whose four quarters have the given colors: top left, top right,
/// bottom left, bottom right (the first row of the file is the top).
fn quarters(path: &Path, colors: [[u8; 3]; 4]) {
    let mut data = Vec::with_capacity(64 * 64 * 3);
    for y in 0..64 {
        for x in 0..64 {
            data.extend_from_slice(&colors[(y / 32) * 2 + x / 32]);
        }
    }
    let mut enc = png::Encoder::new(std::fs::File::create(path).unwrap(), 64, 64);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&data).unwrap();
}

/// A 2 m square in the XY plane facing +Z, with texture coordinates as glTF has them: (0, 0)
/// at the top left corner, v growing downward. `material` is the glTF material's JSON.
fn quad(dir: &Path, material: &str, images: &[&str]) {
    let positions: [[f32; 3]; 4] = [
        [-1.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
        [-1.0, -1.0, 0.0],
        [1.0, -1.0, 0.0],
    ];
    let uvs: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
    let indices: [u16; 6] = [0, 2, 1, 1, 2, 3];
    let mut bin = Vec::new();
    for p in positions {
        p.iter().for_each(|f| bin.extend_from_slice(&f.to_le_bytes()));
    }
    for _ in 0..4 {
        [0.0f32, 0.0, 1.0]
            .iter()
            .for_each(|f| bin.extend_from_slice(&f.to_le_bytes()));
    }
    for uv in uvs {
        uv.iter().for_each(|f| bin.extend_from_slice(&f.to_le_bytes()));
    }
    indices
        .iter()
        .for_each(|i| bin.extend_from_slice(&i.to_le_bytes()));
    std::fs::write(dir.join("quad.bin"), &bin).unwrap();
    let images_json: Vec<String> = images.iter().map(|i| format!(r#"{{"uri": "{i}"}}"#)).collect();
    let textures: Vec<String> = (0..images.len())
        .map(|i| format!(r#"{{"source": {i}, "sampler": 0}}"#))
        .collect();
    let gltf = format!(
        r#"{{"asset": {{"version": "2.0"}}, "scene": 0, "scenes": [{{"nodes": [0]}}], "nodes": [{{"mesh": 0}}],
        "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}}, "indices": 3, "material": 0}}]}}],
        "materials": [{material}],
        "samplers": [{{"magFilter": 9728, "minFilter": 9728}}],
        "images": [{}], "textures": [{}],
        "buffers": [{{"uri": "quad.bin", "byteLength": {}}}],
        "bufferViews": [
          {{"buffer": 0, "byteOffset": 0, "byteLength": 48}}, {{"buffer": 0, "byteOffset": 48, "byteLength": 48}},
          {{"buffer": 0, "byteOffset": 96, "byteLength": 32}}, {{"buffer": 0, "byteOffset": 128, "byteLength": 12}}],
        "accessors": [
          {{"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [-1, -1, 0], "max": [1, 1, 0]}},
          {{"bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC3"}},
          {{"bufferView": 2, "componentType": 5126, "count": 4, "type": "VEC2"}},
          {{"bufferView": 3, "componentType": 5123, "count": 6, "type": "SCALAR"}}]}}"#,
        images_json.join(", "),
        textures.join(", "),
        bin.len()
    );
    std::fs::write(dir.join("quad.gltf"), gltf).unwrap();
}

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-material-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Renders the quad square-on (orthographic, 2 m high, so it fills the height of the image)
/// under one light travelling along `light`, with no ambient light.
fn render(gpu: &Gpu, dir: &Path, light: [f32; 3]) -> Vec<u8> {
    let text = format!(
        r#"{{"format": "marionette.scene", "version": 1,
        "resources": {{ "AmbientLight": {{"color": {{"r": 1, "g": 1, "b": 1, "a": 1}}, "intensity": 0}} }},
        "entities": [
          {{"id": 1, "components": {{
            "Camera": {{"clear_color": {{"r": 0, "g": 0, "b": 0, "a": 1}},
              "projection": {{"Orthographic": {{"height": 2.0, "near": 0.1, "far": 10.0}}}}}},
            "Transform": {{"translation": {{"x": 0, "y": 0, "z": 3}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 1, "y": 1, "z": 1}}}}}}}},
          {{"id": 2, "components": {{ "Model": {{"asset": "quad.gltf"}},
            "Transform": {{"translation": {{"x": 0, "y": 0, "z": 0}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 1, "y": 1, "z": 1}}}}}}}},
          {{"id": 3, "components": {{
            "DirectionalLight": {{"direction": {{"x": {}, "y": {}, "z": {}}}, "color": {{"r": 1, "g": 1, "b": 1, "a": 1}}, "illuminance": 0.8}},
            "Transform": {{"translation": {{"x": 0, "y": 0, "z": 0}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 1, "y": 1, "z": 1}}}}}}}}
        ]}}"#,
        light[0], light[1], light[2]
    );
    let sim = Sim::from_text(&text, None, default_registry(), 1).unwrap();
    let mut off = Offscreen::new(gpu, W, H, 1, dir.to_path_buf());
    off.render(gpu, &extract(&sim.world), None).unwrap();
    off.read_pixels(gpu).unwrap()
}

/// Mean (r, g, b) of the middle of one quarter of the quad: `col` 0 is left, `row` 0 is top.
fn quarter(pixels: &[u8], col: u32, row: u32) -> [f32; 3] {
    // The quad covers the 90 px in the middle of the 160 px row; each quarter is 45 px.
    let (x0, y0) = (35 + col * 45 + 12, row * 45 + 12);
    let mut sum = [0.0f32; 3];
    for y in y0..y0 + 20 {
        for x in x0..x0 + 20 {
            let i = ((y * W + x) * 4) as usize;
            (0..3).for_each(|c| sum[c] += pixels[i + c] as f32);
        }
    }
    sum.map(|s| s / 400.0)
}

#[test]
fn a_normal_map_tilts_the_surface_the_way_its_colors_say() {
    let Some(gpu) = gpu() else { return };
    let d = dir("normal");
    // Tangent space normals (x right, y up in the image, z out) as colors: 0.6 is 204, -0.6
    // is 51, 0.8 is 230, 0 is 128. Quarters: tilted up, tilted down, tilted right, flat.
    quarters(
        &d.join("normal.png"),
        [[128, 204, 230], [128, 51, 230], [204, 128, 230], [128, 128, 255]],
    );
    quad(
        &d,
        r#"{"pbrMetallicRoughness": {"baseColorFactor": [0.8, 0.8, 0.8, 1], "metallicFactor": 0, "roughnessFactor": 1},
            "normalTexture": {"index": 0}}"#,
        &["normal.png"],
    );
    let gray = |p: [f32; 3]| (p[0] + p[1] + p[2]) / 3.0;
    // Light from above the camera, shining down onto the quad.
    let px = render(&gpu, &d, [0.0, -0.6, -0.8]);
    let (up, down, right, flat) = (
        gray(quarter(&px, 0, 0)),
        gray(quarter(&px, 1, 0)),
        gray(quarter(&px, 0, 1)),
        gray(quarter(&px, 1, 1)),
    );
    assert!(flat > 40.0, "the flat quarter is lit: {flat}");
    assert!(
        up > flat * 1.05,
        "tilted up, toward the light, is brighter: {up} and {flat}"
    );
    assert!(
        down < flat * 0.7,
        "tilted down, away from it, is darker: {down} and {flat}"
    );
    assert!(
        right < flat * 0.95 && right > down * 1.3,
        "tilted sideways is between: {right}"
    );
    // Light from the right of the camera.
    let px = render(&gpu, &d, [-0.6, 0.0, -0.8]);
    let (up, down, right, flat) = (
        gray(quarter(&px, 0, 0)),
        gray(quarter(&px, 1, 0)),
        gray(quarter(&px, 0, 1)),
        gray(quarter(&px, 1, 1)),
    );
    assert!(
        right > flat * 1.05,
        "tilted right, toward the light, is brighter: {right} and {flat}"
    );
    assert!(
        up < flat * 0.95 && (up - down).abs() < flat * 0.05,
        "tilted up and tilted down are equally off the light: {up} and {down}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn roughness_is_read_from_green_and_metallic_from_blue() {
    let Some(gpu) = gpu() else { return };
    let d = dir("metal");
    // Left half: green 255, blue 0 (rough, not metal). Right half: green 64, blue 255
    // (smooth metal). Red is not read.
    quarters(
        &d.join("mr.png"),
        [[255, 255, 0], [0, 64, 255], [255, 255, 0], [0, 64, 255]],
    );
    quad(
        &d,
        r#"{"pbrMetallicRoughness": {"baseColorFactor": [0.8, 0.1, 0.1, 1], "metallicFactor": 1, "roughnessFactor": 1,
            "metallicRoughnessTexture": {"index": 0}}}"#,
        &["mr.png"],
    );
    // A light from high above, so the mirror direction is far from the camera.
    let px = render(&gpu, &d, [0.0, -0.95, -0.3]);
    let (rough, metal) = (quarter(&px, 0, 0), quarter(&px, 1, 0));
    // A rough surface that is not metal scatters the light in its own color; a smooth metal
    // has no scattered light and its mirror highlight points away from this camera.
    assert!(
        rough[0] > 60.0 && rough[0] > rough[1] * 2.0,
        "the rough half is lit red: {rough:?}"
    );
    assert!(
        metal[0] < rough[0] * 0.3,
        "the smooth metal half is dark: {metal:?} and {rough:?}"
    );
    // The lower quarters hold the same two materials.
    let (rough_low, metal_low) = (quarter(&px, 0, 1), quarter(&px, 1, 1));
    assert!(
        (rough_low[0] - rough[0]).abs() < 2.0,
        "{rough_low:?} and {rough:?}"
    );
    assert!(metal_low[0] < rough[0] * 0.3, "{metal_low:?} and {rough:?}");
    let _ = std::fs::remove_dir_all(&d);
}
