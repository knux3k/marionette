#![cfg(feature = "ui3d")]
//! The `ui3d_*` operations on a live session (run with `--features ui3d`).

use marionette_agent::Agent;
use marionette_core::scene::SceneId;
use marionette_core::spatial::{Quat, Transform, Vec2, Vec3};
use marionette_ui3d::{UiContent, UiItem, UiPanel, UiWidget};
use serde_json::json;

#[test]
fn point_clicks_a_button_and_panels_lists_it() {
    let dir = std::env::temp_dir().join(format!("ui3d-ops-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("a.scene.json"),
        r#"{"entities":[],"format":"marionette.scene","resources":{},"version":1}"#,
    )
    .unwrap();
    let mut agent = Agent::new(dir.clone(), false);
    assert!(agent.operations().any(|o| o.name == "ui3d_point"));
    let id = agent
        .call("session_start", json!({ "scene": "a.scene.json", "seed": 1 }))
        .unwrap()["session"]
        .as_str()
        .unwrap()
        .to_string();
    let sim = agent.session_sim(&id).unwrap();
    sim.world.spawn((
        SceneId(7),
        UiPanel {
            size: Vec2 { x: 0.4, y: 0.2 },
            ..Default::default()
        },
        UiContent {
            items: vec![UiItem::new(
                "ok",
                UiWidget::Button {
                    text: "OK".into(),
                    action: String::new(),
                },
            )],
        },
        Transform {
            translation: Vec3::new(0.0, 0.0, 0.0),
            rotation: Quat {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
            scale: Vec3::new(1.0, 1.0, 1.0),
        },
    ));
    let listing = agent.call("ui3d_panels", json!({ "session": id })).unwrap();
    let r = &listing["panels"][0]["items"][0]["rect"];
    let (cx, cy) = (
        r[0].as_f64().unwrap() + r[2].as_f64().unwrap() / 2.0,
        r[1].as_f64().unwrap() + r[3].as_f64().unwrap() / 2.0,
    );
    let ray = |pressed: bool| {
        json!({ "session": id, "pointers": [{
            "origin": [0.0, 0.0, 1.0],
            "direction": [cx - 0.2, 0.1 - cy, -1.0],
            "pressed": pressed }] })
    };
    let down = agent.call("ui3d_point", ray(true)).unwrap();
    assert!(down["events"].as_array().unwrap().is_empty());
    let up = agent.call("ui3d_point", ray(false)).unwrap();
    assert_eq!(up["events"][0]["item"], "ok");
    assert_eq!(up["events"][0]["kind"], "click");
    std::fs::remove_dir_all(dir).ok();
}
