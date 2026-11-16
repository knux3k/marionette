//! A game's `SceneChange` request swaps the running scene after the tick (decision 0176).

use marionette_core::module::SceneChange;
use marionette_core::scene::SceneId;
use marionette_core::{Sim, default_registry};
use serde_json::{Value, json};

fn scene(ids: &[u64]) -> String {
    let entities: Vec<Value> = ids
        .iter()
        .map(|id| json!({ "id": id, "name": format!("e{id}"), "components": {} }))
        .collect();
    json!({ "version": 1, "format": "marionette.scene", "resources": {}, "entities": entities }).to_string()
}

fn ids(sim: &mut Sim) -> Vec<u64> {
    let mut q = sim.world.query::<&SceneId>();
    let mut v: Vec<u64> = q.iter(&sim.world).map(|i| i.0).collect();
    v.sort_unstable();
    v
}

#[test]
fn a_requested_scene_replaces_the_running_one_after_the_tick() {
    let dir = std::env::temp_dir().join(format!("scene-change-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let next = dir.join("next.scene.json");
    std::fs::write(&next, scene(&[7, 8])).unwrap();

    let mut sim = Sim::from_text(&scene(&[1]), None, default_registry(), 1).unwrap();
    sim.step();
    assert_eq!(ids(&mut sim), [1]);
    sim.world.insert_resource(SceneChange {
        path: next.to_string_lossy().into_owned(),
    });
    sim.step();
    assert_eq!(ids(&mut sim), [7, 8]);
    assert_eq!(sim.tick(), 2, "the tick count carries over");
    assert!(
        sim.world
            .get_resource::<SceneChange>()
            .is_none_or(|c| c.path.is_empty())
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_missing_scene_leaves_the_world_and_records_an_event() {
    let mut sim = Sim::from_text(&scene(&[1]), None, default_registry(), 1).unwrap();
    sim.world.insert_resource(SceneChange {
        path: "/no/such/file.scene.json".into(),
    });
    sim.step();
    assert_eq!(ids(&mut sim), [1]);
    assert!(
        sim.events()
            .events
            .iter()
            .any(|e| e.kind == "scene_change_failed")
    );
    // The request is spent: the next tick does not try again.
    let before = sim.events().events.len();
    sim.step();
    assert_eq!(sim.events().events.len(), before);
}
