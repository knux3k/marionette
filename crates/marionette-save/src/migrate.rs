//! Taking an old save up to the current version, one version at a time.
//!
//! Step `from: n` turns a version `n` save into version `n + 1`. A version with no step
//! changes nothing. Data steps ([`Change`]) come from the game's migrations file and need no
//! Rust; a game with its own binary adds Rust steps with [`Migrations::rust`]. In one
//! version, data steps run before Rust steps.
//!
//! Migrations file (path in `SaveFormat.migrations`, relative to the scene folder):
//!
//! ```json
//! { "steps": [
//!   { "from": 1, "changes": [ { "op": "rename_type", "from": "Gold", "to": "Wallet" } ] },
//!   { "from": 2, "changes": [ { "op": "set_field", "on": "Wallet", "field": "gems", "value": 0 } ] }
//! ] }
//! ```

use crate::codes;
use marionette_core::scene::Scene;
use marionette_core::{Diagnostic, Errors};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// One data change. `on`/`name` is a registered type name (component or resource);
/// `field` may be a dotted path into nested objects (`stats.hp`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    /// Rename a component or resource type everywhere.
    RenameType { from: String, to: String },
    /// Remove a component or resource type everywhere.
    RemoveType { name: String },
    /// Rename a field of every value of a type.
    RenameField { on: String, from: String, to: String },
    /// Set a field of every value of a type. Without `overwrite`, only where it is missing
    /// (the usual case: a new field with a default).
    SetField {
        on: String,
        field: String,
        value: Value,
        #[serde(default)]
        overwrite: bool,
    },
    /// Remove a field of every value of a type.
    RemoveField { on: String, field: String },
}

/// A Rust migration step.
pub type RustStep = Arc<dyn Fn(&mut Scene) -> Result<(), String> + Send + Sync>;

#[derive(Clone)]
enum Step {
    Data(Vec<Change>),
    Rust(RustStep),
}

/// Migration steps keyed by the version they start from.
#[derive(Clone, Default)]
pub struct Migrations {
    steps: BTreeMap<u32, Vec<Step>>,
}

impl Migrations {
    pub fn new() -> Self {
        Self::default()
    }

    /// Data changes from version `from` to `from + 1`.
    pub fn data(mut self, from: u32, changes: Vec<Change>) -> Self {
        self.steps.entry(from).or_default().push(Step::Data(changes));
        self
    }

    /// A Rust step from version `from` to `from + 1`.
    pub fn rust(
        mut self,
        from: u32,
        step: impl Fn(&mut Scene) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        self.steps
            .entry(from)
            .or_default()
            .push(Step::Rust(Arc::new(step)));
        self
    }

    /// These steps, then `other`'s, per version.
    pub fn merged(mut self, other: Migrations) -> Self {
        for (v, steps) in other.steps {
            self.steps.entry(v).or_default().extend(steps);
        }
        self
    }

    /// Versions that have at least one step.
    pub fn versions(&self) -> Vec<u32> {
        self.steps.keys().copied().collect()
    }

    /// Runs every step from version `from` up to `to`.
    pub fn apply(&self, scene: &mut Scene, from: u32, to: u32) -> Result<(), Errors> {
        for v in from..to {
            for step in self.steps.get(&v).into_iter().flatten() {
                let result = match step {
                    Step::Data(changes) => changes.iter().try_for_each(|c| apply_change(scene, c)),
                    Step::Rust(f) => f(scene),
                };
                result.map_err(|e| {
                    Errors::from(
                        Diagnostic::error(
                            codes::MIGRATION,
                            format!("migration from version {v} to {} failed: {e}", v + 1),
                        )
                        .fix("fix the migration step; the save file is unchanged"),
                    )
                })?;
            }
        }
        Ok(())
    }
}

/// Every value of type `name` in the scene: resource and components.
fn values_of<'a>(scene: &'a mut Scene, name: &'a str) -> impl Iterator<Item = &'a mut Value> + 'a {
    scene.resources.get_mut(name).into_iter().chain(
        scene
            .entities
            .iter_mut()
            .filter_map(move |e| e.components.get_mut(name)),
    )
}

fn rename_key(map: &mut BTreeMap<String, Value>, from: &str, to: &str) -> Result<(), String> {
    if let Some(v) = map.remove(from) {
        if map.contains_key(to) {
            return Err(format!("`{to}` already exists next to `{from}`"));
        }
        map.insert(to.to_string(), v);
    }
    Ok(())
}

/// The object holding the last segment of `path` (created on the way when `create`).
fn parent<'a>(
    value: &'a mut Value,
    path: &'a str,
    create: bool,
) -> Option<(&'a mut serde_json::Map<String, Value>, &'a str)> {
    let mut parts: Vec<&str> = path.split('.').collect();
    let last = parts.pop()?;
    let mut at = value;
    for p in parts {
        let obj = at.as_object_mut()?;
        if create && !obj.contains_key(p) {
            obj.insert(p.to_string(), Value::Object(Default::default()));
        }
        at = obj.get_mut(p)?;
    }
    Some((at.as_object_mut()?, last))
}

fn apply_change(scene: &mut Scene, change: &Change) -> Result<(), String> {
    match change {
        Change::RenameType { from, to } => {
            rename_key(&mut scene.resources, from, to)?;
            for e in &mut scene.entities {
                rename_key(&mut e.components, from, to).map_err(|m| format!("entity {}: {m}", e.id))?;
            }
        }
        Change::RemoveType { name } => {
            scene.resources.remove(name);
            for e in &mut scene.entities {
                e.components.remove(name);
            }
        }
        Change::RenameField { on, from, to } => {
            for v in values_of(scene, on) {
                if let Some((obj, key)) = parent(v, from, false)
                    && let Some(x) = obj.remove(key)
                {
                    let (dest, key) =
                        parent(v, to, true).ok_or_else(|| format!("`{on}.{to}` is not inside an object"))?;
                    dest.insert(key.to_string(), x);
                }
            }
        }
        Change::SetField {
            on,
            field,
            value,
            overwrite,
        } => {
            for v in values_of(scene, on) {
                let (obj, key) = parent(v, field, true)
                    .ok_or_else(|| format!("`{on}.{field}` is not inside an object"))?;
                if *overwrite || !obj.contains_key(key) {
                    // One step can set the field on many objects, each needs its own value.
                    obj.insert(key.to_string(), value.clone());
                }
            }
        }
        Change::RemoveField { on, field } => {
            for v in values_of(scene, on) {
                if let Some((obj, key)) = parent(v, field, false) {
                    obj.remove(key);
                }
            }
        }
    }
    Ok(())
}

/// The migrations file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationsFile {
    pub steps: Vec<FileStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileStep {
    pub from: u32,
    pub changes: Vec<Change>,
}

impl MigrationsFile {
    /// Reads and checks a migrations file (`shown` names it in errors).
    pub fn read(path: &Path, shown: &str) -> Result<Self, Errors> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            Errors::from(
                Diagnostic::error(codes::IO, format!("cannot read migrations file {shown}: {e}"))
                    .file(shown)
                    .fix("fix SaveFormat.migrations (relative to the scene folder) or leave it empty"),
            )
        })?;
        serde_json::from_str(&text).map_err(|e| {
            Errors::from(
                Diagnostic::error(codes::MIGRATION, format!("migrations file is not valid: {e}"))
                    .file(shown)
                    .at(Some(e.line()), Some(e.column())),
            )
        })
    }

    pub fn into_migrations(self) -> Migrations {
        self.steps
            .into_iter()
            .fold(Migrations::new(), |m, s| m.data(s.from, s.changes))
    }
}
