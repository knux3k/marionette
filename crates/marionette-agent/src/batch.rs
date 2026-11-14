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

use crate::{Agent, Op, codes, edit, op, read_text};
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
    // Edits of one scene file in a row write its text once (decision 0270).
    agent.scenes.deferred = true;
    let result = run_steps(agent, &steps, shown.as_deref());
    agent.scenes.deferred = false;
    agent.scenes.load_failed = false;
    let flushed = agent.scenes.flush();
    let ran = result?;
    flushed?;
    Ok(RunOutput {
        steps: ran.len(),
        ran,
    })
}

fn run_step(agent: &mut Agent, steps: &[Step], index: usize, shown: Option<&str>) -> Result<(), Errors> {
    let step = &steps[index];
    agent
        .call(&step.op, step.input.clone())
        .map(|_| ())
        .map_err(|mut e| {
            for d in &mut e.0 {
                d.message = format!("step {index} (`{}`): {}", step.op, d.message);
                if d.file.is_none()
                    && let Some(f) = shown
                {
                    d.file = Some(f.to_string());
                }
            }
            e
        })
}

/// Edits of one live session in a row, held in its index and not yet loaded into its world
/// (decision 0270): the first such step and the session.
type Held = Option<(usize, String)>;

/// Runs steps `from..to` again one edit at a time, after loading them together failed (the
/// world is as it was before `from`). Stops at the step at fault with its own message.
fn one_by_one(
    agent: &mut Agent,
    steps: &[Step],
    range: std::ops::Range<usize>,
    shown: Option<&str>,
) -> Result<(), Errors> {
    agent.scenes.load_failed = false;
    agent.scenes.deferred = false;
    let result = range
        .into_iter()
        .try_for_each(|i| run_step(agent, steps, i, shown));
    agent.scenes.deferred = true;
    result
}

/// Loads held session edits into the world, so that steps before `end` are fully applied.
fn settle(
    agent: &mut Agent,
    steps: &[Step],
    held: &mut Held,
    end: usize,
    shown: Option<&str>,
) -> Result<(), Errors> {
    let Some((from, id)) = held.take() else {
        return Ok(());
    };
    if edit::load_pending(agent, &id).is_err() {
        one_by_one(agent, steps, from..end, shown)?;
    }
    Ok(())
}

fn run_steps(agent: &mut Agent, steps: &[Step], shown: Option<&str>) -> Result<Vec<String>, Errors> {
    let mut ran = Vec::new();
    let mut held: Held = None;
    for (index, step) in steps.iter().enumerate() {
        if step.op == "batch_run" {
            settle(agent, steps, &mut held, index, shown)?;
            return Err(
                Diagnostic::error(codes::BAD_INPUT, format!("step {index}: batches do not nest"))
                    .fix("inline the inner batch's steps")
                    .into(),
            );
        }
        let session = edit::CACHED_EDITS
            .contains(&step.op.as_str())
            .then(|| step.input.get("session").and_then(Value::as_str))
            .flatten();
        if held.as_ref().is_some_and(|(_, id)| Some(id.as_str()) != session) {
            settle(agent, steps, &mut held, index, shown)?;
        }
        let result = run_step(agent, steps, index, shown);
        let from = held.as_ref().map_or(index, |(from, _)| *from);
        if agent.scenes.load_failed {
            // The edits held so far could not be loaded together, and nothing was applied.
            held = None;
            one_by_one(agent, steps, from..index + 1, shown)?;
        } else {
            held = session
                .filter(|id| edit::has_pending(agent, id))
                .map(|id| (from, id.to_string()));
            if let Err(e) = result {
                // Earlier steps stay applied.
                settle(agent, steps, &mut held, index, shown)?;
                return Err(e);
            }
        }
        ran.push(step.op.clone());
    }
    settle(agent, steps, &mut held, steps.len(), shown)?;
    Ok(ran)
}
