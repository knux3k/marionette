//! Scenario tests: a scene, a seed, a number of ticks, assertions on the resulting world and an
//! optional reference image. Results are data (pass/fail per assertion), never a panic.
//!
//! File format (`tests/*.test.json`; paths are relative to the project root):
//!
//! ```json
//! {
//!   "scene": "scenes/main.scene.json", "seed": 1, "ticks": 60,
//!   "assertions": [
//!     { "count": { "with": ["Camera"], "equals": 1 } },
//!     { "field": { "entity": 4, "path": "Position.x", "op": "gt", "value": 0 } },
//!     { "state_hash": "…" }
//!   ],
//!   "image": { "reference": "tests/main.png", "camera": 1, "size": "320x180" }
//! }
//! ```

use crate::observe::{CompareOp, Predicate, holds, lookup, matching, parse_size, render_pixels};
use crate::project::{find_files, rel};
use crate::{Agent, Op, codes, op, read_text};
use marionette_core::{Diagnostic, Errors};
use marionette_render::assets::decode_png;
use marionette_render::compare::{REFERENCE_THRESHOLD, diff, within_reference_tolerance};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

pub(crate) fn ops() -> Vec<Op> {
    vec![op(
        "test_run",
        "Run scenario tests: every *.test.json under `path` (default `tests`), or one inline `scenario`. \
         Returns pass/fail per assertion as data.",
        false,
        run,
    )]
}

#[derive(Deserialize, JsonSchema, Clone, Debug)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Assertion {
    /// A value inside one entity's component.
    Field {
        entity: u64,
        path: String,
        op: CompareOp,
        #[serde(default)]
        value: Value,
    },
    /// How many entities match a query.
    Count {
        #[serde(default)]
        with: Vec<String>,
        #[serde(default, rename = "where")]
        predicates: Vec<Predicate>,
        equals: usize,
    },
    /// The exact state hash after `ticks`.
    StateHash(String),
}

#[derive(Deserialize, JsonSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct ImageCheck {
    /// Reference PNG (relative to the project root).
    pub reference: String,
    pub camera: Option<u64>,
    /// Default 320x180.
    pub size: Option<String>,
    /// Default 4.
    pub msaa: Option<u32>,
}

#[derive(Deserialize, JsonSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub scene: String,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub ticks: u64,
    #[serde(default)]
    pub assertions: Vec<Assertion>,
    pub image: Option<ImageCheck>,
}

pub(crate) fn load_file(_agent: &Agent, path: &Path, shown: &str) -> Result<Scenario, Errors> {
    let text = read_text(path, shown)?;
    serde_json::from_str(&text).map_err(|e| {
        Errors::from(
            Diagnostic::error(codes::BAD_INPUT, format!("invalid scenario: {e}"))
                .file(shown)
                .at(Some(e.line()), Some(e.column()))
                .fix("see the `scenario` part of test_run's input schema"),
        )
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunInput {
    /// A *.test.json file or a directory to search (default `tests`).
    pub path: Option<String>,
    /// Run this scenario instead of files.
    pub scenario: Option<Scenario>,
}

#[derive(Serialize, JsonSchema)]
pub struct AssertionResult {
    pub index: usize,
    pub passed: bool,
    pub message: String,
}

#[derive(Serialize, JsonSchema)]
pub struct ImageResult {
    pub passed: bool,
    pub max: u8,
    pub mean: f64,
    pub over_threshold: f64,
}

#[derive(Serialize, JsonSchema)]
pub struct ScenarioResult {
    pub name: String,
    pub passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_hash: Option<String>,
    pub assertions: Vec<AssertionResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageResult>,
    /// Why the scenario could not run at all (invalid scene, missing file...).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<Vec<Value>>")]
    pub errors: Option<Errors>,
}

#[derive(Serialize, JsonSchema)]
pub struct RunOutput {
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<ScenarioResult>,
}

fn run(agent: &mut Agent, input: RunInput) -> Result<RunOutput, Errors> {
    let mut list: Vec<(String, Result<Scenario, Errors>)> = Vec::new();
    if let Some(s) = input.scenario {
        list.push(("inline".into(), Ok(s)));
    } else {
        let given = input.path.unwrap_or_else(|| "tests".into());
        let p = agent.resolve(&given)?;
        let files = if p.is_dir() {
            find_files(&p, ".test.json")
        } else {
            vec![p]
        };
        if files.is_empty() {
            return Err(
                Diagnostic::error(codes::NOT_FOUND, format!("no *.test.json files under `{given}`"))
                    .fix("add a scenario file, or pass `scenario` inline")
                    .into(),
            );
        }
        for f in files {
            let shown = rel(agent, &f);
            list.push((shown.clone(), load_file(agent, &f, &shown)));
        }
    }
    let results: Vec<ScenarioResult> = list
        .into_iter()
        .map(|(name, s)| match s.and_then(|s| run_one(agent, &s)) {
            Ok((state_hash, assertions, image)) => ScenarioResult {
                passed: assertions.iter().all(|a| a.passed) && image.as_ref().is_none_or(|i| i.passed),
                name,
                state_hash: Some(state_hash),
                assertions,
                image,
                errors: None,
            },
            Err(e) => ScenarioResult {
                name,
                passed: false,
                state_hash: None,
                assertions: Vec::new(),
                image: None,
                errors: Some(e),
            },
        })
        .collect();
    let passed = results.iter().filter(|r| r.passed).count();
    Ok(RunOutput {
        failed: results.len() - passed,
        passed,
        results,
    })
}

type Outcome = (String, Vec<AssertionResult>, Option<ImageResult>);

fn run_one(agent: &mut Agent, s: &Scenario) -> Result<Outcome, Errors> {
    let mut sim = crate::project::load_sim(agent, &s.scene, s.seed)?;
    sim.run(s.ticks);
    let hash = sim.state_hash();
    let scene = sim.snapshot();
    let mut results = Vec::new();
    for (index, a) in s.assertions.iter().enumerate() {
        let (passed, message) = match a {
            Assertion::Field {
                entity,
                path,
                op,
                value,
            } => {
                let actual = scene
                    .entities
                    .iter()
                    .find(|e| e.id == *entity)
                    .and_then(|e| lookup(&e.components, path));
                let ok = holds(actual, *op, value);
                let shown = actual.map_or("missing".to_string(), Value::to_string);
                (
                    ok,
                    format!("entity {entity} {path} = {shown}; expected {op:?} {value}"),
                )
            }
            Assertion::Count {
                with,
                predicates,
                equals,
            } => {
                let n = matching(&scene, with, predicates).len();
                (n == *equals, format!("{n} matching entities; expected {equals}"))
            }
            Assertion::StateHash(expected) => (
                &hash == expected,
                format!("state hash {hash}; expected {expected}"),
            ),
        };
        results.push(AssertionResult {
            index,
            passed,
            message,
        });
    }
    let image = match &s.image {
        None => None,
        Some(img) => {
            let size = parse_size(img.size.as_deref().unwrap_or("320x180"))?;
            let reference_path = agent.resolve(&img.reference)?;
            let bytes = std::fs::read(&reference_path).map_err(|e| {
                Errors::from(
                    Diagnostic::error(
                        codes::NOT_FOUND,
                        format!("cannot read reference {}: {e}", img.reference),
                    )
                    .file(&img.reference)
                    .fix(
                        "save one with observe_screenshot using the same scene, seed, ticks, camera and size",
                    ),
                )
            })?;
            let reference = decode_png(&bytes).map_err(|e| {
                Errors::from(
                    Diagnostic::error(codes::BAD_INPUT, format!("bad reference PNG: {e}"))
                        .file(&img.reference),
                )
            })?;
            if (reference.width, reference.height) != size {
                return Err(Diagnostic::error(
                    codes::BAD_INPUT,
                    format!(
                        "reference is {}x{}, scenario asks for {}x{}",
                        reference.width, reference.height, size.0, size.1
                    ),
                )
                .file(&img.reference)
                .into());
            }
            let (pixels, _, _) =
                render_pixels(agent, &sim, &s.scene, img.camera, size, img.msaa.unwrap_or(4))?;
            let d = diff(&pixels, &reference.rgba, REFERENCE_THRESHOLD);
            Some(ImageResult {
                passed: within_reference_tolerance(&d),
                max: d.max,
                mean: d.mean,
                over_threshold: d.over_threshold,
            })
        }
    };
    Ok((hash, results, image))
}
