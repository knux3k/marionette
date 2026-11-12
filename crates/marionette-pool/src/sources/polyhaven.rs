//! Poly Haven (api.polyhaven.com). `/assets?t=textures|models` lists assets keyed by id;
//! `/info/<id>` describes one (`type` 0 HDRI, 1 texture, 2 model); `/files/<id>` maps
//! map name -> resolution -> format -> {url, size, md5}, and `gltf` -> resolution -> `gltf`
//! -> {url, size, md5, include: {relative path: {url, size, md5}}}. Every download is checked
//! against its published size and MD5.

use super::{OnlineHit, matches, terms, to_png};
use crate::admit::Candidate;
use crate::net::{self, Transport, md5_hex};
use crate::{Origin, PoolKind, Proof, Provenance};
use serde_json::Value;

pub const API: &str = "https://api.polyhaven.com";

fn json(t: &dyn Transport, url: &str) -> Result<Value, String> {
    let body = net::get(t, url)?;
    serde_json::from_slice(&body).map_err(|e| format!("{url} did not answer JSON: {e}"))
}

fn kind_of(type_id: i64) -> Option<PoolKind> {
    match type_id {
        1 => Some(PoolKind::Texture),
        2 => Some(PoolKind::Model),
        _ => None,
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_string))
        .collect()
}

/// Searches textures and/or models by words in id, name, tags and categories.
pub fn search(
    t: &dyn Transport,
    query: &str,
    kind: Option<PoolKind>,
    limit: usize,
) -> Result<Vec<OnlineHit>, String> {
    let words = terms(query);
    let mut hits = Vec::new();
    for (ty, k) in [("textures", PoolKind::Texture), ("models", PoolKind::Model)] {
        if kind.is_some_and(|x| x != k) {
            continue;
        }
        let all = json(t, &format!("{API}/assets?t={ty}"))?;
        let Some(map) = all.as_object() else {
            return Err("Poly Haven asset list is not an object".into());
        };
        for (id, a) in map {
            let name = a["name"].as_str().unwrap_or(id);
            let tags = strings(&a["tags"]);
            let cats = strings(&a["categories"]);
            let mut fields: Vec<&str> = vec![id, name];
            fields.extend(tags.iter().map(String::as_str));
            fields.extend(cats.iter().map(String::as_str));
            if !matches(&words, &fields) {
                continue;
            }
            hits.push(OnlineHit {
                library: "polyhaven".into(),
                source_id: id.clone(),
                name: name.to_string(),
                kind: k,
                tags,
                page: format!("https://polyhaven.com/a/{id}"),
                popularity: a["download_count"].as_u64().unwrap_or(0),
            });
        }
    }
    hits.sort_by(|a, b| {
        b.popularity
            .cmp(&a.popularity)
            .then(a.source_id.cmp(&b.source_id))
    });
    hits.truncate(limit);
    Ok(hits)
}

/// Downloads one file entry ({url, size, md5}) and checks it.
fn fetch(
    t: &dyn Transport,
    entry: &Value,
    what: &str,
    downloads: &mut Vec<String>,
) -> Result<Vec<u8>, String> {
    let url = entry["url"]
        .as_str()
        .ok_or_else(|| format!("{what} has no url"))?;
    if !super::library("polyhaven").expect("known").owns(url) {
        return Err(format!("{what} points outside Poly Haven: {url}"));
    }
    let bytes = net::get(t, url)?;
    if let Some(size) = entry["size"].as_u64()
        && size != bytes.len() as u64
    {
        return Err(format!("{what}: expected {size} bytes, got {}", bytes.len()));
    }
    if let Some(md5) = entry["md5"].as_str()
        && !md5.eq_ignore_ascii_case(&md5_hex(&bytes))
    {
        return Err(format!("{what}: MD5 does not match the published sum"));
    }
    downloads.push(url.to_string());
    Ok(bytes)
}

/// Texture maps the pool keeps, as (Poly Haven map name, file name in the pool, usage).
const MAPS: &[(&str, &str, &str)] = &[
    ("Diffuse", "color.png", "base color (sRGB)"),
    ("nor_gl", "normal.png", "normal map, OpenGL convention (linear)"),
    ("Rough", "roughness.png", "roughness (linear)"),
    ("AO", "ao.png", "ambient occlusion (linear)"),
    ("Displacement", "displacement.png", "height (linear)"),
];

/// Builds a pool candidate for Poly Haven asset `id` at `resolution` (`1k`, `2k`, `4k`, `8k`).
pub fn ingest(t: &dyn Transport, id: &str, resolution: &str) -> Result<Candidate, String> {
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        || id.is_empty()
    {
        return Err(format!("`{id}` is not a Poly Haven asset id"));
    }
    let info = json(t, &format!("{API}/info/{id}"))?;
    let type_id = info["type"].as_i64().ok_or("asset info has no type")?;
    let kind = kind_of(type_id).ok_or("HDRIs are not pooled yet (only textures and models)")?;
    let files = json(t, &format!("{API}/files/{id}"))?;
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut downloads = Vec::new();
    let mut notes = Vec::new();
    match kind {
        PoolKind::Texture => {
            for (map, name, usage) in MAPS {
                let res = &files[*map][resolution];
                let entry = if res["png"].is_object() {
                    &res["png"]
                } else {
                    &res["jpg"]
                };
                if !entry.is_object() {
                    continue;
                }
                let bytes = fetch(t, entry, &format!("{map} {resolution}"), &mut downloads)?;
                let png = to_png(&bytes)?;
                if png != bytes {
                    notes.push(format!("{name} converted from JPEG to PNG"));
                }
                notes.push(format!("{name}: {usage}"));
                out.push((name.to_string(), png));
            }
            if !out.iter().any(|(n, _)| n == "color.png") {
                return Err(format!("no diffuse map at resolution {resolution}"));
            }
        }
        _ => {
            let g = &files["gltf"][resolution]["gltf"];
            if !g.is_object() {
                return Err(format!("no glTF at resolution {resolution}"));
            }
            let gltf_bytes = fetch(t, g, "glTF", &mut downloads)?;
            let mut doc: Value =
                serde_json::from_slice(&gltf_bytes).map_err(|e| format!("glTF is not JSON: {e}"))?;
            let mut include: Vec<(&String, &Value)> =
                g["include"].as_object().into_iter().flatten().collect();
            include.sort_by(|a, b| a.0.cmp(b.0));
            for (rel, entry) in include {
                let clean =
                    crate::zip::clean_name(rel).ok_or_else(|| format!("unsafe include path `{rel}`"))?;
                let bytes = fetch(t, entry, &clean, &mut downloads)?;
                // The engine's loader reads PNG; JPEG textures are converted and the glTF
                // updated to point at the PNG.
                if clean.to_ascii_lowercase().ends_with(".jpg")
                    || clean.to_ascii_lowercase().ends_with(".jpeg")
                {
                    let png_name = format!(
                        "{}.png",
                        clean.rsplit_once('.').map_or(clean.as_str(), |(s, _)| s)
                    );
                    for img in doc["images"].as_array_mut().into_iter().flatten() {
                        if img["uri"].as_str() == Some(rel.as_str())
                            || img["uri"].as_str() == Some(clean.as_str())
                        {
                            img["uri"] = Value::String(png_name.clone());
                            img.as_object_mut().map(|o| o.remove("mimeType"));
                        }
                    }
                    out.push((png_name, to_png(&bytes)?));
                } else {
                    out.push((clean, bytes));
                }
            }
            let main = format!("{id}.gltf");
            out.push((main, serde_json::to_vec_pretty(&doc).expect("json")));
            if notes.is_empty() && out.iter().any(|(n, _)| n.ends_with(".png")) {
                notes.push("JPEG textures converted to PNG; glTF image paths updated".into());
            }
        }
    }
    let authors: Vec<String> = info["authors"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    let mut tags = strings(&info["tags"]);
    tags.extend(strings(&info["categories"]));
    tags.push("polyhaven".into());
    Ok(Candidate {
        kind,
        name: info["name"].as_str().unwrap_or(id).to_string(),
        id: None,
        tags,
        source: Provenance {
            origin: Origin::Online,
            library: Some("polyhaven".into()),
            page: Some(format!("https://polyhaven.com/a/{id}")),
            source_id: Some(id.to_string()),
            author: (!authors.is_empty()).then(|| authors.join(", ")),
            retrieved: None,
            downloads,
            method: None,
            generator: None,
        },
        proof: vec![Proof {
            kind: "download_integrity".into(),
            detail: "every file matched the size and MD5 Poly Haven publishes in /files".into(),
            at: Some(format!("{API}/files/{id}")),
        }],
        main: if kind == PoolKind::Texture {
            Some("color.png".into())
        } else {
            Some(format!("{id}.gltf"))
        },
        files: out,
        notes,
        origin_path: None,
    })
}
