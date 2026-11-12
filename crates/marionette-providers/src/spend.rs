//! The spend cap and the spend ledger.
//!
//! - The cap is set by a person, never by an operation: the environment variable
//!   `MARIONETTE_SPEND_CAP_USD`, else `spend_cap_usd` in the user config file
//!   (`marionette/config.json` in the user's config folder, the same file that names the owned
//!   library), else zero. With a zero cap every paid call is refused.
//! - A person may also give one provider its own cap (decision 0045):
//!   `MARIONETTE_SPEND_CAP_<PROVIDER>_USD` (for example `MARIONETTE_SPEND_CAP_TRIPO_USD=10`), else
//!   `provider_caps_usd: {"tripo": 10, "qwen": "unlimited"}` in the user config file. A
//!   provider with its own cap is checked against that cap and its own spending only;
//!   `unlimited` means its calls are not capped. Other providers share the total cap, counted
//!   over their own spending.
//! - The ledger is `<project>/.marionette/spend.jsonl`: one JSON object per paid call. An
//!   unreadable ledger refuses paid calls rather than guessing.

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

/// A provider's own limit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Limit {
    Usd(f64),
    Unlimited,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, JsonSchema)]
pub struct ProviderCap {
    pub limit: Limit,
    pub source: CapSource,
}

/// Caps set for single providers, by provider name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct ProviderCaps(pub std::collections::BTreeMap<String, ProviderCap>);

/// Environment variable for one provider's cap.
pub fn provider_cap_env(provider: &str) -> String {
    format!("MARIONETTE_SPEND_CAP_{}_USD", provider.to_ascii_uppercase())
}

fn parse_limit(v: &serde_json::Value, what: &str) -> Result<Limit, Errors> {
    let bad = || -> Errors {
        Diagnostic::error(
            codes::SPEND_CAP,
            format!("{what} is not a number of US dollars or `unlimited`"),
        )
        .fix("use a number, zero or more, or the word unlimited")
        .into()
    };
    match v {
        serde_json::Value::String(s) if s.trim().eq_ignore_ascii_case("unlimited") => Ok(Limit::Unlimited),
        serde_json::Value::String(s) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|u| u.is_finite() && *u >= 0.0)
            .map(Limit::Usd)
            .ok_or_else(bad),
        serde_json::Value::Number(n) => n
            .as_f64()
            .filter(|u| u.is_finite() && *u >= 0.0)
            .map(Limit::Usd)
            .ok_or_else(bad),
        _ => Err(bad()),
    }
}

/// Resolves per-provider caps for every paid provider: environment first, then the user
/// config file. Providers without one are absent (they share the total cap).
pub fn resolve_provider_caps(var: &dyn Fn(&str) -> Option<String>) -> Result<ProviderCaps, Errors> {
    let file: Option<serde_json::Value> = user_config_file(var)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok());
    let mut out = ProviderCaps::default();
    for p in crate::all() {
        let info = p.info();
        if info.key_env.is_none() {
            continue;
        }
        let env_name = provider_cap_env(info.name);
        if let Some(v) = var(&env_name).filter(|v| !v.trim().is_empty()) {
            let limit = parse_limit(&serde_json::Value::String(v), &env_name)?;
            out.0.insert(
                info.name.into(),
                ProviderCap {
                    limit,
                    source: CapSource::Environment,
                },
            );
        } else if let Some(v) = file
            .as_ref()
            .and_then(|f| f.pointer(&format!("/provider_caps_usd/{}", info.name)))
        {
            let limit = parse_limit(v, &format!("provider_caps_usd.{}", info.name))?;
            out.0.insert(
                info.name.into(),
                ProviderCap {
                    limit,
                    source: CapSource::UserConfig,
                },
            );
        }
    }
    Ok(out)
}

/// Which spending a provider's call is checked against: its own cap, or the shared total.
pub fn scope<'a>(caps: &'a ProviderCaps, provider: &str) -> Option<&'a ProviderCap> {
    caps.0.get(provider)
}

/// Spending that counts toward `provider`'s cap: its own entries when it has its own cap,
/// else the entries of every provider without one.
pub fn spent_in_scope(entries: &[Entry], caps: &ProviderCaps, provider: &str) -> f64 {
    let own = caps.0.contains_key(provider);
    entries
        .iter()
        .filter(|e| {
            if own {
                e.provider == provider
            } else {
                !caps.0.contains_key(&e.provider)
            }
        })
        .fold(0.0, |acc, e| acc + e.cost_usd)
}

/// Checks one call (plus `pending` estimates already planned in the same scope) against the
/// cap that applies to `provider`.
pub fn check_for(
    total: &SpendCap,
    caps: &ProviderCaps,
    provider: &str,
    entries: &[Entry],
    pending_usd: f64,
    estimate_usd: f64,
) -> Result<(), Errors> {
    let spent = spent_in_scope(entries, caps, provider) + pending_usd;
    match scope(caps, provider) {
        Some(ProviderCap {
            limit: Limit::Unlimited,
            ..
        }) => Ok(()),
        Some(ProviderCap {
            limit: Limit::Usd(cap),
            ..
        }) => {
            if spent + estimate_usd <= *cap {
                return Ok(());
            }
            Err(Diagnostic::error(
                codes::SPEND_CAP,
                format!(
                    "this `{provider}` call may cost up to ${estimate_usd:.4}; `{provider}` spent so far ${spent:.4}; its cap ${cap:.4}"
                ),
            )
            .fix("only the project owner may raise this provider's cap; ask them, or use `mock`")
            .into())
        }
        None => check(total, spent, estimate_usd),
    }
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
