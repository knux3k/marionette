//! What the inspector shows: an entity's components as short text lines, and the entity
//! list. Values come through the registry, the same path scene files use, so the inspector
//! shows exactly what a scene file would hold.

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use marionette_core::scene::{SceneId, SceneName};
use marionette_core::{Kind, Registry};
use serde_json::Value;

/// One entity as the inspector sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Inspected {
    pub id: u64,
    pub name: Option<String>,
    /// Component name and value, in registry (alphabetical) order.
    pub components: Vec<(String, Value)>,
}

/// The ECS entity with stable id `id`.
pub fn find(world: &World, id: u64) -> Option<Entity> {
    let mut q = world.try_query::<(Entity, &SceneId)>()?;
    q.iter(world).find(|(_, s)| s.0 == id).map(|(e, _)| e)
}

/// Reads one entity's registered components. Cost: one lookup per registered component
/// type, so it is cheap enough to run every frame for the selected entity only.
pub fn inspect(world: &World, registry: &Registry, id: u64) -> Option<Inspected> {
    let e = find(world, id)?;
    let name = world.get::<SceneName>(e).map(|n| n.0.clone());
    let components = registry
        .of_kind(Kind::Component)
        .filter_map(|entry| {
            entry
                .get(world, Some(e))
                .map(|v| (entry.name.to_string(), registry.to_json(v.as_partial_reflect())))
        })
        .collect();
    Some(Inspected { id, name, components })
}

/// Every scene entity as (id, name), sorted by id.
pub fn entity_list(world: &World) -> Vec<(u64, Option<String>)> {
    let Some(mut q) = world.try_query::<(&SceneId, Option<&SceneName>)>() else {
        return Vec::new();
    };
    let mut out: Vec<(u64, Option<String>)> = q
        .iter(world)
        .map(|(id, n)| (id.0, n.map(|n| n.0.clone())))
        .collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

/// Short number text: up to three decimals, no trailing zeros.
pub fn number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let s = format!("{n:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::Null => Some("none".into()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(number(n.as_f64().unwrap_or(0.0))),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// One line for objects and arrays whose members are all scalars (`x 1 y 2 z 3`).
fn inline(v: &Value) -> Option<String> {
    match v {
        Value::Object(m) => {
            let mut fields: Vec<(&String, &Value)> = m.iter().collect();
            // Vectors and colors read in their usual order (x y z w, r g b a).
            fields.sort_by_key(|(k, _)| {
                ["x", "y", "z", "w", "r", "g", "b", "a"]
                    .iter()
                    .position(|o| o == k)
                    .unwrap_or(9)
            });
            let parts: Option<Vec<String>> = fields
                .iter()
                .map(|(k, v)| scalar(v).map(|s| format!("{k} {s}")))
                .collect();
            parts.map(|p| p.join("  "))
        }
        Value::Array(a) if a.len() <= 8 => {
            let parts: Option<Vec<String>> = a.iter().map(scalar).collect();
            parts.map(|p| format!("[{}]", p.join(", ")))
        }
        _ => scalar(v),
    }
}

fn value_lines(key: &str, v: &Value, indent: usize, out: &mut Vec<String>) {
    let pad = " ".repeat(indent);
    if let Some(s) = inline(v) {
        out.push(if key.is_empty() {
            format!("{pad}{s}")
        } else {
            format!("{pad}{key}: {s}")
        });
        return;
    }
    if !key.is_empty() {
        out.push(format!("{pad}{key}"));
    }
    match v {
        Value::Object(m) => m.iter().for_each(|(k, v)| value_lines(k, v, indent + 2, out)),
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate().take(16) {
                value_lines(&format!("{i}"), v, indent + 2, out);
            }
            if a.len() > 16 {
                out.push(format!("{pad}  ... {} more", a.len() - 16));
            }
        }
        _ => {}
    }
}

/// Breaks a line into pieces of at most `width` characters (at a space when one is near the
/// end), continuing with two spaces.
fn wrap(line: &str, width: usize, out: &mut Vec<String>) {
    if width < 8 {
        out.push(line.to_string());
        return;
    }
    let mut rest: Vec<char> = line.chars().collect();
    let mut first = true;
    loop {
        let room = if first { width } else { width - 2 };
        let lead = if first { "" } else { "  " };
        if rest.len() <= room {
            out.push(format!("{lead}{}", rest.iter().collect::<String>()));
            return;
        }
        // Prefer the gap between two fields (two spaces), then any space.
        let cut = (room / 2..room)
            .rev()
            .find(|&i| rest[i] == ' ' && rest[i - 1] == ' ')
            .or_else(|| (room / 2..room).rev().find(|&i| rest[i] == ' '))
            .unwrap_or(room);
        out.push(format!(
            "{lead}{}",
            rest[..cut].iter().collect::<String>().trim_end()
        ));
        let mut skip = cut;
        while skip < rest.len() && rest[skip] == ' ' {
            skip += 1;
        }
        rest.drain(..skip);
        first = false;
    }
}

/// The inspector text for an entity: a title line, then each component (name, then its
/// fields indented), wrapped to `width` characters and cut to `max_lines`.
pub fn lines(e: &Inspected, width: usize, max_lines: usize) -> Vec<String> {
    let mut raw = vec![match &e.name {
        Some(n) => format!("#{} {n}", e.id),
        None => format!("#{}", e.id),
    }];
    for (name, v) in &e.components {
        match inline(v) {
            Some(s) if v.is_object() && !s.is_empty() => {
                raw.push(name.clone());
                raw.push(format!("  {s}"));
            }
            Some(s) if !s.is_empty() => raw.push(format!("{name}: {s}")),
            Some(_) => raw.push(name.clone()),
            None => value_lines(name, v, 0, &mut raw),
        }
    }
    let mut out = Vec::new();
    for l in &raw {
        wrap(l, width, &mut out);
    }
    if out.len() > max_lines && max_lines > 0 {
        let more = out.len() - (max_lines - 1);
        out.truncate(max_lines - 1);
        out.push(format!("... {more} more lines"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use marionette_core::builtin::Position;
    use serde_json::json;

    #[test]
    fn numbers_are_short() {
        assert_eq!(number(1.0), "1");
        assert_eq!(number(0.1f32 as f64), "0.1");
        assert_eq!(number(-2.34567), "-2.346");
        assert_eq!(number(-0.0001), "0");
    }

    #[test]
    fn components_read_back_as_scene_values_and_print_compactly() {
        let registry = marionette_core::default_registry();
        let mut world = World::new();
        registry.init_world(&mut world);
        world.spawn((
            SceneId(3),
            SceneName("crate".into()),
            Position {
                x: 1.0,
                y: 0.5,
                z: -2.0,
            },
        ));
        world.spawn(SceneId(1));
        assert_eq!(entity_list(&world), vec![(1, None), (3, Some("crate".into()))]);
        let e = inspect(&world, &registry, 3).unwrap();
        assert_eq!(
            e.components,
            vec![("Position".to_string(), json!({"x": 1.0, "y": 0.5, "z": -2.0}))]
        );
        assert_eq!(
            lines(&e, 40, 20),
            vec!["#3 crate", "Position", "  x 1  y 0.5  z -2"]
        );
        assert!(inspect(&world, &registry, 99).is_none());
    }

    #[test]
    fn nested_values_indent_wrap_and_cut() {
        let e = Inspected {
            id: 1,
            name: None,
            components: vec![
                (
                    "Script".into(),
                    json!({"path": "scripts/a-really-long-script-name-for-wrapping.ts"}),
                ),
                (
                    "Camera".into(),
                    json!({"projection": {"Perspective": {"fov_y_degrees": 60.0}}, "clear_color": {"r": 0.1, "g": 0.2, "b": 0.3, "a": 1.0}}),
                ),
                ("Tag".into(), json!({})),
            ],
        };
        let l = lines(&e, 24, 100);
        assert_eq!(l[0], "#1");
        assert!(l.iter().all(|s| s.chars().count() <= 24), "{l:#?}");
        assert!(l.contains(&"  projection".to_string()), "{l:#?}");
        assert!(l.contains(&"  clear_color: r 0.1".to_string()), "{l:#?}");
        let wide = lines(&e, 40, 100);
        assert!(
            wide.contains(&"    Perspective: fov_y_degrees 60".to_string()),
            "{wide:#?}"
        );
        assert!(l.contains(&"Tag".to_string()), "{l:#?}");
        let cut = lines(&e, 24, 4);
        assert_eq!(cut.len(), 4);
        assert!(cut[3].starts_with("..."));
    }
}
