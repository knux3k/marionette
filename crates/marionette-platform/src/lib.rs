//! Platform services a game uses the same way on every store (decision 0213):
//! - [`Achievements`]: unlocks and stats, checked against a definitions file by [`Tracker`];
//! - [`CloudFiles`]: small files kept by the platform, and [`sync_dir`] to keep a save
//!   folder in step with them (conflicts are kept, never overwritten);
//! - [`OfflinePlatform`]: both, stored in a local folder. The default for tests, builds
//!   without a store, and a store that is not running.
//!
//! The Steam backend lives in the optional `marionette-steam` crate.

mod defs;
mod offline;
mod sync;

pub use defs::{AchievementDef, AchievementsFile, StatDef, StatKind, parse_definitions};
pub use offline::OfflinePlatform;
pub use sync::{SyncReport, sync_dir};

use std::fmt;

/// Stable error codes (`platform.*`).
pub mod codes {
    /// The definitions file is malformed.
    pub const DEFINITIONS: &str = "platform.definitions";
    /// An achievement or stat id not in the definitions.
    pub const UNKNOWN: &str = "platform.unknown";
    /// A stat set with the wrong kind (integer vs float).
    pub const KIND: &str = "platform.kind";
    /// Reading or writing local files failed.
    pub const IO: &str = "platform.io";
    /// The platform refused or is not available.
    pub const UNAVAILABLE: &str = "platform.unavailable";
    /// A cloud file name that is empty, too long or has path separators.
    pub const NAME: &str = "platform.name";
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlatformError {
    pub code: &'static str,
    pub message: String,
}

impl PlatformError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PlatformError {}

pub type Result<T> = std::result::Result<T, PlatformError>;

/// A stat value. Integer and float stats are separate on every store.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum StatValue {
    Int(i32),
    Float(f32),
}

impl StatValue {
    pub fn as_f64(self) -> f64 {
        match self {
            StatValue::Int(v) => v as f64,
            StatValue::Float(v) => v as f64,
        }
    }
}

/// Achievements and stats on a platform. Ids are the platform's API names.
pub trait Achievements {
    /// Unlocks; true when it was locked before.
    fn unlock(&mut self, id: &str) -> Result<bool>;
    fn is_unlocked(&self, id: &str) -> Result<bool>;
    /// Locks again (development and tests only; stores may refuse in release).
    fn clear(&mut self, id: &str) -> Result<()>;
    fn set_stat(&mut self, id: &str, value: StatValue) -> Result<()>;
    fn stat(&self, id: &str) -> Result<Option<StatValue>>;
    /// Shows progress toward an achievement where the platform can (no state change).
    fn show_progress(&mut self, _id: &str, _current: u32, _max: u32) -> Result<()> {
        Ok(())
    }
    /// Sends changes to the platform (stores batch them; call at checkpoints, not per tick).
    fn store(&mut self) -> Result<()>;
}

/// Files the platform keeps for the player (Steam Remote Storage, a folder offline).
pub trait CloudFiles {
    /// Whether cloud storage is on for this player and game.
    fn enabled(&self) -> bool;
    fn write(&mut self, name: &str, data: &[u8]) -> Result<()>;
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>>;
    fn delete(&mut self, name: &str) -> Result<()>;
    /// (name, size in bytes), sorted by name.
    fn list(&self) -> Result<Vec<(String, u64)>>;
}

/// Refuses names a store or a file system would treat badly.
pub fn check_name(name: &str) -> Result<()> {
    let bad = name.is_empty()
        || name.len() > 255
        || name.contains(['/', '\\', ':', '\0'])
        || name == "."
        || name == "..";
    if bad {
        return Err(PlatformError::new(
            codes::NAME,
            format!("cloud file name {name:?}: 1 to 255 bytes, no / \\ : or NUL"),
        ));
    }
    Ok(())
}

/// Achievements and stats checked against definitions: unknown ids and wrong kinds are
/// errors, stats are clamped to their maximum, and achievements tied to a stat unlock when
/// it reaches their threshold.
pub struct Tracker<A: Achievements> {
    pub defs: AchievementsFile,
    pub backend: A,
}

impl<A: Achievements> Tracker<A> {
    pub fn new(defs: AchievementsFile, backend: A) -> Self {
        Self { defs, backend }
    }

    fn achievement(&self, id: &str) -> Result<&AchievementDef> {
        self.defs.achievements.iter().find(|a| a.id == id).ok_or_else(|| {
            PlatformError::new(
                codes::UNKNOWN,
                format!("no achievement {id:?} in the definitions"),
            )
        })
    }

    fn stat_def(&self, id: &str) -> Result<&StatDef> {
        self.defs
            .stats
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| PlatformError::new(codes::UNKNOWN, format!("no stat {id:?} in the definitions")))
    }

    /// Unlocks a defined achievement; true when newly unlocked.
    pub fn unlock(&mut self, id: &str) -> Result<bool> {
        self.achievement(id)?;
        if self.backend.is_unlocked(id)? {
            return Ok(false);
        }
        self.backend.unlock(id)
    }

    /// Sets a stat; returns the achievements this unlocked.
    pub fn set_stat(&mut self, id: &str, value: StatValue) -> Result<Vec<String>> {
        let def = self.stat_def(id)?.clone();
        let value = match (def.kind, value) {
            (StatKind::Int, StatValue::Int(v)) => StatValue::Int(match def.max {
                Some(m) => v.min(m as i32),
                None => v,
            }),
            (StatKind::Float, StatValue::Float(v)) => StatValue::Float(match def.max {
                Some(m) => v.min(m as f32),
                None => v,
            }),
            _ => {
                return Err(PlatformError::new(
                    codes::KIND,
                    format!("stat {id:?} is {:?}", def.kind),
                ));
            }
        };
        self.backend.set_stat(id, value)?;
        let mut unlocked = Vec::new();
        let due: Vec<AchievementDef> = self
            .defs
            .achievements
            .iter()
            .filter(|a| a.stat.as_deref() == Some(id))
            .cloned()
            .collect();
        for a in due {
            let at = a.at.unwrap_or(f64::INFINITY);
            if value.as_f64() >= at {
                if self.backend.unlock(&a.id)? {
                    unlocked.push(a.id.clone());
                }
            } else if !self.backend.is_unlocked(&a.id)? {
                let cur = value.as_f64().max(0.0) as u32;
                self.backend.show_progress(&a.id, cur, at as u32)?;
            }
        }
        Ok(unlocked)
    }

    /// Adds to an integer stat (starting from 0); returns the achievements this unlocked.
    pub fn add_stat(&mut self, id: &str, delta: i32) -> Result<Vec<String>> {
        let cur = match self.backend.stat(id)? {
            Some(StatValue::Int(v)) => v,
            _ => 0,
        };
        self.set_stat(id, StatValue::Int(cur.saturating_add(delta)))
    }

    pub fn store(&mut self) -> Result<()> {
        self.backend.store()
    }
}
