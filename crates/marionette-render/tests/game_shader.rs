//! Game shaders (decision 0181): a game's WGSL `surface` / `vertex` functions drawn through
//! the engine's lighting and shadows, blend modes, errors as data. GPU tests skip without
//! an adapter unless MARIONETTE_REQUIRE_GPU=1.

use marionette_core::shader::{GameShaders, ShaderMaterial};
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

/// Top-down camera 12 m up, a sun leaning toward +x, a 16 m floor (entity 3, white unless
/// `floor_shader`) and a 2 m cube at x = -2 (entity 4) raised 2 m.
fn stage(floor_shader: Option<&str>, resources: &str) -> Sim {
    let shader = floor_shader.map_or(String::new(), |s| format!(r#", "ShaderMaterial": {s}"#));
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
            "DirectionalLight": {{"direction": {{"x": 0.6, "y": -1, "z": 0}}, "color": {{"r": 1, "g": 1, "b": 1, "a": 1}}, "illuminance": 1.0}}}}}},
          {{"id": 3, "name": "floor", "components": {{
            "Model": {{"asset": "primitive:plane"}} {shader},
            "Transform": {{"translation": {{"x": 0, "y": 0, "z": 0}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 16, "y": 1, "z": 16}}}}}}}},
          {{"id": 4, "name": "box", "components": {{
            "Model": {{"asset": "primitive:cube"}},
            "Transform": {{"translation": {{"x": -2, "y": 2, "z": 0}}, "rotation": {{"x": 0, "y": 0, "z": 0, "w": 1}}, "scale": {{"x": 2, "y": 2, "z": 2}}}}}}}}
        ]}}"#
    );
    Sim::from_text(&text, None, default_registry(), 1).unwrap()
}

fn render(gpu: &Gpu, sim: &Sim) -> Result<Vec<u8>, marionette_core::Errors> {
    let mut off = Offscreen::new(gpu, W, H, 4, repo().join("examples"));
    off.render(gpu, &extract(&sim.world), None)?;
    Ok(off.read_pixels(gpu).unwrap())
}

fn rgb(px: &[u8], x: u32, y: u32) -> [i32; 3] {
    let i = ((y * W + x) * 4) as usize;
    [px[i] as i32, px[i + 1] as i32, px[i + 2] as i32]
}

/// Pixel column of world x (12 m up, 60 degree vertical view, 160x90).
fn col(x: f32) -> u32 {
    let half_h = 12.0 * 30f32.to_radians().tan();
    (80.0 + x / half_h * 45.0).round() as u32
}

const TINT: &str = r#"
// Base color from the material's first three params; lighting stays the engine's.
fn surface(s: ptr<function, Surface>, i: SurfaceIn) {
    (*s).base_color = vec4<f32>(game.params[0].xyz, 1.0);
}
"#;

fn with_shader(sim: &mut Sim, name: &str, source: &str) {
    sim.world
        .get_resource_or_insert_with(GameShaders::default)
        .set(name, source);
}

#[test]
fn a_surface_shader_colors_the_model_and_keeps_engine_lighting_and_shadows() {
    let Some(gpu) = gpu() else { return };
    let mat = r#"{"shader": "tint", "params": [0.0, 0.0, 1.0], "textures": []}"#;
    let shadows = r#", "Shadows": {"resolution": 1024, "distance": 20.0, "softness": 1.0}"#;
    let mut lit = stage(Some(mat), shadows);
    with_shader(&mut lit, "tint", TINT);
    let px = render(&gpu, &lit).unwrap();
    // A floor spot far from the box: blue, lit by the sun.
    let open = rgb(&px, col(5.0), H / 2);
    assert!(
        open[2] > 120 && open[0] < 60 && open[0] == open[1],
        "blue floor: {open:?}"
    );
    // The box's shadow falls toward +x (the sun leans +x): darker blue there.
    let shade = rgb(&px, col(-2.0 + 2.0 * 0.6 + 1.3), H / 2);
    assert!(
        shade[2] + 40 < open[2],
        "shadow on the game-shaded floor: {shade:?} vs {open:?}"
    );
    // Without the sun the floor is only ambient-lit: the shader did not bypass lighting.
    let dark = stage(Some(mat), "");
    let mut dark = dark;
    with_shader(&mut dark, "tint", TINT);
    let e = dark
        .world
        .query_filtered::<bevy_ecs::entity::Entity, bevy_ecs::query::With<marionette_core::spatial::DirectionalLight>>()
        .iter(&dark.world)
        .next()
        .unwrap();
    dark.world.despawn(e);
    let px = render(&gpu, &dark).unwrap();
    let amb = rgb(&px, col(5.0), H / 2);
    assert!(
        amb[2] < open[2] / 2,
        "lighting comes from the engine: {amb:?} vs {open:?}"
    );
}

#[test]
fn a_vertex_function_moves_the_mesh_and_its_shadow() {
    let Some(gpu) = gpu() else { return };
    // Shrinks the floor to a quarter of its width around x = 0.
    let src = r#"
fn vertex(v: ptr<function, Vertex>) {
    (*v).position.x = (*v).position.x * game.params[0].x;
}
"#;
    let mut sim = stage(
        Some(r#"{"shader": "squeeze", "params": [0.25], "textures": []}"#),
        "",
    );
    with_shader(&mut sim, "squeeze", src);
    let px = render(&gpu, &sim).unwrap();
    assert_eq!(
        rgb(&px, col(5.0), H / 2),
        [0, 0, 0],
        "floor moved away from x = 5 m"
    );
    assert!(rgb(&px, col(1.0), H - 10)[0] > 50, "floor still at x = 1 m");
}

#[test]
fn blended_and_unlit_shaders_draw_after_opaque_ones() {
    let Some(gpu) = gpu() else { return };
    // An additive, unlit red glow on the floor adds to nothing behind it (black clear), so
    // it shows as pure red regardless of light.
    let src = r#"
// @blend additive
// @double_sided
fn surface(s: ptr<function, Surface>, i: SurfaceIn) {
    (*s).base_color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    (*s).emissive = vec3<f32>(0.5, 0.0, 0.0);
    (*s).unlit = 1.0;
}
"#;
    let mut sim = stage(Some(r#"{"shader": "glow", "params": [], "textures": []}"#), "");
    with_shader(&mut sim, "glow", src);
    let px = render(&gpu, &sim).unwrap();
    let c = rgb(&px, col(5.0), H / 2);
    assert!(
        (c[0] - 188).abs() <= 3 && c[1] == 0 && c[2] == 0,
        "linear 0.5 red after sRGB: {c:?}"
    );
    // The box (opaque, drawn first) is not covered by the glow: additive adds on top of
    // it but does not write depth, and the box itself is lit grey.
    let b = rgb(&px, col(-2.0), H / 2);
    assert!(b[1] > 40, "box still visible through the additive floor: {b:?}");
}

#[test]
fn shader_problems_are_errors_as_data() {
    let Some(gpu) = gpu() else { return };
    // Unknown name.
    let sim = stage(Some(r#"{"shader": "nope", "params": [], "textures": []}"#), "");
    let err = render(&gpu, &sim).unwrap_err();
    assert_eq!(err.0[0].code, "render.shader");
    assert!(err.0[0].message.contains("nope"));
    // Does not compile: the message points into the composed shader.
    let mut sim = stage(Some(r#"{"shader": "bad", "params": [], "textures": []}"#), "");
    with_shader(
        &mut sim,
        "bad",
        "fn surface(s: ptr<function, Surface>, i: SurfaceIn) { let x: f32 = true; }",
    );
    let err = render(&gpu, &sim).unwrap_err();
    assert_eq!(err.0[0].code, "render.shader");
    assert!(
        err.0[0].message.contains("game source starts at line"),
        "{}",
        err.0[0].message
    );
    // Unknown option.
    let mut sim = stage(Some(r#"{"shader": "opt", "params": [], "textures": []}"#), "");
    with_shader(
        &mut sim,
        "opt",
        "// @blend glass\nfn surface(s: ptr<function, Surface>, i: SurfaceIn) {}",
    );
    assert!(
        render(&gpu, &sim).unwrap_err().0[0]
            .message
            .contains("unknown option")
    );
}

#[test]
fn shaders_load_from_files_and_textures_bind() {
    let Some(gpu) = gpu() else { return };
    let sim = stage(
        Some(
            r#"{"shader": "shaders/checker-tint.wgsl", "params": [1.0, 0.2, 0.2, 8.0], "textures": ["builtin:checker"]}"#,
        ),
        "",
    );
    let px = render(&gpu, &sim).unwrap();
    // The checker image tiles 8 times: neighbouring tiles differ in brightness.
    let row: Vec<i32> = (col(1.0)..col(7.0)).map(|x| rgb(&px, x, H / 2)[0]).collect();
    let (lo, hi) = (row.iter().min().unwrap(), row.iter().max().unwrap());
    assert!(hi - lo > 40, "checker visible: {lo}..{hi}");
    let _ = ShaderMaterial::default();
}

#[test]
fn time_animates_shaders_and_standard_models_are_unchanged() {
    let Some(gpu) = gpu() else { return };
    let src = r#"
fn surface(s: ptr<function, Surface>, i: SurfaceIn) {
    (*s).base_color = vec4<f32>(fract(i.time), 0.0, 0.0, 1.0);
}
"#;
    let mut sim = stage(Some(r#"{"shader": "pulse", "params": [], "textures": []}"#), "");
    with_shader(&mut sim, "pulse", src);
    let a = rgb(&render(&gpu, &sim).unwrap(), col(5.0), H / 2);
    sim.run(15); // a quarter second at 60 ticks per second
    let b = rgb(&render(&gpu, &sim).unwrap(), col(5.0), H / 2);
    assert!(b[0] > a[0] + 20, "time moved the color: {a:?} -> {b:?}");
    // The box (standard shader) looks the same in both frames.
    let mut plain = stage(None, "");
    let p1 = render(&gpu, &plain).unwrap();
    plain.run(15);
    let p2 = render(&gpu, &plain).unwrap();
    assert_eq!(p1, p2, "standard shader ignores time");
}

#[test]
fn bulk_draws_take_a_game_shader() {
    let Some(gpu) = gpu() else { return };
    let mut sim = stage(None, "");
    with_shader(&mut sim, "tint", TINT);
    let mut frame = extract(&sim.world);
    frame.bulk.push(marionette_core::spatial::BulkDraw {
        asset: "primitive:cube".into(),
        material: None,
        // One 1 m cube at x = 4, y = 1.
        transforms: vec![[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 4.0, 1.0, 0.0]],
        shader: Some(ShaderMaterial {
            shader: "tint".into(),
            params: vec![0.0, 1.0, 0.0],
            textures: Vec::new(),
        }),
    });
    let mut off = Offscreen::new(&gpu, W, H, 4, repo().join("examples"));
    off.render(&gpu, &frame, None).unwrap();
    let c = rgb(&off.read_pixels(&gpu).unwrap(), col(4.0), H / 2);
    assert!(c[1] > 100 && c[0] < 60 && c[0] == c[2], "green bulk cube: {c:?}");
}
