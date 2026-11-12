//! Prefab operations: place an imported prefab (owned library, kind `prefab`, decision 0040)
//! into a scene file. The library files a prefab refers to are copied beside the scene, so the
//! scene stays self-contained and portable.

use crate::edit::{Diff, EditTarget, apply};
use crate::library::library;
use crate::{Agent, Op, codes, op, write_atomic};
use marionette_core::scene::SceneEntity;
use marionette_core::{Diagnostic, Errors};
use marionette_import::library::AssetKind;
use marionette_import::prefab::{PREFAB_FORMAT, Trs};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::path::{Component, Path};

pub(crate) fn ops() -> Vec<Op> {
    vec![op(
        "prefab_instantiate",
        "Places an imported prefab (an owned-library asset of kind prefab, from a Unity .prefab) into a scene file: one entity per prefab object at `at`, with fresh ids. Library models and materials it uses are copied beside the scene (default folder `library/`). Inactive objects are left out unless `include_inactive` is set.",
        true,
        instantiate,
    )]
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InstantiateInput {
    /// Library id of the prefab (see `library_list` with kind `prefab`).
    prefab: String,
    /// Scene file to add the entities to.
    scene: String,
    /// Where the prefab root goes: a `Transform` value (translation, rotation, scale). Default:
    /// the origin.
    #[serde(default)]
    at: Option<Value>,
    /// Name for the root entity; child names keep their paths under it.
    #[serde(default)]
    name: Option<String>,
    /// Also place objects that were inactive in Unity.
    #[serde(default)]
    include_inactive: bool,
    /// Folder, relative to the scene file, that receives copied library files. Default `library`.
    #[serde(default)]
    assets_dir: Option<String>,
    /// Absolute path of the owned library; defaults to MARIONETTE_LIBRARY, then the user config file.
    #[serde(default)]
    library: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct PlacedEntity {
    id: u64,
    name: String,
    /// The entity's key inside prefab.json.
    key: u64,
}

#[derive(Serialize, JsonSchema)]
struct InstantiateOutput {
    scene: String,
    entities: Vec<PlacedEntity>,
    /// Files copied beside the scene (relative to the scene file); files already there with the
    /// same content are not listed.
    copied: Vec<String>,
    /// Objects left out because they were inactive in Unity.
    inactive_left_out: usize,
    diff: Diff,
    /// The prefab's own notes (what did not map, and why).
    notes: Vec<String>,
}

fn bad(msg: String, fix: &str) -> Errors {
    Diagnostic::error(codes::BAD_INPUT, msg).fix(fix).into()
}

fn safe_dir(dir: &str) -> bool {
    let p = Path::new(dir);
    !dir.is_empty()
        && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// Rewrites every `library:<id>/<file>` string to `<dir>/<id>/<file>` and collects the ids.
fn rewrite(v: &mut Value, dir: &str, ids: &mut BTreeSet<String>) -> Result<(), Errors> {
    match v {
        Value::String(s) => {
            if let Some(rest) = s.strip_prefix("library:") {
                let (id, file) = rest.split_once('/').unwrap_or((rest, ""));
                if id.is_empty() || !safe_dir(id) || !safe_dir(file) {
                    return Err(bad(
                        format!("prefab refers to `{s}`, which is not a library file"),
                        "re-import the prefab",
                    ));
                }
                ids.insert(id.to_string());
                *s = format!("{dir}/{id}/{file}");
            }
        }
        Value::Array(a) => {
            for x in a {
                rewrite(x, dir, ids)?;
            }
        }
        Value::Object(o) => {
            for x in o.values_mut() {
                rewrite(x, dir, ids)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn instantiate(agent: &mut Agent, input: InstantiateInput) -> Result<InstantiateOutput, Errors> {
    let lib = library(agent, input.library.as_deref())?;
    let rec = lib.get(&input.prefab)?;
    if rec.kind != AssetKind::Prefab {
        return Err(bad(
            format!("`{}` is a {:?}, not a prefab", rec.id, rec.kind),
            "pick an asset of kind prefab (library_list with kind `prefab`)",
        ));
    }
    let main = rec.main.clone().unwrap_or_else(|| "prefab.json".into());
    let text = crate::read_text(&lib.asset_dir(&rec.id).join(&main), &format!("{}/{main}", rec.id))?;
    let doc: Value = serde_json::from_str(&text)
        .map_err(|e| bad(format!("prefab `{}` is unreadable: {e}", rec.id), "re-import the prefab"))?;
    if doc["format"] != PREFAB_FORMAT {
        return Err(bad(
            format!("prefab `{}` is not in format {PREFAB_FORMAT}", rec.id),
            "re-import the prefab",
        ));
    }
    let dir = input.assets_dir.clone().unwrap_or_else(|| "library".into());
    if !safe_dir(&dir) {
        return Err(bad(
            format!("assets_dir `{dir}` must be a relative folder below the scene"),
            "use a plain relative folder such as `library` or `assets/owned`",
        ));
    }
    let at = input.at.as_ref().map(Trs::from_json).unwrap_or(Trs::IDENTITY);

    // Entities to place, with library references rewritten.
    let mut ids = BTreeSet::new();
    let mut placed: Vec<(u64, String, Map<String, Value>)> = Vec::new();
    let mut inactive = 0;
    for e in doc["entities"].as_array().into_iter().flatten() {
        if e["active"] == json!(false) && !input.include_inactive {
            inactive += 1;
            continue;
        }
        let key = e["key"].as_u64().unwrap_or(0);
        let mut name = e["name"].as_str().unwrap_or("").to_string();
        if let Some(n) = &input.name {
            name = match name.split_once('/') {
                Some((_, rest)) => format!("{n}/{rest}"),
                None => n.clone(),
            };
        }
        let mut comps = e["components"].as_object().cloned().unwrap_or_default();
        for v in comps.values_mut() {
            rewrite(v, &dir, &mut ids)?;
        }
        let local = comps.get("Transform").map(Trs::from_json).unwrap_or(Trs::IDENTITY);
        let (world, _) = at.then(&local);
        comps.insert("Transform".into(), world.to_json());
        placed.push((key, name, comps));
    }

    // Copy the library files the entities use (never the originals kept as `source.*`).
    let scene_path = agent.resolve(&input.scene)?;
    let scene_dir = scene_path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut copied = Vec::new();
    for id in &ids {
        let r = lib.get(id)?;
        for f in &r.files {
            if f.path.starts_with("source.") || !safe_dir(&f.path) {
                continue;
            }
            let rel = format!("{dir}/{id}/{}", f.path);
            let dest = scene_dir.join(&rel);
            let bytes = std::fs::read(lib.asset_dir(id).join(&f.path)).map_err(|e| {
                Errors::from(Diagnostic::error(
                    marionette_core::error::codes::IO_READ,
                    format!("cannot read library file {id}/{}: {e}", f.path),
                ))
            })?;
            if std::fs::read(&dest).is_ok_and(|b| b == bytes) {
                continue;
            }
            write_atomic(&dest, &rel, &bytes)?;
            copied.push(rel);
        }
    }

    // Add the entities in one validated edit.
    let target = EditTarget {
        scene: Some(input.scene.clone()),
        session: None,
    };
    let mut out_entities = Vec::new();
    let (scene, diff) = apply(agent, &target, |s, _| {
        let mut next = s.entities.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        for (key, name, comps) in &placed {
            s.entities.push(SceneEntity {
                id: next,
                name: Some(name.clone()),
                components: comps.clone().into_iter().collect(),
            });
            out_entities.push(PlacedEntity {
                id: next,
                name: name.clone(),
                key: *key,
            });
            next += 1;
        }
        Ok(())
    })?;
    let notes = doc["notes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| n.as_str().map(str::to_string))
        .collect();
    Ok(InstantiateOutput {
        scene,
        entities: out_entities,
        copied,
        inactive_left_out: inactive,
        diff,
        notes,
    })
}
