//! GPU tests of textured materials (decision 0280): mipmaps, cut-out and see-through
//! materials from glTF alpha modes, double-sided materials, and a textured reference model
//! whose test proves that swapped metallic-roughness channels or a flipped normal map fail
//! it. They skip (pass with a note) when no GPU adapter exists, unless
//! MARIONETTE_REQUIRE_GPU=1.

#[path = "support/materials.rs"]
mod materials;

use marionette_core::{Sim, default_registry};
use marionette_render::compare::{REFERENCE_THRESHOLD, diff, within_reference_tolerance};
use marionette_render::{Gpu, Offscreen, extract};
use std::path::{Path, PathBuf};

const W: u32 = 160;
const H: u32 = 160;

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

/// A new empty folder for this test's generated models, removed when dropped.
fn scratch(name: &str) -> marionette_host::TempDir {
    marionette_host::temp_dir(&format!("render-materials-{name}")).unwrap()
}

fn render_in(gpu: &Gpu, sim: &Sim, base: &Path, size: (u32, u32)) -> Vec<u8> {
    let mut off = Offscreen::new(gpu, size.0, size.1, 4, base.to_path_buf());
    off.render(gpu, &extract(&sim.world), None).unwrap();
    off.read_pixels(gpu).unwrap()
}

/// RGB of the pixel at (x, y) in a `W`-wide image.
fn px(pixels: &[u8], x: u32, y: u32) -> [f32; 3] {
    let i = ((y * W + x) * 4) as usize;
    [pixels[i] as f32, pixels[i + 1] as f32, pixels[i + 2] as f32]
}

fn transform(t: [f32; 3], r: [f32; 4], s: [f32; 3]) -> String {
    format!(
        r#""Transform": {{"translation": {{"x": {}, "y": {}, "z": {}}}, "rotation": {{"x": {}, "y": {}, "z": {}, "w": {}}}, "scale": {{"x": {}, "y": {}, "z": {}}}}}"#,
        t[0], t[1], t[2], r[0], r[1], r[2], r[3], s[0], s[1], s[2]
    )
}

const NO_TURN: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// A scene: a camera at `eye` looking at `target` with y up (60 degree view, black clear), a
/// white ambient light, and `entities` (comma-separated JSON entities with ids from 10).
fn scene(eye: [f32; 3], target: [f32; 3], resources: &str, entities: &str) -> Sim {
    scene_with(eye, target, [0.0, 1.0, 0.0], 1.0, resources, entities)
}

/// [`scene`] with the camera's up direction and the ambient light's intensity given.
fn scene_with(
    eye: [f32; 3],
    target: [f32; 3],
    up: [f32; 3],
    ambient: f32,
    resources: &str,
    entities: &str,
) -> Sim {
    let text = format!(
        r#"{{"format": "marionette.scene", "version": 1,
        "resources": {{ "AmbientLight": {{"color": {{"r": 1, "g": 1, "b": 1, "a": 1}}, "intensity": {ambient}}} {resources} }},
        "entities": [
          {{"id": 1, "name": "camera", "components": {{
            "Camera": {{"clear_color": {{"r": 0, "g": 0, "b": 0, "a": 1}},
              "projection": {{"Perspective": {{"far": 400.0, "fov_y_degrees": 60.0, "near": 0.1}}}}}},
            "LookAt": {{"target": {{"x": {}, "y": {}, "z": {}}}, "up": {{"x": {}, "y": {}, "z": {}}}}},
            {}}}}},
          {entities}
        ]}}"#,
        target[0],
        target[1],
        target[2],
        up[0],
        up[1],
        up[2],
        transform(eye, NO_TURN, [1.0; 3]),
    );
    Sim::from_text(&text, None, default_registry(), 1).unwrap()
}

fn model(id: u32, asset: &str, t: [f32; 3], r: [f32; 4], s: [f32; 3], extra: &str) -> String {
    format!(
        r#"{{"id": {id}, "name": "m{id}", "components": {{ "Model": {{"asset": "{asset}"}}, {} {extra} }}}}"#,
        transform(t, r, s)
    )
}

fn cube(id: u32, t: [f32; 3], s: [f32; 3], rgba: [f32; 4]) -> String {
    model(
        id,
        "primitive:cube",
        t,
        NO_TURN,
        s,
        &format!(
            r#", "Material": {{"base_color": {{"r": {}, "g": {}, "b": {}, "a": {}}}, "metallic": 0, "roughness": 1}}"#,
            rgba[0], rgba[1], rgba[2], rgba[3]
        ),
    )
}

#[test]
fn committed_models_match_their_generator() {
    for (name, bytes) in [
        ("material-ball", materials::material_ball(false, false)),
        ("fence", materials::fence()),
        ("glass", materials::glass()),
    ] {
        let path = repo().join(format!("examples/assets/{name}.glb"));
        assert!(
            std::fs::read(&path).ok() == Some(bytes),
            "{name}.glb differs from its generator; run `cargo run -p marionette-render --example make_materials`"
        );
    }
}

#[test]
fn glb_alpha_modes_and_sides_are_read() {
    let tmp = scratch("import");
    let dir = tmp.path();
    for (name, bytes) in [("fence", materials::fence()), ("glass", materials::glass())] {
        std::fs::write(dir.join(format!("{name}.glb")), bytes).unwrap();
    }
    let fence = marionette_render::assets::load_gltf(&dir.join("fence.glb")).unwrap();
    assert_eq!(
        fence.materials[0].alpha_mode,
        marionette_render::assets::AlphaMode::Mask { cutoff: 0.5 }
    );
    assert!(fence.materials[0].double_sided);
    let glass = marionette_render::assets::load_gltf(&dir.join("glass.glb")).unwrap();
    assert_eq!(
        glass.materials[0].alpha_mode,
        marionette_render::assets::AlphaMode::Blend
    );
    assert!(!glass.materials[0].double_sided);
}

/// The audit's mutations of the textured path: the reference scene's material ball with its
/// metallic and roughness channels swapped, or its normal map's green channel flipped, must
/// each fail the reference comparison. A shader that read the channels the wrong way round
/// would draw the same wrong picture, so this proves the reference image guards the shader.
#[test]
fn reference_catches_swapped_metal_rough_and_flipped_normals() {
    let Some(gpu) = gpu() else { return };
    let text = std::fs::read_to_string(repo().join("examples/materials.scene.json")).unwrap();
    let size = (320, 180);
    let good = {
        let sim = Sim::from_text(&text, None, default_registry(), 1).unwrap();
        render_in(&gpu, &sim, &repo().join("examples"), size)
    };
    let tmp = scratch("mutations");
    let dir = tmp.path();
    for name in ["fence", "glass"] {
        std::fs::copy(
            repo().join(format!("examples/assets/{name}.glb")),
            dir.join(format!("{name}.glb")),
        )
        .unwrap();
    }
    for (what, bytes) in [
        (
            "metal and roughness swapped",
            materials::material_ball(true, false),
        ),
        ("normal map green flipped", materials::material_ball(false, true)),
    ] {
        std::fs::write(dir.join("material-ball.glb"), bytes).unwrap();
        let broken_text = text.replace("assets/", "");
        let sim = Sim::from_text(&broken_text, None, default_registry(), 1).unwrap();
        let broken = render_in(&gpu, &sim, dir, size);
        let d = diff(&good, &broken, REFERENCE_THRESHOLD);
        eprintln!("{what}: {d:?}");
        assert!(
            !within_reference_tolerance(&d),
            "{what} passes the reference check: {d:?}"
        );
    }
}

/// A cut-out panel in front of a red wall: its holes show the wall, its bars do not; it is
/// double-sided, so it looks the same from behind; its shadow has the same holes.
#[test]
fn cut_outs_show_through_both_sides_and_in_shadows() {
    let Some(gpu) = gpu() else { return };
    let tmp = scratch("cutout");
    let dir = tmp.path();
    std::fs::write(dir.join("fence.glb"), materials::fence()).unwrap();
    // Fence bars are at u, v in [0, 0.25) of each quarter; the panel is 2 m wide at z = 0.
    // Seen from 3 m (60 degree view, 160 px): 1 m is about 46 px.
    let k = 160.0 / (2.0 * 3.0 * 30f32.to_radians().tan());
    let at = |x: f32, y: f32| ((80.0 + x * k).round() as u32, (80.0 - y * k).round() as u32);
    let wall = cube(11, [0.0, 0.0, -1.0], [6.0, 6.0, 0.1], [1.0, 0.0, 0.0, 1.0]);
    for (eye, flip) in [([0.0, 0.0, 3.0], 1.0), ([0.0, 0.0, -3.0], -1.0)] {
        // From behind, the wall is moved behind the panel the other way.
        let wall = if flip > 0.0 {
            wall.clone()
        } else {
            cube(11, [0.0, 0.0, 1.0], [6.0, 6.0, 0.1], [1.0, 0.0, 0.0, 1.0])
        };
        let fence = model(10, "fence.glb", [0.0; 3], NO_TURN, [2.0, 2.0, 1.0], "");
        let sim = scene(eye, [0.0; 3], "", &format!("{fence}, {wall}"));
        let pixels = render_in(&gpu, &sim, dir, (W, H));
        // Bars are the first sixteenth of each quarter of the panel: u in [0, 0.0625) is
        // x in [-1, -0.875). The middle of the first hole is u = 0.15625, x = -0.6875. At
        // y = 0.3 (v = 0.35) the row is a hole row.
        let (bx, by) = at(-0.94 * flip, 0.3);
        let (hx, hy) = at(-0.6875 * flip, 0.3);
        let (bar, hole) = (px(&pixels, bx, by), px(&pixels, hx, hy));
        assert!(
            hole[0] > 200.0 && hole[1] < 40.0,
            "hole shows the red wall: {hole:?}"
        );
        assert!(bar[1] > 60.0 && bar[0] < 220.0, "bar is drawn (brown): {bar:?}");
    }

    // Shadows: the panel lies flat 1 m above a white floor (x and z in [-1, 1]); a slanted
    // sun (2 m across per 1 m down) throws its shadow to x in [1, 3], clear of the panel as
    // seen by a camera 6 m above x = 2.5 looking straight down (+x right, +z down the image).
    let floor = cube(11, [0.0, -0.05, 0.0], [12.0, 0.1, 12.0], [1.0, 1.0, 1.0, 1.0]);
    let flat = [
        -std::f32::consts::FRAC_1_SQRT_2,
        0.0,
        0.0,
        std::f32::consts::FRAC_1_SQRT_2,
    ];
    let fence = model(10, "fence.glb", [0.0, 1.0, 0.0], flat, [2.0, 2.0, 1.0], "");
    let shadows = r#", "Shadows": {"distance": 8.0, "resolution": 2048, "softness": 0.0}"#;
    let light = r#"{"id": 12, "name": "sun", "components": {"DirectionalLight": {"direction": {"x": 2.0, "y": -1.0, "z": 0.0}, "color": {"r": 1, "g": 1, "b": 1, "a": 1}, "illuminance": 3.0}}}"#;
    let sim = scene_with(
        [2.5, 6.0, 0.0],
        [2.5, 0.0, 0.0],
        [0.0, 0.0, -1.0],
        0.1,
        shadows,
        &format!("{fence}, {floor}, {light}"),
    );
    let pixels = render_in(&gpu, &sim, dir, (W, H));
    // Panel x = -0.94 (bar column) shadows floor x = 1.06; x = -0.6875 (hole) floor x = 1.3125;
    // z = 0.3 is v = 0.65, a hole row.
    let k6 = 160.0 / (2.0 * 6.0 * 30f32.to_radians().tan());
    let floor_px = |x: f32, z: f32| {
        (
            (80.0 + (x - 2.5) * k6).round() as u32,
            (80.0 + z * k6).round() as u32,
        )
    };
    let (sx, sy) = floor_px(1.06, 0.3);
    let (lx, ly) = floor_px(1.3125, 0.3);
    let (shadowed, lit) = (px(&pixels, sx, sy), px(&pixels, lx, ly));
    assert!(
        lit[0] > shadowed[0] + 40.0,
        "light passes the holes and not the bars: lit {lit:?}, shadowed {shadowed:?}"
    );
}

/// See-through panes mix with what is behind them, and the nearer pane is drawn over the
/// farther one whatever their order in the scene.
#[test]
fn blended_surfaces_mix_and_draw_back_to_front() {
    let Some(gpu) = gpu() else { return };
    let tmp = scratch("blend");
    let dir = tmp.path();
    let white = cube(10, [0.0, 0.0, -2.0], [6.0, 6.0, 0.1], [1.0, 1.0, 1.0, 1.0]);
    let pane = |id: u32, z: f32, rgba: [f32; 4]| cube(id, [0.0, 0.0, z], [1.0, 1.0, 0.01], rgba);
    let center = |sim: &Sim| {
        let pixels = render_in(&gpu, sim, dir, (W, H));
        px(&pixels, 80, 80)
    };
    // One half-see-through black pane over white: about half brightness (sRGB of 0.5 linear).
    let sim = scene(
        [0.0, 0.0, 3.0],
        [0.0; 3],
        "",
        &format!("{white}, {}", pane(11, 0.0, [0.0, 0.0, 0.0, 0.5])),
    );
    let c = center(&sim);
    assert!((120.0..230.0).contains(&c[0]), "white shows through half: {c:?}");

    // Red near, blue far, listed both ways round: red always dominates.
    for (red_id, blue_id) in [(11, 12), (12, 11)] {
        let red = pane(red_id, 0.5, [1.0, 0.0, 0.0, 0.7]);
        let blue = pane(blue_id, 0.0, [0.0, 0.0, 1.0, 0.7]);
        let sim = scene([0.0, 0.0, 3.0], [0.0; 3], "", &format!("{white}, {red}, {blue}"));
        let c = center(&sim);
        assert!(
            c[0] > c[2] + 60.0,
            "nearer red pane over blue (ids {red_id}, {blue_id}): {c:?}"
        );
    }
    // A see-through pane hides nothing behind it from the depth test: a cube behind it stays.
    let behind = cube(13, [0.0, 0.0, -1.0], [0.5, 0.5, 0.5], [0.0, 1.0, 0.0, 1.0]);
    let sim = scene(
        [0.0, 0.0, 3.0],
        [0.0; 3],
        "",
        &format!("{white}, {}, {behind}", pane(11, 0.0, [1.0, 1.0, 1.0, 0.3])),
    );
    let c = center(&sim);
    assert!(c[1] > c[0] + 40.0, "green cube shows through the pane: {c:?}");
}

/// A fine checker on a floor running far into the distance averages to gray instead of
/// sparkling (mipmaps), and stays sharp near the camera.
#[test]
fn mipmaps_average_far_textures() {
    let Some(gpu) = gpu() else { return };
    let tmp = scratch("mips");
    let dir = tmp.path();
    std::fs::write(dir.join("checker.glb"), materials::checker(200.0)).unwrap();
    let flat = [
        -std::f32::consts::FRAC_1_SQRT_2,
        0.0,
        0.0,
        std::f32::consts::FRAC_1_SQRT_2,
    ];
    let floor = model(
        10,
        "checker.glb",
        [0.0, 0.0, -100.0],
        flat,
        [200.0, 200.0, 1.0],
        "",
    );
    let sim = scene([0.0, 1.0, 0.0], [0.0, 0.8, -10.0], "", &floor);
    let pixels = render_in(&gpu, &sim, dir, (W, H));
    if let Ok(out) = std::env::var("MARIONETTE_TEST_DUMP") {
        std::fs::write(out, marionette_render::assets::encode_png(W, H, &pixels)).unwrap();
    }
    // Rows just under the horizon (far floor): about middle gray with little spread.
    let mut values = Vec::new();
    for y in 86..92 {
        for x in 20..140 {
            values.push(px(&pixels, x, y)[0]);
        }
    }
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    let spread = (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32).sqrt();
    eprintln!("far floor: mean {mean}, spread {spread}");
    assert!((100.0..215.0).contains(&mean), "far floor mean {mean}");
    // Without mipmaps this spread is about 18 (moire); with them about 0.
    assert!(spread < 6.0, "far floor sparkles: spread {spread}");
}
