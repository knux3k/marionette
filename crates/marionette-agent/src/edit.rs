//! Edit operations. Each edit is applied to the scene as data, validated against the schema,
//! and only then written (files: atomically, in canonical form; sessions: reloaded into the
//! live world keeping tick and RNG). A refused edit changes nothing.
//!
//! An edit addresses exactly one target: a scene file (`scene`) or a live session
//! (`session`). File edits never touch running sessions; session edits never touch files until
//! `session_save`. See docs/decisions/0009-live-sessions-and-files.md.

use crate::{Agent, Op, codes, op, read_text, write_atomic};
use marionette_core::scene::{self, Scene, SceneEntity};
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "entity_create",
            "Add an entity (id defaults to the next free id) with optional name and components. \
             Target: `scene` (file) or `session` (live world).",
            true,
            entity_create,
        ),
        op(
            "entity_update",
            "Rename an entity and/or set and remove several components in one validated edit.",
            true,
            entity_update,
        ),
        op(
            "entity_delete",
            "Remove an entity. Fails if other entities still refer to it.",
            true,
            entity_delete,
        ),
        op(
            "component_set",
            "Add or replace one component on an entity.",
            true,
            component_set,
        ),
        op(
            "component_remove",
            "Remove one component from an entity.",
            true,
            component_remove,
        ),
        op(
            "resource_set",
            "Add or replace one scene resource (e.g. `PhysicsSettings`, `InputConfig`, `AmbientLight`, \
             `FixedStep`).",
            true,
            resource_set,
        ),
        op(
            "resource_remove",
            "Remove one scene resource.",
            true,
            resource_remove,
        ),
    ]
}

/// Where an edit goes. Exactly one of `scene` and `session` must be set.
#[derive(Deserialize, JsonSchema, Clone)]
pub struct EditTarget {
    /// Scene file to edit, e.g. `scenes/main.scene.json`.
    pub scene: Option<String>,
    /// Live session id (from session_start) to edit instead of a file.
    pub session: Option<String>,
}

/// What an edit changed.
#[derive(Serialize, JsonSchema, Default, Debug, PartialEq)]
pub struct Diff {
    pub added: Vec<u64>,
    pub removed: Vec<u64>,
    pub changed: Vec<EntityChange>,
}

#[derive(Serialize, JsonSchema, Debug, PartialEq)]
pub struct EntityChange {
    pub id: u64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub renamed: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modified: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct EditOutput {
    /// The scene file or session that was changed.
    pub target: String,
    /// The entity the edit was about.
    pub id: u64,
    pub diff: Diff,
}

pub fn diff(old: &Scene, new: &Scene) -> Diff {
    let find = |s: &Scene, id| s.entities.iter().find(|e| e.id == id).cloned();
    let ids = |s: &Scene| s.entities.iter().map(|e| e.id).collect::<BTreeSet<u64>>();
    let (a, b) = (ids(old), ids(new));
    let mut d = Diff {
        added: b.difference(&a).copied().collect(),
        removed: a.difference(&b).copied().collect(),
        changed: Vec::new(),
    };
    for id in a.intersection(&b) {
        let (o, n): (SceneEntity, SceneEntity) =
            (find(old, *id).expect("in old"), find(new, *id).expect("in new"));
        let ok: BTreeSet<&String> = o.components.keys().collect();
        let nk: BTreeSet<&String> = n.components.keys().collect();
        let change = EntityChange {
            id: *id,
            renamed: o.name != n.name,
            added: nk.difference(&ok).map(|s| s.to_string()).collect(),
            removed: ok.difference(&nk).map(|s| s.to_string()).collect(),
            modified: ok
                .intersection(&nk)
                .filter(|k| o.components[**k] != n.components[**k])
                .map(|s| s.to_string())
                .collect(),
        };
        if change.renamed
            || !change.added.is_empty()
            || !change.removed.is_empty()
            || !change.modified.is_empty()
        {
            d.changed.push(change);
        }
    }
    d
}

/// Line numbers would refer to text that was never written, so a refused edit reports the JSON
/// pointer (still exact) instead.
fn without_positions(mut e: Errors, file: Option<&str>) -> Errors {
    for d in &mut e.0 {
        d.line = None;
        d.column = None;
        if let Some(f) = file {
            d.file = Some(f.to_string());
        }
    }
    e
}

fn entity_mut<'a>(s: &'a mut Scene, id: u64, target: &str) -> Result<&'a mut SceneEntity, Errors> {
    let ids: Vec<String> = s.entities.iter().take(20).map(|e| e.id.to_string()).collect();
    s.entities.iter_mut().find(|e| e.id == id).ok_or_else(|| {
        Errors::from(
            Diagnostic::error(codes::NO_ENTITY, format!("{target} has no entity {id}"))
                .entity(id)
                .fix(format!("existing ids: {}", ids.join(", "))),
        )
    })
}

/// Applies `change` to the target, validates, then writes. Returns the diff.
fn apply(
    agent: &mut Agent,
    target: &EditTarget,
    change: impl FnOnce(&mut Scene, &str) -> Result<(), Errors>,
) -> Result<(String, Diff), Errors> {
    match (&target.scene, &target.session) {
        (Some(file), None) => {
            let path = agent.resolve(file)?;
            let text = read_text(&path, file)?;
            let old = scene::parse(&text).map_err(|e| e.with_file(file))?;
            // The file must already be valid; otherwise report its own problems first.
            scene::format_text(&text, agent.registry()).map_err(|e| e.with_file(file))?;
            let mut new = old.clone();
            change(&mut new, file)?;
            let candidate = new.to_text();
            let canonical = scene::format_text(&candidate, agent.registry())
                .map_err(|e| without_positions(e, Some(file)))?;
            let written = scene::parse(&canonical).expect("canonical text parses");
            let d = diff(&old, &written);
            write_atomic(&path, file, canonical.as_bytes())?;
            Ok((file.clone(), d))
        }
        (None, Some(id)) => {
            let session = crate::session::get_mut(agent, id)?;
            let old = session.sim.snapshot();
            let mut new = old.clone();
            change(&mut new, &format!("session {id}"))?;
            session
                .sim
                .reload(&new.to_text())
                .map_err(|e| without_positions(e, None))?;
            let d = diff(&old, &session.sim.snapshot());
            session.log("edit", format!("{d:?}"));
            Ok((format!("session {id}"), d))
        }
        _ => Err(Diagnostic::error(
            codes::TARGET,
            "an edit needs exactly one of `scene` and `session`",
        )
        .fix("pass `scene` to edit a file, or `session` to edit a live world")
        .into()),
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct EntityCreateInput {
    #[serde(flatten)]
    pub target: EditTarget,
    /// Stable id; default: one more than the largest id in use.
    pub id: Option<u64>,
    pub name: Option<String>,
    /// Components keyed by type name, e.g. {"Position": {"x": 0, "y": 1, "z": 0}}.
    #[serde(default)]
    pub components: Map<String, Value>,
}

fn entity_create(agent: &mut Agent, input: EntityCreateInput) -> Result<EditOutput, Errors> {
    let mut new_id = 0;
    let (target, diff) = apply(agent, &input.target, |s, t| {
        let id = input
            .id
            .unwrap_or_else(|| s.entities.iter().map(|e| e.id).max().unwrap_or(0) + 1);
        if s.entities.iter().any(|e| e.id == id) {
            return Err(
                Diagnostic::error(codes::EXISTS, format!("{t} already has entity {id}"))
                    .entity(id)
                    .fix("omit `id` to get the next free one")
                    .into(),
            );
        }
        new_id = id;
        s.entities.push(SceneEntity {
            id,
            name: input.name.clone(),
            components: input.components.clone().into_iter().collect(),
        });
        Ok(())
    })?;
    Ok(EditOutput {
        target,
        id: new_id,
        diff,
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct EntityUpdateInput {
    #[serde(flatten)]
    pub target: EditTarget,
    pub id: u64,
    /// New name; an empty string removes the name.
    pub name: Option<String>,
    /// Components to add or replace.
    #[serde(default)]
    pub set: Map<String, Value>,
    /// Component type names to remove.
    #[serde(default)]
    pub remove: Vec<String>,
}

fn entity_update(agent: &mut Agent, input: EntityUpdateInput) -> Result<EditOutput, Errors> {
    let id = input.id;
    let (target, diff) = apply(agent, &input.target, |s, t| {
        let e = entity_mut(s, id, t)?;
        if let Some(name) = &input.name {
            e.name = (!name.is_empty()).then(|| name.clone());
        }
        for (k, v) in &input.set {
            e.components.insert(k.clone(), v.clone());
        }
        for k in &input.remove {
            if e.components.remove(k).is_none() {
                return Err(
                    Diagnostic::error(codes::NOT_FOUND, format!("entity {id} has no `{k}`"))
                        .entity(id)
                        .component(k.clone())
                        .into(),
                );
            }
        }
        Ok(())
    })?;
    Ok(EditOutput { target, id, diff })
}

#[derive(Deserialize, JsonSchema)]
pub struct EntityDeleteInput {
    #[serde(flatten)]
    pub target: EditTarget,
    pub id: u64,
}

fn entity_delete(agent: &mut Agent, input: EntityDeleteInput) -> Result<EditOutput, Errors> {
    let id = input.id;
    let (target, diff) = apply(agent, &input.target, |s, t| {
        entity_mut(s, id, t)?;
        s.entities.retain(|e| e.id != id);
        Ok(())
    })?;
    Ok(EditOutput { target, id, diff })
}

#[derive(Deserialize, JsonSchema)]
pub struct ComponentSetInput {
    #[serde(flatten)]
    pub target: EditTarget,
    pub id: u64,
    /// Component type name, e.g. `Velocity`.
    pub component: String,
    /// The full component value, e.g. {"x": 1, "y": 0, "z": 0}.
    pub value: Value,
}

fn component_set(agent: &mut Agent, input: ComponentSetInput) -> Result<EditOutput, Errors> {
    let id = input.id;
    let (target, diff) = apply(agent, &input.target, |s, t| {
        entity_mut(s, id, t)?
            .components
            .insert(input.component.clone(), input.value.clone());
        Ok(())
    })?;
    Ok(EditOutput { target, id, diff })
}

#[derive(Deserialize, JsonSchema)]
pub struct ComponentRemoveInput {
    #[serde(flatten)]
    pub target: EditTarget,
    pub id: u64,
    pub component: String,
}

fn component_remove(agent: &mut Agent, input: ComponentRemoveInput) -> Result<EditOutput, Errors> {
    let id = input.id;
    let (target, diff) = apply(agent, &input.target, |s, t| {
        let e = entity_mut(s, id, t)?;
        if e.components.remove(&input.component).is_none() {
            return Err(Diagnostic::error(
                codes::NOT_FOUND,
                format!("entity {id} has no `{}`", input.component),
            )
            .entity(id)
            .component(input.component.clone())
            .into());
        }
        Ok(())
    })?;
    Ok(EditOutput { target, id, diff })
}

#[derive(Deserialize, JsonSchema)]
pub struct ResourceSetInput {
    #[serde(flatten)]
    pub target: EditTarget,
    /// Resource type name, e.g. `PhysicsSettings`.
    pub resource: String,
    /// The full resource value, e.g. {"mode": "Dim3", "gravity": {"x": 0, "y": -9.81, "z": 0}}.
    pub value: Value,
}

#[derive(Serialize, JsonSchema)]
pub struct ResourceEditOutput {
    /// The scene file or session that was changed.
    pub target: String,
    pub resource: String,
    /// True when the resource existed before the edit.
    pub replaced: bool,
}

fn resource_set(agent: &mut Agent, input: ResourceSetInput) -> Result<ResourceEditOutput, Errors> {
    let mut replaced = false;
    let (target, _) = apply(agent, &input.target, |s, _| {
        replaced = s
            .resources
            .insert(input.resource.clone(), input.value.clone())
            .is_some();
        Ok(())
    })?;
    Ok(ResourceEditOutput {
        target,
        resource: input.resource,
        replaced,
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct ResourceRemoveInput {
    #[serde(flatten)]
    pub target: EditTarget,
    pub resource: String,
}

fn resource_remove(agent: &mut Agent, input: ResourceRemoveInput) -> Result<ResourceEditOutput, Errors> {
    let (target, _) = apply(agent, &input.target, |s, t| {
        if s.resources.remove(&input.resource).is_none() {
            return Err(Diagnostic::error(
                codes::NOT_FOUND,
                format!("{t} has no resource `{}`", input.resource),
            )
            .fix("observe_tree or the scene file lists the resources in use")
            .into());
        }
        Ok(())
    })?;
    Ok(ResourceEditOutput {
        target,
        resource: input.resource,
        replaced: true,
    })
}
