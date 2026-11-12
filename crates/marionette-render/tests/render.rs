//! GPU tests. They skip (pass with a note) when no GPU adapter exists, unless
//! MARIONETTE_REQUIRE_GPU=1 is set, which CI and GPU machines use to make skips fail.
//! Set MARIONETTE_UPDATE_REFERENCES=1 to rewrite the reference images.

use marionette_core::{Sim, default_registry};
use marionette_render::assets::{decode_png, encode_png, load_gltf};
use marionette_render::compare::{REFERENCE_THRESHOLD, diff, within_reference_tolerance};
use marionette_render::{Gpu, Offscreen, extract};
use std::path::{Path, PathBuf};

const REF_SIZE: (u32, u32) = (320, 180);
const TICKS: u64 = 60;
const SEED: u64 = 1;

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

fn sim(scene: &str) -> Sim {
    let path = repo().join("examples").join(scene);
    let text = std::fs::read_to_string(&path).unwrap();
    let mut sim = Sim::from_text(&text, None, default_registry(), SEED).unwrap();
    sim.run(TICKS);
    sim
}

fn render(gpu: &Gpu, scene: &str, size: (u32, u32)) -> Vec<u8> {
    let sim = sim(scene);
    let mut off = Offscreen::new(gpu, size.0, size.1, 4, repo().join("examples"));
    off.render(gpu, &extract(&sim.world), None).unwrap();
    off.read_pixels(gpu).unwrap()
}

#[test]
fn same_inputs_give_identical_images() {
    let Some(gpu) = gpu() else { return };
    for scene in ["showcase.scene.json", "sprites.scene.json", "ui.scene.json"] {
        let a = render(&gpu, scene, (480, 270));
        let b = render(&gpu, scene, (480, 270));
        assert!(a == b, "{scene}: two renders differ");
        assert_eq!(encode_png(480, 270, &a), encode_png(480, 270, &b));
    }
}

#[test]
fn rendering_does_not_change_simulation_state() {
    let Some(gpu) = gpu() else { return };
    let mut s = sim("showcase.scene.json");
    let before = s.state_hash();
    let mut off = Offscreen::new(&gpu, 64, 64, 4, repo().join("examples"));
    off.render(&gpu, &extract(&s.world), None).unwrap();
    assert_eq!(s.state_hash(), before);
}

#[test]
fn matches_reference_images() {
    let Some(gpu) = gpu() else { return };
    let update = std::env::var("MARIONETTE_UPDATE_REFERENCES").as_deref() == Ok("1");
    for name in ["showcase", "sprites", "ui"] {
        let pixels = render(&gpu, &format!("{name}.scene.json"), REF_SIZE);
        let path = repo().join(format!("examples/reference/{name}.png"));
        if update {
            std::fs::write(&path, encode_png(REF_SIZE.0, REF_SIZE.1, &pixels)).unwrap();
            continue;
        }
        let reference = decode_png(&std::fs::read(&path).expect("reference image exists")).unwrap();
        assert_eq!((reference.width, reference.height), REF_SIZE);
        let d = diff(&pixels, &reference.rgba, REFERENCE_THRESHOLD);
        eprintln!("{name} on {}: {d:?}", gpu.describe());
        assert!(
            within_reference_tolerance(&d),
            "{name}: {d:?} on {}",
            gpu.describe()
        );
    }
}

#[test]
fn reference_check_catches_real_changes() {
    let Some(gpu) = gpu() else { return };
    let a = render(&gpu, "showcase.scene.json", REF_SIZE);
    let b = render(&gpu, "sprites.scene.json", REF_SIZE);
    assert!(!within_reference_tolerance(&diff(&a, &b, REFERENCE_THRESHOLD)));
}

#[test]
fn bad_camera_and_missing_asset_are_errors_as_data() {
    let Some(gpu) = gpu() else { return };
    let s = sim("showcase.scene.json");
    let mut off = Offscreen::new(&gpu, 32, 32, 1, repo().join("examples"));
    let err = off.render(&gpu, &extract(&s.world), Some(3)).unwrap_err();
    assert_eq!(err.0[0].code, "render.no_camera");
    let mut frame = extract(&s.world);
    frame.meshes[0].asset = "missing/nothing.glb".into();
    let err = off.render(&gpu, &frame, None).unwrap_err();
    assert_eq!(err.0[0].code, "render.asset");
    assert_eq!(err.0[0].entity, Some(frame.meshes[0].entity));
}

#[test]
fn loads_the_example_gltf() {
    let model = load_gltf(&repo().join("examples/assets/gem.glb")).unwrap();
    assert_eq!(model.parts.len(), 1);
    assert_eq!(model.parts[0].mesh.positions.len(), 24);
    assert_eq!(model.images.len(), 3);
    let m = &model.materials[model.parts[0].material];
    assert!(
        m.base_color_texture.is_some()
            && m.metallic_roughness_texture.is_some()
            && m.normal_texture.is_some()
    );
    assert!(model.images[m.base_color_texture.unwrap()].srgb);
    assert!(!model.images[m.normal_texture.unwrap()].srgb);
}
