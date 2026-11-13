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
        "looks.scene.json",
        "raytracing.scene.json",
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
    // The ray tracing example looks different without ray queries (its fallback looks), so it
    // has a reference image for each (decision 0093).
    let rt = if gpu.ray_tracing() {
        "raytracing"
    } else {
        "raytracing-fallback"
    };
    for (name, image) in [
        ("showcase", "showcase"),
        ("sprites", "sprites"),
        ("ui", "ui"),
        ("lighting", "lighting"),
        ("looks", "looks"),
        ("raytracing", rt),
    ] {
        let pixels = render(&gpu, &format!("{name}.scene.json"), REF_SIZE);
        let path = repo().join(format!("examples/reference/{image}.png"));
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
    let frames: Vec<_> = [
        "showcase.scene.json",
        "looks.scene.json",
        "sprites.scene.json",
        "ui.scene.json",
    ]
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
        let freed = off.trim(&frames[3]);
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

/// A top-down test stage: a gray ground, a 1 m red box at x = 2, a camera 12 m up looking
/// down (x to the right, z down in the image), plus `extra` entities and `resources`.
fn stage(extra: &str, resources: &str) -> Sim {
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
            "Material": {{"base_color": {{"r": 0.8, "g": 0.8, "b": 0.8, "a": 1}}, "metallic": 0, "roughness": 0.9}},
            "Transform": {{"translation": {{"x": 0, "y": 0, "z": 0}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 30, "y": 1, "z": 30}}}}}}}},
          {{"id": 3, "name": "box", "components": {{
            "Model": {{"asset": "primitive:cube"}},
            "Material": {{"base_color": {{"r": 0.8, "g": 0.2, "b": 0.2, "a": 1}}, "metallic": 0, "roughness": 0.9}},
            "Transform": {{"translation": {{"x": 2, "y": 0.5, "z": 0}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 1, "y": 1, "z": 1}}}}}}}}
          {extra}
        ]}}"#
    );
    Sim::from_text(&text, None, default_registry(), SEED).unwrap()
}

/// Pixel x of world x on the stage at 160x90 (12 m up, 60 degree vertical view).
fn stage_x(x: f32) -> u32 {
    let half_h = 12.0 * (30f32).to_radians().tan();
    (80.0 + x / half_h * 45.0).round() as u32
}

/// Decision 0065: a point light with `CastShadows` darkens the ground behind a box (as seen
/// from the light) and leaves the near side lit; a spot light does the same; without the
/// component the ground there is lit. Each point light map is six depth passes.
#[test]
fn point_and_spot_lights_cast_shadows_only_with_cast_shadows() {
    let Some(gpu) = gpu() else { return };
    let point = |shadow: &str| {
        format!(
            r#", {{"id": 4, "name": "lamp", "components": {{
              "PointLight": {{"color": {{"r": 1, "g": 1, "b": 1, "a": 1}}, "intensity": 6.0, "range": 10.0}},
              {shadow}
              "Transform": {{"translation": {{"x": 0, "y": 2, "z": 0}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 1, "y": 1, "z": 1}}}}}}}}"#
        )
    };
    let cast = r#""CastShadows": {"resolution": 512, "softness": 1.0},"#;
    let (lit, plain_stats) = render_sim(&gpu, &stage(&point(""), ""));
    let (shadowed, stats) = render_sim(&gpu, &stage(&point(cast), ""));
    // Behind the box (x = 3.5 m), in front of it (x = -1.5 m, same distance from the lamp).
    let behind = (stage_x(3.5) - 1, 43, stage_x(3.5) + 2, 47);
    let front = (stage_x(-1.5) - 1, 43, stage_x(-1.5) + 2, 47);
    assert!(
        mean(&shadowed, behind) + 20.0 < mean(&lit, behind),
        "{} vs {}",
        mean(&shadowed, behind),
        mean(&lit, behind)
    );
    assert!((mean(&shadowed, front) - mean(&lit, front)).abs() < 3.0);
    assert!(stats.draw_calls >= plain_stats.draw_calls + 6 * 2);

    let spot = format!(
        r#", {{"id": 4, "name": "spot", "components": {{
          "SpotLight": {{"direction": {{"x": 0.0, "y": -1.0, "z": 0.0}}, "color": {{"r": 1, "g": 1, "b": 1, "a": 1}},
            "intensity": 8.0, "range": 10.0, "inner_angle_degrees": 65.0, "outer_angle_degrees": 75.0}},
          {cast}
          "Transform": {{"translation": {{"x": 0, "y": 2, "z": 0}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 1, "y": 1, "z": 1}}}}}}}}"#
    );
    let (spot_shadowed, _) = render_sim(&gpu, &stage(&spot, ""));
    let (spot_lit, _) = render_sim(&gpu, &stage(&spot.replace(cast, ""), ""));
    assert!(mean(&spot_shadowed, behind) + 20.0 < mean(&spot_lit, behind));
    assert!((mean(&spot_shadowed, front) - mean(&spot_lit, front)).abs() < 3.0);
}

/// Decision 0064: bloom spreads a bright emissive box's light onto the ground around it,
/// adds the glow passes, and changes nothing far from bright light.
#[test]
fn bloom_spreads_bright_light_and_only_with_bloom() {
    let Some(gpu) = gpu() else { return };
    let glow = r#", {"id": 4, "name": "glow", "components": {
        "Model": {"asset": "primitive:cube"},
        "Material": {"base_color": {"r": 1, "g": 1, "b": 1, "a": 1}, "metallic": 0, "roughness": 0.9},
        "Emissive": {"color": {"r": 1, "g": 0.8, "b": 0.5, "a": 1}, "intensity": 6.0},
        "Transform": {"translation": {"x": -3, "y": 0.5, "z": 0}, "rotation": {"x": 0, "y": 0, "z": 0, "w": 1}, "scale": {"x": 1, "y": 1, "z": 1}}}}"#;
    let light = r#""AmbientLight": {"color": {"r": 1, "g": 1, "b": 1, "a": 1}, "intensity": 0.2},
        "ToneMapping": {"exposure": 1.0}"#;
    let bloom = r#", "Bloom": {"threshold": 1.0, "intensity": 0.8}"#;
    let (plain, plain_stats) = render_sim(&gpu, &stage(glow, light));
    let (bloomed, stats) = render_sim(&gpu, &stage(glow, &format!("{light}{bloom}")));
    // Ground just right of the glowing box (box spans x = -3.5 .. -2.5 m).
    let beside = (stage_x(-2.2), 43, stage_x(-2.2) + 2, 47);
    assert!(
        mean(&bloomed, beside) > mean(&plain, beside) + 5.0,
        "{} vs {}",
        mean(&bloomed, beside),
        mean(&plain, beside)
    );
    // Far corner: no bright light nearby, same image (within rounding).
    let far = (150, 80, 158, 88);
    assert!((mean(&bloomed, far) - mean(&plain, far)).abs() < 1.5);
    assert!(stats.draw_calls > plain_stats.draw_calls + 3);
}

/// Decision 0066: with `Environment`, a mirror ball reflects the sky (its top turns sky
/// colored), a bad image path is an error as data, and the sky background replaces the
/// clear color.
#[test]
fn environment_lights_reflects_and_draws_the_sky() {
    let Some(gpu) = gpu() else { return };
    let ball = r#", {"id": 4, "name": "mirror", "components": {
        "Model": {"asset": "primitive:sphere"},
        "Material": {"base_color": {"r": 1, "g": 1, "b": 1, "a": 1}, "metallic": 1.0, "roughness": 0.05},
        "Transform": {"translation": {"x": -2, "y": 1, "z": 0}, "rotation": {"x": 0, "y": 0, "z": 0, "w": 1}, "scale": {"x": 2, "y": 2, "z": 2}}}}"#;
    let sky = r#""SkyLight": {"sky": {"r": 0.2, "g": 0.4, "b": 1.0, "a": 1}, "ground": {"r": 0.1, "g": 0.1, "b": 0.1, "a": 1}, "intensity": 0.3}"#;
    let env = r#", "Environment": {"image": "", "intensity": 1.0, "background": false}"#;
    let (flat, _) = render_sim(&gpu, &stage(ball, sky));
    let (reflecting, _) = render_sim(&gpu, &stage(ball, &format!("{sky}{env}")));
    let top = (stage_x(-2.0) - 1, 44, stage_x(-2.0) + 2, 46);
    let blue = |p: &[u8], (x0, y0, x1, y1): (u32, u32, u32, u32)| {
        let mut s = 0.0;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = ((y * 160 + x) * 4) as usize;
                s += p[i + 2] as f32 - p[i] as f32;
            }
        }
        s / ((x1 - x0) * (y1 - y0)) as f32
    };
    assert!(
        blue(&reflecting, top) > blue(&flat, top) + 30.0,
        "{} vs {}",
        blue(&reflecting, top),
        blue(&flat, top)
    );

    // Background: a camera looking at the horizon sees sky instead of black.
    let mut s = stage(ball, &format!("{sky}{}", env.replace("false", "true")));
    let mut q = s.world.query::<&mut marionette_core::spatial::Transform>();
    for mut t in q.iter_mut(&mut s.world) {
        if t.translation.y == 12.0 {
            t.translation = marionette_core::spatial::Vec3 {
                x: 0.0,
                y: 0.5,
                z: 8.0,
            };
        }
    }
    let camera = s
        .world
        .query_filtered::<bevy_ecs::entity::Entity, bevy_ecs::query::With<marionette_core::spatial::Camera>>()
        .iter(&s.world)
        .next()
        .unwrap();
    s.world
        .entity_mut(camera)
        .remove::<marionette_core::spatial::LookAt>();
    let (with_sky, _) = render_sim(&gpu, &s);
    assert!(mean(&with_sky, (0, 0, 160, 10)) > 40.0);

    let bad = stage(
        ball,
        &format!(
            "{sky}{}",
            env.replace(r#""image": """#, r#""image": "missing/sky.png""#)
        ),
    );
    let mut off = Offscreen::new(&gpu, 32, 32, 1, repo().join("examples"));
    let err = off.render(&gpu, &extract(&bad.world), None).unwrap_err();
    assert_eq!(err.0[0].component.as_deref(), Some("Environment"));
}

/// Decision 0067: glTF emissive factors, `KHR_materials_emissive_strength` and emissive
/// textures are imported; the lamp's lit panes glow.
#[test]
fn imports_gltf_emissive() {
    let model = load_gltf(&repo().join("examples/assets/lamp.glb")).unwrap();
    let lantern = &model.materials[1];
    assert_eq!(lantern.emissive, [4.0, 2.88, 1.52]);
    let t = lantern.emissive_texture.expect("emissive texture");
    assert!(model.images[t].srgb);
    assert_eq!(model.materials[0].emissive, [0.0; 3]);
}

#[test]
fn streaming_decodes_late_models_off_the_frame() {
    // Decision 0068: with streaming on, a model file first needed after the first frame is
    // decoded in the background; the frame that first needs it does not wait for it.
    let Some(gpu) = gpu() else { return };
    let sprites = extract(&sim("sprites.scene.json").world);
    let showcase = extract(&sim("showcase.scene.json").world);
    let exact = render(&gpu, "showcase.scene.json", (64, 64));

    let mut off = Offscreen::new(&gpu, 64, 64, 4, repo().join("examples"));
    off.set_streaming(true);
    off.render(&gpu, &sprites, None).unwrap(); // first frame: loads in place
    off.render(&gpu, &showcase, None).unwrap();
    assert_eq!(off.models_loading(), 1, "gem.glb decodes in the background");
    off.finish_loading();
    assert_eq!(off.models_loading(), 0);
    off.render(&gpu, &showcase, None).unwrap();
    assert!(
        off.read_pixels(&gpu).unwrap() == exact,
        "once loaded, the frame equals an exact render"
    );

    // The first prepared frame always loads everything, so pictures stay exact.
    let mut first = Offscreen::new(&gpu, 64, 64, 4, repo().join("examples"));
    first.set_streaming(true);
    first.render(&gpu, &showcase, None).unwrap();
    assert_eq!(first.models_loading(), 0);
    assert!(first.read_pixels(&gpu).unwrap() == exact);
}

fn render_with(gpu: &Gpu, sim: &Sim, culling: bool) -> (Vec<u8>, marionette_render::RenderStats) {
    let mut off = Offscreen::new(gpu, 160, 90, 4, repo().join("examples"));
    off.set_culling(culling);
    let stats = off.render(gpu, &extract(&sim.world), None).unwrap();
    (off.read_pixels(gpu).unwrap(), stats)
}

fn cube(id: u32, x: f32, y: f32, z: f32, sy: f32) -> String {
    format!(
        r#", {{"id": {id}, "name": "c{id}", "components": {{
        "Model": {{"asset": "primitive:cube"}},
        "Material": {{"base_color": {{"r": 0.3, "g": 0.6, "b": 0.9, "a": 1}}, "metallic": 0, "roughness": 0.9}},
        "Transform": {{"translation": {{"x": {x}, "y": {y}, "z": {z}}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 1, "y": {sy}, "z": 1}}}}}}}}"#
    )
}

/// Decision 0080: frustum culling leaves the picture unchanged (exactly) while things
/// outside the view are not drawn; a tower outside the view still casts its shadow into it.
#[test]
fn culling_drops_unseen_instances_without_changing_the_image() {
    let Some(gpu) = gpu() else { return };
    let mut extra = String::new();
    // A box up near the camera and off to the side (the view is only 4 m wide up there):
    // the low sun throws its shadow into the middle of the view.
    extra += &cube(10, 10.0, 10.0, 0.0, 1.0);
    // A field of boxes far away in every direction: drawn nowhere.
    let mut id = 20;
    for gx in -5..=5 {
        for gz in -5..=5 {
            if gx == 0 && gz == 0 {
                continue;
            }
            extra += &cube(id, gx as f32 * 80.0, 0.5, gz as f32 * 80.0, 1.0);
            id += 1;
        }
    }
    extra += r#", {"id": 9, "name": "sun", "components": {"DirectionalLight": {"color": {"r": 1, "g": 1, "b": 1, "a": 1}, "direction": {"x": -1, "y": -1, "z": 0}, "illuminance": 2.0}}}"#;
    let shadows = r#""AmbientLight": {"color": {"r": 1, "g": 1, "b": 1, "a": 1}, "intensity": 0.2},
        "Shadows": {"distance": 40.0, "resolution": 1024, "softness": 0.0}"#;
    let sim = stage(&extra, shadows);
    let (all, all_stats) = render_with(&gpu, &sim, false);
    let (culled, stats) = render_with(&gpu, &sim, true);
    assert!(all == culled, "culling changed pixels");
    assert_eq!(all_stats.culled, 0);
    // Ground and red box seen; the high box (shadow only) and the field are not drawn in view.
    assert_eq!(stats.instances, 2, "{stats:?}");
    assert_eq!(stats.culled, 121, "{stats:?}");
    assert!(all_stats.instances > 100);
    // The high box's shadow lands mid-view: darker than open ground.
    let shade = (stage_x(-0.2), 43, stage_x(0.2), 47);
    let open = (stage_x(-8.0), 43, stage_x(-7.6), 47);
    assert!(
        mean(&culled, shade) + 10.0 < mean(&culled, open),
        "{} vs {}",
        mean(&culled, shade),
        mean(&culled, open)
    );
}

/// Decision 0080: with `LevelOfDetail`, a far sphere draws a reduced mesh (fewer triangles,
/// nearly the same picture) and a tiny one is hidden; without it nothing changes.
#[test]
fn level_of_detail_reduces_far_triangles_only_when_asked() {
    let Some(gpu) = gpu() else { return };
    let ball = |id: u32, x: f32, s: f32| {
        format!(
            r#", {{"id": {id}, "name": "b{id}", "components": {{
            "Model": {{"asset": "primitive:sphere"}},
            "Material": {{"base_color": {{"r": 0.9, "g": 0.9, "b": 0.2, "a": 1}}, "metallic": 0, "roughness": 0.6}},
            "Transform": {{"translation": {{"x": {x}, "y": 1, "z": 0}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": {s}, "y": {s}, "z": {s}}}}}}}}}"#
        )
    };
    let extra = format!(
        "{}{}{}",
        ball(4, -4.0, 1.0),
        ball(5, 4.0, 1.0),
        ball(6, -8.0, 0.01)
    );
    let light = r#""AmbientLight": {"color": {"r": 1, "g": 1, "b": 1, "a": 1}, "intensity": 1.0}"#;
    let lod = r#", "LevelOfDetail": {"medium_below": 0.3, "low_below": 0.0, "hide_below": 0.005}"#;
    let (full, full_stats) = render_with(&gpu, &stage(&extra, light), true);
    let (reduced, stats) = render_with(&gpu, &stage(&extra, &format!("{light}{lod}")), true);
    assert_eq!(full_stats.instances, 5);
    assert_eq!(stats.instances, 4, "the tiny ball is hidden: {stats:?}");
    assert!(
        stats.triangles * 10 < full_stats.triangles * 8,
        "{} vs {}",
        stats.triangles,
        full_stats.triangles
    );
    let d = diff(&full, &reduced, REFERENCE_THRESHOLD);
    assert!(d.mean < 2.0, "{d:?}");
}

fn render_scene_with(
    gpu: &Gpu,
    scene: &str,
    resources: &str,
    size: (u32, u32),
) -> (Vec<u8>, marionette_render::RenderStats) {
    let path = repo().join("examples").join(scene);
    let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let extra: serde_json::Value = serde_json::from_str(&format!("{{{resources}}}")).unwrap();
    for (k, val) in extra.as_object().unwrap() {
        v["resources"][k] = val.clone();
    }
    let mut sim = Sim::from_text(&v.to_string(), None, default_registry(), SEED).unwrap();
    sim.run(TICKS);
    let mut off = Offscreen::new(gpu, size.0, size.1, 4, repo().join("examples"));
    let stats = off.render(gpu, &extract(&sim.world), None).unwrap();
    (off.read_pixels(gpu).unwrap(), stats)
}

/// Decision 0081: render scale draws the world smaller (or larger) and resolves it to the
/// output; the picture stays close to native in the plain and the bloom path, and scale 1
/// without foveation takes the direct path (identical image, no extra pass).
#[test]
fn render_scale_resizes_the_world_and_keeps_the_picture() {
    let Some(gpu) = gpu() else { return };
    let size = (320, 180);
    for scene in ["lighting.scene.json", "looks.scene.json"] {
        let (native, native_stats) = render_scene_with(&gpu, scene, "", size);
        let (same, same_stats) = render_scene_with(
            &gpu,
            scene,
            r#""RenderScale": {"scale": 1.0, "sharpness": 0.5}"#,
            size,
        );
        assert!(native == same, "{scene}: scale 1 changed pixels");
        assert_eq!(native_stats.draw_calls, same_stats.draw_calls);
        for (scale, limit) in [(0.75, 4.0), (0.5, 6.0), (2.0, 3.0)] {
            let res = format!(r#""RenderScale": {{"scale": {scale}, "sharpness": 0.3}}"#);
            let (scaled, stats) = render_scene_with(&gpu, scene, &res, size);
            let d = diff(&native, &scaled, REFERENCE_THRESHOLD);
            assert!(d.mean < limit, "{scene} at {scale}: {d:?}");
            // One resolve pass (bloom's glow chain also changes length with the image size).
            if scene == "lighting.scene.json" {
                assert_eq!(stats.draw_calls, native_stats.draw_calls + 1, "{scene}");
            }
        }
    }
}

/// Decision 0082: foveation leaves the middle of the view as it was and fills the skipped
/// quads at the edges from their neighbours (no black holes), in the plain and bloom paths.
#[test]
fn foveation_keeps_the_center_and_fills_the_edges() {
    let Some(gpu) = gpu() else { return };
    let size = (320, 180);
    for scene in ["lighting.scene.json", "looks.scene.json"] {
        let (native, _) = render_scene_with(&gpu, scene, "", size);
        let (fov, _) = render_scene_with(&gpu, scene, r#""Foveation": {"inner_radius": 0.5}"#, size);
        let px = |img: &[u8], x: u32, y: u32| {
            let i = ((y * size.0 + x) * 4) as usize;
            [img[i], img[i + 1], img[i + 2]]
        };
        // Center block: identical without bloom; with bloom the glow blurs a little of the
        // filled edge in, so within 2 of 255.
        let tolerance = if scene == "looks.scene.json" { 2 } else { 0 };
        for y in 80..100 {
            for x in 150..170 {
                let (a, b) = (px(&native, x, y), px(&fov, x, y));
                let worst = (0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap();
                assert!(worst <= tolerance, "{scene} center ({x},{y}): {a:?} vs {b:?}");
            }
        }
        let d = diff(&native, &fov, REFERENCE_THRESHOLD);
        assert!(d.mean < 3.0, "{scene}: {d:?}");
        assert!(d.max < 160, "{scene}: no unfilled holes: {d:?}");
    }
}
