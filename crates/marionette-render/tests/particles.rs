//! GPU tests of particles (decision 0272). They skip (pass with a note) when no GPU adapter
//! exists, unless MARIONETTE_REQUIRE_GPU=1.

use marionette_core::{Sim, default_registry};
use marionette_render::{Gpu, Offscreen, RenderStats, extract};
use std::path::{Path, PathBuf};

const W: u32 = 96;
const H: u32 = 96;

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

fn render(gpu: &Gpu, sim: &Sim) -> (Vec<u8>, RenderStats) {
    let mut off = Offscreen::new(gpu, W, H, 4, repo().join("examples"));
    let stats = off.render(gpu, &extract(&sim.world), None).unwrap();
    (off.read_pixels(gpu).unwrap(), stats)
}

fn example(ticks: u64) -> Sim {
    let text = std::fs::read_to_string(repo().join("examples/particles.scene.json")).unwrap();
    let mut sim = Sim::from_text(&text, None, default_registry(), 1).unwrap();
    sim.run(ticks);
    sim
}

/// A camera 4 m in front of the origin, black background, and an additive emitter at the
/// origin shooting up with `rate` particles per second.
fn one_emitter(rate: f32) -> Sim {
    let text = format!(
        r#"{{"format": "marionette.scene", "version": 1, "resources": {{}},
        "entities": [
          {{"id": 1, "name": "camera", "components": {{
            "Camera": {{"clear_color": {{"r": 0, "g": 0, "b": 0, "a": 1}},
              "projection": {{"Perspective": {{"far": 100.0, "fov_y_degrees": 60.0, "near": 0.1}}}}}},
            "Transform": {{"translation": {{"x": 0, "y": 0, "z": 4}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 1, "y": 1, "z": 1}}}}}}}},
          {{"id": 2, "name": "emitter", "components": {{
            "ParticleEmitter": {{"rate": {rate}, "lifetime": 1.0, "speed": 1.0, "spread_degrees": 10.0,
              "radius": 0.0, "gravity": 0.0, "start_size": 0.3, "end_size": 0.3,
              "start_color": {{"r": 1, "g": 0.5, "b": 0.2, "a": 1}}, "end_color": {{"r": 1, "g": 0.5, "b": 0.2, "a": 1}},
              "image": "builtin:white", "blend": "Additive", "seed": 7}},
            "Transform": {{"translation": {{"x": 0, "y": -1, "z": 0}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 1, "y": 1, "z": 1}}}}}}}}
        ]}}"#
    );
    let mut sim = Sim::from_text(&text, None, default_registry(), 1).unwrap();
    sim.run(90);
    sim
}

/// Brightness summed over a column band (x in `x0..x1`) of the image.
fn band(pixels: &[u8], x0: u32, x1: u32) -> u64 {
    let mut sum = 0u64;
    for y in 0..H {
        for x in x0..x1 {
            let i = ((y * W + x) * 4) as usize;
            sum += pixels[i] as u64 + pixels[i + 1] as u64 + pixels[i + 2] as u64;
        }
    }
    sum
}

/// Particles depend on game time only: the same tick gives the same picture from two fresh
/// runs, the next tick moves them, and drawing them changes no simulation state.
#[test]
fn particles_are_a_pure_function_of_game_time() {
    let Some(gpu) = gpu() else { return };
    let mut a = example(120);
    let before = a.state_hash();
    let (first, _) = render(&gpu, &a);
    assert_eq!(a.state_hash(), before);
    let (again, _) = render(&gpu, &example(120));
    assert!(first == again, "same tick, different particles");
    let (next, _) = render(&gpu, &example(121));
    assert!(first != next, "particles did not move between ticks");
}

/// The CPU cost is one draw per emitter, whatever the particle count.
#[test]
fn one_draw_per_emitter_whatever_the_rate() {
    let Some(gpu) = gpu() else { return };
    let (_, few) = render(&gpu, &one_emitter(10.0));
    let (_, many) = render(&gpu, &one_emitter(20_000.0));
    let (_, none) = render(&gpu, &one_emitter(0.0));
    assert_eq!(few.draw_calls, many.draw_calls);
    assert_eq!(few.draw_calls, none.draw_calls + 1);
}

/// They are drawn where the emitter is and in its direction, and nothing at a rate of 0.
#[test]
fn particles_draw_along_the_emitter() {
    let Some(gpu) = gpu() else { return };
    let (lit, _) = render(&gpu, &one_emitter(200.0));
    let (dark, _) = render(&gpu, &one_emitter(0.0));
    assert_eq!(band(&dark, 0, W), 0, "nothing drawn without particles");
    let middle = band(&lit, W / 2 - 8, W / 2 + 8);
    let sides = band(&lit, 0, 16) + band(&lit, W - 16, W);
    assert!(middle > 1000, "particles visible above the emitter: {middle}");
    assert!(
        sides * 20 < middle,
        "particles stay in their narrow cone: sides {sides}, middle {middle}"
    );
}
