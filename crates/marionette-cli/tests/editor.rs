//! Scene editor view (decision 0130): commands driven headless through a real agent (edits
//! written through the operations and undone), picking, and the `editor_view` operation.
#![cfg(feature = "editor")]

use glam::Vec3;
use marionette_agent::Agent;
use marionette_core::{Errors, Sim};
use marionette_editor::overlay::Layout;
use marionette_editor::{Editor, Host};
use marionette_render::extract;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const SCENE: &str = "scenes/edit.scene.json";

struct AgentHost(Agent);

impl Host for AgentHost {
    fn load(&mut self) -> Result<Sim, Errors> {
        self.0.load_sim(SCENE, 0)
    }
    fn call(&mut self, op: &str, input: Value) -> Result<Value, Errors> {
        self.0.call(op, input)
    }
}

fn gpu_required() -> bool {
    std::env::var("MARIONETTE_REQUIRE_GPU").as_deref() == Ok("1")
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A project with a camera, a light, a crate at x = -1.5 and a moving ball at x = 1.5.
fn project(dir: &Path) -> AgentHost {
    let mut agent = Agent::new(dir, false);
    let t = |x: f64, y: f64, z: f64| {
        json!({ "translation": { "x": x, "y": y, "z": z }, "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
                "scale": { "x": 1, "y": 1, "z": 1 } })
    };
    let calls = [
        ("scene_create", json!({ "scene": SCENE })),
        (
            "entity_create",
            json!({ "scene": SCENE, "id": 1, "name": "camera", "components": {
                "Transform": t(0.0, 1.0, 8.0),
                "Camera": { "projection": { "Perspective": { "fov_y_degrees": 50, "near": 0.1, "far": 100 } },
                            "clear_color": { "r": 0.3, "g": 0.4, "b": 0.6, "a": 1 } } } }),
        ),
        (
            "entity_create",
            json!({ "scene": SCENE, "id": 2, "name": "lamp", "components": {
                "Transform": t(0.0, 3.0, 0.0),
                "PointLight": { "color": { "r": 1, "g": 1, "b": 1, "a": 1 }, "intensity": 5, "range": 10 } } }),
        ),
        (
            "entity_create",
            json!({ "scene": SCENE, "id": 3, "name": "crate", "components": {
                "Transform": t(-1.5, 0.5, 0.0), "Model": { "asset": "primitive:cube" } } }),
        ),
        (
            "entity_create",
            json!({ "scene": SCENE, "id": 4, "name": "ball", "components": {
                "Transform": t(1.5, 0.5, 0.0), "Model": { "asset": "primitive:sphere" },
                "Position": { "x": 1.5, "y": 0.5, "z": 0 }, "Velocity": { "x": 1, "y": 0, "z": 0 } } }),
        ),
    ];
    for (op, input) in calls {
        agent.call(op, input).unwrap_or_else(|e| panic!("{op}: {e}"));
    }
    AgentHost(agent)
}

fn file_entity(dir: &Path, id: u64) -> Option<Value> {
    let text = std::fs::read_to_string(dir.join(SCENE)).unwrap();
    let scene: Value = serde_json::from_str(&text).unwrap();
    scene["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .cloned()
}

#[test]
fn clicks_select_and_edits_go_through_operations_with_undo() {
    let tmp = marionette_host::temp_dir("editor").unwrap();
    let dir = tmp.path().to_path_buf();
    let mut host = project(&dir);
    let sim = host.load().unwrap();
    let mut ed = Editor::new(SCENE, sim);
    assert_eq!(
        ed.camera.position,
        Vec3::new(0.0, 1.0, 8.0),
        "starts at the scene camera"
    );
    assert_eq!(ed.entities().len(), 4);

    // Click the crate in the view, then the ball's entity list row.
    let layout = Layout::new(1600.0, 900.0, true);
    let frame = extract(&ed.sim.world);
    let view = ed.view(&frame, 1600.0 / 900.0);
    let items = ed.items(&frame, &|_| None);
    let (p, _) = marionette_editor::pick::project(&view, Vec3::new(-1.5, 0.5, 0.0), 1600.0, 900.0).unwrap();
    ed.click(&items, &view, &layout, p.x, p.y);
    assert_eq!(ed.selected, Some(3));
    let (lx, ly, _, _) = layout.list().unwrap();
    ed.click(
        &items,
        &view,
        &layout,
        lx + 10.0,
        ly + 6.0 + layout.row() * 4.0 + 2.0,
    );
    assert_eq!(ed.selected, Some(4), "fourth list row");
    // The lamp has no model: it is a marker a few pixels wide.
    let (p, _) = marionette_editor::pick::project(&view, Vec3::new(0.0, 3.0, 0.0), 1600.0, 900.0).unwrap();
    ed.click(&items, &view, &layout, p.x + 4.0, p.y);
    assert_eq!(ed.selected, Some(2));
    let inspected = ed.inspected().unwrap();
    assert!(inspected.components.iter().any(|(n, _)| n == "PointLight"));

    // Move the ball (it has Position, so Position moves) and undo.
    ed.select(Some(4), 10);
    ed.nudge(&mut host, Vec3::new(0.25, 0.0, 0.0));
    assert_eq!(ed.message, "moved #4");
    assert_eq!(
        file_entity(&dir, 4).unwrap()["components"]["Position"]["x"],
        json!(1.75)
    );
    ed.undo(&mut host);
    assert_eq!(
        file_entity(&dir, 4).unwrap()["components"]["Position"]["x"],
        json!(1.5)
    );

    // Duplicate the crate (new id 5, 1 m along +X), delete the original, undo both.
    ed.select(Some(3), 10);
    ed.duplicate(&mut host);
    assert_eq!(ed.selected, Some(5));
    let copy = file_entity(&dir, 5).unwrap();
    assert_eq!(copy["name"], json!("crate"));
    assert_eq!(copy["components"]["Transform"]["translation"]["x"], json!(-0.5));
    ed.select(Some(3), 10);
    ed.delete(&mut host);
    assert!(file_entity(&dir, 3).is_none());
    assert_eq!(ed.selected, None);
    ed.undo(&mut host);
    ed.undo(&mut host);
    assert!(file_entity(&dir, 5).is_none());
    assert_eq!(
        file_entity(&dir, 3).unwrap()["components"]["Model"]["asset"],
        json!("primitive:cube")
    );
    assert_eq!(ed.undo_depth(), 0);

    // After the simulation has run, edits wait for a reset to the file.
    ed.select(Some(4), 10);
    ed.step(&mut host);
    assert_eq!(ed.sim.tick(), 1);
    ed.nudge(&mut host, Vec3::Y);
    assert!(ed.message.contains("reset"), "{}", ed.message);
    assert_eq!(
        file_entity(&dir, 4).unwrap()["components"]["Position"]["y"],
        json!(0.5)
    );
    ed.reset(&mut host);
    assert_eq!(ed.sim.tick(), 0);
    ed.nudge(&mut host, Vec3::Y);
    assert_eq!(
        file_entity(&dir, 4).unwrap()["components"]["Position"]["y"],
        json!(1.5)
    );
    // A refused edit (the operation's validation) shows its message and changes nothing.
    ed.select(Some(1), 10);
    let before = std::fs::read_to_string(dir.join(SCENE)).unwrap();
    ed.select_step(1, 10);
    assert_eq!(ed.selected, Some(2));
    ed.select_step(-1, 10);
    assert_eq!(ed.selected, Some(1));
    assert_eq!(before, std::fs::read_to_string(dir.join(SCENE)).unwrap());
}

#[test]
fn editor_view_operation_picks_inspects_and_renders() {
    let mut agent = Agent::new(repo(), true);
    let tmp = marionette_host::temp_dir("editor-view").unwrap();
    // Picks and the inspector need no GPU image, but the models' bounds come from the
    // renderer, so the operation needs a GPU either way.
    let r = agent.call(
        "editor_view",
        json!({ "scene": "examples/showcase.scene.json", "select": 6, "pick": [[640, 400]], "image": false }),
    );
    let out = match r {
        Ok(v) => v,
        Err(e) if !gpu_required() && e.to_string().contains("adapter") => {
            eprintln!("skipping GPU test: {e}");
            return;
        }
        Err(e) => panic!("{e}"),
    };
    assert!(out.get("path").is_none(), "no image asked for");
    assert_eq!(out["selected"]["name"], json!("gem"));
    assert!(out["selected"]["components"]["Model"].is_object());
    let hits = out["picks"][0]["hits"].as_array().unwrap();
    assert_eq!(hits[0]["name"], json!("gem"), "{hits:?}");
    assert_eq!(hits.last().unwrap()["name"], json!("ground"));

    // In read-only mode images go under .marionette/ only.
    let refused = agent.call(
        "editor_view",
        json!({ "scene": "examples/showcase.scene.json", "out": tmp.path().join("x.png").to_string_lossy() }),
    );
    assert!(refused.unwrap_err().to_string().contains("read-only"));

    // A writable agent renders the image; framing the selection moves the camera.
    let mut agent = Agent::new(repo(), false);
    let png = tmp.path().join("editor.png");
    let out = agent
        .call(
            "editor_view",
            json!({ "scene": "examples/showcase.scene.json", "select": 4, "frame": true, "size": "640x360",
                    "out": png.to_string_lossy() }),
        )
        .unwrap();
    assert_eq!(out["width"], json!(640));
    let img = marionette_render::assets::decode_png(&std::fs::read(&png).unwrap()).unwrap();
    assert_eq!((img.width, img.height), (640, 360));
    assert_ne!(
        out["camera"][0],
        json!([0.0, 2.5, 6.0]),
        "framed, not the scene camera"
    );
    // The selected crate is now under the center of the view.
    let center = agent
        .call(
            "editor_view",
            json!({ "scene": "examples/showcase.scene.json", "select": 4, "frame": true, "size": "640x360",
                    "image": false, "pick": [[320, 180]] }),
        )
        .unwrap();
    assert_eq!(center["picks"][0]["hits"][0]["name"], json!("crate"), "{center}");
}
