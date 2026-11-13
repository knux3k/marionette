//! Job TypeScript in scenes and scripts (decision 0070).
//!
//! - [`JobSchedule`]: a component listing job steps (file, exported function, arguments)
//!   that run every tick in [`Stage::Logic`] over the entity's [`JobData`] arrays, entity by
//!   entity in scene-id order, steps in list order. No script or Rust needed.
//! - [`JobRunner`]: inserted so scripts can run job functions (`engine.jobs.run`) on job
//!   data or on their own typed arrays.
//! - Every job file a schedule names is compiled and checked against its arguments when the
//!   scene loads; problems fail the load with `jobts.*` diagnostics. A step that fails during
//!   a tick (index out of range, time limit) records a `job_error` event and the tick goes
//!   on, like a script error.
//!
//! Determinism: chunks come from the data, results are combined in chunk order, and steps
//! run in a fixed order, so the state hash does not depend on the thread count.

use crate::{Arg, JobLibrary, JobProgram, ParamType, codes};
use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_core::builtin::{DEFAULT_DT, FixedStep, SimClock};
use marionette_core::jobdata::{JobArgMode, JobCall, JobCallArg, JobData, JobRunner, JobValues};
use marionette_core::module::{EventLog, Module, SceneDir, Stage};
use marionette_core::rng::SimRng;
use marionette_core::scene::SceneId;
use marionette_core::{Diagnostic, Errors, Registry};
use marionette_jobs::Jobs;
use std::sync::Arc;

/// Elements per chunk when a step or call does not say (`chunk: 0`).
pub const DEFAULT_CHUNK: usize = 16_384;

/// One job function call of a [`JobSchedule`].
///
/// `args` has one string per parameter, in order:
///
/// | Argument | Parameter | Value |
/// | --- | --- | --- |
/// | `name` | typed array | the entity's `JobData` array, chunked, changes kept |
/// | `in:name` | typed array | the same, read only |
/// | `all:name` | typed array | the whole array to every chunk, read only |
/// | `dt` | number | seconds per tick |
/// | `tick` | number or i32 | the tick being simulated |
/// | `len` | number or i32 | elements in the run |
/// | `chunk_start` | i32 | index of the chunk's first element |
/// | `rand` | number or i32 | a new seeded random value per run: [0, 1) or 0 to 2^31 - 1 |
/// | `1.5`, `-3` | number or i32 | a constant |
/// | `true`, `false` | boolean | a constant |
///
/// The run covers the length of the chunked arrays (they must be equal); with none, the
/// function is called once.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct JobStep {
    /// Job file (`*.job.ts`), relative to the scene file.
    pub path: String,
    /// An exported function of the file.
    pub function: String,
    pub args: Vec<String>,
    /// Elements per chunk (0: 16,384). Chunks run in parallel.
    pub chunk: u32,
}

/// Job steps run every tick over this entity's [`JobData`] (decision 0070). Steps run in
/// order, after scripts and before physics.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct JobSchedule {
    pub steps: Vec<JobStep>,
}

/// A parsed [`JobStep`] argument.
#[derive(Debug, Clone, PartialEq)]
enum Spec {
    Array(String, JobArgMode),
    Dt,
    Tick,
    Len,
    ChunkStart,
    Rand,
    Num(f64),
    Bool(bool),
}

fn parse_spec(s: &str) -> Spec {
    match s {
        "dt" => Spec::Dt,
        "tick" => Spec::Tick,
        "len" => Spec::Len,
        "chunk_start" => Spec::ChunkStart,
        "rand" => Spec::Rand,
        "true" => Spec::Bool(true),
        "false" => Spec::Bool(false),
        _ => {
            if let Some(n) = s.strip_prefix("in:") {
                Spec::Array(n.to_string(), JobArgMode::Read)
            } else if let Some(n) = s.strip_prefix("all:") {
                Spec::Array(n.to_string(), JobArgMode::Whole)
            } else if let Ok(v) = s.parse::<f64>()
                && s.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '.')
            {
                Spec::Num(v)
            } else {
                Spec::Array(s.to_string(), JobArgMode::Write)
            }
        }
    }
}

const ARG_HELP: &str = "arguments are array names (`name`, `in:name`, `all:name`), `dt`, `tick`, `len`, `chunk_start`, `rand`, numbers or `true`/`false`";

/// The module: registers [`JobSchedule`], runs schedules, offers [`JobRunner`] to scripts.
pub struct JobsModule;

impl Module for JobsModule {
    fn name(&self) -> &'static str {
        "jobs"
    }
    fn register(&self, r: &mut Registry) {
        r.register_component::<JobSchedule>();
    }
    fn add_systems(&self, schedule: &mut Schedule) {
        schedule.add_systems(run_schedules.in_set(Stage::Logic));
    }
    fn init(&self, world: &mut World) -> marionette_core::Result<()> {
        let dir = world
            .get_resource::<SceneDir>()
            .map(|d| d.0.clone())
            .unwrap_or_default();
        world.insert_resource(JobLibrary::new(dir));
        world.insert_resource(JobRunner(run_call));
        if !world.contains_resource::<Jobs>() {
            world.insert_resource(Jobs::shared());
        }
        check_schedules(world)
    }
}

/// Entities with a schedule, in scene-id order.
fn scheduled(world: &mut World) -> Vec<(u64, Entity)> {
    let mut q = world.query_filtered::<(Entity, &SceneId), With<JobSchedule>>();
    let mut v: Vec<(u64, Entity)> = q.iter(world).map(|(e, id)| (id.0, e)).collect();
    v.sort_unstable();
    v
}

/// Compiles every scheduled file and checks each step's arguments against its function.
fn check_schedules(world: &mut World) -> marionette_core::Result<()> {
    let mut problems = Vec::new();
    for (id, e) in scheduled(world) {
        let steps = world.get::<JobSchedule>(e).expect("queried").steps.clone();
        for (i, step) in steps.iter().enumerate() {
            let at = |d: Diagnostic| d.entity(id).component("JobSchedule").field(format!("steps[{i}]"));
            let program = match world.resource_mut::<JobLibrary>().get(&step.path) {
                Ok(p) => p,
                Err(errs) => {
                    problems.extend(errs.into_iter().map(at));
                    continue;
                }
            };
            if let Err(d) = check_step(world.get::<JobData>(e), &program, step) {
                problems.push(at(d));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(Errors(problems))
    }
}

fn check_step(data: Option<&JobData>, program: &JobProgram, step: &JobStep) -> Result<(), Diagnostic> {
    let info = program.exported(&step.function)?;
    let err = |m: String| {
        Diagnostic::error(codes::ARGS, m)
            .file(program.file())
            .fix(format!(
                "`{}` takes ({}); {ARG_HELP}",
                info.name,
                crate::signature(info)
            ))
    };
    if step.args.len() != info.params.len() {
        return Err(err(format!(
            "`{}` takes {} arguments, the step gives {}",
            info.name,
            info.params.len(),
            step.args.len()
        )));
    }
    let mut chunked: Option<(String, usize)> = None;
    for (arg, (pname, p)) in step.args.iter().zip(&info.params) {
        let spec = parse_spec(arg);
        let ok = match (&spec, p) {
            (Spec::Array(name, mode), ParamType::Array(elem)) => {
                let Some(values) = data.and_then(|d| d.get(name)) else {
                    let names = data.map(|d| d.names().join(", ")).unwrap_or_default();
                    return Err(Diagnostic::error(
                        codes::ARGS,
                        format!("parameter `{pname}`: the entity has no job array `{name}`"),
                    )
                    .fix(format!(
                        "add it to the entity's `JobData` (arrays: {}); an empty array is fine if a script fills it",
                        if names.is_empty() { "none" } else { &names }
                    )));
                };
                if values.ts_name() != elem.ts_name() {
                    return Err(err(format!(
                        "parameter `{pname}` is {} but job array `{name}` is {}",
                        elem.ts_name(),
                        values.ts_name()
                    )));
                }
                if *mode != JobArgMode::Whole {
                    match &chunked {
                        Some((first, n)) if *n != values.len() => {
                            return Err(err(format!(
                                "chunked arrays differ in length: `{first}` has {n} elements, `{name}` has {}",
                                values.len()
                            )));
                        }
                        None => chunked = Some((name.clone(), values.len())),
                        _ => {}
                    }
                }
                true
            }
            (Spec::Dt | Spec::Num(_), ParamType::Number) => true,
            (Spec::Num(v), ParamType::I32) => v.fract() == 0.0 && v.abs() <= i32::MAX as f64,
            (Spec::Tick | Spec::Len | Spec::Rand, ParamType::Number | ParamType::I32) => true,
            (Spec::ChunkStart, ParamType::I32) => true,
            (Spec::Bool(_), ParamType::Boolean) => true,
            _ => false,
        };
        if !ok {
            return Err(err(format!(
                "argument `{arg}` does not fit parameter `{pname}: {}`",
                p.ts_name()
            )));
        }
    }
    Ok(())
}

/// Runs every [`JobSchedule`] step of the world, entity by entity in scene-id order.
pub fn run_schedules(world: &mut World) {
    let entities = scheduled(world);
    if entities.is_empty() {
        return;
    }
    let tick = world.get_resource::<SimClock>().map_or(0, |c| c.tick);
    let dt = world.get_resource::<FixedStep>().map_or(DEFAULT_DT, |s| s.dt) as f64;
    for (id, e) in entities {
        let Some(schedule) = world.get::<JobSchedule>(e) else {
            continue;
        };
        let steps = schedule.steps.clone();
        for step in &steps {
            if let Err(d) = run_step(world, e, step, tick, dt)
                && let Some(mut log) = world.get_resource_mut::<EventLog>()
            {
                log.push(
                    tick,
                    "job_error",
                    Some(id),
                    None,
                    serde_json::json!({
                        "path": step.path,
                        "function": step.function,
                        "code": d.code,
                        "message": d.message,
                        "line": d.line,
                    }),
                );
            }
        }
    }
}

fn run_step(world: &mut World, e: Entity, step: &JobStep, tick: u64, dt: f64) -> Result<(), Diagnostic> {
    let program = library_get(world, &step.path)?;
    let info = program.exported(&step.function)?;
    let params: Vec<ParamType> = info.params.iter().map(|(_, p)| *p).collect();
    let specs: Vec<Spec> = step.args.iter().map(|a| parse_spec(a)).collect();
    // The run's length, for `len`: the first chunked array's length.
    let len = specs
        .iter()
        .find_map(|s| match s {
            Spec::Array(n, m) if *m != JobArgMode::Whole => Some(
                world
                    .get::<JobData>(e)
                    .and_then(|d| d.get(n))
                    .map_or(0, JobValues::len),
            ),
            _ => None,
        })
        .unwrap_or(1);
    let mut args = Vec::with_capacity(specs.len());
    for (spec, p) in specs
        .into_iter()
        .zip(params.iter().chain(std::iter::repeat(&ParamType::Void)))
    {
        args.push(match spec {
            Spec::Array(name, mode) => JobCallArg::Data {
                entity: e,
                name,
                mode,
            },
            Spec::Dt => JobCallArg::Number(dt),
            Spec::Tick => JobCallArg::Number(tick as f64),
            Spec::Len => JobCallArg::Number(len as f64),
            Spec::ChunkStart => JobCallArg::ChunkStart,
            Spec::Num(v) => JobCallArg::Number(v),
            Spec::Bool(b) => JobCallArg::Bool(b),
            Spec::Rand => {
                let mut rng = world.resource_mut::<SimRng>();
                JobCallArg::Number(if *p == ParamType::I32 {
                    (rng.next_u64() >> 33) as f64
                } else {
                    rng.next_f32() as f64
                })
            }
        });
    }
    let call = JobCall {
        path: &step.path,
        function: &step.function,
        chunk: step.chunk as usize,
    };
    run_call(world, call, &mut args).map(|_| ())
}

fn library_get(world: &mut World, path: &str) -> Result<Arc<JobProgram>, Diagnostic> {
    let Some(mut lib) = world.get_resource_mut::<JobLibrary>() else {
        return Err(Diagnostic::error(
            codes::INTERNAL,
            "the jobs module is not initialised",
        ));
    };
    lib.get(path).map_err(|mut errs| errs.remove(0))
}

/// The [`JobRunner`] function: runs `call` with `args`, using job data in place.
pub fn run_call(
    world: &mut World,
    call: JobCall<'_>,
    args: &mut [JobCallArg<'_>],
) -> Result<Vec<f64>, Diagnostic> {
    let program = library_get(world, call.path)?;
    let info = program.exported(call.function)?;
    let jobs = world.get_resource::<Jobs>().cloned().unwrap_or_else(Jobs::inline);
    let args_err = |m: String| {
        Diagnostic::error(codes::ARGS, m)
            .file(program.file())
            .fix(format!("`{}` takes ({})", info.name, crate::signature(info)))
    };

    // Take the job arrays out of their components for the run (no copy); they go back below
    // whatever happens.
    let mut taken: Vec<Option<(Entity, String, JobValues)>> = Vec::with_capacity(args.len());
    let mut problem = None;
    for a in args.iter() {
        let JobCallArg::Data { entity, name, .. } = a else {
            taken.push(None);
            continue;
        };
        if taken
            .iter()
            .flatten()
            .any(|(te, tn, _)| te == entity && tn == name)
        {
            problem.get_or_insert_with(|| args_err(format!("job array `{name}` is passed twice")));
            taken.push(None);
            continue;
        }
        let values = world
            .get_mut::<JobData>(*entity)
            .and_then(|mut d| d.get_mut(name).map(|v| std::mem::replace(v, empty_like(v))));
        if values.is_none() {
            problem.get_or_insert_with(|| {
                Diagnostic::error(codes::ARGS, format!("the entity has no job array `{name}`"))
                    .file(program.file())
                    .fix("add it to the entity's `JobData` (in the scene, or with `engine.jobs.set`)")
            });
        }
        taken.push(values.map(|v| (*entity, name.clone(), v)));
    }

    let result = match problem {
        Some(d) => Err(d),
        None => run_taken(&program, &jobs, call, info, args, &mut taken),
    };

    for (entity, name, values) in taken.into_iter().flatten() {
        if let Some(mut d) = world.get_mut::<JobData>(entity)
            && let Some(slot) = d.get_mut(&name)
        {
            *slot = values;
        }
    }
    result
}

fn empty_like(v: &JobValues) -> JobValues {
    match v {
        JobValues::F32(_) => JobValues::F32(Vec::new()),
        JobValues::F64(_) => JobValues::F64(Vec::new()),
        JobValues::I32(_) => JobValues::I32(Vec::new()),
    }
}

fn run_taken(
    program: &JobProgram,
    jobs: &Jobs,
    call: JobCall<'_>,
    info: &crate::FunctionInfo,
    args: &mut [JobCallArg<'_>],
    taken: &mut [Option<(Entity, String, JobValues)>],
) -> Result<Vec<f64>, Diagnostic> {
    let mut len: Option<usize> = None;
    let mut run_args: Vec<Arg<'_>> = Vec::with_capacity(args.len());
    for (i, (a, t)) in args.iter_mut().zip(taken.iter_mut()).enumerate() {
        let param = info.params.get(i).map(|(_, p)| *p);
        let arg = match (a, t) {
            (JobCallArg::Data { mode, .. }, Some((_, _, values))) => {
                let mode = *mode;
                match values {
                    JobValues::F32(v) => array_arg(
                        v.as_mut_slice(),
                        mode,
                        &mut len,
                        Arg::F32,
                        Arg::F32In,
                        Arg::F32All,
                    ),
                    JobValues::F64(v) => array_arg(
                        v.as_mut_slice(),
                        mode,
                        &mut len,
                        Arg::F64,
                        Arg::F64In,
                        Arg::F64All,
                    ),
                    JobValues::I32(v) => array_arg(
                        v.as_mut_slice(),
                        mode,
                        &mut len,
                        Arg::I32,
                        Arg::I32In,
                        Arg::I32All,
                    ),
                }
            }
            (JobCallArg::F32(v, mode), _) => array_arg(v, *mode, &mut len, Arg::F32, Arg::F32In, Arg::F32All),
            (JobCallArg::F64(v, mode), _) => array_arg(v, *mode, &mut len, Arg::F64, Arg::F64In, Arg::F64All),
            (JobCallArg::I32(v, mode), _) => array_arg(v, *mode, &mut len, Arg::I32, Arg::I32In, Arg::I32All),
            (JobCallArg::Number(v), _) => {
                if param == Some(ParamType::I32) {
                    if v.fract() != 0.0 || v.abs() > i32::MAX as f64 {
                        return Err(Diagnostic::error(
                            codes::ARGS,
                            format!("argument {i} is {v}, but its parameter is an i32 (a whole number)"),
                        )
                        .file(program.file())
                        .fix("pass a whole number, or `x | 0`"));
                    }
                    Arg::Int(*v as i32)
                } else {
                    Arg::Number(*v)
                }
            }
            (JobCallArg::Bool(b), _) => Arg::Bool(*b),
            (JobCallArg::ChunkStart, _) => Arg::ChunkStart,
            (JobCallArg::Data { .. }, None) => unreachable!("missing arrays are reported before the run"),
        };
        run_args.push(arg);
    }
    let chunk = if call.chunk == 0 {
        DEFAULT_CHUNK
    } else {
        call.chunk
    };
    match len {
        Some(n) => program.run(jobs, call.function, n, chunk, &mut run_args),
        // No chunked array: one call.
        None => program.run(jobs, call.function, 1, 1, &mut run_args),
    }
}

/// Makes the [`Arg`] for an array in `mode`, noting the run length of chunked arrays (the
/// first one sets it; the runtime reports a mismatch).
fn array_arg<'a, T>(
    v: &'a mut [T],
    mode: JobArgMode,
    len: &mut Option<usize>,
    write: fn(&'a mut [T]) -> Arg<'a>,
    read: fn(&'a [T]) -> Arg<'a>,
    whole: fn(&'a [T]) -> Arg<'a>,
) -> Arg<'a> {
    match mode {
        JobArgMode::Write => {
            len.get_or_insert(v.len());
            write(v)
        }
        JobArgMode::Read => {
            len.get_or_insert(v.len());
            read(v)
        }
        JobArgMode::Whole => whole(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_strings_parse() {
        assert_eq!(parse_spec("py"), Spec::Array("py".into(), JobArgMode::Write));
        assert_eq!(parse_spec("in:vy"), Spec::Array("vy".into(), JobArgMode::Read));
        assert_eq!(parse_spec("all:t"), Spec::Array("t".into(), JobArgMode::Whole));
        assert_eq!(parse_spec("-0.5"), Spec::Num(-0.5));
        assert_eq!(parse_spec("3"), Spec::Num(3.0));
        assert_eq!(parse_spec("dt"), Spec::Dt);
        assert_eq!(parse_spec("true"), Spec::Bool(true));
        // Names that `f64::from_str` would accept are still names.
        assert_eq!(parse_spec("inf"), Spec::Array("inf".into(), JobArgMode::Write));
        assert_eq!(parse_spec("NaN"), Spec::Array("NaN".into(), JobArgMode::Write));
    }
}
