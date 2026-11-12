//! Generation operations: write recipes, plan a run (cache hits, cost, refusals) without
//! calling anything, run recipes into engine-ready assets with provenance, and list them.
//! Design: decision 0045; crate `marionette-generate`; guide `docs/ai/GENERATION.md`.
//!
//! Approving an image for a paid 3D model is not an operation: only a person approves, with
//! `marionette generate approve <name>` at a terminal.

use crate::{Agent, Op, op};
use marionette_core::Errors;
use marionette_generate::recipe::{self, ModelOptions, OutputKind, Recipe, TextureOptions};
use marionette_generate::{self as generate, Listed, Plan, RunReport, Target};
use marionette_providers::{Choice, spend};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "generate_recipe",
            "Writes a generation recipe (<name>.recipe.json in assets/generated/ by default): an image, a texture or a 3D model to generate, with post-processing options. Generation is the LAST step of the asset lookup order: search the project, the owned library and CC0 sources first.",
            true,
            write_recipe,
        ),
        op(
            "generate_plan",
            "Shows what generate_run would do, without calling any provider or writing anything: per recipe the provider and model, cache hit or call, estimated cost, whether a person must approve the reference image, and why a step would be refused (spend cap, key, network, approval).",
            false,
            plan,
        ),
        op(
            "generate_run",
            "Builds recipes into engine-ready assets (PNG images and textures, normalized GLB models) with a provenance record each. Cached provider outputs are reused, so the same request is never paid twice; `offline` refuses any call that is not cached. Paid calls need a spend cap set by the project owner, and a model made from an image needs that image approved by a person.",
            true,
            run,
        ),
        op(
            "generate_list",
            "Lists generation recipes and the state of their outputs (built, stale, missing), provider, model, approval and pool status (always blocked).",
            false,
            list,
        ),
    ]
}

fn default_dir() -> String {
    recipe::DEFAULT_DIR.into()
}

fn check_dir(agent: &Agent, dir: &str) -> Result<(), Errors> {
    // Same path rules as every other operation (no climbing out of the project).
    agent.resolve(dir).map(|_| ())
}

fn with_env<T>(agent: &Agent, f: impl FnOnce(&generate::Env) -> Result<T, Errors>) -> Result<T, Errors> {
    let env_var = crate::env_lookup(agent.root());
    let transport = marionette_providers::http::default_transport();
    let wait = |_: u32| std::thread::sleep(std::time::Duration::from_secs(3));
    let env = generate::Env {
        var: &env_var,
        transport: transport.as_ref(),
        cap: spend::resolve_cap(&env_var)?,
        max_polls: 200,
        wait: &wait,
    };
    f(&env)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RecipeInput {
    /// Lowercase letters, digits, `_` and `-`; the file stem of the recipe and its output.
    name: String,
    output: OutputKind,
    #[serde(default)]
    prompt: Option<String>,
    /// Image and texture: requested size `[width, height]` (default 1024 x 1024).
    #[serde(default)]
    size: Option<[u32; 2]>,
    #[serde(default)]
    texture: Option<TextureOptions>,
    /// Model only: name of the image or texture recipe to build the model from.
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    model: Option<ModelOptions>,
    /// Pin a provider (and model) for this recipe; default: providers.json.
    #[serde(default)]
    provider: Option<Choice>,
    #[serde(default)]
    notes: Option<String>,
    /// Recipe folder (default `assets/generated`).
    #[serde(default = "default_dir")]
    dir: String,
    /// Overwrite an existing recipe of the same name.
    #[serde(default)]
    replace: bool,
}

#[derive(Serialize, JsonSchema)]
struct RecipeOutput {
    /// The recipe file written.
    recipe: String,
    /// What a run of this recipe would do now.
    plan: Plan,
}

fn write_recipe(agent: &mut Agent, input: RecipeInput) -> Result<RecipeOutput, Errors> {
    check_dir(agent, &input.dir)?;
    let r = Recipe {
        format: recipe::FORMAT.into(),
        name: input.name.clone(),
        output: input.output,
        prompt: input.prompt,
        size: input.size,
        texture: input.texture,
        from: input.from,
        model: input.model,
        provider: input.provider,
        notes: input.notes,
    };
    let names = [input.name];
    let target = Target {
        root: agent.root(),
        dir: &input.dir,
        names: &names,
    };
    let written = generate::write_recipe(&target, &r, input.replace)?;
    let plan = with_env(agent, |env| generate::plan(&target, env))?;
    Ok(RecipeOutput {
        recipe: written,
        plan,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SelectInput {
    /// Recipe names; empty or absent means every recipe in the folder. A model's `from`
    /// image is included automatically.
    #[serde(default)]
    names: Vec<String>,
    /// Recipe folder (default `assets/generated`).
    #[serde(default = "default_dir")]
    dir: String,
}

fn plan(agent: &mut Agent, input: SelectInput) -> Result<Plan, Errors> {
    check_dir(agent, &input.dir)?;
    let target = Target {
        root: agent.root(),
        dir: &input.dir,
        names: &input.names,
    };
    with_env(agent, |env| generate::plan(&target, env))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RunInput {
    #[serde(default)]
    names: Vec<String>,
    #[serde(default = "default_dir")]
    dir: String,
    /// Use cached provider outputs only; refuse any step that would call a provider.
    #[serde(default)]
    offline: bool,
}

fn run(agent: &mut Agent, input: RunInput) -> Result<RunReport, Errors> {
    check_dir(agent, &input.dir)?;
    let target = Target {
        root: agent.root(),
        dir: &input.dir,
        names: &input.names,
    };
    with_env(agent, |env| generate::run(&target, env, input.offline))
}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    dir: String,
    recipes: Vec<Listed>,
}

fn list(agent: &mut Agent, input: SelectInput) -> Result<ListOutput, Errors> {
    check_dir(agent, &input.dir)?;
    let target = Target {
        root: agent.root(),
        dir: &input.dir,
        names: &input.names,
    };
    let mut recipes = generate::list(&target)?;
    if !input.names.is_empty() {
        recipes.retain(|l| input.names.contains(&l.name));
    }
    Ok(ListOutput {
        dir: input.dir,
        recipes,
    })
}
