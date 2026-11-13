//! Job TypeScript operations: check a job file, run one of its functions on JSON data, and
//! write the job prelude declarations (decision 0030, `docs/ai/JOBS-TS.md`).

use crate::{Agent, Op, op, write_atomic};
use marionette_core::{Diagnostic, Errors};
use marionette_jobs::Jobs;
use marionette_jobs_ts::{Arg, JobProgram, ParamType, Safety, codes};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Instant;

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "jobs_check",
            "Compile a job TypeScript file (`*.job.ts`: typed, numbers-only TypeScript compiled to WebAssembly) and report its functions or every error with line, column and a fix.",
            false,
            check,
        ),
        op(
            "jobs_run",
            "Run one exported function of a job file on JSON arrays and numbers, in chunks, on 1 or more threads; returns the arrays it changed, per-chunk return values and the time.",
            false,
            run,
        ),
        op(
            "jobs_reload",
            "Recompile the changed job files of a live session (hot reload). A file that no longer compiles keeps running its last good version and its errors are returned.",
            true,
            reload,
        ),
        op(
            "jobs_types",
            "Write `jobs.d.ts` (the job prelude: `type i32 = number`) into the project, for editors and `tsc`.",
            true,
            types,
        ),
    ]
}

fn safety(unchecked: Option<bool>) -> Safety {
    if unchecked.unwrap_or(false) {
        Safety::Unchecked
    } else {
        Safety::Checked
    }
}

fn load(agent: &Agent, path: &str, unchecked: Option<bool>) -> Result<(JobProgram, f64), Errors> {
    let full = agent.resolve(path)?;
    let source = std::fs::read_to_string(&full).map_err(|e| {
        Errors::from(
            Diagnostic::error(codes::READ, format!("cannot read the job file: {e}"))
                .file(path)
                .fix("the path is relative to the project directory"),
        )
    })?;
    let t = Instant::now();
    let program = JobProgram::compile_with(&source, path, safety(unchecked)).map_err(Errors)?;
    Ok((program, t.elapsed().as_secs_f64() * 1000.0))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckInput {
    /// The job file, relative to the project.
    pub path: String,
    /// Compile without safety checks (no index checks, no time limit). Default false.
    pub unchecked: Option<bool>,
}

#[derive(Serialize, JsonSchema)]
pub struct Param {
    pub name: String,
    /// `number`, `i32`, `boolean`, `Float32Array`, `Float64Array` or `Int32Array`.
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Serialize, JsonSchema)]
pub struct FunctionOut {
    pub name: String,
    pub exported: bool,
    pub params: Vec<Param>,
    pub returns: String,
}

#[derive(Serialize, JsonSchema)]
pub struct CheckOutput {
    pub path: String,
    pub ok: bool,
    pub functions: Vec<FunctionOut>,
    pub wasm_bytes: usize,
    pub compile_ms: f64,
    /// Every problem found (empty when `ok`).
    #[schemars(with = "Vec<Value>")]
    pub diagnostics: Errors,
}

fn check(agent: &mut Agent, input: CheckInput) -> Result<CheckOutput, Errors> {
    match load(agent, &input.path, input.unchecked) {
        Ok((program, compile_ms)) => Ok(CheckOutput {
            path: input.path,
            ok: true,
            functions: program
                .functions()
                .iter()
                .map(|f| FunctionOut {
                    name: f.name.clone(),
                    exported: f.exported,
                    params: f
                        .params
                        .iter()
                        .map(|(n, p)| Param {
                            name: n.clone(),
                            ty: p.ts_name().into(),
                        })
                        .collect(),
                    returns: f.returns.ts_name().into(),
                })
                .collect(),
            wasm_bytes: program.wasm().len(),
            compile_ms,
            diagnostics: Errors(Vec::new()),
        }),
        Err(errors) if errors.0.iter().any(|d| d.code == codes::READ) => Err(errors),
        Err(errors) => Ok(CheckOutput {
            path: input.path,
            ok: false,
            functions: Vec::new(),
            wasm_bytes: 0,
            compile_ms: 0.0,
            diagnostics: errors,
        }),
    }
}

/// One argument of `jobs_run`, in parameter order.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RunArg {
    /// A chunked array the job may change (returned in `arrays`).
    F32(Vec<f64>),
    F64(Vec<f64>),
    I32(Vec<i32>),
    /// A chunked, read-only array.
    F32In(Vec<f64>),
    F64In(Vec<f64>),
    I32In(Vec<i32>),
    /// A whole read-only array, the same for every chunk.
    F32All(Vec<f64>),
    F64All(Vec<f64>),
    I32All(Vec<i32>),
    Number(f64),
    Int(i32),
    Bool(bool),
    /// Fills an `i32` parameter with the index of the chunk's first element.
    ChunkStart(bool),
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunInput {
    pub path: String,
    pub function: String,
    /// Arguments in parameter order, e.g. `[{"f32": [1, 2]}, {"number": 0.5}]`.
    pub args: Vec<RunArg>,
    /// Elements per chunk (default: the whole length in one chunk).
    pub chunk: Option<usize>,
    /// Worker threads (default 1). Results never depend on it.
    pub threads: Option<usize>,
    pub unchecked: Option<bool>,
}

#[derive(Serialize, JsonSchema)]
pub struct RunOutput {
    /// The changeable arrays after the run, by parameter name.
    pub arrays: serde_json::Map<String, Value>,
    /// One return value per chunk, in chunk order (empty for `void` functions).
    pub results: Vec<f64>,
    /// The per-chunk results added in chunk order.
    pub sum: f64,
    pub chunks: usize,
    pub run_ms: f64,
}

enum Owned {
    F32(Vec<f32>),
    F64(Vec<f64>),
    I32(Vec<i32>),
}

fn run(agent: &mut Agent, input: RunInput) -> Result<RunOutput, Errors> {
    let (program, _) = load(agent, &input.path, input.unchecked)?;
    let Some(info) = program
        .functions()
        .iter()
        .find(|f| f.name == input.function && f.exported)
        .cloned()
    else {
        // Let the runtime produce the standard error with the list of functions.
        return Err(program
            .run(&Jobs::inline(), &input.function, 0, 1, &mut [])
            .err()
            .map_or_else(|| Errors(vec![]), Errors::from));
    };
    let args_err = |m: String| Errors::from(Diagnostic::error(codes::ARGS, m).file(&input.path));
    // Length of the run: the chunked arrays (all must agree; the runtime checks).
    let mut len = None;
    let mut owned: Vec<Option<Owned>> = Vec::new();
    for a in &input.args {
        let (o, chunked) = match a {
            RunArg::F32(v) | RunArg::F32In(v) => {
                (Some(Owned::F32(v.iter().map(|&x| x as f32).collect())), true)
            }
            RunArg::F64(v) | RunArg::F64In(v) => (Some(Owned::F64(v.clone())), true),
            RunArg::I32(v) | RunArg::I32In(v) => (Some(Owned::I32(v.clone())), true),
            RunArg::F32All(v) => (Some(Owned::F32(v.iter().map(|&x| x as f32).collect())), false),
            RunArg::F64All(v) => (Some(Owned::F64(v.clone())), false),
            RunArg::I32All(v) => (Some(Owned::I32(v.clone())), false),
            _ => (None, false),
        };
        if chunked && len.is_none() {
            len = Some(match &o {
                Some(Owned::F32(v)) => v.len(),
                Some(Owned::F64(v)) => v.len(),
                Some(Owned::I32(v)) => v.len(),
                None => 0,
            });
        }
        owned.push(o);
    }
    let Some(len) = len else {
        return Err(args_err("give at least one chunked array (`f32`, `f64`, `i32` or an `_in` form); it sets the run's length".into()));
    };
    let mut call: Vec<Arg> = Vec::new();
    for (a, o) in input.args.iter().zip(owned.iter_mut()) {
        call.push(match (a, o) {
            (RunArg::F32(_), Some(Owned::F32(v))) => Arg::F32(v),
            (RunArg::F64(_), Some(Owned::F64(v))) => Arg::F64(v),
            (RunArg::I32(_), Some(Owned::I32(v))) => Arg::I32(v),
            (RunArg::F32In(_), Some(Owned::F32(v))) => Arg::F32In(v),
            (RunArg::F64In(_), Some(Owned::F64(v))) => Arg::F64In(v),
            (RunArg::I32In(_), Some(Owned::I32(v))) => Arg::I32In(v),
            (RunArg::F32All(_), Some(Owned::F32(v))) => Arg::F32All(v),
            (RunArg::F64All(_), Some(Owned::F64(v))) => Arg::F64All(v),
            (RunArg::I32All(_), Some(Owned::I32(v))) => Arg::I32All(v),
            (RunArg::Number(x), _) => Arg::Number(*x),
            (RunArg::Int(x), _) => Arg::Int(*x),
            (RunArg::Bool(x), _) => Arg::Bool(*x),
            (RunArg::ChunkStart(_), _) => Arg::ChunkStart,
            _ => unreachable!("owned matches the argument"),
        });
    }
    let chunk = input.chunk.unwrap_or(len).max(1);
    let jobs = Jobs::with_threads(input.threads.unwrap_or(1));
    let t = Instant::now();
    let results = program
        .run(&jobs, &input.function, len, chunk, &mut call)
        .map_err(Errors::from)?;
    let run_ms = t.elapsed().as_secs_f64() * 1000.0;
    drop(call);
    let mut arrays = serde_json::Map::new();
    for ((a, o), (name, p)) in input.args.iter().zip(owned).zip(&info.params) {
        let writable = matches!(a, RunArg::F32(_) | RunArg::F64(_) | RunArg::I32(_));
        if !writable || !matches!(p, ParamType::Array(_)) {
            continue;
        }
        let v = match o {
            Some(Owned::F32(v)) => serde_json::json!(v.iter().map(|&x| x as f64).collect::<Vec<_>>()),
            Some(Owned::F64(v)) => serde_json::json!(v),
            Some(Owned::I32(v)) => serde_json::json!(v),
            None => continue,
        };
        arrays.insert(name.clone(), v);
    }
    Ok(RunOutput {
        arrays,
        sum: results.iter().fold(0.0, |a, b| a + b),
        results,
        chunks: len.div_ceil(chunk),
        run_ms,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TypesInput {
    /// Where to write, relative to the project (default `jobs.d.ts`).
    pub out: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct TypesOutput {
    pub path: String,
    pub bytes: usize,
}

fn types(agent: &mut Agent, input: TypesInput) -> Result<TypesOutput, Errors> {
    let out = input.out.unwrap_or_else(|| "jobs.d.ts".into());
    let text = marionette_jobs_ts::PRELUDE_DTS;
    write_atomic(&agent.resolve(&out)?, &out, text.as_bytes())?;
    Ok(TypesOutput {
        path: out,
        bytes: text.len(),
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReloadInput {
    pub session: String,
}

#[derive(Serialize, JsonSchema)]
pub struct ReloadedFile {
    /// Job file, relative to the scene.
    pub path: String,
    /// `unchanged`, `reloaded` or `failed` (the last good version keeps running).
    pub status: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[schemars(with = "Vec<Value>")]
    pub errors: Vec<Diagnostic>,
}

#[derive(Serialize, JsonSchema)]
pub struct ReloadOutput {
    pub session: String,
    /// Job files loaded by the session so far (schedules and scripts).
    pub files: Vec<ReloadedFile>,
    pub tick: u64,
}

fn reload(agent: &mut Agent, input: ReloadInput) -> Result<ReloadOutput, Errors> {
    let s = crate::session::get_mut(agent, &input.session)?;
    let files = match s.sim.world.get_resource_mut::<marionette_jobs_ts::JobLibrary>() {
        Some(mut lib) => lib
            .reload()
            .into_iter()
            .map(|r| ReloadedFile {
                path: r.path,
                status: r.status.to_string(),
                errors: r.errors,
            })
            .collect(),
        None => Vec::new(),
    };
    let changed = files.iter().filter(|f| f.status != "unchanged").count();
    s.log("jobs_reload", format!("{changed} job files changed"));
    Ok(ReloadOutput {
        session: input.session,
        files,
        tick: s.sim.tick(),
    })
}
