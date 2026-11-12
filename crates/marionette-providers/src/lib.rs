//! AI providers for Marionette: language (text and code), image and 3D model generation
//! behind one small interface, so a backend is swapped by changing one line of the project's
//! `providers.json` (operation `provider_use`, or `marionette provider use <kind> <name>`).
//!
//! - [`Provider`] is the plug-in interface. Built in: [`mock`] (offline, free, deterministic;
//!   the default for every kind), [`claude`] (language), [`qwen`] (image), [`tripo`] (3D).
//! - Network plug-ins build [`http::HttpRequest`] values and parse [`http::HttpResponse`]
//!   values; the bytes move through a [`http::Transport`]. This build ships no network
//!   transport ([`http::NoNetwork`]), so nothing here can spend money yet (decision 0019).
//! - Keys come from environment variables only ([`ProviderInfo::key_env`]); the config file
//!   refuses any field it does not know, so a key cannot be stored in it.
//! - Every paid call is checked against a spend cap that only a person sets ([`spend`]).
//!
//! Providers run outside the simulation; nothing here is called during a tick.

pub mod claude;
pub mod config;
mod glb;
pub mod http;
pub mod mock;
pub mod qwen;
pub mod spend;
pub mod tripo;

use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub use config::{Choice, ProvidersConfig};
pub use http::{HttpRequest, HttpResponse, NoNetwork, Transport};
pub use spend::{CapSource, SpendCap};

/// Stable error codes added by this crate.
pub mod codes {
    /// No provider with that name exists.
    pub const UNKNOWN_PROVIDER: &str = "provider.unknown";
    /// The provider does not offer that kind (for example Tripo for language).
    pub const KIND_UNSUPPORTED: &str = "provider.kind_unsupported";
    /// The provider's key environment variable is not set.
    pub const KEY_MISSING: &str = "provider.key_missing";
    /// The provider needs the network and this build has no network transport.
    pub const NETWORK_DISABLED: &str = "provider.network_disabled";
    /// The call's estimated cost would take spending past the cap.
    pub const SPEND_CAP: &str = "provider.spend_cap";
    /// `providers.json` is malformed or holds an unknown field.
    pub const BAD_CONFIG: &str = "provider.bad_config";
    /// The request is invalid (empty prompt, size out of range, missing input).
    pub const BAD_REQUEST: &str = "provider.bad_request";
    /// The service answered with an HTTP error status, or the transport failed.
    pub const HTTP: &str = "provider.http";
    /// The service's answer could not be understood.
    pub const BAD_RESPONSE: &str = "provider.bad_response";
    /// The service reported that the generation job failed.
    pub const JOB_FAILED: &str = "provider.job_failed";
    /// The generation job did not finish within the poll limit.
    pub const JOB_TIMEOUT: &str = "provider.job_timeout";
    /// The model declined the request.
    pub const REFUSED: &str = "provider.refused";
    /// The spend ledger cannot be read or written; no paid call is made.
    pub const LEDGER: &str = "provider.ledger";
}

/// What a provider generates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Text and code.
    Language,
    /// Images (PNG).
    Image,
    /// 3D models (GLB).
    Model3d,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Language, Kind::Image, Kind::Model3d];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Language => "language",
            Kind::Image => "image",
            Kind::Model3d => "model3d",
        }
    }
}

/// A text or code request.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRequest {
    pub system: Option<String>,
    pub prompt: String,
    /// Upper bound on the reply length, in tokens.
    pub max_tokens: u32,
}

/// An image request. The result is always PNG.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRequest {
    pub prompt: String,
    pub width: u32,
    pub height: u32,
}

/// A 3D model request: from a text prompt, or from a reference image URL (or both). The
/// result is always GLB.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelRequest {
    pub prompt: Option<String>,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Text(TextRequest),
    Image(ImageRequest),
    Model(ModelRequest),
}

/// Size limits for image requests, in pixels per side.
pub const MIN_IMAGE_SIDE: u32 = 16;
pub const MAX_IMAGE_SIDE: u32 = 4096;
/// Upper bound on `max_tokens`.
pub const MAX_TOKENS_LIMIT: u32 = 128_000;

impl Request {
    pub fn kind(&self) -> Kind {
        match self {
            Request::Text(_) => Kind::Language,
            Request::Image(_) => Kind::Image,
            Request::Model(_) => Kind::Model3d,
        }
    }

    /// Checks the request before any provider sees it.
    pub fn validate(&self) -> Result<(), Errors> {
        let bad = |msg: String, fix: &str| Err(Diagnostic::error(codes::BAD_REQUEST, msg).fix(fix).into());
        match self {
            Request::Text(t) => {
                if t.prompt.trim().is_empty() {
                    return bad("the prompt is empty".into(), "give a prompt");
                }
                if t.max_tokens == 0 || t.max_tokens > MAX_TOKENS_LIMIT {
                    return bad(
                        format!("max_tokens {} is outside 1..={MAX_TOKENS_LIMIT}", t.max_tokens),
                        "use a max_tokens value in range",
                    );
                }
            }
            Request::Image(i) => {
                if i.prompt.trim().is_empty() {
                    return bad("the prompt is empty".into(), "give a prompt");
                }
                for (name, v) in [("width", i.width), ("height", i.height)] {
                    if !(MIN_IMAGE_SIDE..=MAX_IMAGE_SIDE).contains(&v) {
                        return bad(
                            format!("{name} {v} is outside {MIN_IMAGE_SIDE}..={MAX_IMAGE_SIDE}"),
                            "use a size in range",
                        );
                    }
                }
            }
            Request::Model(m) => {
                let empty = |s: &Option<String>| s.as_deref().is_none_or(|s| s.trim().is_empty());
                if empty(&m.prompt) && empty(&m.image_url) {
                    return bad(
                        "a 3D request needs a prompt or an image_url".into(),
                        "give a prompt, an image_url, or both",
                    );
                }
            }
        }
        Ok(())
    }
}

/// Token counts reported by a language provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    Text(String),
    /// PNG bytes.
    Image(Vec<u8>),
    /// GLB bytes.
    Model(Vec<u8>),
}

/// What a provider returns for one request.
#[derive(Debug, Clone, PartialEq)]
pub struct Generated {
    pub output: Output,
    /// Cost in US dollars as far as the provider knows it (token-priced calls use the
    /// reported usage; job-priced calls use the price table).
    pub cost_usd: f64,
    pub usage: Option<Usage>,
}

/// Static description of a provider.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProviderInfo {
    /// Name used in `providers.json` and operations.
    pub name: &'static str,
    pub title: &'static str,
    pub kinds: &'static [Kind],
    /// Environment variable holding the API key; `None` for providers that need none.
    pub key_env: Option<&'static str>,
    /// Needs the network.
    pub network: bool,
    /// Model used when `providers.json` names none, per kind.
    pub default_models: &'static [(Kind, &'static str)],
    /// The request and response formats have been checked against the live service. False
    /// means they were written from documentation and tested only against recorded shapes.
    pub verified: bool,
    pub notes: &'static str,
}

impl ProviderInfo {
    pub fn supports(&self, kind: Kind) -> bool {
        self.kinds.contains(&kind)
    }

    pub fn default_model(&self, kind: Kind) -> Option<&'static str> {
        self.default_models
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, m)| *m)
    }
}

/// Everything a provider needs for one call.
pub struct Ctx<'a> {
    pub model: &'a str,
    /// The API key, if the provider needs one. Never logged or returned.
    pub key: Option<&'a str>,
    pub transport: &'a dyn Transport,
    /// Most status checks a job-based provider makes before giving up.
    pub max_polls: u32,
    /// Called before each status check after the first (production: sleeps; tests: nothing).
    pub wait: &'a dyn Fn(u32),
}

/// The plug-in interface. Implementations hold no state between calls.
pub trait Provider: Send + Sync {
    fn info(&self) -> &'static ProviderInfo;
    /// Upper estimate of what `request` costs with `model`, in US dollars. Zero for free
    /// providers. Unknown models are priced at the provider's most expensive known model.
    fn estimate_usd(&self, model: &str, request: &Request) -> f64;
    fn run(&self, ctx: &Ctx, request: &Request) -> Result<Generated, Errors>;
}

/// Every built-in provider, in a stable order.
pub fn all() -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(mock::Mock),
        Box::new(claude::Claude),
        Box::new(qwen::Qwen),
        Box::new(tripo::Tripo),
    ]
}

pub fn find(name: &str) -> Result<Box<dyn Provider>, Errors> {
    all().into_iter().find(|p| p.info().name == name).ok_or_else(|| {
        let names: Vec<&str> = all().iter().map(|p| p.info().name).collect();
        Diagnostic::error(codes::UNKNOWN_PROVIDER, format!("no provider named `{name}`"))
            .fix(format!("providers: {}", names.join(", ")))
            .into()
    })
}

/// The provider and model a kind resolves to under `config`.
pub fn resolve(config: &ProvidersConfig, kind: Kind) -> Result<(Box<dyn Provider>, String), Errors> {
    let choice = config.choice(kind);
    let provider = find(&choice.provider)?;
    let info = provider.info();
    if !info.supports(kind) {
        let names: Vec<&str> = all()
            .iter()
            .filter(|p| p.info().supports(kind))
            .map(|p| p.info().name)
            .collect();
        return Err(Diagnostic::error(
            codes::KIND_UNSUPPORTED,
            format!("provider `{}` does not offer {}", info.name, kind.as_str()),
        )
        .fix(format!("{} providers: {}", kind.as_str(), names.join(", ")))
        .into());
    }
    let model = match &choice.model {
        Some(m) => m.clone(),
        None => info
            .default_model(kind)
            .expect("every offered kind has a default model")
            .to_string(),
    };
    Ok((provider, model))
}

/// Why a kind cannot run right now, if it cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Readiness {
    Ready,
    KeyMissing,
    NetworkDisabled,
}

/// Checks key and network for a provider without calling it.
pub fn readiness(
    info: &ProviderInfo,
    env: &dyn Fn(&str) -> Option<String>,
    transport: &dyn Transport,
) -> Readiness {
    if let Some(var) = info.key_env
        && env(var).is_none_or(|v| v.trim().is_empty())
    {
        return Readiness::KeyMissing;
    }
    if info.network && !transport.available() {
        return Readiness::NetworkDisabled;
    }
    Readiness::Ready
}

/// Outside-world access for [`generate`]: injected so tests never touch the real
/// environment or network.
pub struct Env<'a> {
    /// Reads an environment variable.
    pub var: &'a dyn Fn(&str) -> Option<String>,
    pub transport: &'a dyn Transport,
    /// The spend ledger (`<project>/.marionette/spend.jsonl`).
    pub ledger: &'a Path,
    pub cap: SpendCap,
    pub max_polls: u32,
    pub wait: &'a dyn Fn(u32),
}

/// Result of [`generate`].
#[derive(Debug, Clone)]
pub struct Generation {
    pub provider: &'static str,
    pub model: String,
    pub generated: Generated,
    /// Ledger total after this call.
    pub spent_usd: f64,
}

/// Runs one request through the provider configured for its kind. Order of checks: request,
/// provider and kind, key, network, spend cap; then the call; then the ledger entry.
pub fn generate(config: &ProvidersConfig, env: &Env, request: &Request) -> Result<Generation, Errors> {
    request.validate()?;
    let kind = request.kind();
    let (provider, model) = resolve(config, kind)?;
    let info = provider.info();
    let key = match info.key_env {
        Some(var) => match (env.var)(var).filter(|v| !v.trim().is_empty()) {
            Some(k) => Some(k),
            None => {
                return Err(Diagnostic::error(
                    codes::KEY_MISSING,
                    format!(
                        "provider `{}` needs its key in the environment variable {var}",
                        info.name
                    ),
                )
                .fix(format!(
                    "set {var} in the environment (keys never go in files), or switch {} back to `mock`",
                    kind.as_str()
                ))
                .into());
            }
        },
        None => None,
    };
    if info.network && !env.transport.available() {
        return Err(Diagnostic::error(
            codes::NETWORK_DISABLED,
            format!("provider `{}` needs the network and this build has no network transport", info.name),
        )
        .fix(format!(
            "use `mock` for {} for now; network calls are enabled in a later build, after the project owner approves spending (decision 0019)",
            kind.as_str()
        ))
        .into());
    }
    let estimate = provider.estimate_usd(&model, request);
    // Paid calls need a readable ledger; free calls only report its total when they can.
    let spent_before = if estimate > 0.0 {
        let spent = spend::total(env.ledger)?;
        spend::check(&env.cap, spent, estimate)?;
        spent
    } else {
        spend::total(env.ledger).unwrap_or(0.0)
    };
    let ctx = Ctx {
        model: &model,
        key: key.as_deref(),
        transport: env.transport,
        max_polls: env.max_polls,
        wait: env.wait,
    };
    let generated = provider.run(&ctx, request)?;
    let mut spent_usd = spent_before;
    if generated.cost_usd > 0.0 {
        spend::record(
            env.ledger,
            &spend::Entry {
                provider: info.name.to_string(),
                model: model.clone(),
                kind,
                cost_usd: generated.cost_usd,
                estimate_usd: estimate,
            },
        )?;
        spent_usd += generated.cost_usd;
    }
    Ok(Generation {
        provider: info.name,
        model,
        generated,
        spent_usd,
    })
}

/// Lowercase hex SHA-256.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).into()
}

/// Shared helpers for network plug-ins.
pub(crate) mod net {
    use super::codes;
    use crate::http::{HttpRequest, HttpResponse, Transport};
    use marionette_core::{Diagnostic, Errors};
    use serde_json::Value;

    pub fn send(
        transport: &dyn Transport,
        req: &HttpRequest,
        provider: &str,
    ) -> Result<HttpResponse, Errors> {
        let resp = transport.send(req).map_err(|e| {
            Errors::from(
                Diagnostic::error(
                    codes::HTTP,
                    format!("{provider}: request to {} failed: {e}", req.url),
                )
                .fix("check the network connection and try again"),
            )
        })?;
        if !(200..300).contains(&resp.status) {
            let detail = serde_json::from_slice::<Value>(&resp.body)
                .ok()
                .and_then(|v| error_message(&v))
                .unwrap_or_default();
            let fix = match resp.status {
                401 | 403 => "check the key in the provider's environment variable",
                429 => "the service is rate limiting; wait and try again",
                s if s >= 500 => "the service had an error; try again later",
                _ => "check the request against the provider's documentation",
            };
            return Err(Diagnostic::error(
                codes::HTTP,
                format!(
                    "{provider}: HTTP {} from {}{}",
                    resp.status,
                    req.url,
                    prefixed(&detail)
                ),
            )
            .fix(fix)
            .into());
        }
        Ok(resp)
    }

    fn prefixed(detail: &str) -> String {
        if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        }
    }

    /// Finds an error message in common API error shapes.
    fn error_message(v: &Value) -> Option<String> {
        for ptr in ["/error/message", "/message", "/msg"] {
            if let Some(s) = v.pointer(ptr).and_then(Value::as_str) {
                return Some(s.chars().take(300).collect());
            }
        }
        None
    }

    pub fn json(resp: &HttpResponse, provider: &str) -> Result<Value, Errors> {
        serde_json::from_slice(&resp.body)
            .map_err(|e| bad_response(provider, format!("answer is not JSON: {e}")))
    }

    pub fn bad_response(provider: &str, msg: impl std::fmt::Display) -> Errors {
        Diagnostic::error(codes::BAD_RESPONSE, format!("{provider}: {msg}"))
            .fix("the service's answer format may have changed; report it so the plug-in can be updated")
            .into()
    }

    pub fn str_at<'a>(v: &'a Value, ptr: &str, provider: &str) -> Result<&'a str, Errors> {
        v.pointer(ptr)
            .and_then(Value::as_str)
            .ok_or_else(|| bad_response(provider, format!("answer has no string at {ptr}")))
    }

    /// Polls `check` until it returns `Some`, calling `wait` between checks.
    pub fn poll<T>(
        max_polls: u32,
        wait: &dyn Fn(u32),
        provider: &str,
        mut check: impl FnMut() -> Result<Option<T>, Errors>,
    ) -> Result<T, Errors> {
        for i in 0..max_polls.max(1) {
            if i > 0 {
                wait(i);
            }
            if let Some(done) = check()? {
                return Ok(done);
            }
        }
        Err(Diagnostic::error(
            codes::JOB_TIMEOUT,
            format!(
                "{provider}: the job did not finish after {} status checks",
                max_polls.max(1)
            ),
        )
        .fix("try again later, or raise the poll limit")
        .into())
    }

    pub fn job_failed(provider: &str, status: &str, detail: Option<&str>) -> Errors {
        Diagnostic::error(
            codes::JOB_FAILED,
            format!(
                "{provider}: the job ended with status `{status}`{}",
                detail.map(|d| format!(": {d}")).unwrap_or_default()
            ),
        )
        .fix("change the prompt or input and try again")
        .into()
    }

    pub fn check_png(bytes: &[u8], provider: &str) -> Result<(), Errors> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Ok(())
        } else {
            Err(bad_response(provider, "the downloaded image is not PNG"))
        }
    }

    pub fn check_glb(bytes: &[u8], provider: &str) -> Result<(), Errors> {
        if bytes.starts_with(b"glTF") {
            Ok(())
        } else {
            Err(bad_response(provider, "the downloaded model is not GLB"))
        }
    }
}
