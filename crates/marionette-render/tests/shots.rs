//! Camera shots during play (decision 0286): a picture from a pose of the game's choosing, read
//! back to the CPU or kept as a `shot:<name>` image that screen UI and sprites show. GPU tests
//! skip without an adapter unless MARIONETTE_REQUIRE_GPU=1.

use marionette_core::shots::{ShotRequest, Shots};
use marionette_core::{Sim, default_registry};
use marionette_render::shots::take_now;
use marionette_render::{Gpu, Offscreen, ShotDraws, extract};
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

fn cube(id: u32, x: f32, color: (f32, f32, f32)) -> String {
    let (r, g, b) = color;
    format!(
        r#", {{"id": {id}, "components": {{
            "Model": {{"asset": "primitive:cube"}},
            "Material": {{"base_color": {{"r": {r}, "g": {g}, "b": {b}, "a": 1}}, "metallic": 0, "roughness": 1}},
            "Transform": {{"translation": {{"x": {x}, "y": 0, "z": 0}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 4, "y": 4, "z": 4}}}}}}}}"#
    )
}

/// The main camera looks down at a red cube at the origin; a green cube stands 30 m away,
/// out of its view. `extra` adds entities (each starting with a comma).
fn stage(extra: &str) -> Sim {
    let text = format!(
        r#"{{"format": "marionette.scene", "version": 1,
        "resources": {{ "AmbientLight": {{"color": {{"r": 1, "g": 1, "b": 1, "a": 1}}, "intensity": 1.0}} }},
        "entities": [
          {{"id": 1, "name": "camera", "components": {{
            "Camera": {{"clear_color": {{"r": 0, "g": 0, "b": 0, "a": 1}},
              "projection": {{"Perspective": {{"far": 100.0, "fov_y_degrees": 60.0, "near": 0.1}}}}}},
            "LookAt": {{"target": {{"x": 0, "y": 0, "z": 0}}, "up": {{"x": 0, "y": 0, "z": -1}}}},
            "Transform": {{"translation": {{"x": 0, "y": 12, "z": 0.01}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 1, "y": 1, "z": 1}}}}}}}}
          {red}{green}{extra}
        ]}}"#,
        red = cube(2, 0.0, (1.0, 0.0, 0.0)),
        green = cube(3, 30.0, (0.0, 1.0, 0.0)),
    );
    Sim::from_text(&text, None, default_registry(), 1).unwrap()
}

/// Looking straight down (-Y) from 12 m above `x`.
fn down_at(id: u64, x: f32, image: &str) -> ShotRequest {
    // A quarter turn about +X takes the camera's -Z (forward) to -Y.
    let h = std::f32::consts::FRAC_1_SQRT_2;
    ShotRequest {
        id,
        image: image.into(),
        position: [x, 12.0, 0.0],
        rotation: [-h, 0.0, 0.0, h],
        fov_y_degrees: 60.0,
        near: 0.1,
        far: 100.0,
        width: 64,
        height: 36,
    }
}

fn centre(px: &[u8], w: u32, h: u32) -> [u8; 3] {
    let i = (((h / 2) * w + w / 2) * 4) as usize;
    [px[i], px[i + 1], px[i + 2]]
}

#[test]
fn a_shot_reads_back_the_world_from_its_own_pose_without_the_screen_ui() {
    let Some(gpu) = gpu() else { return };
    // A full-screen white UI rect: on the screen, never in a shot.
    let ui = r#", {"id": 9, "components": {"UiRect": {"color": {"r": 1, "g": 1, "b": 1, "a": 1}},
        "UiNode": {"anchor": "TopLeft", "layer": 0, "offset": {"x": 0, "y": 0}, "size": {"x": 4000, "y": 4000}}}}"#;
    let mut sim = stage(ui);
    sim.world.insert_resource(Shots::default());
    let mut off = Offscreen::new(&gpu, W, H, 4, repo().join("examples"));
    let mut draws = ShotDraws::for_renderer(&off.renderer);
    {
        let mut shots = sim.world.resource_mut::<Shots>();
        assert!(shots.ask(down_at(7, 30.0, "")));
        let mut screen_sized = down_at(8, 30.0, "");
        (screen_sized.width, screen_sized.height) = (0, 0);
        assert!(shots.ask(screen_sized));
    }
    let frame = extract(&sim.world);
    let n = take_now(
        &gpu,
        &mut off.renderer,
        &mut draws,
        &frame,
        &mut sim.world,
        ([0.0, 0.0, 0.0, 1.0], (W, H)),
    )
    .unwrap();
    assert_eq!(n, 2);
    let done = sim.world.resource_mut::<Shots>().take_done();
    assert_eq!(done.len(), 2);
    let full = done.iter().find(|d| d.id == 8).unwrap();
    assert_eq!(
        (full.width, full.height),
        (W, H),
        "0 by 0 takes the screen's size"
    );
    let d = &done[0];
    assert_eq!((d.id, d.width, d.height), (7, 64, 36));
    assert!(d.error.is_empty(), "{}", d.error);
    assert_eq!(d.rgba.len(), 64 * 36 * 4);
    let [r, g, b] = centre(&d.rgba, 64, 36);
    assert!(
        g > 120 && r < 60 && b < 60,
        "the green cube under the shot, no white UI: {r} {g} {b}"
    );
    // The corner is the clear color: the screen's UI rect is not in the shot.
    assert_eq!(&d.rgba[..3], &[0, 0, 0]);
    assert!(sim.world.resource::<Shots>().requests.is_empty());
}

#[test]
fn a_camera_screen_shows_the_last_shot_drawn_into_it() {
    let Some(gpu) = gpu() else { return };
    // The screen image fills the main view's top-left quarter.
    let ui = r#", {"id": 9, "components": {"UiImage": {"color": {"r": 1, "g": 1, "b": 1, "a": 1}, "image": "shot:cam"},
        "UiNode": {"anchor": "TopLeft", "layer": 0, "offset": {"x": 0, "y": 0}, "size": {"x": 80, "y": 45}}}}"#;
    let mut sim = stage(ui);
    sim.world.insert_resource(Shots::default());
    let mut off = Offscreen::new(&gpu, W, H, 1, repo().join("examples"));
    let mut draws = ShotDraws::for_renderer(&off.renderer);

    // Before any shot the screen is black.
    let px = |off: &mut Offscreen, sim: &Sim| {
        off.render(&gpu, &extract(&sim.world), None).unwrap();
        let p = off.read_pixels(&gpu).unwrap();
        let i = ((20 * W + 40) * 4) as usize;
        [p[i], p[i + 1], p[i + 2]]
    };
    assert_eq!(px(&mut off, &sim), [0, 0, 0]);

    sim.world.resource_mut::<Shots>().ask(down_at(1, 30.0, "cam"));
    let frame = extract(&sim.world);
    let n = take_now(
        &gpu,
        &mut off.renderer,
        &mut draws,
        &frame,
        &mut sim.world,
        ([0.0, 0.0, 0.0, 1.0], (W, H)),
    )
    .unwrap();
    assert_eq!(n, 1);
    assert!(
        sim.world.resource::<Shots>().done.is_empty(),
        "screens are not read back"
    );
    let [r, g, b] = px(&mut off, &sim);
    assert!(
        g > 120 && r < 60 && b < 60,
        "the screen shows the green cube: {r} {g} {b}"
    );

    // A new pose replaces the picture (the screen alternates its two textures).
    sim.world.resource_mut::<Shots>().ask(down_at(2, 0.0, "cam"));
    let frame = extract(&sim.world);
    take_now(
        &gpu,
        &mut off.renderer,
        &mut draws,
        &frame,
        &mut sim.world,
        ([0.0, 0.0, 0.0, 1.0], (W, H)),
    )
    .unwrap();
    let [r, g, b] = px(&mut off, &sim);
    assert!(r > 120 && g < 60 && b < 60, "now the red cube: {r} {g} {b}");
}

#[test]
fn shots_wait_without_a_renderer_and_stay_bounded() {
    let mut sim = stage("");
    sim.world.insert_resource(Shots::default());
    for i in 0..20 {
        sim.world.resource_mut::<Shots>().ask(down_at(i, 0.0, ""));
        sim.step();
    }
    assert_eq!(sim.world.resource::<Shots>().requests.len(), Shots::LIMIT);
}

#[test]
fn screen_ui_under_the_cutoff_layer_is_not_drawn() {
    let Some(gpu) = gpu() else { return };
    let ui = r#", {"id": 9, "components": {"UiRect": {"color": {"r": 1, "g": 1, "b": 1, "a": 1}},
        "UiNode": {"anchor": "TopLeft", "layer": 0, "offset": {"x": 0, "y": 0}, "size": {"x": 20, "y": 20}}}},
        {"id": 10, "components": {"UiRect": {"color": {"r": 0, "g": 0, "b": 1, "a": 1}},
        "UiNode": {"anchor": "TopLeft", "layer": 50, "offset": {"x": 30, "y": 0}, "size": {"x": 20, "y": 20}}}}"#;
    let mut sim = stage(ui);
    let mut off = Offscreen::new(&gpu, W, H, 1, repo().join("examples"));
    let mut at = |sim: &Sim, x: u32| {
        off.render(&gpu, &extract(&sim.world), None).unwrap();
        let p = off.read_pixels(&gpu).unwrap();
        let i = ((5 * W + x) * 4) as usize;
        [p[i], p[i + 1], p[i + 2]]
    };
    assert_eq!(at(&sim, 5), [255, 255, 255]);
    sim.world.insert_resource(marionette_core::ui::ScreenUiCutoff {
        on: true,
        min_layer: 10,
    });
    assert_eq!(at(&sim, 5), [0, 0, 0], "the game's layer-0 UI is hidden");
    assert_eq!(at(&sim, 35), [0, 0, 255], "the overlay above the cutoff stays");
}
