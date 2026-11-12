//! Script operations: hot reload in a live session, and the generated `marionette.d.ts`.

use crate::{Agent, Op, op, write_atomic};
use marionette_core::Errors;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "script_reload",
            "Re-read every script file of a live session and swap the running scripts (hot reload). Components, script state, tick and RNG are kept; on error the old scripts keep running.",
            true,
            reload,
        ),
        op(
            "script_types",
            "Write `marionette.d.ts` (TypeScript declarations for scripts, generated from the schema) into the project.",
            true,
            types,
        ),
    ]
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReloadInput {
    pub session: String,
}

#[derive(Serialize, JsonSchema)]
pub struct ReloadOutput {
    pub session: String,
    /// Script files now running (relative to the scene).
    pub scripts: Vec<String>,
    /// Wall-clock time of the reload in milliseconds (read, strip, compile, swap).
    pub ms: f64,
    pub tick: u64,
    pub state_hash: String,
}

fn reload(agent: &mut Agent, input: ReloadInput) -> Result<ReloadOutput, Errors> {
    let s = crate::session::get_mut(agent, &input.session)?;
    let t = Instant::now();
    let scripts = marionette_script::reload(&mut s.sim.world)?;
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    s.log(
        "script_reload",
        format!("reloaded {} scripts in {ms:.2} ms", scripts.len()),
    );
    Ok(ReloadOutput {
        session: input.session,
        scripts,
        ms,
        tick: s.sim.tick(),
        state_hash: s.sim.state_hash(),
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TypesInput {
    /// Where to write, relative to the project (default `marionette.d.ts`).
    pub out: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct TypesOutput {
    pub path: String,
    pub bytes: usize,
}

fn types(agent: &mut Agent, input: TypesInput) -> Result<TypesOutput, Errors> {
    let out = input.out.unwrap_or_else(|| "marionette.d.ts".into());
    let text = marionette_script::typings::typings(agent.registry());
    write_atomic(&agent.resolve(&out)?, &out, text.as_bytes())?;
    Ok(TypesOutput {
        path: out,
        bytes: text.len(),
    })
}
