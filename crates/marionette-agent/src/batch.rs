//! Batches: a list of operations run in order, from a `*.ops.json` file or inline. A batch is
//! a build recipe for a scene: replaying it in a fresh project must give the same files
//! (the proof scenes are built and checked this way, decision 0050).
//!
//! File format:
//!
//! ```json
//! { "format": "marionette.ops", "version": 1,
//!   "ops": [ { "op": "scene_create", "input": { "scene": "scenes/main.scene.json" } }, ... ] }
//! ```

use crate::{Agent, Op, codes, op, read_text};
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) fn ops() -> Vec<Op> {
    vec![op(
        "batch_run",
        "Run a list of operations in order, from a `*.ops.json` file (`file`) or inline (`ops`). \
         Stops at the first error and reports which step failed; earlier steps stay applied.",
        true,
        run,
    )]
}

/// One step of a batch.
#[derive(Deserialize, Serialize, JsonSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Operation name, e.g. `entity_create`.
    pub op: String,
    /// Its input object.
    #[serde(default)]
    pub input: Value,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct BatchFile {
    format: String,
    version: u32,
    ops: Vec<Step>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunInput {
    /// A `*.ops.json` file (relative to the project root).
    pub file: Option<String>,
    /// Steps to run instead of a file.
    pub ops: Option<Vec<Step>>,
}

#[derive(Serialize, JsonSchema)]
pub struct RunOutput {
    /// Steps run (all of them: a failing step returns an error instead).
    pub steps: usize,
    /// Operation name of each step, in order.
    pub ran: Vec<String>,
}

fn run(agent: &mut Agent, input: RunInput) -> Result<RunOutput, Errors> {
    let (steps, shown) = match (input.file, input.ops) {
        (Some(file), None) => {
            let path = agent.resolve(&file)?;
            let text = read_text(&path, &file)?;
            let parsed: BatchFile = serde_json::from_str(&text).map_err(|e| {
                Errors::from(
                    Diagnostic::error(codes::BAD_INPUT, format!("invalid batch file: {e}"))
                        .file(&file)
                        .at(Some(e.line()), Some(e.column()))
                        .fix("a batch file is {\"format\": \"marionette.ops\", \"version\": 1, \"ops\": [{\"op\": ..., \"input\": {...}}]}"),
                )
            })?;
            if parsed.format != "marionette.ops" || parsed.version != 1 {
                return Err(Diagnostic::error(
                    codes::BAD_INPUT,
                    format!("{file} is not a `marionette.ops` version 1 file"),
                )
                .file(&file)
                .into());
            }
            (parsed.ops, Some(file))
        }
        (None, Some(ops)) => (ops, None),
        _ => {
            return Err(Diagnostic::error(
                codes::BAD_INPUT,
                "batch_run needs exactly one of `file` and `ops`",
            )
            .fix("pass `file` (a *.ops.json path) or `ops` (a list of {op, input})")
            .into());
        }
    };
    let mut ran = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        if step.op == "batch_run" {
            return Err(
                Diagnostic::error(codes::BAD_INPUT, format!("step {index}: batches do not nest"))
                    .fix("inline the inner batch's steps")
                    .into(),
            );
        }
        agent.call(&step.op, step.input.clone()).map_err(|mut e| {
            for d in &mut e.0 {
                d.message = format!("step {index} (`{}`): {}", step.op, d.message);
                if d.file.is_none()
                    && let Some(f) = &shown
                {
                    d.file = Some(f.clone());
                }
            }
            e
        })?;
        ran.push(step.op.clone());
    }
    Ok(RunOutput {
        steps: ran.len(),
        ran,
    })
}
