//! The spend cap and the spend ledger.
//!
//! - The cap is set by a person, never by an operation: the environment variable
//!   `MARIONETTE_SPEND_CAP_USD`, else `spend_cap_usd` in the user config file
//!   (`marionette/config.json` in the user's config folder, the same file that names the owned
//!   library), else zero. With a zero cap every paid call is refused.
//! - The ledger is `<project>/.marionette/spend.jsonl`: one JSON object per paid call. The cap
//!   applies to its total. An unreadable ledger refuses paid calls rather than guessing.

use crate::{Kind, codes};
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const CAP_ENV: &str = "MARIONETTE_SPEND_CAP_USD";
pub const LEDGER_PATH: &str = ".marionette/spend.jsonl";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapSource {
    /// `MARIONETTE_SPEND_CAP_USD`.
    Environment,
    /// `spend_cap_usd` in the user config file.
    UserConfig,
    /// Nothing set: zero.
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, JsonSchema)]
pub struct SpendCap {
    pub usd: f64,
    pub source: CapSource,
}

impl SpendCap {
    pub const ZERO: SpendCap = SpendCap {
        usd: 0.0,
        source: CapSource::Default,
    };
}

/// The per-user configuration file: `%APPDATA%\marionette\config.json` on Windows,
/// `$XDG_CONFIG_HOME/marionette/config.json` or `~/.config/marionette/config.json` elsewhere.
/// (Same rule as `marionette_import::library::user_config_file`.)
pub fn user_config_file(var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let base = if cfg!(windows) {
        var("APPDATA").map(PathBuf::from)
    } else {
        var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join("marionette").join("config.json"))
}

#[derive(Deserialize)]
struct UserConfig {
    spend_cap_usd: Option<f64>,
}

/// Resolves the cap. A malformed value is an error, not zero, so a typo is noticed.
pub fn resolve_cap(var: &dyn Fn(&str) -> Option<String>) -> Result<SpendCap, Errors> {
    let bad = |msg: String| -> Errors {
        Diagnostic::error(codes::SPEND_CAP, msg)
            .fix("set the cap to a number of US dollars, zero or more")
            .into()
    };
    if let Some(v) = var(CAP_ENV).filter(|v| !v.trim().is_empty()) {
        let usd: f64 = v
            .trim()
            .parse()
            .map_err(|_| bad(format!("{CAP_ENV} is not a number")))?;
        if !usd.is_finite() || usd < 0.0 {
            return Err(bad(format!("{CAP_ENV} must be zero or more")));
        }
        return Ok(SpendCap {
            usd,
            source: CapSource::Environment,
        });
    }
    if let Some(path) = user_config_file(var)
        && let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(cfg) = serde_json::from_str::<UserConfig>(&text)
        && let Some(usd) = cfg.spend_cap_usd
    {
        if !usd.is_finite() || usd < 0.0 {
            return Err(bad(
                "spend_cap_usd in the user config file must be zero or more".into()
            ));
        }
        return Ok(SpendCap {
            usd,
            source: CapSource::UserConfig,
        });
    }
    Ok(SpendCap::ZERO)
}

/// One ledger line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub provider: String,
    pub model: String,
    pub kind: Kind,
    pub cost_usd: f64,
    pub estimate_usd: f64,
}

fn ledger_error(path: &Path, msg: String) -> Errors {
    Diagnostic::error(codes::LEDGER, msg)
        .file(path.to_string_lossy())
        .fix("repair or move the ledger file; paid calls stay refused until it can be read")
        .into()
}

/// Sum of every ledger entry; zero when the ledger does not exist.
pub fn total(ledger: &Path) -> Result<f64, Errors> {
    Ok(entries(ledger)?.iter().fold(0.0, |acc, e| acc + e.cost_usd))
}

pub fn entries(ledger: &Path) -> Result<Vec<Entry>, Errors> {
    let text = match std::fs::read_to_string(ledger) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(ledger_error(ledger, format!("cannot read the spend ledger: {e}"))),
    };
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: Entry = serde_json::from_str(line)
            .map_err(|e| ledger_error(ledger, format!("spend ledger line {} is invalid: {e}", i + 1)))?;
        if !entry.cost_usd.is_finite() || entry.cost_usd < 0.0 {
            return Err(ledger_error(
                ledger,
                format!("spend ledger line {} has a bad cost", i + 1),
            ));
        }
        out.push(entry);
    }
    Ok(out)
}

/// Refuses a call whose estimate would take the total past the cap.
pub fn check(cap: &SpendCap, spent_usd: f64, estimate_usd: f64) -> Result<(), Errors> {
    if spent_usd + estimate_usd <= cap.usd {
        return Ok(());
    }
    let how = match cap.source {
        CapSource::Default => format!(
            "no spend cap is set, so paid calls are refused; the project owner sets one with {CAP_ENV} or spend_cap_usd in the user config file"
        ),
        _ => "only the project owner may raise the cap; ask them, or use `mock`".to_string(),
    };
    Err(Diagnostic::error(
        codes::SPEND_CAP,
        format!(
            "this call may cost up to ${estimate_usd:.4}; spent so far ${spent_usd:.4}; cap ${:.4}",
            cap.usd
        ),
    )
    .fix(how)
    .into())
}

/// Appends one entry (creating the folder and file as needed).
pub fn record(ledger: &Path, entry: &Entry) -> Result<(), Errors> {
    let err = |e: std::io::Error| ledger_error(ledger, format!("cannot write the spend ledger: {e}"));
    if let Some(dir) = ledger.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(ledger)
        .map_err(err)?;
    let mut line = serde_json::to_string(entry).expect("entries serialize");
    line.push('\n');
    file.write_all(line.as_bytes()).map_err(err)
}
