//! Provider operations: list providers, show the project's choices and spending, switch a
//! provider with one call, and generate text, images and 3D models through whichever
//! provider is configured. Design: decision 0019; crate `marionette-providers`.
//!
//! Keys come only from environment variables; the spend cap is set only by a person
//! (`MARIONETTE_SPEND_CAP_USD` or the user config file); no operation writes either.

use crate::{Agent, Op, op};
use marionette_core::{Diagnostic, Errors};
use marionette_providers::{
    self as providers, Choice, Env, ImageRequest, Kind, ModelRequest, Output, ProvidersConfig, Readiness,
    Request, SpendCap, TextRequest, Usage, config, spend,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "provider_list",
            "Lists every AI provider: kinds offered (language, image, model3d), key environment variable and whether it is set, network need, default models, and whether its API format has been verified live.",
            false,
            list,
        ),
        op(
            "provider_status",
            "Shows which provider and model each kind uses in this project (providers.json; mock when unset), whether each can run now, money spent (ledger) and the spend cap set by the project owner.",
            false,
            status,
        ),
        op(
            "provider_use",
            "Switches the provider (and optionally the model) for one kind by rewriting providers.json. Never stores keys: they come from environment variables.",
            true,
            use_provider,
        ),
        op(
            "provider_generate",
            "Generates text (language), a PNG (image) or a GLB (model3d) with the configured provider. Paid providers are checked against the spend cap first and every paid call is logged in .marionette/spend.jsonl.",
            true,
            generate,
        ),
    ]
}

/// The network transport of this build (none unless built with the `net` feature).
fn transport() -> Box<dyn providers::Transport> {
    providers::http::default_transport()
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {}

#[derive(Serialize, JsonSchema)]
struct ProviderEntry {
    name: &'static str,
    title: &'static str,
    kinds: Vec<Kind>,
    key_env: Option<&'static str>,
    /// The key variable is set (its value is never shown).
    key_present: bool,
    network: bool,
    readiness: Readiness,
    default_models: Vec<DefaultModel>,
    verified: bool,
    notes: &'static str,
}

#[derive(Serialize, JsonSchema)]
struct DefaultModel {
    kind: Kind,
    model: &'static str,
}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    providers: Vec<ProviderEntry>,
}

fn list(agent: &mut Agent, _: ListInput) -> Result<ListOutput, Errors> {
    let env_var = crate::env_lookup(agent.root());
    let providers = providers::all()
        .iter()
        .map(|p| {
            let info = p.info();
            ProviderEntry {
                name: info.name,
                title: info.title,
                kinds: info.kinds.to_vec(),
                key_env: info.key_env,
                key_present: info
                    .key_env
                    .is_some_and(|v| env_var(v).is_some_and(|k| !k.trim().is_empty())),
                network: info.network,
                readiness: providers::readiness(info, &env_var, transport().as_ref()),
                default_models: info
                    .default_models
                    .iter()
                    .map(|(kind, model)| DefaultModel { kind: *kind, model })
                    .collect(),
                verified: info.verified,
                notes: info.notes,
            }
        })
        .collect();
    Ok(ListOutput { providers })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StatusInput {}

#[derive(Serialize, JsonSchema)]
struct KindStatus {
    kind: Kind,
    provider: String,
    model: String,
    readiness: Readiness,
}

#[derive(Serialize, JsonSchema)]
struct StatusOutput {
    /// `providers.json` exists (otherwise every kind uses mock).
    config_file: bool,
    kinds: Vec<KindStatus>,
    /// Total of the project's spend ledger, US dollars.
    spent_usd: f64,
    cap: SpendCap,
    /// The person's keys file (decision 0045): absent when none is configured, `true` when it
    /// exists, `false` when it is configured but missing. Its values are never shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    keys_file_found: Option<bool>,
    /// Caps the project owner set for single providers (decision 0045).
    provider_caps: providers::ProviderCaps,
}

fn status(agent: &mut Agent, _: StatusInput) -> Result<StatusOutput, Errors> {
    let env_var = crate::env_lookup(agent.root());
    let (cfg, exists) = ProvidersConfig::load(agent.root())?;
    let mut kinds = Vec::new();
    for kind in Kind::ALL {
        let (provider, model) = providers::resolve(&cfg, kind)?;
        kinds.push(KindStatus {
            kind,
            provider: provider.info().name.to_string(),
            model,
            readiness: providers::readiness(provider.info(), &env_var, transport().as_ref()),
        });
    }
    Ok(StatusOutput {
        config_file: exists,
        kinds,
        spent_usd: spend::total(&agent.root().join(spend::LEDGER_PATH))?,
        cap: spend::resolve_cap(&env_var)?,
        keys_file_found: providers::keys::status(&|n| std::env::var(n).ok(), agent.root())?,
        provider_caps: spend::resolve_provider_caps(&env_var)?,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UseInput {
    kind: Kind,
    /// Provider name from `provider_list`.
    provider: String,
    /// Model name; the provider's default when absent.
    #[serde(default)]
    model: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct UseOutput {
    kind: Kind,
    provider: String,
    model: String,
    /// The previous provider and model for this kind.
    previous: Choice,
    readiness: Readiness,
    file: &'static str,
}

fn use_provider(agent: &mut Agent, input: UseInput) -> Result<UseOutput, Errors> {
    let env_var = crate::env_lookup(agent.root());
    let (mut cfg, _) = ProvidersConfig::load(agent.root())?;
    let previous = cfg.choice(input.kind);
    if let Some(m) = &input.model
        && m.trim().is_empty()
    {
        return Err(
            Diagnostic::error(providers::codes::BAD_REQUEST, "the model name is empty")
                .fix("leave `model` out to use the provider's default")
                .into(),
        );
    }
    cfg.set(
        input.kind,
        Choice {
            provider: input.provider.clone(),
            model: input.model.clone(),
        },
    );
    // Validates the provider name and that it offers this kind before anything is written.
    let (provider, model) = providers::resolve(&cfg, input.kind)?;
    let path = ProvidersConfig::path(agent.root());
    crate::write_atomic(&path, config::FILE_NAME, cfg.to_text().as_bytes())?;
    Ok(UseOutput {
        kind: input.kind,
        provider: provider.info().name.to_string(),
        model,
        previous,
        readiness: providers::readiness(provider.info(), &env_var, transport().as_ref()),
        file: config::FILE_NAME,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GenerateInput {
    kind: Kind,
    /// What to generate. Optional for model3d when `image_url` is given.
    #[serde(default)]
    prompt: Option<String>,
    /// Language only: system instructions.
    #[serde(default)]
    system: Option<String>,
    /// Language only: reply length limit in tokens (default 1024).
    #[serde(default)]
    max_tokens: Option<u32>,
    /// Image only: pixels (default 512 each).
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    /// Model3d only: reference image URL.
    #[serde(default)]
    image_url: Option<String>,
    /// Image and model3d: output file (PNG or GLB), relative to the project. Required for
    /// those kinds.
    #[serde(default)]
    out: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct GenerateOutput {
    kind: Kind,
    provider: &'static str,
    model: String,
    /// Language: the reply.
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    /// Image and model3d: the file written.
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    /// SHA-256 of the text (UTF-8) or of the file.
    sha256: String,
    bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<Usage>,
    cost_usd: f64,
    /// Ledger total after this call.
    spent_usd: f64,
}

fn generate(agent: &mut Agent, input: GenerateInput) -> Result<GenerateOutput, Errors> {
    let env_var = crate::env_lookup(agent.root());
    let prompt = input.prompt.clone().unwrap_or_default();
    let request = match input.kind {
        Kind::Language => Request::Text(TextRequest {
            system: input.system.clone(),
            prompt,
            max_tokens: input.max_tokens.unwrap_or(1024),
        }),
        Kind::Image => Request::Image(ImageRequest {
            prompt,
            width: input.width.unwrap_or(512),
            height: input.height.unwrap_or(512),
        }),
        Kind::Model3d => Request::Model(ModelRequest {
            prompt: input.prompt.clone(),
            image_url: input.image_url.clone(),
            ..Default::default()
        }),
    };
    let out = match (input.kind, &input.out) {
        (Kind::Language, _) => None,
        (_, Some(o)) => Some((agent.resolve(o)?, o.clone())),
        (kind, None) => {
            let ext = if kind == Kind::Image { "png" } else { "glb" };
            return Err(Diagnostic::error(
                providers::codes::BAD_REQUEST,
                format!("{} needs an `out` file", kind.as_str()),
            )
            .fix(format!("add \"out\": \"generated/<name>.{ext}\""))
            .into());
        }
    };
    let (cfg, _) = ProvidersConfig::load(agent.root())?;
    let ledger = agent.root().join(spend::LEDGER_PATH);
    let wait = |_: u32| std::thread::sleep(std::time::Duration::from_secs(3));
    let transport = transport();
    let env = Env {
        var: &env_var,
        transport: transport.as_ref(),
        ledger: &ledger,
        cap: spend::resolve_cap(&env_var)?,
        provider_caps: &spend::resolve_provider_caps(&env_var)?,
        max_polls: 200,
        wait: &wait,
    };
    let generation = providers::generate(&cfg, &env, &request)?;
    let (text, bytes) = match generation.generated.output {
        Output::Text(t) => (Some(t.clone()), t.into_bytes()),
        Output::Image(b) | Output::Model(b) => (None, b),
    };
    if let Some((path, shown)) = &out {
        crate::write_atomic(path, shown, &bytes)?;
    }
    Ok(GenerateOutput {
        kind: input.kind,
        provider: generation.provider,
        model: generation.model,
        text,
        file: out.map(|(_, shown)| shown),
        sha256: crate::sha256_hex(&bytes),
        bytes: bytes.len(),
        usage: generation.generated.usage,
        cost_usd: generation.generated.cost_usd,
        spent_usd: generation.spent_usd,
    })
}
