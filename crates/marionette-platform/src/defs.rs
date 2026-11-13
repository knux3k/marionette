//! The achievements and stats definitions file (`*.achievements.json`):
//!
//! ```json
//! { "format": "marionette.achievements", "version": 1,
//!   "achievements": [
//!     { "id": "FIRST_BUILD", "name": "First build", "description": "Build anything." },
//!     { "id": "BUILD_100", "name": "Builder", "description": "Build 100 things.",
//!       "stat": "things_built", "at": 100 } ],
//!   "stats": [ { "id": "things_built", "kind": "int", "max": 1000000 } ] }
//! ```
//! Ids are the store's API names (the same ids must be set up on the store's site).

use crate::{PlatformError, Result, codes};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const FORMAT: &str = "marionette.achievements";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AchievementsFile {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub achievements: Vec<AchievementDef>,
    #[serde(default)]
    pub stats: Vec<StatDef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AchievementDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Hidden until unlocked (shown on the store page as "hidden").
    #[serde(default)]
    pub hidden: bool,
    /// Unlocks when this stat reaches `at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stat: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatKind {
    Int,
    Float,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatDef {
    pub id: String,
    pub kind: StatKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

fn bad(msg: String) -> PlatformError {
    PlatformError::new(codes::DEFINITIONS, msg)
}

fn check_id(kind: &str, id: &str) -> Result<()> {
    let ok = !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(bad(format!(
            "{kind} id {id:?}: use 1 to 128 ASCII letters, digits or _ (store API names)"
        )))
    }
}

/// Parses and checks a definitions file: format, unique ids, store-safe ids, stat links.
pub fn parse_definitions(text: &str) -> Result<AchievementsFile> {
    let f: AchievementsFile =
        serde_json::from_str(text).map_err(|e| bad(format!("achievements file: {e}")))?;
    if f.format != FORMAT || f.version != 1 {
        return Err(bad(format!(
            "format must be \"{FORMAT}\" version 1, found {:?} version {}",
            f.format, f.version
        )));
    }
    let mut stats = BTreeSet::new();
    for s in &f.stats {
        check_id("stat", &s.id)?;
        if !stats.insert(s.id.as_str()) {
            return Err(bad(format!("stat {:?} defined twice", s.id)));
        }
    }
    let mut seen = BTreeSet::new();
    for a in &f.achievements {
        check_id("achievement", &a.id)?;
        if !seen.insert(a.id.as_str()) {
            return Err(bad(format!("achievement {:?} defined twice", a.id)));
        }
        match (&a.stat, a.at) {
            (Some(s), Some(_)) if !stats.contains(s.as_str()) => {
                return Err(bad(format!("achievement {:?} uses undefined stat {s:?}", a.id)));
            }
            (Some(_), None) | (None, Some(_)) => {
                return Err(bad(format!(
                    "achievement {:?}: give both stat and at, or neither",
                    a.id
                )));
            }
            _ => {}
        }
    }
    Ok(f)
}
