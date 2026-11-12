//! Archive sources (Kenney, Quaternius, or any known library): a ZIP as the library publishes
//! it, from a URL on the library's domain or from a file a person downloaded. Admission
//! requires the archive's own license file to say CC0. `include` picks part of a pack (path
//! prefixes); license files are always kept.

use crate::admit::{Candidate, is_license_file_name, strip_common_root};
use crate::net::{self, Transport};
use crate::zip::{self, ZipLimits};
use crate::{Origin, PoolKind, Provenance};

/// Where the archive bytes come from.
pub enum ArchiveFrom<'a> {
    Url(&'a str),
    Bytes(Vec<u8>),
}

pub struct ArchiveRequest<'a> {
    pub library: &'a str,
    /// The pack's page on the library's site.
    pub page: &'a str,
    pub from: ArchiveFrom<'a>,
    pub kind: PoolKind,
    pub name: &'a str,
    pub tags: Vec<String>,
    /// Path prefixes inside the archive (after its common top folder) to keep; empty keeps all.
    pub include: Vec<String>,
    pub main: Option<String>,
}

pub fn ingest(t: &dyn Transport, r: ArchiveRequest) -> Result<Candidate, String> {
    let lib =
        super::library(r.library).ok_or_else(|| format!("`{}` is not a known CC0 library", r.library))?;
    let mut downloads = Vec::new();
    let bytes = match r.from {
        ArchiveFrom::Url(u) => {
            if !lib.owns(u) {
                return Err(format!(
                    "`{u}` is not on {}; download the archive yourself and pass it as a file",
                    lib.name
                ));
            }
            downloads.push(u.to_string());
            net::get(t, u)?
        }
        ArchiveFrom::Bytes(b) => b,
    };
    let entries = zip::read(&bytes, ZipLimits::default())?;
    let entries = strip_common_root(entries.into_iter().map(|e| (e.name, e.data)).collect());
    let total = entries.len();
    let files: Vec<(String, Vec<u8>)> = entries
        .into_iter()
        .filter(|(p, _)| {
            is_license_file_name(p)
                || r.include.is_empty()
                || r.include.iter().any(|i| p.starts_with(i.as_str()))
        })
        .collect();
    if !files.iter().any(|(p, _)| !is_license_file_name(p)) {
        return Err(format!("nothing in the archive matches {:?}", r.include));
    }
    let kept = files.len();
    let mut tags = r.tags;
    tags.push(lib.id.to_string());
    Ok(Candidate {
        kind: r.kind,
        name: r.name.to_string(),
        id: None,
        tags,
        source: Provenance {
            origin: Origin::Online,
            library: Some(lib.id.into()),
            page: Some(r.page.to_string()),
            source_id: None,
            author: Some(lib.name.to_string()),
            retrieved: None,
            downloads,
            method: None,
            generator: None,
        },
        proof: Vec::new(),
        files,
        main: r.main,
        notes: vec![format!("{kept} of {total} files of the archive kept")],
        origin_path: None,
    })
}
