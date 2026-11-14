//! GPU tests of single-pass stereo (decision 0285): both eyes drawn by one multiview pass
//! must give the same pictures as one pass per eye, with each draw recorded once. They skip
//! (pass with a note) without a GPU adapter, unless MARIONETTE_REQUIRE_GPU=1, and without
//! multiview support.

use marionette_core::{Sim, default_registry};
use marionette_render::compare::{REFERENCE_THRESHOLD, diff};
use marionette_render::{Gpu, Offscreen, RenderStats, extract};
use std::path::{Path, PathBuf};

const W: u32 = 192;
const H: u32 = 160;
const IPD: f32 = 0.064;

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

/// Loads an example scene with the core registry; components from other modules (the
/// animation scene's `Animator` and `Script`) are dropped, so skinned models stand in their
/// rest pose.
fn sim(scene: &str, ticks: u64) -> Sim {
    let text = std::fs::read_to_string(repo().join("examples").join(scene)).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    for e in json["entities"].as_array_mut().unwrap() {
        if let Some(c) = e["components"].as_object_mut() {
            c.remove("Animator");
            c.remove("Script");
        }
    }
    let text = json.to_string();
    let mut sim = Sim::from_text(&text, None, default_registry(), 1).unwrap();
    sim.run(ticks);
    sim
}

/// Both eyes, the last eye's picture and the stats, with or without single-pass stereo.
fn eyes(gpu: &Gpu, sim: &Sim, single_pass: bool) -> Option<(Vec<u8>, RenderStats)> {
    let mut off = Offscreen::new(gpu, W, H, 4, repo().join("examples"));
    if !off.set_single_pass_stereo(gpu, single_pass) && single_pass {
        return None;
    }
    let frame = extract(&sim.world);
    // Twice: models stream in on the first frame.
    off.render_eyes(gpu, &frame, None, 2, IPD).unwrap();
    off.finish_loading();
    let stats = off.render_eyes(gpu, &frame, None, 2, IPD).unwrap();
    Some((off.read_pixels(gpu).unwrap(), stats))
}

fn compare(scene: &str, ticks: u64) -> Option<(RenderStats, RenderStats)> {
    let gpu = gpu()?;
    let sim = sim(scene, ticks);
    let (two, two_stats) = eyes(&gpu, &sim, false).unwrap();
    let Some((one, one_stats)) = eyes(&gpu, &sim, true) else {
        eprintln!("skipping: no multiview on {}", gpu.describe());
        return None;
    };
    let d = diff(&two, &one, REFERENCE_THRESHOLD);
    eprintln!("{scene}: {d:?}, passes {two_stats:?} / single {one_stats:?}");
    assert!(d.max <= 2, "single-pass stereo draws a different picture: {d:?}");
    Some((two_stats, one_stats))
}

/// Textured, cut-out and see-through models, shadows, environment and tone mapping: one
/// pass, the same right eye, half the draw calls of the eyes.
#[test]
fn one_pass_draws_both_eyes_like_two() {
    let Some((two, one)) = compare("materials.scene.json", 1) else {
        return;
    };
    assert_eq!(two.views, 2);
    assert_eq!(one.views, 1);
    assert!(
        one.draw_calls < two.draw_calls,
        "{} vs {}",
        one.draw_calls,
        two.draw_calls
    );
}

/// Skinned models in one pass.
#[test]
fn skinned_models_draw_in_one_pass() {
    let Some((_, one)) = compare("animation.scene.json", 30) else {
        return;
    };
    assert_eq!(one.views, 1);
}

/// Frames with something the single pass does not cover (bloom here) fall back to a pass
/// per eye and still match.
#[test]
fn bloom_falls_back_to_a_pass_per_eye() {
    let Some((_, one)) = compare("particles.scene.json", 60) else {
        return;
    };
    assert_eq!(one.views, 2);
}
