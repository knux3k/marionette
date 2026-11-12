//! Owned library operations: import (Unity projects, `.unitypackage`, Unreal projects, plain
//! owned files),
//! list and read records, check pasted license text, and the pool admission gate.
//! Rules: `docs/ai/ASSETS-AND-LICENSES.md`; design: decision 0018.

use crate::{Agent, Op, op};
use marionette_core::Errors;
use marionette_import::library::{AssetKind, AssetRecord, ConfiguredFrom};
use marionette_import::license::{EngineLimit, engine_limits};
use marionette_import::pool::PoolVerdict;
use marionette_import::{ImportOptions, ImportReport, Library};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "library_info",
            "Where the user's owned library is (operation input, MARIONETTE_LIBRARY, or the user config file) and how many assets it holds.",
            false,
            info,
        ),
        op(
            "library_import",
            "Imports owned assets from a .unitypackage, a Unity project folder, an Unreal project/plugin/content folder (exported FBX, glTF, images and T3D materials; .uasset packages are diagnosed, not read) or a plain file/folder into the owned library. Every asset needs a person-written declaration (*.license.json); engine content and undeclared or engine-locked licenses are refused, each listed in the report with its reason.",
            true,
            import,
        ),
        op(
            "library_list",
            "Lists owned library assets (id, kind, name, license, engine readiness), sorted by id, paged.",
            false,
            list,
        ),
        op(
            "library_get",
            "Reads one owned asset's full record (license, source, files, notes).",
            false,
            get,
        ),
        op(
            "license_check_text",
            "Checks license text a person pasted in for phrases that limit use to one engine. Finding none never grants permission; a declaration is still required.",
            false,
            check_text,
        ),
        op(
            "pool_admission_check",
            "The owned-asset gate the CC0 pool must pass: refuses anything inside the owned library, carrying an owned record or declaration, or with the same content as an owned file.",
            false,
            pool_check,
        ),
    ]
}

fn library(agent: &Agent, explicit: Option<&str>) -> Result<Library, Errors> {
    let path = explicit.map(|p| agent.resolve(p)).transpose()?;
    Library::locate(path.as_deref(), agent.root())
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InfoInput {
    /// Absolute path of the owned library; defaults to MARIONETTE_LIBRARY, then the user config file.
    #[serde(default)]
    library: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct InfoOutput {
    path: String,
    configured_from: ConfiguredFrom,
    exists: bool,
    assets: usize,
}

fn info(agent: &mut Agent, input: InfoInput) -> Result<InfoOutput, Errors> {
    let lib = library(agent, input.library.as_deref())?;
    Ok(InfoOutput {
        path: lib.root().to_string_lossy().into_owned(),
        configured_from: lib.configured_from,
        exists: lib.exists(),
        assets: lib.records().len(),
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImportInput {
    /// A .unitypackage file, a Unity project folder, an Unreal project, plugin or content folder, or an owned file or folder.
    source: String,
    /// Declaration files (written by a person) to use besides `<source>.license.json`.
    #[serde(default)]
    declarations: Vec<String>,
    /// Report what would be imported and refused without writing anything.
    #[serde(default)]
    dry_run: bool,
    /// Absolute path of the owned library; defaults to MARIONETTE_LIBRARY, then the user config file.
    #[serde(default)]
    library: Option<String>,
}

fn import(agent: &mut Agent, input: ImportInput) -> Result<ImportReport, Errors> {
    let lib = library(agent, input.library.as_deref())?;
    let mut opts = ImportOptions::new(agent.resolve(&input.source)?);
    opts.dry_run = input.dry_run;
    for d in &input.declarations {
        opts.declarations.push(agent.resolve(d)?);
    }
    marionette_import::import(&lib, &opts)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {
    /// Only this kind: texture, model or material.
    #[serde(default)]
    kind: Option<AssetKind>,
    #[serde(default)]
    offset: usize,
    /// Default 50, at most 500.
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    library: Option<String>,
}

#[derive(Clone, Serialize, JsonSchema)]
struct AssetSummary {
    id: String,
    kind: AssetKind,
    name: String,
    product: String,
    license_type: String,
    engine_ready: bool,
    main: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    total: usize,
    assets: Vec<AssetSummary>,
    /// Offset of the next page, if any.
    next: Option<usize>,
}

fn list(agent: &mut Agent, input: ListInput) -> Result<ListOutput, Errors> {
    let lib = library(agent, input.library.as_deref())?;
    let all: Vec<AssetSummary> = lib
        .records()
        .into_iter()
        .filter(|r| input.kind.is_none_or(|k| r.kind == k))
        .map(|r| AssetSummary {
            id: r.id,
            kind: r.kind,
            name: r.name,
            product: r.license.product,
            license_type: r.license.license_type,
            engine_ready: r.engine_ready,
            main: r.main,
        })
        .collect();
    let (assets, next) = crate::page(&all, input.offset, input.limit);
    Ok(ListOutput {
        total: all.len(),
        assets,
        next,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetInput {
    id: String,
    #[serde(default)]
    library: Option<String>,
}

fn get(agent: &mut Agent, input: GetInput) -> Result<AssetRecord, Errors> {
    library(agent, input.library.as_deref())?.get(&input.id)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TextInput {
    /// License text a person pasted in.
    text: String,
}

#[derive(Serialize, JsonSchema)]
struct TextOutput {
    /// True when the text limits use to one engine: assets under it must be refused.
    limits_to_one_engine: bool,
    matches: Vec<EngineLimit>,
}

fn check_text(_: &mut Agent, input: TextInput) -> Result<TextOutput, Errors> {
    let matches = engine_limits(&input.text);
    Ok(TextOutput {
        limits_to_one_engine: !matches.is_empty(),
        matches,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PoolInput {
    /// The pool candidate: a file or folder.
    path: String,
    /// Owned library to compare contents with; defaults to the configured one, if any.
    #[serde(default)]
    library: Option<String>,
}

fn pool_check(agent: &mut Agent, input: PoolInput) -> Result<PoolVerdict, Errors> {
    let path = agent.resolve(&input.path)?;
    // Without a configured library the structural checks still run.
    let lib = match library(agent, input.library.as_deref()) {
        Ok(l) => Some(l),
        Err(e)
            if input.library.is_none()
                && e.0
                    .iter()
                    .all(|d| d.code == marionette_import::codes::LIBRARY_NOT_CONFIGURED) =>
        {
            None
        }
        Err(e) => return Err(e),
    };
    Ok(marionette_import::pool::check(&path, lib.as_ref()))
}
