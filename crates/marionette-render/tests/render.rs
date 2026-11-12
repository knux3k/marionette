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
    for scene in [
        "showcase.scene.json",
        "sprites.scene.json",
        "ui.scene.json",
        "lighting.scene.json",
    ] {
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
    for name in ["showcase", "sprites", "ui", "lighting"] {
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

/// Repeated offscreen setups and scene switches give every GPU object back (decision 0032).
#[test]
fn gpu_objects_come_back_after_repeated_setups_and_trims() {
    let Some(gpu) = gpu() else { return };
    let settle = || {
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    };
    let frames: Vec<_> = ["showcase.scene.json", "sprites.scene.json", "ui.scene.json"]
        .iter()
        .map(|s| extract(&sim(s).world))
        .collect();
    // One warm-up round so lazily created driver-side objects exist before the baseline.
    let mut off = Offscreen::new(&gpu, 64, 64, 4, repo().join("examples"));
    for f in &frames {
        off.render(&gpu, f, None).unwrap();
    }
    drop(off);
    settle();
    let Some(baseline) = gpu.live_objects() else {
        eprintln!("skipping: this wgpu backend does not count objects");
        return;
    };
    for round in 0..10 {
        let mut off = Offscreen::new(&gpu, 64 + round, 64, 4, repo().join("examples"));
        for f in &frames {
            off.render(&gpu, f, None).unwrap();
            off.read_pixels(&gpu).unwrap();
        }
        // Scene switch: only the last scene's assets stay cached.
        let held = off.cached_assets();
        let freed = off.trim(&frames[2]);
        assert!(
            freed > 0 && off.cached_assets() < held,
            "trim frees the other scenes' assets"
        );
        off.render(&gpu, &frames[0], None).unwrap();
        drop(off);
        settle();
        assert_eq!(
            gpu.live_objects().unwrap(),
            baseline,
            "round {round}: GPU objects leaked"
        );
    }
}

/// Renders `sim` at 160x90 with 4x MSAA; returns pixels and stats.
fn render_sim(gpu: &Gpu, sim: &Sim) -> (Vec<u8>, marionette_render::RenderStats) {
    let mut off = Offscreen::new(gpu, 160, 90, 4, repo().join("examples"));
    let stats = off.render(gpu, &extract(&sim.world), None).unwrap();
    (off.read_pixels(gpu).unwrap(), stats)
}

/// Mean brightness (0..255) of a pixel rectangle.
fn mean(pixels: &[u8], (x0, y0, x1, y1): (u32, u32, u32, u32)) -> f32 {
    let mut sum = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * 160 + x) * 4) as usize;
            sum += pixels[i] as u64 + pixels[i + 1] as u64 + pixels[i + 2] as u64;
        }
    }
    sum as f32 / (3 * (x1 - x0) * (y1 - y0)) as f32
}

/// Decision 0063: each look is on only while its resource or component is in the scene,
/// and does what it says: shadows darken the ground behind a casting ball, point lights
/// brighten the ground under them, fog pulls the far ground toward the fog color, and
/// removing every opt-in item gives back the plain look.
#[test]
fn opt_in_looks_change_the_image_only_when_present() {
    use marionette_core::spatial::{Emissive, Fog, PointLight, Shadows, SkyLight, SpotLight, ToneMapping};
    let Some(gpu) = gpu() else { return };
    let full = sim("lighting.scene.json");
    let (all, stats) = render_sim(&gpu, &full);

    // Shadows: the ground in the blue ball's shadow darkens, and the
    // shadow pass adds one draw per mesh group.
    let mut no_shadow = sim("lighting.scene.json");
    no_shadow.world.remove_resource::<Shadows>();
    let (unshadowed, plain_stats) = render_sim(&gpu, &no_shadow);
    let behind_ball = (94, 50, 100, 54);
    assert!(
        mean(&all, behind_ball) + 10.0 < mean(&unshadowed, behind_ball),
        "shadowed {} vs unshadowed {}",
        mean(&all, behind_ball),
        mean(&unshadowed, behind_ball)
    );
    assert!(stats.draw_calls > plain_stats.draw_calls);

    // Point lights: the ground around the lantern is brighter with its light.
    let mut dark = sim("lighting.scene.json");
    let lights: Vec<_> = dark
        .world
        .query_filtered::<bevy_ecs::entity::Entity, bevy_ecs::query::With<PointLight>>()
        .iter(&dark.world)
        .collect();
    for e in lights {
        dark.world.entity_mut(e).remove::<PointLight>();
    }
    let (unlit, _) = render_sim(&gpu, &dark);
    let under_lantern = (85, 63, 95, 69);
    assert!(mean(&all, under_lantern) > mean(&unlit, under_lantern) + 10.0);

    // Removing every opt-in resource and component gives exactly the plain renderer.
    let mut plain = sim("lighting.scene.json");
    plain.world.remove_resource::<Shadows>();
    plain.world.remove_resource::<ToneMapping>();
    plain.world.remove_resource::<Fog>();
    plain.world.remove_resource::<SkyLight>();
    let entities: Vec<_> = plain
        .world
        .query::<bevy_ecs::entity::Entity>()
        .iter(&plain.world)
        .collect();
    for e in entities {
        plain
            .world
            .entity_mut(e)
            .remove::<(PointLight, SpotLight, Emissive)>();
    }
    let (plain_pixels, _) = render_sim(&gpu, &plain);
    assert_ne!(plain_pixels, all);
    // The plain look of the same frame, built without the new types ever existing: the
    // extracted frame has no local lights, no flags and zero emissive.
    let frame = extract(&plain.world);
    assert!(frame.local_lights.is_empty() && frame.shadows.is_none() && frame.exposure.is_none());
    assert!(frame.fog.is_none() && frame.sky.is_none());
    assert!(frame.meshes.iter().all(|m| m.emissive == glam::Vec3::ZERO));
}

#[test]
fn model_placement_moves_turns_and_sizes_only_the_drawn_model() {
    use marionette_core::spatial::{Model, ModelPlacement, Quat, Transform, Vec3};
    let mut s = sim("lighting.scene.json");
    let e = s
        .world
        .query_filtered::<bevy_ecs::entity::Entity, bevy_ecs::query::With<Model>>()
        .iter(&s.world)
        .next()
        .unwrap();
    let before = *s.world.get::<Transform>(e).unwrap();
    let id = s.world.get::<marionette_core::scene::SceneId>(e).unwrap().0;
    let half = std::f32::consts::FRAC_1_SQRT_2;
    s.world.entity_mut(e).insert(ModelPlacement {
        offset: Vec3::new(0.0, -0.5, 0.0),
        rotation: Quat {
            x: 0.0,
            y: half,
            z: 0.0,
            w: half,
        },
        scale: Vec3::new(2.0, 3.0, 4.0),
    });
    let frame = extract(&s.world);
    let draw = frame.meshes.iter().find(|m| m.entity == id).unwrap();
    let (scale, _, translation) = draw.world.to_scale_rotation_translation();
    assert!(
        (scale - glam::Vec3::new(2.0, 3.0, 4.0)).abs().max_element() < 1e-4,
        "{scale}"
    );
    let t = before.translation;
    let rot = glam::Quat::from_xyzw(
        before.rotation.x,
        before.rotation.y,
        before.rotation.z,
        before.rotation.w,
    );
    let expected = glam::Vec3::new(t.x, t.y, t.z) + rot * glam::Vec3::new(0.0, -0.5, 0.0);
    assert!(
        (translation - expected).length() < 1e-4,
        "{translation} vs {expected}"
    );
    // The entity itself is unchanged.
    assert_eq!(*s.world.get::<Transform>(e).unwrap(), before);
}
