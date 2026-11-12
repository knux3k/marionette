//! CC0 pool operations and the asset lookup order (workstream W3, decision 0041). Rules:
//! `docs/ai/ASSETS-AND-LICENSES.md`.

use crate::library::library;
use crate::{Agent, Op, op, write_atomic};
use marionette_core::{Diagnostic, Errors};
use marionette_import::Library;
use marionette_pool::admit::{AddReport, Candidate, add, read_folder};
use marionette_pool::net::{Curl, Transport};
use marionette_pool::search::{SearchRequest, SearchResult, search};
use marionette_pool::sources::archive::{ArchiveFrom, ArchiveRequest};
use marionette_pool::sources::{self, LIBRARIES};
use marionette_pool::{Origin, Pool, PoolAsset, PoolFrom, PoolKind, Provenance, codes};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "asset_search",
            "Finds an asset in the required order: (1) project files, (2) the owned library, (3) the CC0 pool, (4) online CC0 libraries (Poly Haven, ambientCG; only with `online: true`), (5) generation (a pointer to generate_plan). Stops at the first step with results unless `all_steps` is set. Read-only.",
            false,
            asset_search,
        ),
        op(
            "pool_info",
            "Where the CC0 pool is (operation input, MARIONETTE_POOL, or cc0_pool in the user config file), how many assets it holds, and the CC0 libraries it knows with their license pages.",
            false,
            info,
        ),
        op(
            "pool_list",
            "Lists pooled CC0 assets (id, kind, name, tags, origin), sorted by id, paged.",
            false,
            list,
        ),
        op(
            "pool_get",
            "Reads one pooled asset's record: files with SHA-256, source, and the proof that it is CC0.",
            false,
            get,
        ),
        op(
            "pool_add",
            "Adds an asset made for the project (origin `made`, with `method`) or a library archive a person downloaded (origin `online`, with `library` and `page`) to the CC0 pool. Every rule runs: owned-asset gate, generated-asset gate, license files must say CC0, proof of origin, files must load. `dry_run` checks without writing.",
            true,
            pool_add,
        ),
        op(
            "pool_ingest",
            "Downloads an asset from an online CC0 library and adds it to the pool with its source and proof: Poly Haven or ambientCG by `source_id` (from asset_search), Kenney or Quaternius by archive `url` on the library's site. Downloads are checked (size, and MD5 where published). Needs network (the system curl).",
            true,
            pool_ingest,
        ),
        op(
            "pool_fetch",
            "Copies a pooled CC0 asset into the project (default folder `assets/pool/<id>/`) and returns the path of its main file.",
            true,
            fetch,
        ),
        op(
            "pool_index",
            "Writes the pool's index.json (one entry per asset, sorted) for publishing the assets repository.",
            true,
            index,
        ),
    ]
}

fn bad(msg: String, fix: &str) -> Errors {
    Diagnostic::error(codes::BAD_INPUT, msg).fix(fix).into()
}

fn pool(agent: &Agent, explicit: Option<&str>) -> Result<Pool, Errors> {
    match explicit {
        Some(p) => Pool::at(&agent.resolve(p)?, PoolFrom::Input),
        None => Pool::locate(None),
    }
}

/// The owned library if one is configured; any other error is returned.
fn optional_library(agent: &Agent, explicit: Option<&str>) -> Result<Option<Library>, Errors> {
    match library(agent, explicit) {
        Ok(l) => Ok(Some(l)),
        Err(e)
            if explicit.is_none()
                && e.0
                    .iter()
                    .all(|d| d.code == marionette_import::codes::LIBRARY_NOT_CONFIGURED) =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

fn transport() -> Box<dyn Transport> {
    Box::new(Curl)
}

// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchInput {
    /// Words to look for (all must match the name, path, tags or id).
    query: String,
    /// Only this kind: texture, model, material, sound or other.
    #[serde(default)]
    kind: Option<PoolKind>,
    /// Also search online CC0 libraries (step 4). Default false.
    #[serde(default)]
    online: bool,
    /// Search every step instead of stopping at the first with results.
    #[serde(default)]
    all_steps: bool,
    /// Results per step; default 20, at most 100.
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    library: Option<String>,
    #[serde(default)]
    pool: Option<String>,
}

fn asset_search(agent: &mut Agent, input: SearchInput) -> Result<SearchResult, Errors> {
    if input.query.trim().is_empty() {
        return Err(bad(
            "an empty query matches everything".into(),
            "give one or more words",
        ));
    }
    let (lib, lib_note) = match optional_library(agent, input.library.as_deref()) {
        Ok(Some(l)) => (Some(l), None),
        Ok(None) => (
            None,
            Some("no owned library configured (MARIONETTE_LIBRARY)".to_string()),
        ),
        Err(e) => return Err(e),
    };
    let (pl, pool_note) = match pool(agent, input.pool.as_deref()) {
        Ok(p) => (Some(p), None),
        Err(e) if input.pool.is_none() => (
            None,
            Some(e.0.first().map(|d| d.message.clone()).unwrap_or_default()),
        ),
        Err(e) => return Err(e),
    };
    let t = transport();
    Ok(search(&SearchRequest {
        query: &input.query,
        kind: input.kind,
        project_root: agent.root(),
        library: lib.as_ref(),
        library_note: lib_note,
        pool: pl.as_ref(),
        pool_note,
        online: input.online,
        transport: t.as_ref(),
        limit: input.limit.unwrap_or(20).clamp(1, 100),
        stop_at_first: !input.all_steps,
    }))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PoolOnly {
    /// The pool folder (a checkout of the assets repository); defaults to MARIONETTE_POOL, then
    /// cc0_pool in the user config file.
    #[serde(default)]
    pool: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct KnownLibrary {
    id: String,
    name: String,
    license_url: String,
    license_quote: String,
    searchable: bool,
    browse: String,
}

#[derive(Serialize, JsonSchema)]
struct InfoOutput {
    path: Option<String>,
    configured_from: Option<PoolFrom>,
    exists: bool,
    assets: usize,
    /// Why no pool is available, if none is.
    #[serde(skip_serializing_if = "Option::is_none")]
    problem: Option<String>,
    libraries: Vec<KnownLibrary>,
}

fn info(agent: &mut Agent, input: PoolOnly) -> Result<InfoOutput, Errors> {
    let libraries = LIBRARIES
        .iter()
        .map(|l| KnownLibrary {
            id: l.id.into(),
            name: l.name.into(),
            license_url: l.license_url.into(),
            license_quote: l.license_quote.into(),
            searchable: l.searchable,
            browse: l.browse.into(),
        })
        .collect();
    Ok(match pool(agent, input.pool.as_deref()) {
        Ok(p) => InfoOutput {
            path: Some(p.root().to_string_lossy().into_owned()),
            configured_from: Some(p.configured_from),
            exists: p.exists(),
            assets: p.records().len(),
            problem: None,
            libraries,
        },
        Err(e) => InfoOutput {
            path: None,
            configured_from: None,
            exists: false,
            assets: 0,
            problem: e.0.first().map(|d| d.message.clone()),
            libraries,
        },
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {
    #[serde(default)]
    kind: Option<PoolKind>,
    #[serde(default)]
    offset: usize,
    /// Default 50, at most 500.
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    pool: Option<String>,
}

#[derive(Clone, Serialize, JsonSchema)]
struct Summary {
    id: String,
    kind: PoolKind,
    name: String,
    tags: Vec<String>,
    origin: Origin,
    library: Option<String>,
    main: Option<String>,
    engine_ready: bool,
}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    total: usize,
    assets: Vec<Summary>,
    next: Option<usize>,
}

fn list(agent: &mut Agent, input: ListInput) -> Result<ListOutput, Errors> {
    let p = pool(agent, input.pool.as_deref())?;
    let all: Vec<Summary> = p
        .records()
        .into_iter()
        .filter(|r| input.kind.is_none_or(|k| r.kind == k))
        .map(|r| Summary {
            id: r.id,
            kind: r.kind,
            name: r.name,
            tags: r.tags,
            origin: r.source.origin,
            library: r.source.library,
            main: r.main,
            engine_ready: r.engine_ready,
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
    pool: Option<String>,
}

fn get(agent: &mut Agent, input: GetInput) -> Result<PoolAsset, Errors> {
    pool(agent, input.pool.as_deref())?.get(&input.id)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddInput {
    /// A file or folder holding the asset's files.
    path: String,
    kind: PoolKind,
    name: String,
    #[serde(default)]
    tags: Vec<String>,
    /// `made` (made for the project) or `online` (an archive or files from a known CC0 library).
    origin: Origin,
    /// Made: how it was made.
    #[serde(default)]
    method: Option<String>,
    /// Made: the program in the engine repository that makes it (repo-relative path).
    #[serde(default)]
    generator: Option<String>,
    /// Online: library id (polyhaven, ambientcg, kenney, quaternius).
    #[serde(default)]
    library: Option<String>,
    /// Online: the asset's page on the library's site.
    #[serde(default)]
    page: Option<String>,
    /// Online: creator as credited by the library.
    #[serde(default)]
    author: Option<String>,
    /// For a .zip `path`: path prefixes inside the archive to keep (license files are always kept).
    #[serde(default)]
    include: Vec<String>,
    /// The file the engine loads; default by kind (first PNG, GLB/glTF, WAV/OGG).
    #[serde(default)]
    main: Option<String>,
    /// Asset id; default `<name slug>-<content hash>`.
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default)]
    pool: Option<String>,
    /// Owned library to compare contents with; defaults to the configured one, if any.
    #[serde(default)]
    owned_library: Option<String>,
    /// Extra folders of generated assets to compare contents with; the project's
    /// `assets/generated/` is always checked (decision 0045).
    #[serde(default)]
    generated_dirs: Vec<String>,
}

fn pool_add(agent: &mut Agent, input: AddInput) -> Result<AddReport, Errors> {
    let p = pool(agent, input.pool.as_deref())?;
    let lib = optional_library(agent, input.owned_library.as_deref())?;
    let path = agent.resolve(&input.path)?;
    let is_zip = path.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    let mut cand = if is_zip {
        let bytes = std::fs::read(&path)
            .map_err(|e| bad(format!("cannot read the archive: {e}"), "check the path"))?;
        let library = input.library.as_deref().unwrap_or("");
        let mut c = sources::archive::ingest(
            &marionette_pool::net::NoNetwork,
            ArchiveRequest {
                library,
                page: input.page.as_deref().unwrap_or(""),
                from: ArchiveFrom::Bytes(bytes),
                kind: input.kind,
                name: &input.name,
                tags: input.tags.clone(),
                include: input.include.clone(),
                main: input.main.clone(),
            },
        )
        .map_err(|e| {
            Errors::from(Diagnostic::error(codes::BAD_INPUT, e).fix("check the archive and `include`"))
        })?;
        c.source.origin = input.origin;
        if input.origin == Origin::Made {
            c.source.library = None;
            c.source.page = None;
            c.source.author = None;
        }
        c
    } else {
        Candidate {
            kind: input.kind,
            name: input.name.clone(),
            id: None,
            tags: input.tags.clone(),
            source: Provenance {
                origin: input.origin,
                library: input.library.clone(),
                page: input.page.clone(),
                source_id: None,
                author: None,
                retrieved: None,
                downloads: Vec::new(),
                method: None,
                generator: None,
            },
            proof: Vec::new(),
            files: read_folder(&path)?,
            main: input.main.clone(),
            notes: Vec::new(),
            origin_path: None,
            generated_dirs: Vec::new(),
        }
    };
    cand.id = input.id.clone();
    cand.origin_path = Some(path);
    cand.generated_dirs = vec![agent.root().join(marionette_generate::recipe::DEFAULT_DIR)];
    for d in &input.generated_dirs {
        cand.generated_dirs.push(agent.resolve(d)?);
    }
    cand.source.method = input.method.clone();
    cand.source.generator = input.generator.clone();
    if input.author.is_some() {
        cand.source.author = input.author.clone();
    }
    if input.origin == Origin::Made {
        cand.source.author = Some("Marionette Contributors".into());
    }
    add(&p, cand, lib.as_ref(), input.dry_run)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct IngestInput {
    /// polyhaven, ambientcg, kenney or quaternius.
    library: String,
    /// Poly Haven or ambientCG asset id (from asset_search).
    #[serde(default)]
    source_id: Option<String>,
    /// Kenney or Quaternius: the archive URL on the library's site.
    #[serde(default)]
    url: Option<String>,
    /// Kenney or Quaternius: the pack's page.
    #[serde(default)]
    page: Option<String>,
    /// Kenney or Quaternius: what the asset is, its name and tags.
    #[serde(default)]
    kind: Option<PoolKind>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    /// Kenney or Quaternius: path prefixes inside the archive to keep.
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    main: Option<String>,
    /// Texture or model resolution: Poly Haven `1k`/`2k`/`4k`/`8k`, ambientCG `1K`/`2K`/...
    /// Default 1k.
    #[serde(default)]
    resolution: Option<String>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default)]
    pool: Option<String>,
    #[serde(default)]
    owned_library: Option<String>,
}

fn pool_ingest(agent: &mut Agent, input: IngestInput) -> Result<AddReport, Errors> {
    let p = pool(agent, input.pool.as_deref())?;
    let lib = optional_library(agent, input.owned_library.as_deref())?;
    let t = transport();
    ingest_with(&p, lib.as_ref(), t.as_ref(), &input)
}

fn ingest_with(
    p: &Pool,
    lib: Option<&Library>,
    t: &dyn Transport,
    input: &IngestInput,
) -> Result<AddReport, Errors> {
    let net = |e: String| -> Errors {
        let code = if e.contains("HTTP") || e.contains("curl") || e.contains("network") {
            codes::NETWORK
        } else {
            codes::SOURCE_FORMAT
        };
        Diagnostic::error(code, e)
            .fix("check the id, the network, or download the archive and use pool_add")
            .into()
    };
    let res = input.resolution.clone().unwrap_or_else(|| "1k".into());
    let cand = match input.library.as_str() {
        "polyhaven" | "ambientcg" => {
            let id = input.source_id.as_deref().ok_or_else(|| {
                bad(
                    "`source_id` is required for this library".into(),
                    "take it from asset_search",
                )
            })?;
            if input.library == "polyhaven" {
                sources::polyhaven::ingest(t, id, &res.to_ascii_lowercase()).map_err(net)?
            } else {
                sources::ambientcg::ingest(t, id, &res.to_ascii_uppercase()).map_err(net)?
            }
        }
        "kenney" | "quaternius" => {
            let url = input.url.as_deref().ok_or_else(|| {
                bad(
                    "`url` of the archive is required for this library".into(),
                    "copy the download link from the pack's page",
                )
            })?;
            sources::archive::ingest(
                t,
                ArchiveRequest {
                    library: &input.library,
                    page: input.page.as_deref().unwrap_or(""),
                    from: ArchiveFrom::Url(url),
                    kind: input.kind.unwrap_or(PoolKind::Model),
                    name: input.name.as_deref().unwrap_or("pack"),
                    tags: input.tags.clone(),
                    include: input.include.clone(),
                    main: input.main.clone(),
                },
            )
            .map_err(net)?
        }
        other => {
            return Err(Diagnostic::error(
                codes::UNKNOWN_SOURCE,
                format!("`{other}` is not a CC0 library the pool knows"),
            )
            .fix("use polyhaven, ambientcg, kenney or quaternius")
            .into());
        }
    };
    add(p, cand, lib, input.dry_run)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FetchInput {
    id: String,
    /// Project folder to copy into; default `assets/pool`. The asset lands in `<dir>/<id>/`.
    #[serde(default)]
    dir: Option<String>,
    #[serde(default)]
    pool: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct FetchOutput {
    id: String,
    /// Project-relative path of the main file, if any.
    main: Option<String>,
    /// Files written (project-relative); files already there with the same content are skipped.
    copied: Vec<String>,
    license: String,
}

fn fetch(agent: &mut Agent, input: FetchInput) -> Result<FetchOutput, Errors> {
    let p = pool(agent, input.pool.as_deref())?;
    let rec = p.get(&input.id)?;
    let dir = input.dir.clone().unwrap_or_else(|| "assets/pool".into());
    if dir.is_empty()
        || !Path::new(&dir)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(bad(
            format!("`{dir}` must be a relative folder in the project"),
            "use e.g. assets/pool",
        ));
    }
    let mut copied = Vec::new();
    for f in &rec.files {
        let rel = format!("{dir}/{}/{}", rec.id, f.path);
        let dest = agent.resolve(&rel)?;
        let bytes = std::fs::read(p.asset_dir(&rec.id).join(&f.path)).map_err(|e| {
            Errors::from(Diagnostic::error(
                marionette_core::error::codes::IO_READ,
                format!("cannot read pool file {}/{}: {e}", rec.id, f.path),
            ))
        })?;
        if marionette_pool::sha256_hex(&bytes) != f.sha256 {
            return Err(Diagnostic::error(
                codes::BAD_FILE,
                format!(
                    "pool file {}/{} does not match its recorded SHA-256",
                    rec.id, f.path
                ),
            )
            .fix("restore the pool from the assets repository")
            .into());
        }
        if std::fs::read(&dest).is_ok_and(|b| b == bytes) {
            continue;
        }
        write_atomic(&dest, &rel, &bytes)?;
        copied.push(rel);
    }
    Ok(FetchOutput {
        main: rec.main.as_ref().map(|m| format!("{dir}/{}/{m}", rec.id)),
        id: rec.id,
        copied,
        license: "CC0-1.0".into(),
    })
}

#[derive(Serialize, JsonSchema)]
struct IndexOutput {
    path: String,
    assets: usize,
}

fn index(agent: &mut Agent, input: PoolOnly) -> Result<IndexOutput, Errors> {
    let p = pool(agent, input.pool.as_deref())?;
    let idx = p.index();
    let n = idx["assets"].as_array().map_or(0, Vec::len);
    let text = serde_json::to_string_pretty(&idx).expect("json") + "\n";
    marionette_pool::write_file(&p.root().join("index.json"), text.as_bytes())?;
    Ok(IndexOutput {
        path: p.root().join("index.json").to_string_lossy().into_owned(),
        assets: n,
    })
}
