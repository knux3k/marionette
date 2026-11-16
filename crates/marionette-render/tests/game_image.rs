//! Images made in game code (decision 0184): drawn through `game:<name>`, rewritten in place
//! every frame, resized, removed. GPU tests skip without an adapter unless
//! MARIONETTE_REQUIRE_GPU=1.

use marionette_core::image::{GameImage, GameImages};
use marionette_core::{Sim, default_registry};
use marionette_render::{Gpu, Offscreen, extract};
use std::path::{Path, PathBuf};

const W: u32 = 64;
const H: u32 = 64;

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

/// A camera 10 m back looking down -z at a sprite that fills the view.
fn stage() -> Sim {
    let text = r#"{"format": "marionette.scene", "version": 1, "resources": {},
        "entities": [
          {"id": 1, "name": "camera", "components": {
            "Camera": {"clear_color": {"r": 0, "g": 0, "b": 0, "a": 1},
              "projection": {"Perspective": {"far": 100.0, "fov_y_degrees": 60.0, "near": 0.1}}},
            "Transform": {"translation": {"x": 0, "y": 0, "z": 10}, "rotation": {"x": 0, "y": 0, "z": 0, "w": 1}, "scale": {"x": 1, "y": 1, "z": 1}}}},
          {"id": 2, "name": "screen", "components": {
            "Sprite": {"image": "game:screen", "size": {"x": 40, "y": 40}, "color": {"r": 1, "g": 1, "b": 1, "a": 1}, "layer": 0},
            "Transform": {"translation": {"x": 0, "y": 0, "z": 0}, "rotation": {"x": 0, "y": 0, "z": 0, "w": 1}, "scale": {"x": 1, "y": 1, "z": 1}}}}
        ]}"#;
    Sim::from_text(text, None, default_registry(), 1).unwrap()
}

fn solid(w: u32, h: u32, c: [u8; 4]) -> GameImage {
    GameImage {
        width: w,
        height: h,
        rgba: c.repeat((w * h) as usize),
        nearest: true,
    }
}

fn centre(px: &[u8]) -> [u8; 3] {
    let i = (((H / 2) * W + W / 2) * 4) as usize;
    [px[i], px[i + 1], px[i + 2]]
}

fn images(sim: &mut Sim) -> bevy_ecs::world::Mut<'_, GameImages> {
    sim.world.get_resource_or_insert_with(GameImages::default)
}

#[test]
fn an_image_made_in_code_draws_updates_and_goes_away() {
    let Some(gpu) = gpu() else { return };
    let mut sim = stage();
    let mut off = Offscreen::new(&gpu, W, H, 1, repo().join("examples"));

    // Not stored yet: an error as data naming the image.
    let err = off.render(&gpu, &extract(&sim.world), None).unwrap_err();
    assert!(err.0[0].message.contains("screen"), "{}", err.0[0].message);

    images(&mut sim)
        .set("screen", solid(8, 8, [255, 0, 0, 255]))
        .unwrap();
    off.render(&gpu, &extract(&sim.world), None).unwrap();
    let [r, g, b] = centre(&off.read_pixels(&gpu).unwrap());
    assert!(r > 200 && g < 40 && b < 40, "red: {r} {g} {b}");
    let held = off.cached_assets();

    // Same size: written into the same texture, nothing new cached.
    images(&mut sim)
        .update("screen", |px| {
            for p in px.chunks_mut(4) {
                p.copy_from_slice(&[0, 255, 0, 255]);
            }
        })
        .unwrap();
    off.render(&gpu, &extract(&sim.world), None).unwrap();
    let [r, g, _] = centre(&off.read_pixels(&gpu).unwrap());
    assert!(g > 200 && r < 40, "updated in place to green: {r} {g}");
    assert_eq!(off.cached_assets(), held);

    // New size: a new texture.
    images(&mut sim)
        .set("screen", solid(3, 5, [0, 0, 255, 255]))
        .unwrap();
    off.render(&gpu, &extract(&sim.world), None).unwrap();
    let [r, _, b] = centre(&off.read_pixels(&gpu).unwrap());
    assert!(b > 200 && r < 40, "resized and blue: {r} {b}");

    // Removed while a sprite still draws it: an error again.
    images(&mut sim).remove("screen");
    assert!(off.render(&gpu, &extract(&sim.world), None).is_err());
}
