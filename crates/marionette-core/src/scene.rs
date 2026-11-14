//! The text scene format: JSON with stable entity ids and canonical output.
//!
//! ```json
//! {
//!   "entities": [
//!     { "components": { "Position": { "x": 0.0, "y": 1.0, "z": 0.0 } }, "id": 1, "name": "ball" }
//!   ],
//!   "format": "marionette.scene",
//!   "resources": { "FixedStep": { "dt": 0.016666668 } },
//!   "version": 1
//! }
//! ```
//!
//! Canonical form (what [`Scene::to_text`] writes): object keys sorted, entities sorted by
//! id, two-space indent, floats in shortest round-trip form, trailing newline. Loading a
//! canonical file and saving it gives back the same bytes.

use crate::builtin::SceneRef;
use crate::error::{Diagnostic, Errors, Result, codes};
use crate::json_span::{self, Spans, pointer};
use crate::registry::{Kind, Registry, TypeEntry};
use bevy_ecs::prelude::*;
use bevy_reflect::{PartialReflect, Reflect, ReflectRef, TypeInfo};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const FORMAT: &str = "marionette.scene";
pub const VERSION: u64 = 1;

/// Stable scene id of an entity. Internal component; written as the entity's `id`.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SceneId(pub u64);

/// Optional entity name from the scene file. Internal component.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct SceneName(pub String);

/// A scene as plain data (untyped component values).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub resources: BTreeMap<String, Value>,
    pub entities: Vec<SceneEntity>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneEntity {
    pub id: u64,
    pub name: Option<String>,
    pub components: BTreeMap<String, Value>,
}

impl Scene {
    /// Canonical text. Deterministic: same scene, same bytes.
    pub fn to_text(&self) -> String {
        let mut entities: Vec<&SceneEntity> = self.entities.iter().collect();
        entities.sort_by_key(|e| e.id);
        let entities: Vec<Value> = entities
            .into_iter()
            .map(|e| {
                let mut obj = Map::new();
                obj.insert("id".into(), json!(e.id));
                if let Some(name) = &e.name {
                    obj.insert("name".into(), json!(name));
                }
                obj.insert("components".into(), json!(e.components));
                Value::Object(obj)
            })
            .collect();
        let doc = json!({
            "format": FORMAT,
            "version": VERSION,
            "resources": self.resources,
            "entities": entities,
        });
        // serde_json::Map is a BTreeMap (no `preserve_order` feature), so keys are sorted.
        let mut text = serde_json::to_string_pretty(&doc).expect("JSON values always serialize");
        text.push('\n');
        text
    }
}

/// Reads scene text into a [`Scene`], checking only the file's shape (not component types).
pub fn parse(text: &str) -> Result<Scene> {
    let doc: Value = serde_json::from_str(text).map_err(|e| {
        Diagnostic::error(codes::SCENE_PARSE, format!("invalid JSON: {e}"))
            .at(Some(e.line()), Some(e.column()))
            .fix("fix the JSON syntax at this position")
    })?;
    let spans = json_span::spans(text);
    let mut errs = Vec::new();
    let Value::Object(root) = doc else {
        return Err(
            Diagnostic::error(codes::SCENE_SHAPE, "scene must be a JSON object")
                .at(Some(1), Some(1))
                .into(),
        );
    };
    match root.get("format") {
        Some(Value::String(f)) if f == FORMAT => {}
        _ => errs.push(
            Diagnostic::error(codes::SCENE_FORMAT, "missing or wrong `format`")
                .at_pos(spans.get("/format"))
                .fix(format!("add \"format\": \"{FORMAT}\" at the top level")),
        ),
    }
    match root.get("version") {
        Some(v) if v.as_u64() == Some(VERSION) => {}
        other => errs.push(
            Diagnostic::error(
                codes::SCENE_VERSION,
                format!(
                    "unsupported scene version {}",
                    other.map_or("(missing)".into(), |v| v.to_string())
                ),
            )
            .at_pos(spans.get("/version"))
            .fix(format!("set \"version\": {VERSION}")),
        ),
    }
    for key in root.keys() {
        if !matches!(key.as_str(), "format" | "version" | "resources" | "entities") {
            errs.push(
                Diagnostic::error(codes::SCENE_SHAPE, format!("unknown top-level key `{key}`"))
                    .at_pos(spans.get(&pointer([key])))
                    .fix("remove it; allowed keys are format, version, resources, entities"),
            );
        }
    }

    let mut scene = Scene::default();
    match root.get("resources") {
        None => {}
        Some(Value::Object(res)) => {
            scene.resources = res.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        }
        Some(_) => errs.push(
            Diagnostic::error(codes::SCENE_SHAPE, "`resources` must be an object")
                .at_pos(spans.get("/resources")),
        ),
    }
    match root.get("entities") {
        Some(Value::Array(list)) => {
            for (index, item) in list.iter().enumerate() {
                match parse_entity(item) {
                    Ok(e) => scene.entities.push(e),
                    Err(msg) => errs.push(
                        Diagnostic::error(codes::SCENE_SHAPE, format!("entities[{index}]: {msg}"))
                            .at_pos(spans.get(&format!("/entities/{index}")))
                            .fix("each entity is {\"id\": <non-zero integer>, \"name\"?: <string>, \"components\": {...}}"),
                    ),
                }
            }
        }
        _ => errs.push(
            Diagnostic::error(codes::SCENE_SHAPE, "missing `entities` array")
                .fix("add \"entities\": [] at the top level"),
        ),
    }
    if errs.is_empty() {
        Ok(scene)
    } else {
        Err(Errors(errs))
    }
}

fn parse_entity(item: &Value) -> std::result::Result<SceneEntity, String> {
    let Value::Object(obj) = item else {
        return Err("entity must be an object".into());
    };
    for key in obj.keys() {
        if !matches!(key.as_str(), "id" | "name" | "components") {
            return Err(format!("unknown key `{key}`"));
        }
    }
    let id = obj
        .get("id")
        .and_then(Value::as_u64)
        .ok_or("`id` must be a non-negative integer")?;
    let name = match obj.get("name") {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err("`name` must be a string".into()),
    };
    let components = match obj.get("components") {
        Some(Value::Object(c)) => c.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => return Err(format!("entity {id}: `components` must be an object")),
    };
    Ok(SceneEntity { id, name, components })
}

/// One typed value ready to insert.
pub(crate) type Typed<'r> = (&'r TypeEntry, Box<dyn Reflect>);

/// A scene whose values have all been checked against the registry.
pub struct ValidScene<'r> {
    resources: Vec<Typed<'r>>,
    entities: Vec<(u64, Option<String>, Vec<Typed<'r>>)>,
}

/// Checks a parsed scene against the registry. `text` (if given, the text `scene` was
/// parsed from) is used for exact line and column numbers. Returns every problem found, not just the first.
pub fn validate<'r>(scene: &Scene, registry: &'r Registry, text: Option<&str>) -> Result<ValidScene<'r>> {
    let mut errs = Vec::new();
    let spans = text.map(json_span::spans);
    let spans = spans.as_ref();
    let pos = |p: &str| spans.and_then(|s| s.get(p));

    let mut ids = BTreeSet::new();
    for (i, e) in scene.entities.iter().enumerate() {
        // Looked up only for a problem: the first lookup scans the whole text.
        let id_pos = || pos(&format!("/entities/{i}/id"));
        if e.id == 0 {
            errs.push(
                Diagnostic::error(codes::SCENE_INVALID_ID, "entity id 0 is reserved")
                    .at_pos(id_pos())
                    .fix("use a positive integer id"),
            );
        } else if !ids.insert(e.id) {
            errs.push(
                Diagnostic::error(
                    codes::SCENE_DUPLICATE_ID,
                    format!("entity id {} is used more than once", e.id),
                )
                .at_pos(id_pos())
                .entity(e.id)
                .fix(format!(
                    "give one of them an unused id, e.g. {}",
                    scene.entities.iter().map(|e| e.id).max().unwrap_or(0) + 1
                )),
            );
        }
    }

    let mut resources = Vec::new();
    for (name, value) in &scene.resources {
        match typed(
            registry,
            Kind::Resource,
            name,
            value,
            None,
            spans,
            &pointer(["resources", name]),
            &mut errs,
        ) {
            Some(v) => resources.push(v),
            None => continue,
        }
    }

    let mut entities = Vec::new();
    for (i, e) in scene.entities.iter().enumerate() {
        let mut comps = Vec::new();
        for (name, value) in &e.components {
            let ptr = pointer(["entities", &i.to_string(), "components", name]);
            let Some((entry, concrete)) = typed(
                registry,
                Kind::Component,
                name,
                value,
                Some(e.id),
                spans,
                &ptr,
                &mut errs,
            ) else {
                continue;
            };
            let mut refs = Vec::new();
            collect_refs(concrete.as_partial_reflect(), &mut refs);
            for target in refs {
                if !ids.contains(&target) {
                    errs.push(
                        Diagnostic::error(
                            codes::SCENE_DANGLING_REF,
                            format!("`{name}` refers to entity {target}, which does not exist"),
                        )
                        .at_pos(pos(&ptr))
                        .entity(e.id)
                        .component(name.clone())
                        .fix(format!(
                            "point it at an existing id ({}) or add entity {target}",
                            ids_hint(&ids)
                        )),
                    );
                }
            }
            comps.push((entry, concrete));
        }
        entities.push((e.id, e.name.clone(), comps));
    }
    entities.sort_by_key(|(id, _, _)| *id);

    if errs.is_empty() {
        Ok(ValidScene { resources, entities })
    } else {
        Err(Errors(errs))
    }
}

fn ids_hint(ids: &BTreeSet<u64>) -> String {
    let shown: Vec<String> = ids.iter().take(8).map(u64::to_string).collect();
    if ids.len() > 8 {
        format!("{}, ...", shown.join(", "))
    } else {
        shown.join(", ")
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn typed<'r>(
    registry: &'r Registry,
    kind: Kind,
    name: &str,
    value: &Value,
    entity: Option<u64>,
    spans: Option<&Spans>,
    ptr: &str,
    errs: &mut Vec<Diagnostic>,
) -> Option<Typed<'r>> {
    let pos = |p: &str| spans.and_then(|s| s.get(p));
    let base = |code, msg: String| {
        let mut d = Diagnostic::error(code, msg)
            .at_pos(pos(ptr))
            .pointer(ptr)
            .component(name);
        if let Some(id) = entity {
            d = d.entity(id);
        }
        d
    };
    let entry = match registry.get(name) {
        Some(entry) if entry.kind == kind => entry,
        other => {
            let (code, what) = match kind {
                Kind::Component => (codes::SCENE_UNKNOWN_COMPONENT, "component"),
                Kind::Resource => (codes::SCENE_UNKNOWN_RESOURCE, "resource"),
            };
            let candidates: Vec<&str> = registry.of_kind(kind).map(|e| e.name).collect();
            let fix = if other.is_some() {
                format!("`{name}` is not a {what}; move it to the right section")
            } else if let Some(best) = closest(name, &candidates) {
                format!("did you mean `{best}`?")
            } else {
                format!("known {what}s: {}", candidates.join(", "))
            };
            errs.push(base(code, format!("unknown {what} `{name}`")).fix(fix));
            return None;
        }
    };

    // Field-level checks first, for precise messages.
    if let (TypeInfo::Struct(info), Value::Object(obj)) = (entry.info, value) {
        let mut ok = true;
        for key in obj.keys() {
            if info.field(key).is_none() {
                let names: Vec<&str> = info.field_names().to_vec();
                let fix = closest(key, &names).map_or_else(
                    || format!("fields of `{name}`: {}", names.join(", ")),
                    |b| format!("did you mean `{b}`?"),
                );
                errs.push(
                    base(
                        codes::SCENE_UNKNOWN_FIELD,
                        format!("`{name}` has no field `{key}`"),
                    )
                    .at_pos(pos(&format!("{ptr}{}", pointer([key]))))
                    .pointer(format!("{ptr}{}", pointer([key])))
                    .field(key.clone())
                    .fix(fix),
                );
                ok = false;
            }
        }
        for field in info.field_names() {
            if !obj.contains_key(*field) {
                errs.push(
                    base(
                        codes::SCENE_MISSING_FIELD,
                        format!("`{name}` is missing field `{field}`"),
                    )
                    .field(*field)
                    .fix(format!("add \"{field}\": <value>")),
                );
                ok = false;
            }
        }
        if !ok {
            return None;
        }
    }

    match registry.from_json(entry, value) {
        Ok(concrete) => Some((entry, concrete)),
        Err(msg) => {
            errs.push(
                base(
                    codes::SCENE_FIELD_TYPE,
                    format!("`{name}` has a wrong value: {msg}"),
                )
                .fix(format!("see the `{name}` definition in `marionette schema`")),
            );
            None
        }
    }
}

/// Finds every [`SceneRef`] inside a reflected value.
pub(crate) fn collect_refs(value: &dyn PartialReflect, out: &mut Vec<u64>) {
    if let Some(r) = value.try_downcast_ref::<SceneRef>() {
        out.push(r.0);
        return;
    }
    match value.reflect_ref() {
        ReflectRef::Struct(s) => (0..s.field_len())
            .filter_map(|i| s.field_at(i))
            .for_each(|f| collect_refs(f, out)),
        ReflectRef::TupleStruct(s) => (0..s.field_len())
            .filter_map(|i| s.field(i))
            .for_each(|f| collect_refs(f, out)),
        ReflectRef::Tuple(t) => (0..t.field_len())
            .filter_map(|i| t.field(i))
            .for_each(|f| collect_refs(f, out)),
        ReflectRef::List(l) => l.iter().for_each(|f| collect_refs(f, out)),
        ReflectRef::Array(a) => a.iter().for_each(|f| collect_refs(f, out)),
        ReflectRef::Map(m) => m.iter().for_each(|(_, v)| collect_refs(v, out)),
        ReflectRef::Enum(e) => e.iter_fields().for_each(|f| collect_refs(f.value(), out)),
        _ => {}
    }
}

/// Edit distance suggestion ("did you mean").
fn closest<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    fn lev(a: &str, b: &str) -> usize {
        let b: Vec<char> = b.to_lowercase().chars().collect();
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        for (i, ca) in a.to_lowercase().chars().enumerate() {
            let mut cur = vec![i + 1];
            for (j, cb) in b.iter().enumerate() {
                cur.push(
                    (prev[j] + usize::from(ca != *cb))
                        .min(prev[j + 1] + 1)
                        .min(cur[j] + 1),
                );
            }
            prev = cur;
        }
        prev[b.len()]
    }
    candidates
        .iter()
        .map(|c| (lev(name, c), *c))
        .filter(|(d, _)| *d <= 3)
        .min()
        .map(|(_, c)| c)
}

/// Spawns a validated scene into `world`.
pub fn instantiate(scene: ValidScene<'_>, world: &mut World) {
    for (entry, value) in scene.resources {
        entry.insert(world, None, value);
    }
    for (id, name, comps) in scene.entities {
        let mut e = world.spawn(SceneId(id));
        if let Some(name) = name {
            e.insert(SceneName(name));
        }
        let entity = e.id();
        for (entry, value) in comps {
            entry.insert(world, Some(entity), value);
        }
    }
}

/// Parses, validates and spawns scene text into `world`. `file` is used in errors.
pub fn load(text: &str, file: Option<&str>, registry: &Registry, world: &mut World) -> Result<()> {
    let result = parse(text).and_then(|scene| validate(&scene, registry, Some(text)));
    match result {
        Ok(valid) => {
            instantiate(valid, world);
            Ok(())
        }
        Err(e) => Err(match file {
            Some(f) => e.with_file(f),
            None => e,
        }),
    }
}

/// Reads the scene entities (those with a [`SceneId`]) and registered resources back
/// out of `world`.
pub fn extract(world: &mut World, registry: &Registry) -> Scene {
    let mut scene = Scene::default();
    for entry in registry.of_kind(Kind::Resource) {
        if let Some(value) = entry.get(world, None) {
            scene.resources.insert(
                entry.name.to_string(),
                registry.to_json(value.as_partial_reflect()),
            );
        }
    }
    let mut query = world.query::<(Entity, &SceneId, Option<&SceneName>)>();
    let mut found: Vec<(u64, Entity, Option<String>)> = query
        .iter(world)
        .map(|(e, id, name)| (id.0, e, name.map(|n| n.0.clone())))
        .collect();
    found.sort_by_key(|(id, _, _)| *id);
    for (id, entity, name) in found {
        let mut components = BTreeMap::new();
        for entry in registry.of_kind(Kind::Component) {
            if let Some(value) = entry.get(world, Some(entity)) {
                components.insert(
                    entry.name.to_string(),
                    registry.to_json(value.as_partial_reflect()),
                );
            }
        }
        scene.entities.push(SceneEntity { id, name, components });
    }
    scene
}

/// A copy of a world's saved state, taken on the main thread so the slow part of saving
/// (turning it into scene text) can run on another thread while the simulation goes on.
/// [`Capture::into_scene`] gives exactly what [`extract`] gives for the same world.
pub struct Capture {
    resources: Vec<Copied>,
    entities: Vec<(u64, Option<String>, Vec<Copied>)>,
}

/// A copied value and its registered type name.
type Copied = (&'static str, Box<dyn Reflect>);

/// Copies every saved resource and component of `world` (see [`Capture`]).
pub fn capture(world: &mut World, registry: &Registry) -> Capture {
    let mut resources = Vec::new();
    for entry in registry.of_kind(Kind::Resource) {
        if let Some(value) = entry.copy(world, None) {
            resources.push((entry.name, value));
        }
    }
    let mut query = world.query::<(Entity, &SceneId, Option<&SceneName>)>();
    let mut found: Vec<(u64, Entity, Option<String>)> = query
        .iter(world)
        .map(|(e, id, name)| (id.0, e, name.map(|n| n.0.clone())))
        .collect();
    found.sort_by_key(|(id, _, _)| *id);
    let components: Vec<&TypeEntry> = registry.of_kind(Kind::Component).collect();
    let mut entities = Vec::with_capacity(found.len());
    for (id, entity, name) in found {
        let mut list = Vec::new();
        for entry in &components {
            if let Some(value) = entry.copy(world, Some(entity)) {
                list.push((entry.name, value));
            }
        }
        entities.push((id, name, list));
    }
    Capture { resources, entities }
}

impl Capture {
    /// The captured state as a scene. Needs only the registry, so it can run on any thread.
    pub fn into_scene(self, registry: &Registry) -> Scene {
        let mut scene = Scene::default();
        for (name, value) in self.resources {
            scene
                .resources
                .insert(name.to_string(), registry.to_json(value.as_partial_reflect()));
        }
        for (id, name, list) in self.entities {
            let components = list
                .into_iter()
                .map(|(n, v)| (n.to_string(), registry.to_json(v.as_partial_reflect())))
                .collect();
            scene.entities.push(SceneEntity { id, name, components });
        }
        scene
    }
}

/// Rewrites scene text in canonical form. Values go through the typed path, so this also
/// normalizes numbers (e.g. `1` for an `f32` field becomes `1.0`).
pub fn format_text(text: &str, registry: &Registry) -> Result<String> {
    let mut world = World::new();
    load(text, None, registry, &mut world)?;
    Ok(extract(&mut world, registry).to_text())
}
