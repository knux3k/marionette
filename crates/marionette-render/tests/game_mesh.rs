//! Meshes built in code (decision 0180): drawn through `mesh:<name>`, updated in place,
//! freed when removed, culled and shadowed like any model. GPU tests skip without an
//! adapter unless MARIONETTE_REQUIRE_GPU=1.

use marionette_core::mesh::{GameMesh, GameMeshes};
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

/// Camera 12 m above the origin looking down, one white sun straight down, and the
/// entities in `extra` (JSON, each starting with a comma).
fn stage(extra: &str, resources: &str) -> Sim {
    let text = format!(
        r#"{{"format": "marionette.scene", "version": 1,
        "resources": {{ "AmbientLight": {{"color": {{"r": 1, "g": 1, "b": 1, "a": 1}}, "intensity": 0.2}} {resources} }},
        "entities": [
          {{"id": 1, "name": "camera", "components": {{
            "Camera": {{"clear_color": {{"r": 0, "g": 0, "b": 0, "a": 1}},
              "projection": {{"Perspective": {{"far": 100.0, "fov_y_degrees": 60.0, "near": 0.1}}}}}},
            "LookAt": {{"target": {{"x": 0, "y": 0, "z": 0}}, "up": {{"x": 0, "y": 0, "z": -1}}}},
            "Transform": {{"translation": {{"x": 0, "y": 12, "z": 0.01}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 1, "y": 1, "z": 1}}}}}}}},
          {{"id": 2, "name": "sun", "components": {{
            "DirectionalLight": {{"direction": {{"x": 0.5, "y": -1, "z": 0.3}}, "color": {{"r": 1, "g": 1, "b": 1, "a": 1}}, "illuminance": 1.0}}}}}}
          {extra}
        ]}}"#
    );
    Sim::from_text(&text, None, default_registry(), 1).unwrap()
}

fn entity(id: u32, asset: &str, x: f32, y: f32, z: f32) -> String {
    format!(
        r#", {{"id": {id}, "components": {{
            "Model": {{"asset": "{asset}"}},
            "Transform": {{"translation": {{"x": {x}, "y": {y}, "z": {z}}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 1, "y": 1, "z": 1}}}}}}}}"#
    )
}

fn patch(cells: u32, color: [f32; 4]) -> GameMesh {
    let mut m = GameMesh::grid(cells, cells, 8.0, 8.0, |_, _| 0.0);
    m.colors = vec![color; m.positions.len()];
    m
}

fn rgb(px: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * W + x) * 4) as usize;
    [px[i], px[i + 1], px[i + 2]]
}

fn meshes(sim: &mut Sim) -> bevy_ecs::world::Mut<'_, GameMeshes> {
    sim.world.get_resource_or_insert_with(GameMeshes::default)
}

#[test]
fn a_mesh_built_in_code_draws_updates_and_goes_away() {
    let Some(gpu) = gpu() else { return };
    let mut sim = stage(&entity(3, "mesh:patch", 0.0, 0.0, 0.0), "");
    let mut off = Offscreen::new(&gpu, W, H, 4, repo().join("examples"));

    // Not stored yet: an error as data naming the mesh.
    let err = off.render(&gpu, &extract(&sim.world), None).unwrap_err();
    assert_eq!(err.0[0].code, "render.asset");
    assert!(err.0[0].message.contains("patch"), "{}", err.0[0].message);

    meshes(&mut sim)
        .set("patch", patch(8, [1.0, 0.0, 0.0, 1.0]))
        .unwrap();
    let stats = off.render(&gpu, &extract(&sim.world), None).unwrap();
    let px = off.read_pixels(&gpu).unwrap();
    let [r, g, b] = rgb(&px, W / 2, H / 2);
    assert!(
        r > 100 && r > 2 * g && g == b,
        "vertex colors tint the mesh red: {r} {g} {b}"
    );
    assert_eq!(stats.triangles, 8 * 8 * 2);
    assert_eq!(
        rgb(&px, 2, 2),
        [0, 0, 0],
        "outside the 8 m patch is the clear color"
    );

    // Same size: written into the same buffers; the picture follows the new data.
    meshes(&mut sim)
        .set("patch", patch(8, [0.0, 1.0, 0.0, 1.0]))
        .unwrap();
    off.render(&gpu, &extract(&sim.world), None).unwrap();
    let [r, g, _] = rgb(&off.read_pixels(&gpu).unwrap(), W / 2, H / 2);
    assert!(g > 100 && g > 2 * r, "updated in place to green: {r} {g}");

    // New size: new buffers.
    meshes(&mut sim)
        .set("patch", patch(3, [0.0, 0.0, 1.0, 1.0]))
        .unwrap();
    let stats = off.render(&gpu, &extract(&sim.world), None).unwrap();
    let [r, _, b] = rgb(&off.read_pixels(&gpu).unwrap(), W / 2, H / 2);
    assert!(b > 100 && b > 2 * r, "replaced by a blue mesh: {r} {b}");
    assert_eq!(stats.triangles, 3 * 3 * 2);

    // Removed while an entity still draws it: error again; once nothing draws it, the GPU
    // copy is gone.
    meshes(&mut sim).remove("patch");
    assert!(off.render(&gpu, &extract(&sim.world), None).is_err());
}

#[test]
fn removed_meshes_free_their_gpu_copy() {
    let Some(gpu) = gpu() else { return };
    let mut sim = stage(&entity(3, "mesh:a", 0.0, 0.0, 0.0), "");
    meshes(&mut sim).set("a", patch(2, [1.0; 4])).unwrap();
    let mut off = Offscreen::new(&gpu, W, H, 1, repo().join("examples"));
    off.render(&gpu, &extract(&sim.world), None).unwrap();
    let held = off.cached_assets();
    // The entity now draws a cube; the mesh is removed by the game.
    let e = sim
        .world
        .query::<(bevy_ecs::entity::Entity, &marionette_core::spatial::Model)>()
        .iter(&sim.world)
        .next()
        .unwrap()
        .0;
    sim.world.entity_mut(e).insert(marionette_core::spatial::Model {
        asset: "primitive:cube".into(),
    });
    meshes(&mut sim).remove("a");
    off.render(&gpu, &extract(&sim.world), None).unwrap();
    assert_eq!(off.cached_assets(), held, "mesh freed, cube loaded");
}

#[test]
fn without_vertex_colors_the_material_color_is_used_and_meshes_cast_shadows() {
    let Some(gpu) = gpu() else { return };
    // A floor patch and a raised 2 m slab above part of it, both game meshes.
    let extra = format!(
        "{}{}",
        entity(3, "mesh:floor", 0.0, 0.0, 0.0),
        entity(4, "mesh:slab", 0.0, 3.0, 0.0)
    );
    let shadowed = stage(
        &extra,
        r#", "Shadows": {"resolution": 1024, "distance": 20.0, "softness": 1.0}"#,
    );
    let plain = stage(&extra, "");
    let mut out = Vec::new();
    for mut sim in [plain, shadowed] {
        let mut floor = GameMesh::grid(4, 4, 16.0, 16.0, |_, _| 0.0);
        floor.normals.clear();
        meshes(&mut sim).set("floor", floor).unwrap();
        meshes(&mut sim)
            .set("slab", GameMesh::grid(1, 1, 2.0, 2.0, |_, _| 0.0))
            .unwrap();
        let mut off = Offscreen::new(&gpu, W, H, 4, repo().join("examples"));
        off.render(&gpu, &extract(&sim.world), None).unwrap();
        out.push(off.read_pixels(&gpu).unwrap());
    }
    // Plain white material: grey-white, equal channels.
    let [r, g, b] = rgb(&out[0], 40, H / 2);
    assert!(r > 100 && r == g && g == b, "untinted floor: {r} {g} {b}");
    // The slab's shadow falls on the floor beside it (light leans +x, +z): find a pixel
    // that got darker with shadows on.
    let darker = (0..W)
        .flat_map(|x| (0..H).map(move |y| (x, y)))
        .filter(|&(x, y)| {
            let (a, b) = (rgb(&out[0], x, y)[0] as i32, rgb(&out[1], x, y)[0] as i32);
            a - b > 40
        })
        .count();
    assert!(
        darker > 20,
        "a game mesh casts a shadow on another ({darker} pixels darker)"
    );
}

/// Cost of rewriting a large mesh every frame (a 256x256-cell terrain patch, 131,072
/// triangles): storing it (checks and normals) and the frame that writes it into the
/// existing buffers. Prints numbers; run with `--ignored --nocapture` (decision 0180).
#[test]
#[ignore]
fn large_mesh_rewrite_cost() {
    let Some(gpu) = gpu() else { return };
    let mut sim = stage(&entity(3, "mesh:terrain", 0.0, 0.0, 0.0), "");
    let mut off = Offscreen::new(&gpu, W, H, 4, repo().join("examples"));
    let terrain = |t: f32| {
        GameMesh::grid(256, 256, 8.0, 8.0, move |x, z| {
            (x * 3.0 + t).sin() * 0.1 + z * 0.01
        })
    };
    meshes(&mut sim).set("terrain", terrain(0.0)).unwrap();
    off.render(&gpu, &extract(&sim.world), None).unwrap();
    let (mut set, mut frame) = (Vec::new(), Vec::new());
    for i in 1..=10 {
        let m = terrain(i as f32);
        let t0 = std::time::Instant::now();
        meshes(&mut sim).set("terrain", m).unwrap();
        set.push(t0.elapsed().as_secs_f64() * 1e3);
        let t1 = std::time::Instant::now();
        off.render(&gpu, &extract(&sim.world), None).unwrap();
        frame.push(t1.elapsed().as_secs_f64() * 1e3);
    }
    let static_frame = {
        let t = std::time::Instant::now();
        off.render(&gpu, &extract(&sim.world), None).unwrap();
        t.elapsed().as_secs_f64() * 1e3
    };
    set.sort_by(f64::total_cmp);
    frame.sort_by(f64::total_cmp);
    println!(
        "set (checks + normals) median {:.2} ms; frame with rewrite median {:.2} ms; frame without rewrite {:.2} ms",
        set[5], frame[5], static_frame
    );
}
