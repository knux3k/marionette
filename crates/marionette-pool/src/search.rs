//! The asset lookup order (ASSETS-AND-LICENSES.md rule 1): (1) the project's own files, (2) the
//! user's owned library, (3) the CC0 pool, (4) online CC0 libraries, (5) generation. By default
//! the search stops at the first step with results. Step 4 reaches the network only when asked
//! (`online`); step 5 is a pointer to the generation operations, never a call.

use crate::net::Transport;
use crate::sources::{self, OnlineHit, matches, terms};
use crate::{Pool, PoolKind};
use marionette_import::Library;
use marionette_import::library::AssetKind;
use schemars::JsonSchema;
use serde::Serialize;
use std::path::Path;

/// One result.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Hit {
    /// Project-relative path, library id, pool id, or online source id.
    pub id: String,
    pub name: String,
    pub kind: Option<PoolKind>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Online: the page. Library: the license type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// How to use it (the operation or path).
    pub use_with: String,
}

/// One step of the order.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Step {
    pub step: u8,
    /// `project_files`, `owned_library`, `cc0_pool`, `online_cc0` or `generate`.
    pub name: String,
    pub searched: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<Hit>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchResult {
    pub query: String,
    pub steps: Vec<Step>,
    /// The first step with results, if any.
    pub found_in: Option<String>,
}

pub struct SearchRequest<'a> {
    pub query: &'a str,
    pub kind: Option<PoolKind>,
    pub project_root: &'a Path,
    pub library: Option<&'a Library>,
    pub library_note: Option<String>,
    pub pool: Option<&'a Pool>,
    pub pool_note: Option<String>,
    pub online: bool,
    pub transport: &'a dyn Transport,
    pub limit: usize,
    pub stop_at_first: bool,
}

fn kind_of_ext(path: &str) -> Option<PoolKind> {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "png" | "jpg" | "jpeg" => PoolKind::Texture,
        "glb" | "gltf" => PoolKind::Model,
        "wav" | "ogg" => PoolKind::Sound,
        _ => return None,
    })
}

fn library_kind(k: AssetKind) -> PoolKind {
    match k {
        AssetKind::Texture => PoolKind::Texture,
        AssetKind::Model => PoolKind::Model,
        AssetKind::Material => PoolKind::Material,
        AssetKind::Prefab => PoolKind::Other,
    }
}

const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".marionette"];

fn project_files(root: &Path, words: &[String], kind: Option<PoolKind>, limit: usize) -> Vec<Hit> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    let mut seen = 0usize;
    while let Some((dir, rel)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            seen += 1;
            if seen > 50_000 {
                return out;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && name != "." {
                continue;
            }
            let child = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let p = e.path();
            if p.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push((p, child));
                }
                continue;
            }
            let Some(k) = kind_of_ext(&name) else { continue };
            if kind.is_some_and(|x| x != k) || !matches(words, &[&child]) {
                continue;
            }
            out.push(Hit {
                id: child.clone(),
                name,
                kind: Some(k),
                tags: Vec::new(),
                detail: None,
                use_with: format!("reference `{child}` relative to the project root"),
            });
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.truncate(limit);
    out
}

/// Runs the lookup order.
pub fn search(r: &SearchRequest) -> SearchResult {
    let words = terms(r.query);
    let mut steps: Vec<Step> = Vec::new();
    let done = |steps: &[Step]| r.stop_at_first && steps.iter().any(|s| !s.results.is_empty());

    // 1. Project files.
    steps.push(Step {
        step: 1,
        name: "project_files".into(),
        searched: true,
        results: project_files(r.project_root, &words, r.kind, r.limit),
        notes: Vec::new(),
    });

    // 2. Owned library.
    let mut s = Step {
        step: 2,
        name: "owned_library".into(),
        searched: false,
        results: Vec::new(),
        notes: Vec::new(),
    };
    if !done(&steps) {
        match r.library {
            Some(lib) => {
                s.searched = true;
                for rec in lib.records() {
                    let k = library_kind(rec.kind);
                    if r.kind.is_some_and(|x| x != k) {
                        continue;
                    }
                    if !matches(
                        &words,
                        &[&rec.id, &rec.name, &rec.source.path, &rec.license.product],
                    ) {
                        continue;
                    }
                    s.results.push(Hit {
                        id: rec.id.clone(),
                        name: rec.name.clone(),
                        kind: Some(k),
                        tags: Vec::new(),
                        detail: Some(format!("owned, private: {}", rec.license.license_type)),
                        use_with: if rec.kind == AssetKind::Prefab {
                            "prefab_instantiate".into()
                        } else {
                            "library_get (owned; never pooled)".into()
                        },
                    });
                }
                s.results.truncate(r.limit);
            }
            None => s.notes.push(
                r.library_note
                    .clone()
                    .unwrap_or_else(|| "no owned library configured".into()),
            ),
        }
    }
    steps.push(s);

    // 3. CC0 pool.
    let mut s = Step {
        step: 3,
        name: "cc0_pool".into(),
        searched: false,
        results: Vec::new(),
        notes: Vec::new(),
    };
    if !done(&steps) {
        match r.pool {
            Some(pool) => {
                s.searched = true;
                for rec in pool.records() {
                    if r.kind.is_some_and(|x| x != rec.kind) {
                        continue;
                    }
                    let mut fields: Vec<&str> = vec![&rec.id, &rec.name];
                    fields.extend(rec.tags.iter().map(String::as_str));
                    if !matches(&words, &fields) {
                        continue;
                    }
                    s.results.push(Hit {
                        id: rec.id.clone(),
                        name: rec.name.clone(),
                        kind: Some(rec.kind),
                        tags: rec.tags.clone(),
                        detail: Some("CC0".into()),
                        use_with: "pool_fetch".into(),
                    });
                }
                s.results.truncate(r.limit);
            }
            None => s.notes.push(
                r.pool_note
                    .clone()
                    .unwrap_or_else(|| "no CC0 pool configured".into()),
            ),
        }
    }
    steps.push(s);

    // 4. Online CC0 libraries.
    let mut s = Step {
        step: 4,
        name: "online_cc0".into(),
        searched: false,
        results: Vec::new(),
        notes: Vec::new(),
    };
    if !done(&steps) {
        if r.online {
            s.searched = true;
            let mut hits: Vec<OnlineHit> = Vec::new();
            for lib in sources::LIBRARIES {
                if !lib.searchable {
                    s.notes.push(format!(
                        "{} has no search API: browse {} and add a pack with pool_ingest (archive)",
                        lib.name, lib.browse
                    ));
                    continue;
                }
                let found = match lib.id {
                    "polyhaven" => sources::polyhaven::search(r.transport, r.query, r.kind, r.limit),
                    "ambientcg" => sources::ambientcg::search(r.transport, r.query, r.kind, r.limit),
                    _ => Ok(Vec::new()),
                };
                match found {
                    Ok(h) => hits.extend(h),
                    Err(e) => s.notes.push(format!("{}: {e}", lib.name)),
                }
            }
            hits.sort_by(|a, b| {
                b.popularity
                    .cmp(&a.popularity)
                    .then(a.source_id.cmp(&b.source_id))
            });
            hits.truncate(r.limit);
            s.results = hits
                .into_iter()
                .map(|h| Hit {
                    use_with: format!(
                        "pool_ingest with library `{}` and source_id `{}`",
                        h.library, h.source_id
                    ),
                    id: h.source_id,
                    name: h.name,
                    kind: Some(h.kind),
                    tags: h.tags,
                    detail: Some(h.page),
                })
                .collect();
        } else {
            s.notes
                .push("not searched: pass `online: true` to search Poly Haven and ambientCG".into());
        }
    }
    steps.push(s);

    // 5. Generation.
    let mut s = Step {
        step: 5,
        name: "generate".into(),
        searched: false,
        results: Vec::new(),
        notes: Vec::new(),
    };
    if !done(&steps) {
        s.notes.push(
            "nothing acceptable found: plan generation with `generate_plan` (read-only: cache, cost, cap), then `generate_run` after the user approves spending; generated assets are not CC0 and stay out of the pool unless a person dedicates them"
                .into(),
        );
    }
    steps.push(s);

    let found_in = steps
        .iter()
        .find(|s| !s.results.is_empty())
        .map(|s| s.name.clone());
    SearchResult {
        query: r.query.to_string(),
        steps,
        found_in,
    }
}
