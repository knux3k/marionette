//! The `.msave` file.
//!
//! ```text
//! MARIONETTE-SAVE 1\n
//! {"game":"..","version":3,"tick":..,"label":"..","meta":{..},"sha256":"..",...}\n
//! <scene text><module bytes, in header order>
//! ```
//!
//! The header is one line of JSON, so a slot menu reads only the first line. The body is
//! checked against `sha256`, which catches truncated and damaged files.

use crate::{SaveFormat, SaveInfo, codes};
use marionette_core::{Diagnostic, Errors, SimSnapshot};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// First line of every save file.
pub const MAGIC: &str = "MARIONETTE-SAVE";
/// Container version this build writes and the newest it reads.
pub const FORMAT: u32 = 1;
/// Largest `meta` as JSON text, in bytes. Keeps headers small for slot menus.
pub const META_MAX: usize = 16 * 1024;
/// Largest header line read when only the header is wanted.
pub const HEADER_MAX: usize = META_MAX + 8 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleBytes {
    pub name: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveHeader {
    pub format: u32,
    pub game: String,
    pub version: u32,
    pub tick: u64,
    pub rng: u64,
    pub label: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub meta: serde_json::Value,
    /// Seconds since 1970 when written (wall clock, for slot menus; never read by the game).
    pub saved_unix: u64,
    /// Engine version that wrote it.
    pub engine: String,
    pub scene_bytes: u64,
    pub modules: Vec<ModuleBytes>,
    /// SHA-256 (hex) of the body.
    pub sha256: String,
}

/// A whole save: header and body.
#[derive(Debug, Clone)]
pub struct SaveFile {
    pub header: SaveHeader,
    pub body: Vec<u8>,
}

fn corrupt(what: impl Into<String>) -> Errors {
    Diagnostic::error(codes::CORRUPT, what.into())
        .fix("load the slot's backup or another slot")
        .into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl SaveFile {
    pub fn new(
        snap: &SimSnapshot,
        format: &SaveFormat,
        info: SaveInfo,
        saved_unix: u64,
    ) -> Result<Self, Errors> {
        if !info.meta.is_null() && info.meta.to_string().len() > META_MAX {
            return Err(
                Diagnostic::error(codes::META, format!("save meta is larger than {META_MAX} bytes"))
                    .fix("keep slot-menu data small; game state belongs in components")
                    .into(),
            );
        }
        let mut body =
            Vec::with_capacity(snap.scene.len() + snap.modules.values().map(Vec::len).sum::<usize>());
        body.extend_from_slice(snap.scene.as_bytes());
        let mut modules = Vec::new();
        for (name, bytes) in &snap.modules {
            body.extend_from_slice(bytes);
            modules.push(ModuleBytes {
                name: (*name).to_string(),
                bytes: bytes.len() as u64,
            });
        }
        let header = SaveHeader {
            format: FORMAT,
            game: format.game.clone(),
            version: format.version,
            tick: snap.tick,
            rng: snap.rng,
            label: info.label,
            meta: info.meta,
            saved_unix,
            engine: env!("CARGO_PKG_VERSION").to_string(),
            scene_bytes: snap.scene.len() as u64,
            modules,
            sha256: hex(&Sha256::digest(&body)),
        };
        Ok(Self { header, body })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let head = serde_json::to_string(&self.header).expect("headers always serialize");
        let mut out = Vec::with_capacity(MAGIC.len() + head.len() + self.body.len() + 8);
        out.extend_from_slice(format!("{MAGIC} {FORMAT}\n").as_bytes());
        out.extend_from_slice(head.as_bytes());
        out.push(b'\n');
        out.extend_from_slice(&self.body);
        out
    }

    /// Decodes and checks a whole file.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Errors> {
        let (header, start) = parse_header(bytes)?;
        let body = &bytes[start..];
        let expected = header.scene_bytes + header.modules.iter().map(|m| m.bytes).sum::<u64>();
        if body.len() as u64 != expected {
            return Err(corrupt(format!(
                "save body is {} bytes, header says {expected} (truncated or damaged)",
                body.len()
            )));
        }
        if hex(&Sha256::digest(body)) != header.sha256 {
            return Err(corrupt("save body does not match its checksum"));
        }
        Ok(Self {
            header,
            body: body.to_vec(),
        })
    }

    /// The state as a snapshot. `modules` are the running module names; bytes of modules
    /// not running are dropped and the second value is then false.
    pub fn into_snapshot(self, modules: &[&'static str]) -> Result<(SimSnapshot, bool), Errors> {
        let n = self.header.scene_bytes as usize;
        let scene = String::from_utf8(self.body[..n].to_vec())
            .map_err(|_| corrupt("save scene text is not UTF-8"))?;
        let mut at = n;
        let mut map = BTreeMap::new();
        let mut all = true;
        for m in &self.header.modules {
            let end = at + m.bytes as usize;
            match modules.iter().find(|name| **name == m.name) {
                Some(name) => {
                    map.insert(*name, self.body[at..end].to_vec());
                }
                None => all = false,
            }
            at = end;
        }
        Ok((
            SimSnapshot {
                scene,
                tick: self.header.tick,
                rng: self.header.rng,
                modules: map,
            },
            all,
        ))
    }
}

/// Reads the magic line and header; returns the header and where the body starts. Needs
/// only the first [`HEADER_MAX`] bytes.
pub fn parse_header(bytes: &[u8]) -> Result<(SaveHeader, usize), Errors> {
    let line = |from: usize| {
        bytes[from..]
            .iter()
            .take(HEADER_MAX)
            .position(|b| *b == b'\n')
            .map(|i| from + i)
    };
    let Some(end1) = line(0) else {
        return Err(corrupt("not a save file (no magic line)"));
    };
    let magic = std::str::from_utf8(&bytes[..end1]).unwrap_or("");
    let Some(version) = magic.strip_prefix(MAGIC).map(str::trim) else {
        return Err(corrupt("not a save file (wrong magic line)"));
    };
    let format: u32 = version
        .parse()
        .map_err(|_| corrupt("not a save file (bad format number)"))?;
    if format > FORMAT {
        return Err(Diagnostic::error(
            codes::FORMAT,
            format!("save file format {format} is newer than this engine's {FORMAT}"),
        )
        .fix("update the engine")
        .into());
    }
    let Some(end2) = line(end1 + 1) else {
        return Err(corrupt("save header is cut off"));
    };
    let header: SaveHeader = serde_json::from_slice(&bytes[end1 + 1..end2])
        .map_err(|e| corrupt(format!("save header is not valid: {e}")))?;
    Ok((header, end2 + 1))
}
