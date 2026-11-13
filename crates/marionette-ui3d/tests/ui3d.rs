use marionette_agent::Engine;
use marionette_core::Sim;
use marionette_core::scene::SceneId;
use marionette_core::spatial::{Quat as CQuat, Transform, Vec2, Vec3 as CVec3};
use marionette_render::extract;
use marionette_ui3d::font::{FontData, sdf_from_coverage};
use marionette_ui3d::*;

const SCENE: &str = r#"{"entities":[{"components":{"Camera":{"clear_color":{"a":1.0,"b":0.0,"g":0.0,"r":0.0},"projection":{"Perspective":{"far":50.0,"fov_y_degrees":60.0,"near":0.05}}},
"Transform":{"rotation":{"w":1.0,"x":0.0,"y":0.0,"z":0.0},"scale":{"x":1.0,"y":1.0,"z":1.0},"translation":{"x":0.0,"y":0.0,"z":1.0}}},"id":1,"name":"camera"}],
"format":"marionette.scene","resources":{},"version":1}"#;

fn sim() -> Sim {
    let e = Engine::builder().module(Ui3dModule).build().unwrap();
    e.load_text(SCENE, None, 1, std::path::Path::new(".")).unwrap()
}

fn item(id: &str, widget: UiWidget) -> UiItem {
    UiItem::new(id, widget)
}

fn panel_content() -> UiContent {
    UiContent {
        items: vec![
            item("title", UiWidget::Label { text: "WORLD PANEL".into(), size: 0.0, align: UiAlign::Center }),
            item("go", UiWidget::Button { text: "GO".into(), action: "go".into() }),
            item("sound", UiWidget::Toggle { text: "SOUND".into(), on: false }),
            item("vol", UiWidget::Slider { text: "VOLUME".into(), min: 0.0, max: 1.0, value: 0.0 }),
            item(
                "files",
                UiWidget::List {
                    rows: (0..10).map(|i| format!("ROW {i}")).collect(),
                    visible: 3,
                    scroll: 0.0,
                    selected: -1,
                },
            ),
        ],
    }
}

fn spawn_panel(sim: &mut Sim, id: u64, z: f32, face: UiFace) -> bevy_ecs::entity::Entity {
    let panel = UiPanel { size: Vec2 { x: 0.6, y: 0.8 }, face, ..Default::default() };
    sim.world
        .spawn((
            SceneId(id),
            panel,
            panel_content(),
            Transform {
                translation: CVec3::new(0.0, 0.0, z),
                rotation: CQuat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 },
                scale: CVec3::new(1.0, 1.0, 1.0),
            },
        ))
        .id()
}

fn set_pointer(sim: &mut Sim, p: Option<UiPointer>) {
    sim.world.resource_mut::<UiPointers>().0 = p.into_iter().collect();
}

/// A ray from the viewer toward panel point (x, y) given in panel top-left meters (y down).
fn ray_at(sim: &mut Sim, entity: bevy_ecs::entity::Entity, x: f32, y: f32, pressed: bool) -> UiPointer {
    let size = sim.world.get::<UiPanel>(entity).unwrap().size;
    let t = *sim.world.get::<Transform>(entity).unwrap();
    let target = [t.translation.x + x - size.x / 2.0, t.translation.y + size.y / 2.0 - y, t.translation.z];
    let origin = [0.0, 0.0, 1.0];
    UiPointer {
        id: 0,
        kind: PointerKind::Ray,
        origin,
        direction: [target[0] - origin[0], target[1] - origin[1], target[2] - origin[2]],
        pressed,
        scroll: 0.0,
    }
}

fn rect_of(sim: &mut Sim, e: bevy_ecs::entity::Entity, item: usize) -> layout::Rect {
    let panel = sim.world.get::<UiPanel>(e).unwrap().clone();
    let content = sim.world.get::<UiContent>(e).unwrap().clone();
    let id = sim.world.get::<SceneId>(e).unwrap().0;
    sim.world.resource::<UiRuntime>().layout(id, &panel, &content).rects[item]
}

fn click(sim: &mut Sim, e: bevy_ecs::entity::Entity, x: f32, y: f32) -> Vec<UiEvent> {
    let p = ray_at(sim, e, x, y, true);
    set_pointer(sim, Some(p));
    sim.step();
    let mut events = sim.world.resource::<UiEvents>().0.clone();
    set_pointer(sim, Some(UiPointer { pressed: false, ..p }));
    sim.step();
    events.extend(sim.world.resource::<UiEvents>().0.clone());
    events
}

#[test]
fn distance_field_edges_sit_at_one_half() {
    let (w, h) = (24usize, 24usize);
    let mut cov = vec![0u8; w * h];
    for y in 6..18 {
        for x in 6..18 {
            cov[y * w + x] = 255;
        }
    }
    let f = sdf_from_coverage(&cov, w, h, 6.0);
    assert!(f[12 * w + 12] > 200);
    assert_eq!(f[0], 0);
    // One pixel outside the edge is just below one half, one inside just above.
    let (out, inn) = (f[12 * w + 5], f[12 * w + 6]);
    assert!(out < 128 && out > 90 && inn > 128 && inn < 170, "{out} {inn}");
}

#[test]
fn the_built_in_font_has_glyphs_and_a_solid_area() {
    let f = FontData::builtin();
    assert!(f.glyphs.contains_key(&'A') && f.glyphs.contains_key(&' '));
    let px = |u: f32, v: f32| {
        let (x, y) = ((u * f.atlas.width as f32) as u32, (v * f.atlas.height as f32) as u32);
        f.atlas.rgba[((y * f.atlas.width + x) * 4) as usize]
    };
    assert_eq!(px(f.white_uv[0], f.white_uv[1]), 255);
    let a = f.glyph('A');
    // The crossbar of an A is pixel row 3 of the 5x7 glyph: column 2, row 3, at 8x with a border.
    let (cu, cv) = (
        a.uv[0] + (8.0 + 2.5 * 8.0) / f.atlas.width as f32,
        a.uv[3] + (8.0 + 3.5 * 8.0) / f.atlas.height as f32,
    );
    assert!(px(cu, cv) > 150, "the crossbar of an A is inside the letter");
    assert_eq!(px(a.uv[0] + 0.0005, a.uv[3] + 0.0005), 0, "corner of the cell is far outside");
    assert!(marionette_assets::generated::contains(f.image_name.strip_prefix("generated:").unwrap()));
}

#[test]
fn layout_flows_items_across_columns_and_wraps_text() {
    let font = FontData::builtin();
    let panel = UiPanel { size: Vec2 { x: 0.6, y: 0.8 }, columns: 2, ..Default::default() };
    let content = UiContent {
        items: vec![
            item("a", UiWidget::Button { text: "A".into(), action: String::new() }),
            item("b", UiWidget::Button { text: "B".into(), action: String::new() }),
            UiItem { span: 2, ..item("c", UiWidget::Label { text: "WORD ".repeat(30), size: 0.0, align: UiAlign::Left }) },
        ],
    };
    let lay = layout::compute(&panel, &content, &font);
    assert_eq!(lay.rects[0].y, lay.rects[1].y);
    assert!(lay.rects[1].x > lay.rects[0].x + lay.rects[0].w - 1e-5);
    assert!(lay.rects[2].y > lay.rects[0].y + lay.rects[0].h);
    assert!(lay.rects[2].h > 3.0 * font.line_height * panel.text_size, "long text wrapped to several lines");
    assert!(lay.overflow == (lay.rects[2].y + lay.rects[2].h > 0.8 - panel.padding + 1e-4));
}

#[test]
fn a_ray_clicks_a_button_and_sets_its_action() {
    let mut s = sim();
    let e = spawn_panel(&mut s, 10, 0.0, UiFace::Fixed);
    s.world.insert_resource(marionette_core::actions::ActionState::default());
    let r = rect_of(&mut s, e, 1);
    let events = click(&mut s, e, r.x + r.w / 2.0, r.y + r.h / 2.0);
    let ev: Vec<_> = events.iter().filter(|e| e.item == "go").collect();
    assert_eq!(ev.len(), 1, "{events:?}");
    assert_eq!(ev[0].kind, UiEventKind::Click);
    assert_eq!(ev[0].panel, 10);
}

#[test]
fn toggle_slider_and_list_change_their_own_values() {
    let mut s = sim();
    let e = spawn_panel(&mut s, 10, 0.0, UiFace::Fixed);
    let r = rect_of(&mut s, e, 2);
    click(&mut s, e, r.x + 0.01, r.y + r.h / 2.0);
    let UiWidget::Toggle { on, .. } = s.world.get::<UiContent>(e).unwrap().items[2].widget.clone() else { panic!() };
    assert!(on);

    // Drag the slider: press at the left end, move to the middle, release.
    let r = rect_of(&mut s, e, 3);
    let s0 = 0.03;
    let (l, rt, cy) = layout::slider_track(r, s0);
    let p = ray_at(&mut s, e, l, cy, true);
    set_pointer(&mut s, Some(p));
    s.step();
    let p = ray_at(&mut s, e, (l + rt) / 2.0, cy, true);
    set_pointer(&mut s, Some(p));
    s.step();
    let UiWidget::Slider { value, .. } = s.world.get::<UiContent>(e).unwrap().items[3].widget.clone() else { panic!() };
    assert!((value - 0.5).abs() < 0.02, "{value}");
    assert!(s.world.resource::<UiEvents>().0.iter().any(|e| e.kind == UiEventKind::Slide));
    // Off the panel entirely, the grab still holds and clamps to the end.
    let p = ray_at(&mut s, e, rt + 5.0, cy, true);
    set_pointer(&mut s, Some(p));
    s.step();
    let UiWidget::Slider { value, .. } = s.world.get::<UiContent>(e).unwrap().items[3].widget.clone() else { panic!() };
    assert_eq!(value, 1.0);
    let p = UiPointer { pressed: false, ..p };
    set_pointer(&mut s, Some(p));
    s.step();

    // Scroll the list by two rows, then select the second visible row.
    let r = rect_of(&mut s, e, 4);
    let mut p = ray_at(&mut s, e, r.x + 0.05, r.y + 0.01, false);
    p.scroll = 2.0;
    set_pointer(&mut s, Some(p));
    s.step();
    let rh = layout::row_height(s0);
    let events = click(&mut s, e, r.x + 0.05, r.y + rh * 1.5);
    let sel = events.iter().find(|e| e.kind == UiEventKind::Select).expect("select event");
    assert_eq!(sel.value, 3.0, "scrolled by 2, second visible row is row 3");
    let UiWidget::List { selected, scroll, .. } = s.world.get::<UiContent>(e).unwrap().items[4].widget.clone() else { panic!() };
    assert_eq!((selected, scroll), (3, 2.0));
}

#[test]
fn a_fingertip_pushing_into_the_panel_presses() {
    let mut s = sim();
    let e = spawn_panel(&mut s, 10, 0.0, UiFace::Fixed);
    let r = rect_of(&mut s, e, 1);
    let at = |z: f32| UiPointer {
        id: 3,
        kind: PointerKind::Touch,
        origin: [r.x + r.w / 2.0 - 0.3, 0.4 - (r.y + r.h / 2.0), z],
        direction: [0.0; 3],
        pressed: false,
        scroll: 0.0,
    };
    set_pointer(&mut s, Some(at(0.05)));
    s.step();
    assert!(s.world.resource::<UiEvents>().0.is_empty(), "hovering only");
    set_pointer(&mut s, Some(at(0.004)));
    s.step();
    set_pointer(&mut s, Some(at(0.05)));
    s.step();
    let events = &s.world.resource::<UiEvents>().0;
    assert_eq!(events.len(), 1);
    assert_eq!((events[0].item.as_str(), events[0].kind, events[0].pointer), ("go", UiEventKind::Click, 3));
}

#[test]
fn panels_face_the_viewer_and_follow_their_anchor() {
    let mut s = sim();
    let e = spawn_panel(&mut s, 10, 0.0, UiFace::Camera);
    // Move the camera sideways; the panel turns toward it.
    s.world
        .query::<(&mut Transform, &marionette_core::spatial::Camera)>()
        .single_mut(&mut s.world)
        .unwrap()
        .0
        .translation = CVec3::new(1.0, 0.0, 1.0);
    s.step();
    let r = s.world.get::<Transform>(e).unwrap().rotation;
    let n = glam::Quat::from_xyzw(r.x, r.y, r.z, r.w) * glam::Vec3::Z;
    let want = glam::Vec3::new(1.0, 0.0, 1.0).normalize();
    assert!(n.distance(want) < 1e-4, "{n:?}");

    // Anchored to the camera (entity 1) 0.5 m ahead in its own frame, no lag.
    s.world.entity_mut(e).insert(UiAnchor {
        target: marionette_core::builtin::SceneRef(1),
        offset: CVec3::new(0.0, 0.0, -0.5),
        relative: true,
        smoothing: 0.0,
    });
    s.step();
    let t = s.world.get::<Transform>(e).unwrap().translation;
    assert!((t.x - 1.0).abs() < 1e-5 && (t.z - 0.5).abs() < 1e-5, "{t:?}");
}

#[test]
fn quads_are_drawn_from_the_front_only_and_far_panels_first() {
    let mut s = sim();
    let near = spawn_panel(&mut s, 10, 0.0, UiFace::Fixed);
    spawn_panel(&mut s, 11, -0.5, UiFace::Fixed);
    let frame = extract(&s.world);
    assert_eq!(frame.quads.len(), 1);
    let q = &frame.quads[0];
    assert!(q.image.starts_with("generated:sdf-"));
    assert!(q.vertices.len() >= 8 && q.vertices.len() % 4 == 0);
    // Far panel (z -0.5) comes first, then the near one at z 0 (layer offsets are millimetres).
    assert!(q.vertices[0].position[2] < -0.4);
    assert!(q.vertices.last().unwrap().position[2] > -0.1);
    // Turn the near panel around: seen from behind it is skipped.
    s.world.get_mut::<Transform>(near).unwrap().rotation = CQuat { x: 0.0, y: 1.0, z: 0.0, w: 0.0 };
    let after = extract(&s.world);
    assert!(after.quads[0].vertices.len() < q.vertices.len());
}

#[test]
fn runs_are_deterministic_and_the_cache_forgets_removed_panels() {
    let run = || {
        let mut s = sim();
        let e = spawn_panel(&mut s, 10, 0.0, UiFace::Camera);
        let r = rect_of(&mut s, e, 2);
        click(&mut s, e, r.x + 0.01, r.y + r.h / 2.0);
        s.run(5);
        s.state_hash()
    };
    assert_eq!(run(), run());
    let mut s = sim();
    let e = spawn_panel(&mut s, 10, 0.0, UiFace::Fixed);
    extract(&s.world);
    s.step();
    assert_eq!(s.world.resource::<UiRuntime>().cached_panels(), 1);
    s.world.despawn(e);
    s.step();
    assert_eq!(s.world.resource::<UiRuntime>().cached_panels(), 0);
}

fn gpu() -> Option<marionette_render::Gpu> {
    match marionette_render::Gpu::headless() {
        Ok(g) => Some(g),
        Err(e) if std::env::var("MARIONETTE_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipping GPU test: {e}");
            None
        }
        Err(e) => panic!("GPU required but unavailable: {e}"),
    }
}

/// Renders the panel scene at `size` from the front; `yaw` turns the panel (radians).
fn picture(gpu: &marionette_render::Gpu, size: (u32, u32), yaw: f32, distance: f32) -> Vec<u8> {
    let mut s = sim();
    let e = spawn_panel(&mut s, 10, 0.0, UiFace::Fixed);
    let mut t = s.world.get_mut::<Transform>(e).unwrap();
    t.rotation = CQuat { x: 0.0, y: (yaw / 2.0).sin(), z: 0.0, w: (yaw / 2.0).cos() };
    s.world
        .query::<(&mut Transform, &marionette_core::spatial::Camera)>()
        .single_mut(&mut s.world)
        .unwrap()
        .0
        .translation = CVec3::new(0.0, 0.0, distance);
    // Show some state: slider half way, toggle on, a row selected, hover on the button.
    let mut c = s.world.get_mut::<UiContent>(e).unwrap();
    if let UiWidget::Slider { value, .. } = &mut c.items[3].widget {
        *value = 0.5;
    }
    if let UiWidget::Toggle { on, .. } = &mut c.items[2].widget {
        *on = true;
    }
    if let UiWidget::List { selected, .. } = &mut c.items[4].widget {
        *selected = 1;
    }
    s.step();
    let mut off = marionette_render::Offscreen::new(gpu, size.0, size.1, 4, ".".into());
    off.render(gpu, &extract(&s.world), None).unwrap();
    off.read_pixels(gpu).unwrap()
}

#[test]
fn a_panel_renders_with_crisp_text() {
    let Some(gpu) = gpu() else { return };
    let a = picture(&gpu, (640, 480), 0.0, 1.0);
    let b = picture(&gpu, (640, 480), 0.0, 1.0);
    assert!(a == b, "two renders differ");
    // Not just the clear colour: the panel and its text put many different pixels on screen.
    let distinct: std::collections::BTreeSet<[u8; 4]> = a.chunks(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
    assert!(distinct.len() > 30, "{}", distinct.len());
    if let Ok(dir) = std::env::var("UI3D_PICTURES") {
        for (name, yaw, dist) in [("front", 0.0, 1.0), ("angled", 0.9, 1.0), ("far", 0.0, 3.0)] {
            let px = picture(&gpu, (640, 480), yaw, dist);
            std::fs::write(
                format!("{dir}/{name}.png"),
                marionette_render::assets::encode_png(640, 480, &px),
            )
            .unwrap();
        }
    }
}

/// `cargo test --release -p marionette-ui3d -- --ignored --nocapture heavy`
#[test]
#[ignore]
fn heavy_text_cost() {
    let mut s = sim();
    for n in 0..100u64 {
        let e = spawn_panel(&mut s, 100 + n, -(n as f32) * 0.01, UiFace::Fixed);
        let mut c = s.world.get_mut::<UiContent>(e).unwrap();
        c.items[0].widget = UiWidget::Label { text: "LOREM IPSUM DOLOR SIT AMET ".repeat(20), size: 0.0, align: UiAlign::Left };
        c.items[0].height = 0.3;
    }
    let time = |label: &str, f: &mut dyn FnMut()| {
        let t = std::time::Instant::now();
        for _ in 0..20 {
            f();
        }
        eprintln!("{label}: {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
    };
    let t = std::time::Instant::now();
    let frame = extract(&s.world);
    let verts: usize = frame.quads.iter().map(|q| q.vertices.len()).sum();
    eprintln!("cold extract (layout + mesh of 100 panels): {:.2} ms, {} quads", t.elapsed().as_secs_f64() * 1000.0, verts / 4);
    time("warm extract (cached meshes, world transform)", &mut || {
        std::hint::black_box(extract(&s.world));
    });
    time("sim tick with no pointer (anchors, facing, cache sweep)", &mut || s.step());
    let mut n = 0.0;
    time("tick + extract while one panel's text changes every frame", &mut || {
        n += 1.0;
        let e = s.world.query::<(bevy_ecs::entity::Entity, &SceneId)>().iter(&s.world).find(|(_, i)| i.0 == 100).unwrap().0;
        if let UiWidget::Slider { value, .. } = &mut s.world.get_mut::<UiContent>(e).unwrap().items[3].widget {
            *value = n;
        }
        s.step();
        std::hint::black_box(extract(&s.world));
    });
}
