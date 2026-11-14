//! A scene kept ready for small edits (decision 0270).
//!
//! [`crate::scene::format_text`] checks and rewrites a whole scene, so an edit made that way
//! costs time in proportion to the scene. [`SceneIndex`] holds a scene that is already
//! canonical, with each entity's text and outgoing references kept beside it, so that one
//! edit checks and rewrites only the entity or resource it touches.
//!
//! Every edit method returns `None` when the edit is not plainly valid (an unknown type, a
//! wrong field, a reference to a missing entity, an id in use). The index is then unchanged
//! and the caller runs the whole-scene path, which reports the problem in full. So this type
//! never produces diagnostics, and a successful edit gives exactly the scene and text the
//! whole-scene path would have given (`tests/scene_index.rs` holds the two against each other).

use crate::registry::{Kind, Registry};
use crate::scene::{FORMAT, Scene, SceneEntity, VERSION, collect_refs, typed};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

struct Entry {
    entity: SceneEntity,
    /// The entity as it stands in the file: pretty JSON, indented four spaces, no newline at
    /// the end.
    text: String,
    /// Ids this entity's components refer to (itself left out).
    refs: Vec<u64>,
}

/// What one edit did to an entity: its value before and after (both canonical).
#[derive(Debug, Clone, PartialEq)]
pub struct Changed {
    pub before: Option<SceneEntity>,
    pub after: Option<SceneEntity>,
}

/// A canonical scene indexed by entity id. See the module text.
pub struct SceneIndex {
    resources: BTreeMap<String, Value>,
    resources_text: String,
    entities: BTreeMap<u64, Entry>,
    /// How many other entities refer to each id.
    incoming: BTreeMap<u64, u32>,
    /// Bytes of all entity texts together.
    entity_bytes: usize,
}

fn indent(text: &str, by: &str, first: bool) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if i > 0 || first {
            out.push_str(by);
        }
        out.push_str(line);
    }
    out
}

fn entity_text(e: &SceneEntity) -> String {
    let mut obj = Map::new();
    obj.insert("id".into(), json!(e.id));
    if let Some(name) = &e.name {
        obj.insert("name".into(), json!(name));
    }
    obj.insert("components".into(), json!(e.components));
    let text = serde_json::to_string_pretty(&Value::Object(obj)).expect("JSON values always serialize");
    indent(&text, "    ", true)
}

fn resources_text(resources: &BTreeMap<String, Value>) -> String {
    let text = serde_json::to_string_pretty(&json!(resources)).expect("JSON values always serialize");
    indent(&text, "  ", false)
}

/// The entity with every component value in canonical form, and the ids it refers to
/// (itself left out, sorted, each once per reference). `None` when any component is not valid.
fn canonical(e: &SceneEntity, registry: &Registry) -> Option<(SceneEntity, Vec<u64>)> {
    if e.id == 0 {
        return None;
    }
    let mut errs = Vec::new();
    let mut components = BTreeMap::new();
    let mut refs = Vec::new();
    for (name, value) in &e.components {
        let (_, concrete) = typed(
            registry,
            Kind::Component,
            name,
            value,
            Some(e.id),
            None,
            "",
            &mut errs,
        )?;
        collect_refs(concrete.as_partial_reflect(), &mut refs);
        components.insert(name.clone(), registry.to_json(concrete.as_partial_reflect()));
    }
    if !errs.is_empty() {
        return None;
    }
    refs.retain(|r| *r != e.id);
    refs.sort_unstable();
    Some((
        SceneEntity {
            id: e.id,
            name: e.name.clone(),
            components,
        },
        refs,
    ))
}

impl SceneIndex {
    /// Indexes a scene. `None` unless `scene` is valid and already canonical (as
    /// [`crate::scene::format_text`] leaves it).
    pub fn new(scene: &Scene, registry: &Registry) -> Option<Self> {
        let mut index = Self {
            resources: BTreeMap::new(),
            resources_text: String::new(),
            entities: BTreeMap::new(),
            incoming: BTreeMap::new(),
            entity_bytes: 0,
        };
        for (name, value) in &scene.resources {
            if index.canonical_resource(name, value, registry)? != *value {
                return None;
            }
            index.resources.insert(name.clone(), value.clone());
        }
        index.resources_text = resources_text(&index.resources);
        for e in &scene.entities {
            let (entity, refs) = canonical(e, registry)?;
            if entity != *e || index.entities.contains_key(&e.id) {
                return None;
            }
            index.put(entity, refs);
        }
        // Every reference must land on an entity.
        if index.incoming.keys().any(|id| !index.entities.contains_key(id)) {
            return None;
        }
        Some(index)
    }

    fn canonical_resource(&self, name: &str, value: &Value, registry: &Registry) -> Option<Value> {
        let mut errs = Vec::new();
        let (_, concrete) = typed(registry, Kind::Resource, name, value, None, None, "", &mut errs)?;
        errs.is_empty()
            .then(|| registry.to_json(concrete.as_partial_reflect()))
    }

    fn put(&mut self, entity: SceneEntity, refs: Vec<u64>) {
        for r in &refs {
            *self.incoming.entry(*r).or_insert(0) += 1;
        }
        let text = entity_text(&entity);
        self.entity_bytes += text.len();
        self.entities.insert(entity.id, Entry { entity, text, refs });
    }

    fn take(&mut self, id: u64) -> Option<SceneEntity> {
        let entry = self.entities.remove(&id)?;
        self.entity_bytes -= entry.text.len();
        for r in &entry.refs {
            if let Some(n) = self.incoming.get_mut(r) {
                *n -= 1;
                if *n == 0 {
                    self.incoming.remove(r);
                }
            }
        }
        Some(entry.entity)
    }

    fn refs_exist(&self, refs: &[u64]) -> bool {
        refs.iter().all(|r| self.entities.contains_key(r))
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn entity(&self, id: u64) -> Option<&SceneEntity> {
        self.entities.get(&id).map(|e| &e.entity)
    }

    /// One more than the largest id in use.
    pub fn next_id(&self) -> u64 {
        self.entities.keys().next_back().copied().unwrap_or(0) + 1
    }

    /// Adds an entity. `None` (nothing changed) when its id is taken or it is not valid.
    pub fn create(&mut self, entity: &SceneEntity, registry: &Registry) -> Option<Changed> {
        if self.entities.contains_key(&entity.id) {
            return None;
        }
        let (entity, refs) = canonical(entity, registry)?;
        if !self.refs_exist(&refs) {
            return None;
        }
        self.put(entity.clone(), refs);
        Some(Changed {
            before: None,
            after: Some(entity),
        })
    }

    /// Replaces an entity (same id) with `entity`. `None` when there is no such entity or
    /// the new value is not valid.
    pub fn replace(&mut self, entity: &SceneEntity, registry: &Registry) -> Option<Changed> {
        if !self.entities.contains_key(&entity.id) {
            return None;
        }
        let (entity, refs) = canonical(entity, registry)?;
        if !self.refs_exist(&refs) {
            return None;
        }
        let before = self.take(entity.id);
        self.put(entity.clone(), refs);
        Some(Changed {
            before,
            after: Some(entity),
        })
    }

    /// Removes an entity. `None` when it does not exist or another entity refers to it.
    pub fn delete(&mut self, id: u64) -> Option<Changed> {
        if !self.entities.contains_key(&id) || self.incoming.contains_key(&id) {
            return None;
        }
        Some(Changed {
            before: self.take(id),
            after: None,
        })
    }

    /// Adds or replaces a resource; gives whether it was there before. `None` when not valid.
    pub fn set_resource(&mut self, name: &str, value: &Value, registry: &Registry) -> Option<bool> {
        let value = self.canonical_resource(name, value, registry)?;
        let replaced = self.resources.insert(name.to_string(), value).is_some();
        self.resources_text = resources_text(&self.resources);
        Some(replaced)
    }

    /// Removes a resource. `None` when it is not there.
    pub fn remove_resource(&mut self, name: &str) -> Option<()> {
        self.resources.remove(name)?;
        self.resources_text = resources_text(&self.resources);
        Some(())
    }

    /// The canonical text: the same bytes as [`Scene::to_text`] of [`SceneIndex::to_scene`].
    pub fn to_text(&self) -> String {
        let mut out = String::with_capacity(
            self.entity_bytes + self.resources_text.len() + 2 * self.entities.len() + 96,
        );
        out.push_str("{\n  \"entities\": [");
        if self.entities.is_empty() {
            out.push(']');
        } else {
            for (i, e) in self.entities.values().enumerate() {
                out.push_str(if i == 0 { "\n" } else { ",\n" });
                out.push_str(&e.text);
            }
            out.push_str("\n  ]");
        }
        out.push_str(",\n  \"format\": \"");
        out.push_str(FORMAT);
        out.push_str("\",\n  \"resources\": ");
        out.push_str(&self.resources_text);
        out.push_str(",\n  \"version\": ");
        out.push_str(&VERSION.to_string());
        out.push_str("\n}\n");
        out
    }

    pub fn to_scene(&self) -> Scene {
        Scene {
            resources: self.resources.clone(),
            entities: self.entities.values().map(|e| e.entity.clone()).collect(),
        }
    }
}
