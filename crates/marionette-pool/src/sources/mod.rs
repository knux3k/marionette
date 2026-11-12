//! Online CC0 libraries the pool knows, and how to search and ingest from them.
//!
//! Each library's license policy is recorded with the URL and the sentence it was checked
//! against (decision 0041). Poly Haven and ambientCG have public JSON APIs (search and ingest);
//! Kenney and Quaternius publish archives that carry a license file, which admission requires.
//! The API formats were read from live responses on 2026-11-12; parsing is by key name.

pub mod ambientcg;
pub mod archive;
pub mod polyhaven;

use crate::{PoolKind, Proof};
use schemars::JsonSchema;
use serde::Serialize;

/// A known CC0 library.
#[derive(Debug, Clone, Copy)]
pub struct Library {
    pub id: &'static str,
    pub name: &'static str,
    /// Pages and downloads must be on one of these domains (or their subdomains).
    pub domains: &'static [&'static str],
    pub license_url: &'static str,
    /// The library's own words, as read on its license page.
    pub license_quote: &'static str,
    /// Assets must carry the library's license file (archive sources).
    pub needs_license_file: bool,
    /// Has a search API the pool uses.
    pub searchable: bool,
    /// Where a person browses it.
    pub browse: &'static str,
}

impl Library {
    pub fn owns(&self, url: &str) -> bool {
        self.domains.iter().any(|d| crate::net::on_domain(url, d))
    }

    pub fn policy_proof(&self) -> Proof {
        Proof {
            kind: "library_policy".into(),
            detail: format!("{}: \"{}\"", self.name, self.license_quote),
            at: Some(self.license_url.into()),
        }
    }
}

pub const LIBRARIES: &[Library] = &[
    Library {
        id: "polyhaven",
        name: "Poly Haven",
        domains: &["polyhaven.com", "polyhaven.org"],
        license_url: "https://polyhaven.com/license",
        license_quote: "Our assets are all licensed as CC0",
        needs_license_file: false,
        searchable: true,
        browse: "https://polyhaven.com/all",
    },
    Library {
        id: "ambientcg",
        name: "ambientCG",
        domains: &["ambientcg.com"],
        license_url: "https://docs.ambientcg.com/license/",
        license_quote: "All ambientCG assets are provided under the Creative Commons CC0 1.0 Universal License.",
        needs_license_file: false,
        searchable: true,
        browse: "https://ambientcg.com/list",
    },
    Library {
        id: "kenney",
        name: "Kenney",
        domains: &["kenney.nl"],
        license_url: "https://kenney.nl/support",
        license_quote: "Yes, all game assets on the asset pages are public domain licensed (CC0).",
        needs_license_file: true,
        searchable: false,
        browse: "https://kenney.nl/assets",
    },
    Library {
        id: "quaternius",
        name: "Quaternius",
        domains: &["quaternius.com"],
        license_url: "https://quaternius.com/faq.html",
        license_quote: "All models are under the CC0 License.",
        needs_license_file: true,
        searchable: false,
        browse: "https://quaternius.com",
    },
];

pub fn library(id: &str) -> Option<&'static Library> {
    LIBRARIES.iter().find(|l| l.id == id)
}

/// One search result from an online library (nothing downloaded yet).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct OnlineHit {
    pub library: String,
    /// The id to pass to `pool_ingest`.
    pub source_id: String,
    pub name: String,
    pub kind: PoolKind,
    pub tags: Vec<String>,
    pub page: String,
    /// Higher is more popular (the library's download count).
    pub popularity: u64,
}

/// Lowercase search terms.
pub fn terms(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

/// True when every term occurs in one of the fields.
pub fn matches(terms: &[String], fields: &[&str]) -> bool {
    let hay: Vec<String> = fields.iter().map(|f| f.to_lowercase()).collect();
    terms.iter().all(|t| hay.iter().any(|h| h.contains(t.as_str())))
}

/// Percent-encodes a query parameter.
pub fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// JPEG to PNG (the engine's texture format), keeping PNG as it is.
pub fn to_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        marionette_assets::decode_png(bytes)?;
        return Ok(bytes.to_vec());
    }
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)
        .map_err(|e| format!("cannot decode image: {e}"))?
        .to_rgba8();
    Ok(marionette_assets::encode_png(
        img.width(),
        img.height(),
        img.as_raw(),
    ))
}
