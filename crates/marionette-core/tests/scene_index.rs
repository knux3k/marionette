//! Decision 0270: a [`SceneIndex`] edit gives exactly the scene and text that editing the
//! whole scene and formatting it gives, and refuses whatever formatting would refuse.

use marionette_core::scene::{self, Scene, SceneEntity};
use marionette_core::scene_index::SceneIndex;
use marionette_core::{Registry, default_registry};
use serde_json::{Value, json};

const EXAMPLE: &str = include_str!("../../../examples/drift.scene.json");

fn entity(id: u64, name: Option<&str>, components: Value) -> SceneEntity {
    SceneEntity {
        id,
        name: name.map(str::to_string),
        components: components
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    }
}

/// The whole-scene answer: the text after `change`, or `None` when it is refused.
fn whole(scene: &Scene, registry: &Registry, change: impl FnOnce(&mut Scene)) -> Option<String> {
    let mut s = scene.clone();
    change(&mut s);
    scene::format_text(&s.to_text(), registry).ok()
}

fn same(index: &SceneIndex, registry: &Registry, want: &str) {
    assert_eq!(index.to_text(), want);
    assert_eq!(index.to_scene().to_text(), want);
    assert_eq!(scene::format_text(&index.to_text(), registry).unwrap(), want);
}

#[test]
fn the_example_indexes_to_its_own_text() {
    let registry = default_registry();
    let scene = scene::parse(EXAMPLE).unwrap();
    let index = SceneIndex::new(&scene, &registry).expect("canonical");
    same(&index, &registry, EXAMPLE);
    assert_eq!(index.len(), scene.entities.len());
}

#[test]
fn a_scene_that_is_not_canonical_or_not_valid_is_not_indexed() {
    let registry = default_registry();
    let parse = |t: &str| scene::parse(t).unwrap();
    // Whole numbers where the canonical form writes floats.
    let loose = parse(
        r#"{"version":1,"format":"marionette.scene","entities":[{"id":1,"components":{"Position":{"x":1,"y":0,"z":0}}}]}"#,
    );
    assert!(SceneIndex::new(&loose, &registry).is_none());
    let unknown =
        parse(r#"{"version":1,"format":"marionette.scene","entities":[{"id":1,"components":{"Nope":{}}}]}"#);
    assert!(SceneIndex::new(&unknown, &registry).is_none());
    let dangling = parse(
        r#"{"version":1,"format":"marionette.scene","entities":[{"id":1,"components":{"Follow":{"target":9,"gain":1.0}}}]}"#,
    );
    assert!(SceneIndex::new(&dangling, &registry).is_none());
}

#[test]
fn edits_match_the_whole_scene_path() {
    let registry = default_registry();
    let mut scene = scene::parse(EXAMPLE).unwrap();
    let mut index = SceneIndex::new(&scene, &registry).unwrap();
    let next = index.next_id();
    assert_eq!(next, scene.entities.iter().map(|e| e.id).max().unwrap() + 1);

    let creates = [
        entity(
            next,
            Some("new"),
            json!({ "Position": { "x": 1, "y": 2, "z": 3 } }),
        ),
        entity(next, None, json!({})), // id in use
        entity(0, None, json!({})),    // reserved
        entity(next + 5, None, json!({ "Position": { "x": "east" } })),
        entity(next + 5, None, json!({ "Nope": {} })),
        entity(
            next + 5,
            None,
            json!({ "Follow": { "target": next + 99, "gain": 1 } }),
        ),
        entity(
            next + 5,
            Some("tail"),
            json!({ "Follow": { "target": next, "gain": 2 } }),
        ),
        entity(next + 2, None, json!({})),
    ];
    let (mut made, mut refused) = (0, 0);
    for e in creates {
        let want = whole(&scene, &registry, |s| s.entities.push(e.clone()))
            .filter(|_| e.id != 0 && scene.entities.iter().all(|o| o.id != e.id));
        let got = index.create(&e, &registry);
        assert_eq!(got.is_some(), want.is_some(), "create {e:?}");
        match want {
            Some(text) => {
                same(&index, &registry, &text);
                scene = scene::parse(&text).unwrap();
                made += 1;
            }
            None => refused += 1,
        }
    }
    assert_eq!((made, refused), (3, 5));

    // Replace: a new value, a wrong value, a missing entity.
    let moved = entity(
        next,
        Some("moved"),
        json!({ "Position": { "x": -1, "y": 0.25, "z": 0 }, "Velocity": { "x": 0, "y": 0, "z": 1 } }),
    );
    let want = whole(&scene, &registry, |s| {
        *s.entities.iter_mut().find(|e| e.id == next).unwrap() = moved.clone();
    })
    .unwrap();
    let changed = index.replace(&moved, &registry).unwrap();
    assert_eq!(changed.before.unwrap().name.as_deref(), Some("new"));
    same(&index, &registry, &want);
    assert!(
        index
            .replace(&entity(next, None, json!({ "Position": 3 })), &registry)
            .is_none()
    );
    assert!(
        index
            .replace(&entity(next + 77, None, json!({})), &registry)
            .is_none()
    );
    same(&index, &registry, &want);

    // Delete: refused while referred to, then in the order that frees it.
    assert!(index.delete(next).is_none(), "the tail still follows it");
    assert!(index.delete(next + 77).is_none());
    assert!(index.delete(next + 5).is_some());
    assert!(index.delete(next).is_some());

    // Resources.
    assert!(
        index
            .set_resource("FixedStep", &json!({ "dt": 0.02 }), &registry)
            .is_some()
    );
    assert!(
        index
            .set_resource("FixedStep", &json!({ "dt": "slow" }), &registry)
            .is_none()
    );
    assert!(index.set_resource("Nope", &json!({}), &registry).is_none());
    let text = index.to_text();
    assert_eq!(scene::format_text(&text, &registry).unwrap(), text);
    assert!(index.remove_resource("FixedStep").is_some());
    assert!(index.remove_resource("FixedStep").is_none());
    let text = index.to_text();
    assert_eq!(scene::format_text(&text, &registry).unwrap(), text);
}
