//! ambientCG (ambientcg.com/api/v2/full_json). Answers wrap assets in `foundAssets`; each has
//! `assetId`, `displayName`, `dataType`, `tags`, `downloadCount` and
//! `downloadFolders.default.downloadFiletypeCategories.zip.downloads[]` with `attribute`
//! (`1K-PNG`, `2K-JPG`, ...), `downloadLink`, `fileName` and `size`. Materials come as ZIP
//! archives holding one image per map (`<id>_<res>_Color.png`, `_NormalGL`, `_Roughness`, ...).

use super::{OnlineHit, encode, terms, to_png};
use crate::admit::Candidate;
use crate::net::{self, Transport};
use crate::zip::{self, ZipLimits};
use crate::{Origin, PoolKind, Proof, Provenance};
use serde_json::Value;

pub const API: &str = "https://ambientcg.com/api/v2/full_json";

fn json(t: &dyn Transport, url: &str) -> Result<Value, String> {
    let body = net::get(t, url)?;
    serde_json::from_slice(&body).map_err(|e| format!("{url} did not answer JSON: {e}"))
}

fn tags(a: &Value) -> Vec<String> {
    a["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_string))
        .collect()
}

/// Searches materials (the only ambientCG type the pool takes).
pub fn search(
    t: &dyn Transport,
    query: &str,
    kind: Option<PoolKind>,
    limit: usize,
) -> Result<Vec<OnlineHit>, String> {
    if kind.is_some_and(|k| k != PoolKind::Texture) {
        return Ok(Vec::new());
    }
    let q = terms(query).join(" ");
    let url = format!(
        "{API}?q={}&type=Material&sort=popular&limit={}&include=downloadData,tagData",
        encode(&q),
        limit.clamp(1, 100)
    );
    let doc = json(t, &url)?;
    let found = doc["foundAssets"]
        .as_array()
        .ok_or("ambientCG answer has no foundAssets")?;
    Ok(found
        .iter()
        .filter_map(|a| {
            let id = a["assetId"].as_str()?;
            Some(OnlineHit {
                library: "ambientcg".into(),
                source_id: id.to_string(),
                name: a["displayName"].as_str().unwrap_or(id).to_string(),
                kind: PoolKind::Texture,
                tags: tags(a),
                page: format!("https://ambientcg.com/view?id={id}"),
                popularity: a["downloadCount"].as_u64().unwrap_or(0),
            })
        })
        .collect())
}

/// Map suffixes kept, as (suffix in the archive, file name in the pool, usage).
const MAPS: &[(&str, &str, &str)] = &[
    ("_Color", "color.png", "base color (sRGB)"),
    (
        "_NormalGL",
        "normal.png",
        "normal map, OpenGL convention (linear)",
    ),
    ("_Roughness", "roughness.png", "roughness (linear)"),
    ("_AmbientOcclusion", "ao.png", "ambient occlusion (linear)"),
    ("_Metalness", "metalness.png", "metalness (linear)"),
    ("_Displacement", "displacement.png", "height (linear)"),
    ("_Opacity", "opacity.png", "opacity (linear)"),
];

/// Builds a pool candidate for material `id` at `resolution` (`1K`, `2K`, ...; PNG preferred).
pub fn ingest(t: &dyn Transport, id: &str, resolution: &str) -> Result<Candidate, String> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!("`{id}` is not an ambientCG asset id"));
    }
    let doc = json(
        t,
        &format!("{API}?id={}&include=downloadData,tagData", encode(id)),
    )?;
    let a = doc["foundAssets"]
        .as_array()
        .and_then(|f| f.iter().find(|a| a["assetId"] == id))
        .ok_or_else(|| format!("ambientCG has no asset `{id}`"))?;
    if a["dataType"] != "Material" {
        return Err(format!(
            "`{id}` is a {}; the pool takes ambientCG materials only",
            a["dataType"].as_str().unwrap_or("different type")
        ));
    }
    let downloads = a["downloadFolders"]["default"]["downloadFiletypeCategories"]["zip"]["downloads"]
        .as_array()
        .ok_or("no zip downloads listed")?;
    let res = resolution.to_ascii_uppercase();
    let pick = |fmt: &str| {
        downloads
            .iter()
            .find(|d| d["attribute"].as_str() == Some(&format!("{res}-{fmt}")))
    };
    let d = pick("PNG")
        .or_else(|| pick("JPG"))
        .ok_or_else(|| format!("no {res} PNG or JPG download"))?;
    let link = d["downloadLink"].as_str().ok_or("download has no link")?;
    if !super::library("ambientcg").expect("known").owns(link) {
        return Err(format!("download points outside ambientCG: {link}"));
    }
    let bytes = net::get(t, link)?;
    if let Some(size) = d["size"].as_u64()
        && size != bytes.len() as u64
    {
        return Err(format!("download: expected {size} bytes, got {}", bytes.len()));
    }
    let entries = zip::read(&bytes, ZipLimits::default())?;
    let mut files = Vec::new();
    let mut notes = Vec::new();
    for (suffix, name, usage) in MAPS {
        let found = entries.iter().find(|e| {
            let stem = e.name.rsplit('/').next().unwrap_or(&e.name);
            let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
            stem.ends_with(suffix)
                && (e.name.to_ascii_lowercase().ends_with(".png")
                    || e.name.to_ascii_lowercase().ends_with(".jpg"))
        });
        if let Some(e) = found {
            let png = to_png(&e.data)?;
            if png != e.data {
                notes.push(format!("{name} converted from JPEG to PNG"));
            }
            notes.push(format!("{name}: {usage}"));
            files.push((name.to_string(), png));
        }
    }
    if !files.iter().any(|(n, _)| n == "color.png") {
        return Err("the archive has no color map".into());
    }
    let mut tg = tags(a);
    tg.push("ambientcg".into());
    Ok(Candidate {
        kind: PoolKind::Texture,
        name: a["displayName"].as_str().unwrap_or(id).to_string(),
        id: None,
        tags: tg,
        source: Provenance {
            origin: Origin::Online,
            library: Some("ambientcg".into()),
            page: Some(format!("https://ambientcg.com/view?id={id}")),
            source_id: Some(id.to_string()),
            author: None,
            retrieved: None,
            downloads: vec![link.to_string()],
            method: None,
            generator: None,
        },
        proof: vec![Proof {
            kind: "download_integrity".into(),
            detail: format!(
                "archive {} matched the published size",
                d["fileName"].as_str().unwrap_or("")
            ),
            at: Some(link.to_string()),
        }],
        files,
        main: Some("color.png".into()),
        notes,
        origin_path: None,
        generated_dirs: Vec::new(),
    })
}
