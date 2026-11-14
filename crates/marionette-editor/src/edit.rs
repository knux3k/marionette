//! Small edits from the editor: move, duplicate and delete the selected entity, with undo.
//!
//! The editor never writes scene files itself. Each edit is an operation call
//! (`component_set`, `entity_create`, `entity_delete`) handed to the host, so it goes
//! through the same validation and atomic write as every other scene change, and each
//! comes with the operation calls that undo it.

use crate::inspect::Inspected;
use glam::Vec3;
use serde_json::{Value, json};

/// One operation call: name and JSON input.
pub type Call = (String, Value);

/// An edit and the calls that undo it (run in order).
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub call: Call,
    pub undo: Vec<Call>,
    /// What the status bar says when it worked.
    pub done: String,
}

/// Most undo steps kept (memory rule: every buffer bounded).
pub const UNDO_KEEP: usize = 64;

fn component<'a>(e: &'a Inspected, name: &str) -> Option<&'a Value> {
    e.components.iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

fn add(v: &Value, key: &str, d: f32) -> Value {
    json!(v.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32 + d)
}

/// Moves an entity by `delta` meters: its `Position` if it has one, else its `Transform`
/// translation. Entities with neither cannot move.
pub fn nudge(scene: &str, e: &Inspected, delta: Vec3) -> Result<Plan, String> {
    if component(e, "ChildOf").is_some() {
        // A child's Transform is written by the engine from its parent (decision 0272).
        return Err(format!(
            "#{} is a child: move its parent, or set its ChildOf translation",
            e.id
        ));
    }
    let (name, old, new) = if let Some(p) = component(e, "Position") {
        let new = json!({ "x": add(p, "x", delta.x), "y": add(p, "y", delta.y), "z": add(p, "z", delta.z) });
        ("Position", p.clone(), new)
    } else if let Some(t) = component(e, "Transform") {
        let mut new = t.clone();
        let tr = t.get("translation").cloned().unwrap_or_else(|| json!({}));
        new["translation"] =
            json!({ "x": add(&tr, "x", delta.x), "y": add(&tr, "y", delta.y), "z": add(&tr, "z", delta.z) });
        ("Transform", t.clone(), new)
    } else {
        return Err(format!("#{} has no Position or Transform to move", e.id));
    };
    let set = |value: Value| {
        (
            "component_set".to_string(),
            json!({ "scene": scene, "id": e.id, "component": name, "value": value }),
        )
    };
    Ok(Plan {
        call: set(new),
        undo: vec![set(old)],
        done: format!("moved #{}", e.id),
    })
}

/// Copies an entity under id `new_id`, 1 m along +X when it has a place.
pub fn duplicate(scene: &str, e: &Inspected, new_id: u64) -> Plan {
    let mut copy = Inspected {
        id: new_id,
        ..e.clone()
    };
    if let Ok(p) = nudge(scene, e, Vec3::X)
        && let Some(c) = copy
            .components
            .iter_mut()
            .find(|(n, _)| *n == p.call.1["component"])
    {
        c.1 = p.call.1["value"].clone();
    }
    Plan {
        call: create(scene, &copy),
        undo: vec![delete_call(scene, new_id)],
        done: format!("duplicated #{} as #{new_id}", e.id),
    }
}

/// Removes an entity; undo puts it back with the same id, name and components.
pub fn delete(scene: &str, e: &Inspected) -> Plan {
    Plan {
        call: delete_call(scene, e.id),
        undo: vec![create(scene, e)],
        done: format!("deleted #{}", e.id),
    }
}

fn delete_call(scene: &str, id: u64) -> Call {
    ("entity_delete".into(), json!({ "scene": scene, "id": id }))
}

fn create(scene: &str, e: &Inspected) -> Call {
    let components: serde_json::Map<String, Value> = e.components.iter().cloned().collect();
    let mut input = json!({ "scene": scene, "id": e.id, "components": components });
    if let Some(n) = &e.name {
        input["name"] = json!(n);
    }
    ("entity_create".into(), input)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crate_at(x: f32) -> Inspected {
        Inspected {
            id: 3,
            name: Some("crate".into()),
            components: vec![
                ("Model".into(), json!({"asset": "primitive:cube"})),
                ("Position".into(), json!({"x": x, "y": 0.0, "z": 0.0})),
            ],
        }
    }

    #[test]
    fn nudge_sets_position_and_undo_restores_it() {
        let p = nudge("a.scene.json", &crate_at(1.0), Vec3::new(0.25, 0.0, 0.0)).unwrap();
        assert_eq!(p.call.0, "component_set");
        assert_eq!(p.call.1["value"], json!({"x": 1.25, "y": 0.0, "z": 0.0}));
        assert_eq!(p.undo[0].1["value"], json!({"x": 1.0, "y": 0.0, "z": 0.0}));
    }

    #[test]
    fn nudge_uses_transform_translation_and_keeps_the_rest() {
        let e = Inspected {
            id: 1,
            name: None,
            components: vec![(
                "Transform".into(),
                json!({"translation": {"x": 0.0, "y": 1.0, "z": 0.0}, "rotation": {"x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0}, "scale": {"x": 2.0, "y": 2.0, "z": 2.0}}),
            )],
        };
        let p = nudge("s", &e, Vec3::Y).unwrap();
        assert_eq!(p.call.1["value"]["translation"]["y"], json!(2.0));
        assert_eq!(p.call.1["value"]["scale"]["x"], json!(2.0));
        let none = Inspected {
            id: 2,
            name: None,
            components: vec![],
        };
        assert!(nudge("s", &none, Vec3::Y).is_err());
        let child = Inspected {
            id: 3,
            name: None,
            components: vec![
                e.components[0].clone(),
                ("ChildOf".into(), json!({ "parent": 1 })),
            ],
        };
        assert!(nudge("s", &child, Vec3::Y).unwrap_err().contains("child"));
    }

    #[test]
    fn duplicate_and_delete_carry_their_undo() {
        let d = duplicate("s", &crate_at(0.0), 9);
        assert_eq!(d.call.0, "entity_create");
        assert_eq!(d.call.1["id"], json!(9));
        assert_eq!(d.call.1["name"], json!("crate"));
        assert_eq!(d.call.1["components"]["Position"]["x"], json!(1.0));
        assert_eq!(
            d.undo,
            vec![("entity_delete".to_string(), json!({"scene": "s", "id": 9}))]
        );
        let x = delete("s", &crate_at(0.0));
        assert_eq!(x.call.1, json!({"scene": "s", "id": 3}));
        assert_eq!(
            x.undo[0].1["components"]["Model"],
            json!({"asset": "primitive:cube"})
        );
    }
}
