//! GPU tests of the remaining lighting (decisions 0090 to 0093): shadow cascades, HDR
//! environment files, reflection probes and ray tracing. They skip (pass with a note) when no
//! GPU adapter exists, unless MARIONETTE_REQUIRE_GPU=1. Ray tracing tests also skip on a GPU
//! without ray queries, unless MARIONETTE_REQUIRE_RAY_TRACING=1.

use marionette_core::{Sim, default_registry};
use marionette_render::{Gpu, Offscreen, RenderStats, extract};
use std::path::{Path, PathBuf};

const W: u32 = 160;
const H: u32 = 90;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

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

/// A GPU that can trace rays, or `None` (skip) when there is none.
fn rt_gpu() -> Option<Gpu> {
    let gpu = gpu()?;
    if gpu.ray_tracing() {
        return Some(gpu);
    }
    if std::env::var("MARIONETTE_REQUIRE_RAY_TRACING").as_deref() == Ok("1") {
        panic!("ray tracing required but {} has no ray queries", gpu.describe());
    }
    eprintln!("skipping ray tracing test: {} has no ray queries", gpu.describe());
    None
}

fn render(gpu: &Gpu, sim: &Sim) -> (Vec<u8>, RenderStats) {
    let mut off = Offscreen::new(gpu, W, H, 4, repo().join("examples"));
    let stats = off.render(gpu, &extract(&sim.world), None).unwrap();
    (off.read_pixels(gpu).unwrap(), stats)
}

/// Mean brightness (0..255) of a pixel rectangle.
fn mean(pixels: &[u8], (x0, y0, x1, y1): (u32, u32, u32, u32)) -> f32 {
    channel(pixels, (x0, y0, x1, y1), [1.0, 1.0, 1.0]) / 3.0
}

/// Mean of `weights` dot (r, g, b) over a pixel rectangle.
fn channel(pixels: &[u8], (x0, y0, x1, y1): (u32, u32, u32, u32), weights: [f32; 3]) -> f32 {
    let mut sum = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * W + x) * 4) as usize;
            sum += (0..3).map(|c| pixels[i + c] as f32 * weights[c]).sum::<f32>();
        }
    }
    sum / ((x1 - x0) * (y1 - y0)) as f32
}

/// The test stage: a camera 12 m above a 30 m gray ground looking down, a red box at x = 2 m.
fn stage(extra: &str, resources: &str) -> Sim {
    stage_with(r#""metallic": 0, "roughness": 0.9"#, extra, resources)
}

/// The stage with the ground's metallic and roughness given.
fn stage_with(ground: &str, extra: &str, resources: &str) -> Sim {
    let text = format!(
        r#"{{"format": "marionette.scene", "version": 1,
        "resources": {{ {resources} }},
        "entities": [
          {{"id": 1, "name": "camera", "components": {{
            "Camera": {{"clear_color": {{"r": 0, "g": 0, "b": 0, "a": 1}},
              "projection": {{"Perspective": {{"far": 100.0, "fov_y_degrees": 60.0, "near": 0.1}}}}}},
            "LookAt": {{"target": {{"x": 0, "y": 0, "z": 0}}, "up": {{"x": 0, "y": 0, "z": -1}}}},
            "Transform": {{"translation": {{"x": 0, "y": 12, "z": 0.01}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 1, "y": 1, "z": 1}}}}}}}},
          {{"id": 2, "name": "ground", "components": {{
            "Model": {{"asset": "primitive:plane"}},
            "Material": {{"base_color": {{"r": 0.8, "g": 0.8, "b": 0.8, "a": 1}}, {ground}}},
            "Transform": {{"translation": {{"x": 0, "y": 0, "z": 0}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 30, "y": 1, "z": 30}}}}}}}},
          {{"id": 3, "name": "box", "components": {{
            "Model": {{"asset": "primitive:cube"}},
            "Material": {{"base_color": {{"r": 0.8, "g": 0.2, "b": 0.2, "a": 1}}, "metallic": 0, "roughness": 0.9}},
            "Transform": {{"translation": {{"x": 2, "y": 0.5, "z": 0}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 1, "y": 1, "z": 1}}}}}}}}
          {extra}
        ]}}"#
    );
    Sim::from_text(&text, None, default_registry(), 1).unwrap()
}

/// Pixel x of world x on the ground at 160x90 (12 m up, 60 degree vertical view).
fn stage_x(x: f32) -> u32 {
    let half_h = 12.0 * (30f32).to_radians().tan();
    (80.0 + x / half_h * 45.0).round() as u32
}

/// A low sun from -x (light travelling toward +x), so the box's shadow falls at x > 2.5 m.
const SUN: &str = r#""DirectionalLight": {"direction": {"x": 1.0, "y": -0.7, "z": 0.0}, "color": {"r": 1, "g": 1, "b": 1, "a": 1}, "illuminance": 2.0}"#;

fn sun_entity() -> String {
    format!(
        r#", {{"id": 9, "name": "sun", "components": {{ {SUN},
        "Transform": {{"translation": {{"x": 0, "y": 0, "z": 0}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 1, "y": 1, "z": 1}}}}}}}}"#
    )
}

const AMBIENT: &str = r#""AmbientLight": {"color": {"r": 1, "g": 1, "b": 1, "a": 1}, "intensity": 0.1}"#;

/// Decision 0090: a sun shadow map too small to reach the box (`distance` 1 m) leaves its
/// shadow out; cascades reaching 40 m bring it back, one depth pass per cascade.
#[test]
fn cascades_reach_shadows_a_single_small_map_misses() {
    let Some(gpu) = gpu() else { return };
    let shadows = r#", "Shadows": {"distance": 1.0, "resolution": 1024, "softness": 1.0}"#;
    let cascades = r#", "ShadowCascades": {"count": 3, "far": 40.0}"#;
    let (single, single_stats) = render(&gpu, &stage(&sun_entity(), &format!("{AMBIENT}{shadows}")));
    let (split, split_stats) = render(
        &gpu,
        &stage(&sun_entity(), &format!("{AMBIENT}{shadows}{cascades}")),
    );
    // Ground just past the box on the far side from the sun (box spans x = 1.5 .. 2.5 m).
    let behind = (stage_x(3.0), 43, stage_x(3.0) + 2, 47);
    assert!(
        mean(&split, behind) + 20.0 < mean(&single, behind),
        "{} vs {}",
        mean(&split, behind),
        mean(&single, behind)
    );
    // Sunlit ground on the near side is the same.
    let front = (stage_x(-2.0), 43, stage_x(-2.0) + 2, 47);
    assert!((mean(&split, front) - mean(&single, front)).abs() < 3.0);
    // Two more cascades: two more depth passes over the two mesh groups.
    assert_eq!(split_stats.draw_calls, single_stats.draw_calls + 2 * 2);
}

/// Decision 0093: ray-traced shadows darken the ground behind the box from a point light
/// without any shadow map pass, like `CastShadows` does with six.
#[test]
fn ray_traced_shadows_need_no_shadow_maps() {
    let Some(gpu) = rt_gpu() else { return };
    let lamp = r#", {"id": 4, "name": "lamp", "components": {
          "PointLight": {"color": {"r": 1, "g": 1, "b": 1, "a": 1}, "intensity": 6.0, "range": 10.0},
          "Transform": {"translation": {"x": 0, "y": 2, "z": 0}, "rotation": {"x": 0, "y": 0, "z": 0, "w": 1}, "scale": {"x": 1, "y": 1, "z": 1}}}}"#;
    let rt = r#""RayTracing": {"shadows": true, "reflections": false, "max_roughness": 0.5, "sun_size_degrees": 0.0}"#;
    let (lit, plain_stats) = render(&gpu, &stage(lamp, ""));
    let (traced, stats) = render(&gpu, &stage(lamp, rt));
    let behind = (stage_x(3.5) - 1, 43, stage_x(3.5) + 2, 47);
    let front = (stage_x(-1.5) - 1, 43, stage_x(-1.5) + 2, 47);
    assert!(
        mean(&traced, behind) + 20.0 < mean(&lit, behind),
        "{} vs {}",
        mean(&traced, behind),
        mean(&lit, behind)
    );
    assert!((mean(&traced, front) - mean(&lit, front)).abs() < 3.0);
    // Each mesh draw once more for the depth prepass, and no shadow map draws.
    assert_eq!(stats.draw_calls, 2 * plain_stats.draw_calls);

    // The sun too, and the box's own top stays lit.
    let (sun_lit, _) = render(&gpu, &stage(&sun_entity(), AMBIENT));
    let (sun_traced, _) = render(&gpu, &stage(&sun_entity(), &format!("{AMBIENT}, {rt}")));
    let behind = (stage_x(3.0), 43, stage_x(3.0) + 2, 47);
    assert!(mean(&sun_traced, behind) + 20.0 < mean(&sun_lit, behind));
    let top = (stage_x(2.0) - 1, 43, stage_x(2.0) + 1, 47);
    assert!((mean(&sun_traced, top) - mean(&sun_lit, top)).abs() < 3.0);
}

/// Decision 0093: a mirror ground shows the bright red box floating above it only with
/// ray-traced reflections; without them it reflects the ambient light alone.
#[test]
fn ray_traced_reflections_show_other_objects() {
    let Some(gpu) = rt_gpu() else { return };
    // Mirror ground, and a glowing red box 3 m up at x = -3 m.
    let extra = r#", {"id": 4, "name": "glow", "components": {
          "Model": {"asset": "primitive:cube"},
          "Material": {"base_color": {"r": 1, "g": 0, "b": 0, "a": 1}, "metallic": 0, "roughness": 0.9},
          "Emissive": {"color": {"r": 1, "g": 0, "b": 0, "a": 1}, "intensity": 1.0},
          "Transform": {"translation": {"x": -3, "y": 3, "z": 0}, "rotation": {"x": 0, "y": 0, "z": 0, "w": 1}, "scale": {"x": 1, "y": 1, "z": 1}}}}"#;
    let mirror = r#""metallic": 1.0, "roughness": 0.05"#;
    let rt = r#""RayTracing": {"shadows": false, "reflections": true, "max_roughness": 0.5, "sun_size_degrees": 0.0}"#;
    let (plain, _) = render(&gpu, &stage_with(mirror, extra, AMBIENT));
    let (traced, _) = render(&gpu, &stage_with(mirror, extra, &format!("{AMBIENT}, {rt}")));
    // The reflection of a point 3 m up appears where its mirror image (3 m below the ground)
    // projects: at x * 12 / 15 on the ground plane's scale.
    let at = stage_x(-3.0 * 12.0 / 15.0);
    let spot = (at - 1, 43, at + 2, 47);
    let red = [1.0, -0.5, -0.5];
    assert!(
        channel(&traced, spot, red) > channel(&plain, spot, red) + 30.0,
        "{} vs {}",
        channel(&traced, spot, red),
        channel(&plain, spot, red)
    );
}

/// Decision 0092: with a reflection probe, the mirror ground shows the glowing box above it
/// (the probe's box ceiling sits at the box's underside, so the corrected reflection lands
/// where the real one would); without it the ground reflects ambient light only. No probe,
/// no change.
#[test]
fn reflection_probes_show_other_objects() {
    let Some(gpu) = gpu() else { return };
    let glow = r#", {"id": 4, "name": "glow", "components": {
          "Model": {"asset": "primitive:cube"},
          "Material": {"base_color": {"r": 1, "g": 0, "b": 0, "a": 1}, "metallic": 0, "roughness": 0.9},
          "Emissive": {"color": {"r": 1, "g": 0, "b": 0, "a": 1}, "intensity": 1.0},
          "Transform": {"translation": {"x": -3, "y": 3, "z": 0}, "rotation": {"x": 0, "y": 0, "z": 0, "w": 1}, "scale": {"x": 1, "y": 1, "z": 1}}}}"#;
    let probe = r#", {"id": 5, "name": "probe", "components": {
          "ReflectionProbe": {"extent": {"x": 10, "y": 1.25, "z": 10}, "resolution": 128},
          "Transform": {"translation": {"x": 0, "y": 1.25, "z": 0}, "rotation": {"x": 0, "y": 0, "z": 0, "w": 1}, "scale": {"x": 1, "y": 1, "z": 1}}}}"#;
    let mirror = r#""metallic": 1.0, "roughness": 0.05"#;
    let (plain, plain_stats) = render(&gpu, &stage_with(mirror, glow, AMBIENT));
    let (probed, stats) = render(&gpu, &stage_with(mirror, &format!("{glow}{probe}"), AMBIENT));
    let at = stage_x(-3.0 * 12.0 / 15.0);
    let spot = (at - 1, 43, at + 2, 47);
    let red = [1.0, -0.5, -0.5];
    assert!(
        channel(&probed, spot, red) > channel(&plain, spot, red) + 30.0,
        "{} vs {}",
        channel(&probed, spot, red),
        channel(&plain, spot, red)
    );
    // The capture happens in `prepare`, not in the frame: the frame draws the same.
    assert_eq!(stats.draw_calls, plain_stats.draw_calls);
}

/// Decision 0091: a Radiance `.hdr` environment keeps light above 1.0. A flat 2.0 sky at
/// half intensity lights the ground like a white (1.0) PNG sky at full intensity, and at full
/// intensity twice as much. A missing file is an error as data.
#[test]
fn hdr_environment_keeps_light_above_one() {
    let Some(gpu) = gpu() else { return };
    let env = |image: &str, intensity: f32| {
        format!(r#""Environment": {{"image": "{image}", "intensity": {intensity}, "background": false}}"#)
    };
    let ground = (20, 20, 30, 30);
    let (white, _) = render(&gpu, &stage("", &env("builtin:white", 0.5)));
    let (half, _) = render(&gpu, &stage("", &env("assets/flat-2.hdr", 0.25)));
    let (full, _) = render(&gpu, &stage("", &env("assets/flat-2.hdr", 0.5)));
    assert!(
        (mean(&half, ground) - mean(&white, ground)).abs() < 3.0,
        "{} vs {}",
        mean(&half, ground),
        mean(&white, ground)
    );
    assert!(mean(&full, ground) > mean(&white, ground) + 20.0);
    let sim = stage("", &env("assets/missing.hdr", 1.0));
    let mut off = Offscreen::new(&gpu, W, H, 4, repo().join("examples"));
    let err = off.render(&gpu, &extract(&sim.world), None).unwrap_err();
    assert!(format!("{err:?}").contains("Environment"), "{err:?}");
}
