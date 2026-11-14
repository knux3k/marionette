//! Edit operations. Each edit is applied to the scene as data, validated against the schema,
//! and only then written (files: atomically, in canonical form; sessions: reloaded into the
//! live world keeping tick and RNG). A refused edit changes nothing.
//!
//! An edit addresses exactly one target: a scene file (`scene`) or a live session
//! (`session`). File edits never touch running sessions; session edits never touch files until
//! `session_save`. See docs/decisions/0009-live-sessions-and-files.md.

use crate::{Agent, Op, codes, op, read_text, write_atomic};
use marionette_core::scene::{self, Scene, SceneEntity};
use marionette_core::scene_index::{Changed, SceneIndex};
use marionette_core::{Diagnostic, Errors, Registry};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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

/// Operations that edit a scene file through the [`SceneCache`]. Any other operation first
/// has pending scene text written (see [`Agent::call`]).
pub(crate) const CACHED_EDITS: [&str; 7] = [
    "entity_create",
    "entity_update",
    "entity_delete",
    "component_set",
    "component_remove",
    "resource_set",
    "resource_remove",
];

/// Scene files kept indexed between edits (decision 0270), so an edit checks and rewrites
/// only what it touches instead of the whole scene. An entry is used only while the file on
/// disk still holds exactly the text this process last wrote; anything else (a hand edit,
/// another process, another operation) drops it and the whole-scene path runs.
///
/// Inside `batch_run` the text is written once, when the batch ends or before the next
/// operation that is not an edit, instead of after every step.
#[derive(Default)]
pub(crate) struct SceneCache {
    entries: Vec<CachedScene>,
    /// True while a batch runs: edits mark the entry dirty instead of writing, and edits of
    /// a live session are loaded into its world together (see [`SessionEdits`]).
    pub(crate) deferred: bool,
    /// Set when loading a session's pending edits failed: the batch then runs those steps
    /// again one at a time, to stop at the step at fault with its own message.
    pub(crate) load_failed: bool,
    /// Test switch: every edit takes the whole-scene path.
    pub(crate) off: bool,
}

/// A live session's scene kept indexed between edits. Outside a batch each edit still
/// reloads the world (so a refused edit is refused at once), but skips the two snapshots and
/// the comparison. Inside `batch_run`, edits in a row are loaded into the world once.
pub(crate) struct SessionEdits {
    index: SceneIndex,
    /// Log lines of edits not yet loaded into the world (empty: the world is current).
    pending: Vec<String>,
}

impl SessionEdits {
    pub(crate) fn pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

/// Loads a session's pending edits into its world. On failure the world is as it was before
/// those edits and `load_failed` is set.
pub(crate) fn load_pending(agent: &mut Agent, id: &str) -> Result<(), Errors> {
    let Some(session) = agent.sessions.get_mut(id) else {
        return Ok(());
    };
    let Some(mut edits) = session.edits.take() else {
        return Ok(());
    };
    if !edits.pending() {
        session.edits = Some(edits);
        return Ok(());
    }
    match session.sim.reload_scene(&edits.index.to_scene()) {
        Ok(()) => {
            for line in edits.pending.drain(..) {
                session.log("edit", line);
            }
            session.edits = Some(edits);
            Ok(())
        }
        Err(e) => {
            agent.scenes.load_failed = true;
            Err(without_positions(e, None))
        }
    }
}

/// True when session `id` has edits not yet loaded into its world.
pub(crate) fn has_pending(agent: &Agent, id: &str) -> bool {
    agent
        .sessions
        .get(id)
        .is_some_and(|s| s.edits.as_ref().is_some_and(SessionEdits::pending))
}

/// The indexed path for a live session. `Ok(None)`: not taken, nothing changed.
fn indexed_session<T>(
    agent: &mut Agent,
    id: &str,
    edit: impl FnOnce(&mut SceneIndex, &Registry) -> Option<T>,
    line: impl FnOnce(&T) -> String,
) -> Result<Option<(String, T)>, Errors> {
    let deferred = agent.scenes.deferred;
    let registry = agent.engine.registry();
    let Some(session) = agent.sessions.get_mut(id) else {
        return Ok(None);
    };
    let mut edits = match session.edits.take() {
        Some(e) => Some(e),
        None => {
            let scene = session.sim.snapshot();
            SceneIndex::new(&scene, registry).map(|index| SessionEdits {
                index,
                pending: Vec::new(),
            })
        }
    };
    let mut out = None;
    if let Some(e) = &mut edits
        && let Some(t) = edit(&mut e.index, registry)
    {
        let line = line(&t);
        if deferred {
            e.pending.push(line);
            out = Some(t);
        } else if session.sim.reload_scene(&e.index.to_scene()).is_ok() {
            session.log("edit", line);
            out = Some(t);
        } else {
            // Refused by the world (a module's own check): the whole-scene path reports it.
            edits = None;
        }
    }
    session.edits = edits;
    Ok(out.map(|t| (format!("session {id}"), t)))
}

struct CachedScene {
    path: PathBuf,
    shown: String,
    index: SceneIndex,
    /// The file's text as last written or read by this process (stale while `dirty`).
    disk: String,
    /// The index is ahead of the file.
    dirty: bool,
}

/// Most scenes kept indexed at once (memory rule: every cache bounded).
const CACHE_KEEP: usize = 4;

impl SceneCache {
    fn take(&mut self, path: &Path) -> Option<CachedScene> {
        let at = self.entries.iter().position(|e| e.path == path)?;
        Some(self.entries.remove(at))
    }

    /// Writes every scene whose text is pending.
    pub(crate) fn flush(&mut self) -> Result<(), Errors> {
        let mut result = Ok(());
        for e in &mut self.entries {
            if e.dirty {
                e.disk = e.index.to_text();
                e.dirty = false;
                if let Err(err) = write_atomic(&e.path, &e.shown, e.disk.as_bytes()) {
                    result = Err(err);
                }
            }
        }
        if result.is_err() {
            // What is on disk is no longer known.
            self.entries.clear();
        }
        result
    }

    fn keep(&mut self, entry: CachedScene) -> Result<(), Errors> {
        let mut result = Ok(());
        while self.entries.len() >= CACHE_KEEP {
            let old = self.entries.remove(0);
            if old.dirty {
                result = write_atomic(&old.path, &old.shown, old.index.to_text().as_bytes());
            }
        }
        self.entries.push(entry);
        result
    }

    /// Remembers a scene the whole-scene path just wrote (`text` is canonical, `index` is
    /// made from it).
    fn remember(&mut self, path: &Path, shown: &str, text: String, index: Option<SceneIndex>) {
        self.take(path);
        if let Some(index) = index {
            // Evicted entries are never dirty here: this runs outside a batch's edits or
            // right after `flush`.
            let _ = self.keep(CachedScene {
                path: path.to_path_buf(),
                shown: shown.to_string(),
                index,
                disk: text,
                dirty: false,
            });
        }
    }
}

fn change_of(c: &Changed) -> Diff {
    let mut d = Diff::default();
    match (&c.before, &c.after) {
        (None, Some(n)) => d.added.push(n.id),
        (Some(o), None) => d.removed.push(o.id),
        (Some(o), Some(n)) => {
            let ok: BTreeSet<&String> = o.components.keys().collect();
            let nk: BTreeSet<&String> = n.components.keys().collect();
            let change = EntityChange {
                id: n.id,
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
        (None, None) => {}
    }
    d
}

/// Runs `edit` on the indexed copy of a scene file, when there is a current one, and writes
/// the result (or leaves it pending inside a batch). `Ok(None)` means the indexed path does
/// not apply (no entry, the file changed underneath, a session target, or the edit is not
/// plainly valid): nothing changed and the caller runs [`apply`], which reports problems.
fn indexed<T>(
    agent: &mut Agent,
    target: &EditTarget,
    edit: impl FnOnce(&mut SceneIndex, &Registry) -> Option<T>,
    line: impl FnOnce(&T) -> String,
) -> Result<Option<(String, T)>, Errors> {
    if agent.scenes.off {
        return Ok(None);
    }
    if let (None, Some(id)) = (&target.scene, &target.session) {
        return indexed_session(agent, id, edit, line);
    }
    let (Some(file), None) = (&target.scene, &target.session) else {
        return Ok(None);
    };
    let Ok(path) = agent.resolve(file) else {
        return Ok(None);
    };
    let Some(mut entry) = agent.scenes.take(&path) else {
        return Ok(None);
    };
    if !entry.dirty && !std::fs::read(&path).is_ok_and(|bytes| bytes == entry.disk.as_bytes()) {
        return Ok(None); // changed underneath: the entry is dropped
    }
    let Some(out) = edit(&mut entry.index, agent.registry()) else {
        // Unchanged; keep it for the whole-scene path's flush and for later edits.
        agent.scenes.keep(entry)?;
        return Ok(None);
    };
    if agent.scenes.deferred {
        entry.dirty = true;
    } else {
        entry.disk = entry.index.to_text();
        write_atomic(&path, file, entry.disk.as_bytes())?;
    }
    entry.shown = file.clone();
    agent.scenes.keep(entry)?;
    Ok(Some((file.clone(), out)))
}

/// An entity edit on the indexed path, falling back to `change` on the whole scene.
fn entity_edit(
    agent: &mut Agent,
    target: &EditTarget,
    fast: impl FnOnce(&mut SceneIndex, &Registry) -> Option<Changed>,
    change: impl FnOnce(&mut Scene, &str) -> Result<(), Errors>,
) -> Result<(String, Diff, Option<u64>), Errors> {
    if let Some((shown, c)) = indexed(agent, target, fast, |c| format!("{:?}", change_of(c)))? {
        let id = c.after.as_ref().or(c.before.as_ref()).map(|e| e.id);
        return Ok((shown, change_of(&c), id));
    }
    let (shown, diff) = apply(agent, target, change)?;
    Ok((shown, diff, None))
}

/// The entity `id` with `set` added or replaced and `remove` taken away, as the indexed
/// path's form of an update. `None` when the entity or a component to remove is missing.
fn updated(
    index: &SceneIndex,
    id: u64,
    name: Option<&String>,
    set: &Map<String, Value>,
    remove: &[String],
) -> Option<SceneEntity> {
    let mut e = index.entity(id)?.clone();
    if let Some(name) = name {
        e.name = (!name.is_empty()).then(|| name.clone());
    }
    for (k, v) in set {
        e.components.insert(k.clone(), v.clone());
    }
    for k in remove {
        e.components.remove(k)?;
    }
    Some(e)
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
pub(crate) fn apply(
    agent: &mut Agent,
    target: &EditTarget,
    change: impl FnOnce(&mut Scene, &str) -> Result<(), Errors>,
) -> Result<(String, Diff), Errors> {
    match (&target.scene, &target.session) {
        (Some(file), None) => {
            let path = agent.resolve(file)?;
            agent.scenes.flush()?;
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
            let index = SceneIndex::new(&written, agent.registry());
            agent.scenes.remember(&path, file, canonical, index);
            Ok((file.clone(), d))
        }
        (None, Some(id)) => {
            load_pending(agent, id)?;
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
    let fast = |index: &mut SceneIndex, registry: &Registry| {
        index.create(
            &SceneEntity {
                id: input.id.unwrap_or_else(|| index.next_id()),
                name: input.name.clone(),
                components: input.components.clone().into_iter().collect(),
            },
            registry,
        )
    };
    let (target, diff, fast_id) = entity_edit(agent, &input.target, fast, |s, t| {
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
        id: fast_id.unwrap_or(new_id),
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
    let fast = |index: &mut SceneIndex, registry: &Registry| {
        let e = updated(index, id, input.name.as_ref(), &input.set, &input.remove)?;
        index.replace(&e, registry)
    };
    let (target, diff, _) = entity_edit(agent, &input.target, fast, |s, t| {
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
    let fast = |index: &mut SceneIndex, _: &Registry| index.delete(id);
    let (target, diff, _) = entity_edit(agent, &input.target, fast, |s, t| {
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
    let fast = |index: &mut SceneIndex, registry: &Registry| {
        let set = Map::from_iter([(input.component.clone(), input.value.clone())]);
        index.replace(&updated(index, id, None, &set, &[])?, registry)
    };
    let (target, diff, _) = entity_edit(agent, &input.target, fast, |s, t| {
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
    let fast = |index: &mut SceneIndex, registry: &Registry| {
        let gone = [input.component.clone()];
        index.replace(&updated(index, id, None, &Map::new(), &gone)?, registry)
    };
    let (target, diff, _) = entity_edit(agent, &input.target, fast, |s, t| {
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
    let fast = |index: &mut SceneIndex, registry: &Registry| {
        index.set_resource(&input.resource, &input.value, registry)
    };
    if let Some((target, replaced)) =
        indexed(agent, &input.target, fast, |_| format!("{:?}", Diff::default()))?
    {
        return Ok(ResourceEditOutput {
            target,
            resource: input.resource,
            replaced,
        });
    }
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
    let fast = |index: &mut SceneIndex, _: &Registry| index.remove_resource(&input.resource);
    if let Some((target, ())) = indexed(agent, &input.target, fast, |_| format!("{:?}", Diff::default()))? {
        return Ok(ResourceEditOutput {
            target,
            resource: input.resource,
            replaced: true,
        });
    }
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
