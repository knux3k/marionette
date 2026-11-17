//! Skinned game meshes (decision 0185): a mesh built in code with joints and weights, bent
//! per copy by the joint palettes of a bulk draw, with the standard shader or a game shader.
//! GPU tests skip without an adapter unless MARIONETTE_REQUIRE_GPU=1.

use marionette_core::mesh::{GameMesh, GameMeshes};
use marionette_core::shader::{GameShaders, ShaderMaterial};
use marionette_core::spatial::BulkDraw;
use marionette_core::{Sim, default_registry};
use marionette_render::{Gpu, Offscreen, extract};
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

/// Top-down camera 12 m up, a sun straight down, the entities in `extra` (JSON, each
/// starting with a comma), black background.
fn stage(extra: &str) -> Sim {
    let head = r#"{"format": "marionette.scene", "version": 1,
        "resources": { "AmbientLight": {"color": {"r": 1, "g": 1, "b": 1, "a": 1}, "intensity": 0.3} },
        "entities": [
          {"id": 1, "name": "camera", "components": {
            "Camera": {"clear_color": {"r": 0, "g": 0, "b": 0, "a": 1},
              "projection": {"Perspective": {"far": 100.0, "fov_y_degrees": 60.0, "near": 0.1}}},
            "LookAt": {"target": {"x": 0, "y": 0, "z": 0}, "up": {"x": 0, "y": 0, "z": -1}},
            "Transform": {"translation": {"x": 0, "y": 12, "z": 0.01}, "rotation": {"x": 0, "y": 0, "z": 0, "w": 1}, "scale": {"x": 1, "y": 1, "z": 1}}}},
          {"id": 2, "name": "sun", "components": {
            "DirectionalLight": {"direction": {"x": 0, "y": -1, "z": 0}, "color": {"r": 1, "g": 1, "b": 1, "a": 1}, "illuminance": 1.0}}}"#;
    let text = format!("{head}{extra}\n]}}");
    Sim::from_text(&text, None, default_registry(), 1).unwrap()
}

/// A 4 m by 2 m white strip on the ground from x = -2 to x = 2: the left half on joint 0,
/// the right half on joint 1.
fn strip() -> GameMesh {
    let mut m = GameMesh::grid(8, 2, 4.0, 2.0, |_, _| 0.0);
    m.colors = vec![[1.0; 4]; m.positions.len()];
    m.joints = m
        .positions
        .iter()
        .map(|p| [u16::from(p[0] > 0.0), 0, 0, 0])
        .collect();
    m.weights = vec![[1.0, 0.0, 0.0, 0.0]; m.positions.len()];
    m
}

fn translate(x: f32, y: f32, z: f32) -> [f32; 12] {
    [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, x, y, z]
}

fn bulk(palettes: Vec<[f32; 12]>, shader: Option<ShaderMaterial>) -> BulkDraw {
    BulkDraw {
        asset: "mesh:strip".into(),
        transforms: vec![translate(0.0, 0.0, 0.0)],
        palettes,
        shader,
        ..Default::default()
    }
}

/// Pixel column of world x (12 m up, 60 degree vertical view, 160x90).
fn col(x: f32) -> u32 {
    let half_h = 12.0 * 30f32.to_radians().tan();
    (80.0 + x / half_h * 45.0).round() as u32
}

fn rgb(px: &[u8], x: u32) -> [i32; 3] {
    let i = (((H / 2) * W + x) * 4) as usize;
    [px[i] as i32, px[i + 1] as i32, px[i + 2] as i32]
}

fn draw(gpu: &Gpu, sim: &Sim, bulk: BulkDraw) -> Vec<u8> {
    let mut frame = extract(&sim.world);
    frame.bulk.push(bulk);
    let mut off = Offscreen::new(gpu, W, H, 4, repo().join("examples"));
    off.render(gpu, &frame, None).unwrap();
    off.read_pixels(gpu).unwrap()
}

fn lit(c: [i32; 3]) -> bool {
    c.iter().all(|&v| v > 60)
}

#[test]
fn bulk_palettes_bend_a_skinned_game_mesh() {
    let Some(gpu) = gpu() else { return };
    let mut sim = stage("");
    sim.world
        .get_resource_or_insert_with(GameMeshes::default)
        .set("strip", strip())
        .unwrap();

    // No palettes: the bind pose, x = -1.5 and x = 1.5 both covered, x = 3.5 empty.
    let px = draw(&gpu, &sim, bulk(Vec::new(), None));
    assert!(lit(rgb(&px, col(-1.5))) && lit(rgb(&px, col(1.5))), "bind pose");
    assert!(
        !lit(rgb(&px, col(3.5))),
        "nothing past the strip: {:?}",
        rgb(&px, col(3.5))
    );

    // Joint 1 moved 2 m along +x: the right half now covers 2..4, leaving a gap at 1.
    let moved = vec![translate(0.0, 0.0, 0.0), translate(2.0, 0.0, 0.0)];
    let px = draw(&gpu, &sim, bulk(moved.clone(), None));
    assert!(
        lit(rgb(&px, col(-1.5))),
        "joint 0 stays: {:?}",
        rgb(&px, col(-1.5))
    );
    assert!(lit(rgb(&px, col(3.5))), "joint 1 moved: {:?}", rgb(&px, col(3.5)));
    // The quad joining the halves stretches across the gap, so look past the strip's old end.
    assert!(lit(rgb(&px, col(2.5))));

    // The same with a game shader: its surface color, on the bent mesh.
    sim.world.get_resource_or_insert_with(GameShaders::default).set(
        "green",
        "fn surface(s: ptr<function, Surface>, i: SurfaceIn) {\n    (*s).base_color = vec4<f32>(0.0, 1.0, 0.0, 1.0);\n}\n",
    );
    let green = ShaderMaterial {
        shader: "green".into(),
        ..Default::default()
    };
    let px = draw(&gpu, &sim, bulk(moved, Some(green)));
    let c = rgb(&px, col(3.5));
    assert!(c[1] > c[0] + 80 && c[1] > c[2] + 80, "green, bent: {c:?}");
    assert!(!lit(rgb(&px, col(-3.0))), "nothing left of the strip");
}

#[test]
fn an_entity_drawing_a_skinned_game_mesh_shows_the_bind_pose() {
    let Some(gpu) = gpu() else { return };
    let text = r#", {"id": 3, "components": {
        "Model": {"asset": "mesh:strip"},
        "Transform": {"translation": {"x": 0, "y": 0, "z": 0}, "rotation": {"x": 0, "y": 0, "z": 0, "w": 1}, "scale": {"x": 1, "y": 1, "z": 1}}}}"#;
    let mut sim = stage(text);
    sim.world
        .get_resource_or_insert_with(GameMeshes::default)
        .set("strip", strip())
        .unwrap();
    let mut off = Offscreen::new(&gpu, W, H, 4, repo().join("examples"));
    off.render(&gpu, &extract(&sim.world), None).unwrap();
    let px = off.read_pixels(&gpu).unwrap();
    assert!(lit(rgb(&px, col(-1.5))) && lit(rgb(&px, col(1.5))), "bind pose");
}
