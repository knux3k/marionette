//! Asset generation, the last step of the asset lookup order (project files, owned library,
//! CC0 pool, online CC0 libraries, then generation; ASSETS-AND-LICENSES.md). Design: decision
//! 0045. Guide: `docs/ai/GENERATION.md`.
//!
//! - A generated asset is a [`recipe`] (`<name>.recipe.json`) plus a cached provider output
//!   ([`cache`]) keyed by the request, so the same request is never paid for twice and every
//!   asset can be rebuilt from its recipe offline.
//! - Images and textures come from the project's image provider (Qwen in production), models
//!   from its 3D provider (Tripo), both through `marionette-providers` with its spend cap and
//!   ledger. The mock provider is the default and is free.
//! - A model made from an image needs a person's [`approval`] of that exact image when the 3D
//!   provider is paid.
//! - Outputs are post-processed into engine formats ([`texture`], [`model`]) and get a
//!   [`provenance`] record. Generated assets are blocked from the CC0 pool.
//!
//! Nothing here runs during a simulation tick.

pub mod approval;
pub mod cache;
pub mod model;
pub mod provenance;
pub mod recipe;
pub mod slim;
pub mod texture;

use marionette_core::{Diagnostic, Errors};
use marionette_providers::{
    self as providers, ImageRequest, Kind, ModelRequest, Output, Provider, ProviderCaps, ProvidersConfig,
    Readiness, Request, SpendCap, Transport, spend,
};
use recipe::{Loaded, OutputKind, Recipe};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Stable error codes added by this crate.
pub mod codes {
    /// A recipe file is malformed or inconsistent.
    pub const BAD_RECIPE: &str = "generate.bad_recipe";
    /// No recipe with that name in the folder.
    pub const NOT_FOUND: &str = "generate.not_found";
    /// A paid model would be made from an image no person has approved.
    pub const NOT_APPROVED: &str = "generate.not_approved";
    /// Only a person at an interactive terminal may approve an image.
    pub const APPROVAL_NEEDS_PERSON: &str = "generate.approval_needs_person";
    /// A tool tried to write an approval file.
    pub const APPROVAL_WRITE_REFUSED: &str = "generate.approval_write_refused";
    /// An offline run needs a provider call that is not in the cache.
    pub const NOT_CACHED: &str = "generate.not_cached";
    /// A cache entry is damaged.
    pub const CACHE: &str = "generate.cache";
    /// Post-processing failed (the provider's output is not a usable PNG or GLB).
    pub const POSTPROCESS: &str = "generate.postprocess";
    /// The model step waits for its reference image, which this run did not produce.
    pub const WAITS_FOR_IMAGE: &str = "generate.waits_for_image";
}

/// Lowercase hex SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes a file atomically. Refuses license declarations (`*.license.json`) and image
/// approvals (`*.approval.json`): only a person writes those.
pub fn write_file(path: &Path, shown: &str, bytes: &[u8]) -> Result<(), Errors> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if approval::is_approval_file_name(&name) {
        return Err(approval::write_refused(shown).into());
    }
    if name.ends_with(".license.json") {
        return Err(Diagnostic::error(
            "import.declaration_write_refused",
            format!("`{shown}` is a license declaration; only a person may write one"),
        )
        .file(shown)
        .into());
    }
    let err = |e: std::io::Error| {
        Errors::from(
            Diagnostic::error(
                marionette_core::error::codes::IO_WRITE,
                format!("cannot write {shown}: {e}"),
            )
            .file(shown),
        )
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    let tmp = path.with_extension("tmp-write");
    std::fs::write(&tmp, bytes).map_err(err)?;
    std::fs::rename(&tmp, path).map_err(err)
}

/// Outside-world access, injected so tests never touch the real environment or network.
pub struct Env<'a> {
    pub var: &'a dyn Fn(&str) -> Option<String>,
    pub transport: &'a dyn Transport,
    /// The total cap (for providers without their own).
    pub cap: SpendCap,
    /// Caps a person set for single providers.
    pub provider_caps: &'a ProviderCaps,
    pub max_polls: u32,
    pub wait: &'a dyn Fn(u32),
}

/// What to work on.
pub struct Target<'a> {
    /// Project root.
    pub root: &'a Path,
    /// Recipe folder, relative to the root (shown in messages).
    pub dir: &'a str,
    /// Recipe names; empty means all. A model's `from` image is included automatically.
    pub names: &'a [String],
}

impl Target<'_> {
    fn dir_path(&self) -> PathBuf {
        self.root.join(self.dir)
    }
    fn ledger(&self) -> PathBuf {
        self.root.join(spend::LEDGER_PATH)
    }
}

/// A problem as data (code, message, fix).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Problem {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl From<&Errors> for Problem {
    fn from(e: &Errors) -> Self {
        let d = &e.0[0];
        Problem {
            code: d.code.to_string(),
            message: d.message.clone(),
            fix: d.fix.clone(),
        }
    }
}

fn problem(code: &str, message: String, fix: &str) -> Problem {
    Problem {
        code: code.into(),
        message,
        fix: Some(fix.into()),
    }
}

/// What will happen to one recipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// The output, its record and the recipe match: nothing to do.
    UpToDate,
    /// The provider's output is cached: rebuilt for free.
    Cached,
    /// Needs a provider call (free for mock, paid otherwise).
    Call,
    /// A model whose reference image is not built yet; it runs after the image.
    WaitsForImage,
    /// Cannot run now; see `blocked`.
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct StepPlan {
    pub name: String,
    pub output: OutputKind,
    /// Output file, relative to the recipe folder.
    pub file: String,
    pub provider: String,
    pub model: String,
    pub provider_verified: bool,
    /// Known once the request is known (for a model from an image: once the image exists).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_key: Option<String>,
    pub status: StepStatus,
    /// Upper estimate of what this step's call costs (zero when cached or free).
    pub estimate_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// A person must approve the `from` image first (paid 3D provider).
    pub needs_approval: bool,
    pub approved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<Problem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Plan {
    pub dir: String,
    pub steps: Vec<StepPlan>,
    /// Sum of the estimates of every step that needs a provider call now (blocked or not).
    pub total_estimate_usd: f64,
    /// Ledger total so far.
    pub spent_usd: f64,
    pub cap: SpendCap,
    /// Every step that needs a paid call fits under the cap that applies to its provider,
    /// counting the run's earlier steps.
    pub within_cap: bool,
    /// Caps set for single providers (they replace the total cap for that provider).
    pub provider_caps: ProviderCaps,
    /// Generation is the last step of the lookup order; these come first.
    pub before_generating: Vec<String>,
}

/// The lookup steps an agent must try before generating (ASSETS-AND-LICENSES.md rule 1).
pub fn lookup_order() -> Vec<String> {
    [
        "1. the project's own files",
        "2. the owned library (`library_list`, `library_get`)",
        "3. the CC0 pool (not built yet)",
        "4. free CC0 libraries online: Poly Haven, ambientCG, Kenney, Quaternius (not built yet)",
        "5. generation (this step)",
    ]
    .map(String::from)
    .to_vec()
}

/// A resolved step: provider, model, request.
struct Prepared {
    provider: Box<dyn Provider>,
    model: String,
    config: ProvidersConfig,
    request: Request,
    key: String,
    image: Option<(String, String)>, // (file name, sha256) of the reference image
}

/// Provider choice for a recipe: its pin; else, while the recipe is unchanged since its
/// output was built, the provider and model recorded in its provenance (so switching
/// `providers.json` never invalidates assets already made); else `providers.json`.
fn config_for(base: &ProvidersConfig, dir: &Path, l: &Loaded) -> ProvidersConfig {
    let r = &l.recipe;
    let mut c = base.clone();
    if let Some(pin) = &r.provider {
        c.set(r.output.provider_kind(), pin.clone());
    } else if let Some(p) = provenance::read(dir, &r.name)
        && p.recipe_sha256 == l.sha256
    {
        c.set(
            r.output.provider_kind(),
            marionette_providers::Choice {
                provider: p.source.provider,
                model: Some(p.source.model),
            },
        );
    }
    c
}

fn prepare(
    base: &ProvidersConfig,
    dir: &Path,
    l: &Loaded,
    image: Option<(&str, &[u8])>,
) -> Result<Prepared, Errors> {
    let r = &l.recipe;
    let config = config_for(base, dir, l);
    let kind = r.output.provider_kind();
    let (provider, model) = providers::resolve(&config, kind)?;
    let prompt = r.prompt.as_deref().map(str::trim).filter(|p| !p.is_empty());
    let image_sha = image.map(|(_, b)| sha256_hex(b));
    let (request, size) = match r.output {
        OutputKind::Image | OutputKind::Texture => {
            let [width, height] = r.size();
            (
                Request::Image(ImageRequest {
                    prompt: prompt.unwrap_or_default().to_string(),
                    width,
                    height,
                }),
                Some([width, height]),
            )
        }
        OutputKind::Model => (
            Request::Model(ModelRequest {
                prompt: prompt.map(String::from),
                image_url: None,
                image_png: image.map(|(_, b)| b.to_vec()),
            }),
            None,
        ),
    };
    let key = cache::key(&cache::KeyInput {
        provider: provider.info().name,
        model: &model,
        kind,
        prompt,
        size,
        image_sha256: image_sha.as_deref(),
    });
    Ok(Prepared {
        provider,
        model,
        config,
        request,
        key,
        image: image.map(|(f, _)| f.to_string()).zip(image_sha),
    })
}

/// Selected recipes in run order (images and textures first, then models), each with its
/// dependencies.
fn select(all: Vec<Loaded>, names: &[String], dir: &str) -> Result<Vec<Loaded>, Errors> {
    let mut wanted: Vec<String> = Vec::new();
    for n in names {
        let Some(l) = all.iter().find(|l| &l.recipe.name == n) else {
            let have: Vec<&str> = all.iter().map(|l| l.recipe.name.as_str()).collect();
            return Err(
                Diagnostic::error(codes::NOT_FOUND, format!("no recipe `{n}` in {dir}"))
                    .fix(if have.is_empty() {
                        "write one with `generate_recipe`".to_string()
                    } else {
                        format!("recipes: {}", have.join(", "))
                    })
                    .into(),
            );
        };
        wanted.push(n.clone());
        if let Some(f) = &l.recipe.from {
            wanted.push(f.clone());
        }
    }
    let mut out: Vec<Loaded> = all
        .into_iter()
        .filter(|l| names.is_empty() || wanted.contains(&l.recipe.name))
        .collect();
    out.sort_by_key(|l| (l.recipe.output == OutputKind::Model, l.recipe.name.clone()));
    Ok(out)
}

fn info_step(r: &Recipe, p: &Prepared) -> StepPlan {
    StepPlan {
        name: r.name.clone(),
        output: r.output,
        file: r.output_file_name(),
        provider: p.provider.info().name.into(),
        model: p.model.clone(),
        provider_verified: p.provider.info().verified,
        cache_key: Some(p.key.clone()),
        status: StepStatus::Call,
        estimate_usd: 0.0,
        from: r.from.clone(),
        needs_approval: false,
        approved: false,
        blocked: None,
    }
}

fn up_to_date(dir: &Path, l: &Loaded, key: &str) -> bool {
    let r = &l.recipe;
    let Some(p) = provenance::read(dir, &r.name) else {
        return false;
    };
    let Ok(bytes) = std::fs::read(dir.join(r.output_file_name())) else {
        return false;
    };
    p.sha256 == sha256_hex(&bytes) && p.recipe_sha256 == l.sha256 && p.source.cache_key == key
}

/// Post-processes provider bytes for a recipe. Returns the asset bytes and the report.
fn postprocess(r: &Recipe, raw: &[u8]) -> Result<(Vec<u8>, serde_json::Value), Errors> {
    Ok(match r.output {
        OutputKind::Image | OutputKind::Texture => {
            let (b, rep) = texture::process(raw, r.output, r.texture.as_ref())?;
            (b, serde_json::to_value(rep).expect("serializes"))
        }
        OutputKind::Model => {
            let (b, rep) = model::process(raw, r.model.as_ref())?;
            (b, serde_json::to_value(rep).expect("serializes"))
        }
    })
}

/// Checks that would refuse a paid call, in the order the run applies them: approval of the
/// reference image, the spend cap (counting `pending` earlier estimates), the key, the network.
fn paid_checks(
    env: &Env,
    dir: &Path,
    r: &Recipe,
    p: &Prepared,
    estimate: f64,
    entries: &[spend::Entry],
    pending: f64,
) -> Result<(bool, bool), Problem> {
    let waived = approval::waived_providers(env.var)
        .iter()
        .any(|n| n == &p.provider.info().name.to_ascii_lowercase());
    let needs_approval = estimate > 0.0 && r.from.is_some() && !waived;
    let approved = match (&r.from, &p.image) {
        (Some(from), Some((_, sha))) => approval::is_approved(dir, from, sha),
        _ => false,
    };
    if needs_approval && !approved {
        let from = r.from.as_deref().unwrap_or_default();
        return Err(problem(
            codes::NOT_APPROVED,
            format!(
                "`{}` would spend credit on a 3D model made from `{from}`, and no person has approved that image",
                r.name
            ),
            &format!(
                "ask the project owner to look at {from}.png and run `marionette generate approve {from}`"
            ),
        ));
    }
    if estimate > 0.0 {
        if let Err(e) = spend::check_for(
            &env.cap,
            env.provider_caps,
            p.provider.info().name,
            entries,
            pending,
            estimate,
        ) {
            return Err(Problem::from(&e));
        }
        match providers::readiness(p.provider.info(), env.var, env.transport) {
            Readiness::Ready => {}
            Readiness::KeyMissing => {
                let var = p.provider.info().key_env.unwrap_or_default();
                return Err(problem(
                    providers::codes::KEY_MISSING,
                    format!("provider `{}` needs its key in {var}", p.provider.info().name),
                    &format!("set {var} in the environment, or use `mock`"),
                ));
            }
            Readiness::NetworkDisabled => {
                return Err(problem(
                    providers::codes::NETWORK_DISABLED,
                    format!(
                        "provider `{}` needs the network and this build has none",
                        p.provider.info().name
                    ),
                    "build with `--features net` after the project owner approves live calls (decision 0045)",
                ));
            }
        }
    }
    Ok((needs_approval, approved))
}

/// Plans a run without calling any provider or writing anything.
pub fn plan(target: &Target, env: &Env) -> Result<Plan, Errors> {
    let dir = target.dir_path();
    let all = recipe::load_dir(&dir, target.dir)?;
    let selected = select(all, target.names, target.dir)?;
    let (base, _) = ProvidersConfig::load(target.root)?;
    let base = providers::endpoints::apply_models(base, env.var);
    let entries = spend::entries(&target.ledger()).unwrap_or_default();
    let spent = entries.iter().fold(0.0, |acc, e| acc + e.cost_usd);
    // Planned estimates per cap scope: a provider with its own cap, or the shared total ("").
    let mut scoped: BTreeMap<String, f64> = BTreeMap::new();
    let scope_of = |name: &str| -> String {
        if env.provider_caps.0.contains_key(name) {
            name.to_string()
        } else {
            String::new()
        }
    };
    let mut within_cap = true;
    let waived_list = approval::waived_providers(env.var);
    // Processed bytes of images that will exist (on disk, from cache) for model steps.
    let mut images: BTreeMap<String, Option<Vec<u8>>> = BTreeMap::new();
    let mut steps = Vec::new();
    let mut pending = 0.0;
    for l in &selected {
        let r = &l.recipe;
        let image = match &r.from {
            Some(f) => match images.get(f).cloned().flatten() {
                Some(b) => Some((format!("{f}.png"), b)),
                None => {
                    let config = config_for(&base, &dir, l);
                    let (provider, model) = providers::resolve(&config, Kind::Model3d)?;
                    let estimate = provider.estimate_usd(&model, &Request::Model(ModelRequest::default()));
                    let waived = waived_list.contains(&provider.info().name.to_ascii_lowercase());
                    steps.push(StepPlan {
                        name: r.name.clone(),
                        output: r.output,
                        file: r.output_file_name(),
                        provider: provider.info().name.into(),
                        model,
                        provider_verified: provider.info().verified,
                        cache_key: None,
                        status: StepStatus::WaitsForImage,
                        estimate_usd: estimate,
                        from: r.from.clone(),
                        needs_approval: estimate > 0.0 && !waived,
                        approved: false,
                        blocked: (estimate > 0.0 && !waived).then(|| {
                            problem(
                                codes::NOT_APPROVED,
                                format!("`{f}` must be generated and approved by a person before `{}` can spend credit", r.name),
                                &format!("run `{f}` first, then ask the project owner to run `marionette generate approve {f}`"),
                            )
                        }),
                    });
                    continue;
                }
            },
            None => None,
        };
        let p = prepare(
            &base,
            &dir,
            l,
            image.as_ref().map(|(f, b)| (f.as_str(), b.as_slice())),
        )?;
        let mut step = info_step(r, &p);
        let cached = cache::get(target.root, &p.key, r.output.provider_kind())?;
        if up_to_date(&dir, l, &p.key) {
            step.status = StepStatus::UpToDate;
        } else if cached.is_some() {
            step.status = StepStatus::Cached;
        } else {
            let estimate = p.provider.estimate_usd(&p.model, &p.request);
            step.estimate_usd = estimate;
            let scope = scope_of(p.provider.info().name);
            let in_scope = scoped.get(&scope).copied().unwrap_or(0.0);
            match paid_checks(env, &dir, r, &p, estimate, &entries, in_scope) {
                Ok((needs, approved)) => {
                    step.needs_approval = needs;
                    step.approved = approved;
                    pending += estimate;
                    *scoped.entry(scope).or_default() += estimate;
                }
                Err(prob) => {
                    // Counted too, so the whole run's estimate is what the cap is checked
                    // against before anything is sent.
                    pending += estimate;
                    *scoped.entry(scope).or_default() += estimate;
                    if prob.code == providers::codes::SPEND_CAP {
                        within_cap = false;
                    }
                    step.needs_approval = estimate > 0.0
                        && r.from.is_some()
                        && !waived_list.contains(&p.provider.info().name.to_ascii_lowercase());
                    step.status = StepStatus::Blocked;
                    step.blocked = Some(prob);
                }
            }
        }
        if r.output != OutputKind::Model {
            let processed = match (&cached, step.status) {
                (_, StepStatus::UpToDate) => std::fs::read(dir.join(r.output_file_name())).ok(),
                (Some((raw, _)), _) => postprocess(r, raw).ok().map(|(b, _)| b),
                _ => None,
            };
            images.insert(r.name.clone(), processed);
        }
        steps.push(step);
    }
    Ok(Plan {
        dir: target.dir.into(),
        total_estimate_usd: pending,
        within_cap,
        spent_usd: spent,
        cap: env.cap,
        provider_caps: env.provider_caps.clone(),
        steps,
        before_generating: lookup_order(),
    })
}

/// One asset written (or found up to date) by a run.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Built {
    pub name: String,
    pub output: OutputKind,
    /// Asset file, relative to the recipe folder.
    pub file: String,
    pub provenance: String,
    pub sha256: String,
    pub provider: String,
    pub model: String,
    /// `up_to_date` (nothing written), `cached` (rebuilt from the cache) or `called`.
    pub how: String,
    /// Spent by this step now (zero unless `called` a paid provider).
    pub cost_usd: f64,
    /// Post-processing notes worth a person's attention.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Blocked {
    pub name: String,
    pub problem: Problem,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct RunReport {
    pub dir: String,
    pub built: Vec<Built>,
    pub blocked: Vec<Blocked>,
    /// Provider calls made (free or paid).
    pub calls: u32,
    /// Spent by this run.
    pub cost_usd: f64,
    /// Ledger total after the run.
    pub spent_usd: f64,
}

/// Builds the selected recipes. Images first, then models. Cached calls are reused; with
/// `offline`, a call that is not cached refuses the step instead. Paid steps are refused
/// before anything is sent when the whole run's estimate would pass the cap, when an image is
/// not approved, or when the key or network is missing. Returns an error only when nothing was
/// built and something was refused (or a recipe is invalid).
pub fn run(target: &Target, env: &Env, offline: bool) -> Result<RunReport, Errors> {
    let dir = target.dir_path();
    let all = recipe::load_dir(&dir, target.dir)?;
    let selected = select(all, target.names, target.dir)?;
    let (base, _) = ProvidersConfig::load(target.root)?;
    let base = providers::endpoints::apply_models(base, env.var);
    let ledger = target.ledger();

    // Whole-run budget first, so a run never spends half and then stops at the cap.
    if !offline {
        let p = plan(target, env)?;
        if !p.within_cap {
            let over: Vec<_> = p
                .steps
                .iter()
                .filter_map(|s| {
                    s.blocked
                        .as_ref()
                        .filter(|b| b.code == providers::codes::SPEND_CAP)
                        .map(|b| (s, b))
                })
                .map(|(s, b)| {
                    let mut d = Diagnostic::error(
                        providers::codes::SPEND_CAP,
                        format!(
                            "{}: {} (the run's paid steps together would pass the cap, so nothing was sent)",
                            s.name, b.message
                        ),
                    );
                    if let Some(f) = &b.fix {
                        d = d.fix(f.clone());
                    }
                    d
                })
                .collect();
            return Err(Errors(over));
        }
    }

    let mut report = RunReport {
        dir: target.dir.into(),
        built: Vec::new(),
        blocked: Vec::new(),
        calls: 0,
        cost_usd: 0.0,
        spent_usd: 0.0,
    };
    let mut images: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for l in &selected {
        match build_one(
            target,
            env,
            &base,
            &ledger,
            &dir,
            l,
            &images,
            offline,
            &mut report,
        ) {
            Ok(Some(bytes)) => {
                images.insert(l.recipe.name.clone(), bytes);
            }
            Ok(None) => {}
            Err(prob) => report.blocked.push(Blocked {
                name: l.recipe.name.clone(),
                problem: prob,
            }),
        }
    }
    report.spent_usd = spend::total(&ledger).unwrap_or(0.0);
    if report.built.is_empty() && !report.blocked.is_empty() {
        return Err(Errors(
            report
                .blocked
                .iter()
                .map(|b| {
                    let code = leak(&b.problem.code);
                    let mut d = Diagnostic::error(code, format!("{}: {}", b.name, b.problem.message));
                    if let Some(f) = &b.problem.fix {
                        d = d.fix(f.clone());
                    }
                    d
                })
                .collect(),
        ));
    }
    Ok(report)
}

/// Maps a problem code back to its `&'static str` (every code comes from a known table).
fn leak(code: &str) -> &'static str {
    const KNOWN: &[&str] = &[
        codes::BAD_RECIPE,
        codes::NOT_FOUND,
        codes::NOT_APPROVED,
        codes::NOT_CACHED,
        codes::CACHE,
        codes::POSTPROCESS,
        codes::WAITS_FOR_IMAGE,
        providers::codes::UNKNOWN_PROVIDER,
        providers::codes::KIND_UNSUPPORTED,
        providers::codes::KEY_MISSING,
        providers::codes::NETWORK_DISABLED,
        providers::codes::SPEND_CAP,
        providers::codes::BAD_CONFIG,
        providers::codes::BAD_REQUEST,
        providers::codes::HTTP,
        providers::codes::BAD_RESPONSE,
        providers::codes::JOB_FAILED,
        providers::codes::JOB_TIMEOUT,
        providers::codes::REFUSED,
        providers::codes::LEDGER,
        marionette_core::error::codes::IO_WRITE,
        marionette_core::error::codes::IO_READ,
    ];
    KNOWN
        .iter()
        .find(|k| **k == code)
        .copied()
        .unwrap_or("generate.failed")
}

#[allow(clippy::too_many_arguments)]
fn build_one(
    target: &Target,
    env: &Env,
    base: &ProvidersConfig,
    ledger: &Path,
    dir: &Path,
    l: &Loaded,
    images: &BTreeMap<String, Vec<u8>>,
    offline: bool,
    report: &mut RunReport,
) -> Result<Option<Vec<u8>>, Problem> {
    let r = &l.recipe;
    let image = match &r.from {
        Some(f) => match images.get(f) {
            Some(b) => Some((format!("{f}.png"), b.as_slice())),
            None => {
                return Err(problem(
                    codes::WAITS_FOR_IMAGE,
                    format!("the reference image `{f}` was not built in this run"),
                    &format!("fix `{f}` first (see its entry in `blocked`)"),
                ));
            }
        },
        None => None,
    };
    let p = prepare(base, dir, l, image.as_ref().map(|(f, b)| (f.as_str(), *b)))
        .map_err(|e| Problem::from(&e))?;
    let kind = r.output.provider_kind();
    let file = r.output_file_name();
    let provenance_file = format!("{}{}", r.name, provenance::SUFFIX);

    if up_to_date(dir, l, &p.key) {
        let bytes = std::fs::read(dir.join(&file))
            .map_err(|e| problem("io.read", e.to_string(), "check the file"))?;
        report.built.push(Built {
            name: r.name.clone(),
            output: r.output,
            file,
            provenance: provenance_file,
            sha256: sha256_hex(&bytes),
            provider: p.provider.info().name.into(),
            model: p.model.clone(),
            how: "up_to_date".into(),
            cost_usd: 0.0,
            notes: Vec::new(),
        });
        return Ok((r.output != OutputKind::Model).then_some(bytes));
    }

    let (raw, meta, how, cost_now) =
        match cache::get(target.root, &p.key, kind).map_err(|e| Problem::from(&e))? {
            Some((raw, meta)) => (raw, meta, "cached", 0.0),
            None => {
                if offline {
                    return Err(problem(
                        codes::NOT_CACHED,
                        format!("`{}` needs a provider call and the run is offline", r.name),
                        "run without `offline` (free with `mock`; paid providers need the owner's cap)",
                    ));
                }
                let estimate = p.provider.estimate_usd(&p.model, &p.request);
                let entries = spend::entries(ledger).map_err(|e| Problem::from(&e))?;
                paid_checks(env, dir, r, &p, estimate, &entries, 0.0)?;
                let penv = providers::Env {
                    var: env.var,
                    transport: env.transport,
                    ledger,
                    cap: env.cap,
                    provider_caps: env.provider_caps,
                    max_polls: env.max_polls,
                    wait: env.wait,
                };
                let g = providers::generate(&p.config, &penv, &p.request).map_err(|e| Problem::from(&e))?;
                report.calls += 1;
                let bytes = match g.generated.output {
                    Output::Image(b) | Output::Model(b) => b,
                    Output::Text(_) => unreachable!("image and model requests return files"),
                };
                let meta = cache::Meta {
                    key: p.key.clone(),
                    provider: g.provider.into(),
                    model: g.model.clone(),
                    kind,
                    cost_usd: g.generated.cost_usd,
                    provider_verified: p.provider.info().verified,
                    sha256: sha256_hex(&bytes),
                };
                cache::put(target.root, &meta, &bytes).map_err(|e| Problem::from(&e))?;
                report.cost_usd += g.generated.cost_usd;
                let cost = g.generated.cost_usd;
                (bytes, meta, "called", cost)
            }
        };

    let (asset, post) = postprocess(r, &raw).map_err(|e| Problem::from(&e))?;
    let notes: Vec<String> = post
        .get("notes")
        .and_then(|n| serde_json::from_value(n.clone()).ok())
        .unwrap_or_default();
    let from = match (&r.from, &p.image) {
        (Some(name), Some((f, sha))) => Some(provenance::FromImage {
            name: name.clone(),
            file: f.clone(),
            sha256: sha.clone(),
            approved: approval::is_approved(dir, name, sha),
            approval_waived: !approval::is_approved(dir, name, sha)
                && approval::waived_providers(env.var).contains(&p.provider.info().name.to_ascii_lowercase()),
        }),
        _ => None,
    };
    let rec = provenance::Provenance {
        format: provenance::FORMAT.into(),
        name: r.name.clone(),
        output: r.output,
        file: file.clone(),
        sha256: sha256_hex(&asset),
        recipe: r.file_name(),
        recipe_sha256: l.sha256.clone(),
        source: provenance::Source {
            provider: meta.provider.clone(),
            model: meta.model.clone(),
            provider_verified: meta.provider_verified,
            kind,
            prompt: r
                .prompt
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from),
            size: (r.output != OutputKind::Model).then(|| r.size()),
            from,
            cache_key: p.key.clone(),
            raw_sha256: meta.sha256.clone(),
            cost_usd: meta.cost_usd,
        },
        postprocess: post,
        license: provenance::License {
            status: "generated".into(),
            terms: provenance::terms(&meta.provider),
        },
        pool: provenance::POOL_BLOCKED.into(),
        pool_reason: provenance::POOL_REASON.into(),
    };
    let shown = |f: &str| format!("{}/{f}", target.dir);
    write_file(&dir.join(&file), &shown(&file), &asset).map_err(|e| Problem::from(&e))?;
    write_file(
        &dir.join(&provenance_file),
        &shown(&provenance_file),
        rec.to_text().as_bytes(),
    )
    .map_err(|e| Problem::from(&e))?;
    if r.output == OutputKind::Model {
        // The engine's own loader must accept the result.
        marionette_assets::load_gltf(&dir.join(&file)).map_err(|e| {
            problem(
                codes::POSTPROCESS,
                format!("the engine cannot load {file}: {e}"),
                "regenerate the model or report the provider's output",
            )
        })?;
    }
    report.built.push(Built {
        name: r.name.clone(),
        output: r.output,
        file,
        provenance: provenance_file,
        sha256: rec.sha256.clone(),
        provider: meta.provider,
        model: meta.model,
        how: how.into(),
        cost_usd: cost_now,
        notes,
    });
    Ok((r.output != OutputKind::Model).then_some(asset))
}

/// One recipe and the state of its output.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Listed {
    pub name: String,
    pub output: OutputKind,
    pub recipe: String,
    pub file: String,
    /// `built` (output and record match the recipe), `stale` (recipe or output changed since),
    /// or `missing`.
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// For image and texture recipes: a person approved the current image.
    pub approved: bool,
    pub pool: String,
}

/// Lists recipes with the state of their outputs. Reads files only.
pub fn list(target: &Target) -> Result<Vec<Listed>, Errors> {
    let dir = target.dir_path();
    let all = recipe::load_dir(&dir, target.dir)?;
    Ok(all
        .iter()
        .map(|l| {
            let r = &l.recipe;
            let p = provenance::read(&dir, &r.name);
            let bytes = std::fs::read(dir.join(r.output_file_name())).ok();
            let sha = bytes.as_deref().map(sha256_hex);
            let state = match (&p, &sha) {
                (Some(p), Some(s)) if &p.sha256 == s && p.recipe_sha256 == l.sha256 => "built",
                (_, Some(_)) => "stale",
                _ => "missing",
            };
            Listed {
                name: r.name.clone(),
                output: r.output,
                recipe: r.file_name(),
                file: r.output_file_name(),
                state: state.into(),
                provider: p.as_ref().map(|p| p.source.provider.clone()),
                model: p.as_ref().map(|p| p.source.model.clone()),
                approved: r.output != OutputKind::Model
                    && sha
                        .as_deref()
                        .is_some_and(|s| approval::is_approved(&dir, &r.name, s)),
                sha256: sha,
                from: r.from.clone(),
                pool: provenance::POOL_BLOCKED.into(),
            }
        })
        .collect())
}

/// Writes a recipe file (validated, canonical). Refuses to replace an existing recipe unless
/// `replace` is set.
pub fn write_recipe(target: &Target, recipe: &Recipe, replace: bool) -> Result<String, Errors> {
    let shown = format!("{}/{}", target.dir, recipe.file_name());
    recipe.validate(&shown)?;
    let path = target.dir_path().join(recipe.file_name());
    if path.exists() && !replace {
        return Err(
            Diagnostic::error(codes::BAD_RECIPE, format!("{shown} already exists"))
                .file(&shown)
                .fix("pass `replace: true` to overwrite it (the old output then shows as stale)")
                .into(),
        );
    }
    if let Some(from) = &recipe.from {
        let others = recipe::load_dir(&target.dir_path(), target.dir)?;
        match others.iter().find(|l| &l.recipe.name == from) {
            Some(l) if l.recipe.output != OutputKind::Model => {}
            found => {
                let why = if found.is_some() {
                    "is a model recipe"
                } else {
                    "does not exist"
                };
                return Err(Diagnostic::error(
                    codes::BAD_RECIPE,
                    format!("`from` names `{from}`, which {why} in {}", target.dir),
                )
                .file(&shown)
                .fix("write the image or texture recipe first, then the model")
                .into());
            }
        }
    }
    write_file(&path, &shown, recipe.to_text().as_bytes())?;
    Ok(shown)
}

/// For `marionette generate approve`: the image file and its SHA-256, when the recipe is an
/// image or texture recipe whose output exists.
pub fn approvable(target: &Target, name: &str) -> Result<(PathBuf, String, String), Errors> {
    let dir = target.dir_path();
    let all = recipe::load_dir(&dir, target.dir)?;
    let l = all.iter().find(|l| l.recipe.name == name).ok_or_else(|| {
        Errors::from(Diagnostic::error(
            codes::NOT_FOUND,
            format!("no recipe `{name}` in {}", target.dir),
        ))
    })?;
    if l.recipe.output == OutputKind::Model {
        return Err(Diagnostic::error(
            codes::BAD_RECIPE,
            format!("`{name}` is a model; only images are approved"),
        )
        .fix("approve the image recipe the model names in `from`")
        .into());
    }
    let file = l.recipe.output_file_name();
    let path = dir.join(&file);
    let bytes = std::fs::read(&path).map_err(|_| {
        Errors::from(
            Diagnostic::error(
                codes::NOT_FOUND,
                format!("{}/{file} does not exist yet", target.dir),
            )
            .fix("generate it first (`marionette generate run <name>`)"),
        )
    })?;
    Ok((path, file, sha256_hex(&bytes)))
}

/// One image waiting for a person's approval (see [`pending_approvals`]).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PendingApproval {
    /// Recipe folder, relative to the project root, with `/` separators.
    pub dir: String,
    pub name: String,
    pub file: String,
    pub sha256: String,
}

/// Every reference image under `root` that a model recipe names in `from`, that exists, and
/// that no approval covers yet: what `marionette generate approve --all` shows a person.
/// Walks at most six folders deep and skips `target`, `.git`, `.marionette` and
/// `node_modules`. Sorted by folder, then name.
pub fn pending_approvals(root: &Path) -> Result<Vec<PendingApproval>, Errors> {
    fn walk(base: &Path, rel: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(base.join(rel)) else {
            return;
        };
        let mut has_recipe = false;
        let mut subdirs = Vec::new();
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            match e.file_type() {
                Ok(t) if t.is_dir() => {
                    if !matches!(name.as_str(), "target" | ".git" | ".marionette" | "node_modules") {
                        subdirs.push(rel.join(&name));
                    }
                }
                Ok(t) if t.is_file() && name.ends_with(".recipe.json") => has_recipe = true,
                _ => {}
            }
        }
        if has_recipe {
            out.push(rel.to_path_buf());
        }
        if depth < 6 {
            subdirs.sort();
            for d in subdirs {
                walk(base, &d, depth + 1, out);
            }
        }
    }
    let mut dirs = Vec::new();
    walk(root, Path::new(""), 0, &mut dirs);
    let mut out = Vec::new();
    for rel in dirs {
        let shown = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let all = recipe::load_dir(&root.join(&rel), &shown)?;
        let mut refs: Vec<&str> = all
            .iter()
            .filter(|l| l.recipe.output == OutputKind::Model)
            .filter_map(|l| l.recipe.from.as_deref())
            .collect();
        refs.sort();
        refs.dedup();
        let target = Target {
            root,
            dir: &shown,
            names: &[],
        };
        for name in refs {
            let Ok((_, file, sha)) = approvable(&target, name) else {
                continue;
            };
            if !approval::is_approved(&root.join(&rel), name, &sha) {
                out.push(PendingApproval {
                    dir: shown.clone(),
                    name: name.to_string(),
                    file,
                    sha256: sha,
                });
            }
        }
    }
    Ok(out)
}
