//! Online CC0 libraries the pool knows, and how to search and ingest from them.
//!
//! Each library's license policy is recorded with the URL and the sentence it was checked
//! against (decision 0041). Poly Haven and ambientCG have public JSON APIs (search and ingest);
//! Kenney and Quaternius publish archives that carry a license file, which admission requires;
//! KayKit publishes its packs on GitHub with LICENSE.txt (same rule); cgbookcase states CC0 for
//! every texture on its site. ShareTextures is not a library here: its license is a custom one
//! that forbids redistribution and automated downloads (decision 0071).
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
    /// For libraries hosted on a shared site (GitHub): https URL prefixes that belong to the
    /// library, such as its GitHub organization.
    pub prefixes: &'static [&'static str],
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
            || self
                .prefixes
                .iter()
                .any(|p| url.starts_with(p) && !url[p.len()..].contains(".."))
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
        prefixes: &[],
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
        prefixes: &[],
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
        prefixes: &[],
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
        prefixes: &[],
        license_url: "https://quaternius.com/faq.html",
        license_quote: "All models are under the CC0 License.",
        needs_license_file: true,
        searchable: false,
        browse: "https://quaternius.com",
    },
    Library {
        id: "kaykit",
        name: "KayKit",
        domains: &["kaylousberg.com", "kaylousberg.itch.io"],
        // The official GitHub mirrors of the packs (each with LICENSE.txt).
        prefixes: &[
            "https://github.com/KayKit-Game-Assets/",
            "https://raw.githubusercontent.com/KayKit-Game-Assets/",
        ],
        license_url: "https://kaylousberg.itch.io/kaykit-dungeon-remastered",
        license_quote: "Free for personal and commercial use, no attribution required. (CC0 Licensed)",
        needs_license_file: true,
        searchable: false,
        browse: "https://kaylousberg.com/game-assets",
    },
    Library {
        id: "cgbookcase",
        name: "cgbookcase",
        domains: &["cgbookcase.com", "cgbookcase.b-cdn.net"],
        prefixes: &[],
        license_url: "https://www.cgbookcase.com/textures",
        license_quote: "The textures are published under the CC0 1.0 license, which means means you can use them for free without giving credit.",
        needs_license_file: false,
        searchable: false,
        browse: "https://www.cgbookcase.com/textures",
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

#[cfg(test)]
mod tests {
    use super::library;

    #[test]
    fn libraries_on_shared_hosts_own_only_their_prefix() {
        let kk = library("kaykit").unwrap();
        assert!(kk.owns("https://github.com/KayKit-Game-Assets/KayKit-Prototype-Bits-1.0"));
        assert!(kk.owns(
            "https://raw.githubusercontent.com/KayKit-Game-Assets/KayKit-Prototype-Bits-1.0/main/LICENSE.txt"
        ));
        assert!(kk.owns("https://kaylousberg.itch.io/kaykit-dungeon-remastered"));
        assert!(!kk.owns("https://github.com/someone-else/KayKit-Copy"));
        assert!(!kk.owns("https://github.com/KayKit-Game-Assets/../someone-else/x"));
        assert!(!kk.owns("http://github.com/KayKit-Game-Assets/x"));
        assert!(library("sharetextures").is_none());
        assert!(
            library("cgbookcase")
                .unwrap()
                .owns("https://www.cgbookcase.com/textures/bark-19")
        );
    }
}
