//! Stability suites (decision 0031): every game mode the engine supports is run for a long,
//! seeded, replayable stretch and must not crash, hang, lose determinism, miss its time budget
//! or grow its memory. Results are data; a failure carries everything an agent needs to
//! reproduce and fix it (mode, scene, seed, tick, message, backtrace, the input it was fed, and
//! the command that replays it).
//!
//! Suite file (`stability/suite.json`, paths relative to the project root):
//!
//! ```json
//! {
//!   "format": "marionette.stability-suite", "version": 1, "seed": 1,
//!   "modes": [
//!     { "name": "physics-3d", "kind": "sim", "scene": "examples/playground3d.scene.json",
//!       "random_input": true, "fast_ticks": 600, "long_ticks": 36000, "tick_budget_ms": 2.0 }
//!   ],
//!   "exempt": { "examples/other.scene.json": "why it has no mode of its own" }
//! }
//! ```
//!
//! Kinds: `sim` (headless ticks), `render` (one tick and one offscreen frame per step; `views`
//! 2 renders both eyes of an emulated headset), `agent` (an agent drives a live session through
//! the operation table: step with input, observe, edit, snapshot, restore).
//!
//! Wall-clock time is read here only around the simulation (as `session` stats do), never by it.

use crate::{Agent, Op, op, read_text};
use marionette_core::scene::SceneId;
use marionette_core::{Diagnostic, Errors, EventLog, Sim, SimSnapshot};
use marionette_input::{ActionMap, InputEvent};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once, mpsc};
use std::time::{Duration, Instant};

pub(crate) fn ops() -> Vec<Op> {
    vec![op(
        "stability_run",
        "Run the stability suite (default `stability/suite.json`): every game mode for `fast` or `long` \
         length with seeded random input. Checks no panic, no hang, same hash on replay and after a \
         snapshot/restore, time budgets and memory growth. Failures come back as data with seed, tick, \
         backtrace and a reproduce command.",
        false,
        run,
    )]
}

pub const SUITE_FORMAT: &str = "marionette.stability-suite";
pub const REPORT_FORMAT: &str = "marionette.stability-report";
pub const DEFAULT_SUITE: &str = "stability/suite.json";
const THREAD_PREFIX: &str = "stability-";

/// Stable error codes of this module.
pub mod errors {
    pub const SUITE: &str = "stability.suite";
    pub const UNKNOWN_MODE: &str = "stability.unknown_mode";
}

/// What kind of failure a mode had. Stable strings: agents branch on them.
pub mod failure {
    /// The engine panicked. `backtrace` is set.
    pub const PANIC: &str = "panic";
    /// No tick finished for `hang_ms`.
    pub const HANG: &str = "hang";
    /// A second run with the same scene, seed and input gave another state hash.
    pub const NONDETERMINISM: &str = "nondeterminism";
    /// Restoring the mid-run snapshot and running to the end gave another state hash.
    pub const SNAPSHOT: &str = "snapshot";
    /// A time budget was missed (p99 above budget x scale).
    pub const BUDGET: &str = "budget";
    /// Memory grew more than the mode allows after warm-up.
    pub const MEMORY: &str = "memory";
    /// A script threw during a tick (`script_error` events).
    pub const SCRIPT_ERROR: &str = "script_error";
    /// Loading or an operation returned an error.
    pub const ERROR: &str = "error";
}

// ---------------------------------------------------------------------------------------------
// Suite file

#[derive(Deserialize, Serialize, JsonSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub format: String,
    pub version: u32,
    /// Seed of the fast suite (long runs use `seed`, `seed + 1`, ...).
    #[serde(default = "one")]
    pub seed: u64,
    pub modes: Vec<Mode>,
    /// Scene files that deliberately have no mode, with the reason.
    #[serde(default)]
    pub exempt: BTreeMap<String, String>,
}

fn one() -> u64 {
    1
}

#[derive(Deserialize, Serialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Headless ticks only.
    Sim,
    /// One tick and one offscreen frame per step (needs a GPU; software GPUs count).
    Render,
    /// An agent drives a live session through the operation table.
    Agent,
}

/// One operation run before the mode (for example to generate its scene).
#[derive(Deserialize, Serialize, JsonSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct SetupOp {
    pub op: String,
    #[serde(default)]
    pub input: Value,
}

#[derive(Deserialize, Serialize, JsonSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Mode {
    /// Unique name, e.g. `physics-3d`.
    pub name: String,
    /// What game mode this stands for (one line).
    #[serde(default)]
    pub description: String,
    pub kind: Kind,
    pub scene: String,
    /// Feed seeded random presses of every input in the scene's action map.
    #[serde(default)]
    pub random_input: bool,
    /// Scripted input file instead (or as well).
    pub input: Option<String>,
    pub fast_ticks: u64,
    pub long_ticks: u64,
    /// p99 budget of one simulation tick, milliseconds (release build).
    pub tick_budget_ms: Option<f64>,
    /// p99 budget of one whole frame (tick + extract + render + GPU wait), milliseconds.
    pub frame_budget_ms: Option<f64>,
    /// Allowed resident memory growth after warm-up, MiB (default 32).
    pub memory_growth_mib: Option<f64>,
    /// Render size (default `320x180`).
    pub size: Option<String>,
    /// Views per frame: 1, or 2 for both eyes of an emulated headset (default 1).
    pub views: Option<u32>,
    /// Seconds without a finished tick that count as a hang (default 20).
    pub hang_s: Option<f64>,
    /// Script errors during the run are allowed (default false: any is a failure).
    #[serde(default)]
    pub allow_script_errors: bool,
    /// Skip the replay check (same input, same hash).
    #[serde(default)]
    pub no_replay: bool,
    /// Skip the snapshot check (restore the mid-run snapshot, run on, same hash).
    #[serde(default)]
    pub no_snapshot: bool,
    /// Operations to run first (with the caller's read-only setting, so read-only refuses them).
    #[serde(default)]
    pub setup: Vec<SetupOp>,
}

/// Reads and checks a suite file.
pub fn load_suite(agent: &Agent, path: &str) -> Result<Suite, Errors> {
    let text = read_text(&agent.resolve(path)?, path)?;
    let suite: Suite = serde_json::from_str(&text).map_err(|e| {
        Errors::from(
            Diagnostic::error(errors::SUITE, format!("invalid stability suite: {e}"))
                .file(path)
                .at(Some(e.line()), Some(e.column()))
                .fix("see `stability_run` in docs/ai/reference/operations.md and docs/ai/STABILITY.md"),
        )
    })?;
    let mut problems = Vec::new();
    if suite.format != SUITE_FORMAT {
        problems.push(format!("format must be `{SUITE_FORMAT}`"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for m in &suite.modes {
        if !seen.insert(m.name.as_str()) {
            problems.push(format!("mode `{}` appears twice", m.name));
        }
        if m.fast_ticks == 0 || m.long_ticks < m.fast_ticks {
            problems.push(format!("mode `{}`: need 0 < fast_ticks <= long_ticks", m.name));
        }
        if !matches!(m.views.unwrap_or(1), 1 | 2) {
            problems.push(format!("mode `{}`: views must be 1 or 2", m.name));
        }
        if m.views.unwrap_or(1) == 2 && m.kind != Kind::Render {
            problems.push(format!("mode `{}`: views 2 needs kind `render`", m.name));
        }
    }
    if problems.is_empty() {
        Ok(suite)
    } else {
        Err(Diagnostic::error(
            errors::SUITE,
            format!("invalid stability suite: {}", problems.join("; ")),
        )
        .file(path)
        .into())
    }
}

// ---------------------------------------------------------------------------------------------
// Operation input and output

#[derive(Deserialize, Serialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Length {
    /// `fast_ticks` per mode, one seed: every push.
    #[default]
    Fast,
    /// `long_ticks` per mode, `seeds` seeds: scheduled or on demand.
    Long,
}

/// Makes the harness fail on purpose, to test that failures are caught and reported.
#[derive(Deserialize, Serialize, JsonSchema, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Inject {
    /// Only this mode (default: every mode).
    pub mode: Option<String>,
    /// Panic at this tick.
    pub panic_at: Option<u64>,
    /// Stop making progress at this tick (until the hang is detected).
    pub hang_at: Option<u64>,
    /// Seed the replay run's simulation RNG differently (its input stays the same).
    #[serde(default)]
    pub nondeterminism: bool,
    /// Sleep this long in every tick (to miss a budget).
    pub slow_ms: Option<f64>,
    /// Keep this many KiB per tick alive until the run ends (to fail the memory check).
    pub leak_kib: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunInput {
    /// Suite file (default `stability/suite.json`).
    pub suite: Option<String>,
    /// Only these modes (default: all).
    #[serde(default)]
    pub modes: Vec<String>,
    /// `fast` (default) or `long`.
    #[serde(default)]
    pub length: Length,
    /// First seed (default: the suite's seed).
    pub seed: Option<u64>,
    /// Seeds per mode (default 1 for fast, 3 for long).
    pub seeds: Option<u32>,
    /// Ticks per mode, overriding fast_ticks/long_ticks.
    pub ticks: Option<u64>,
    /// Multiplies every time budget (default: env `MARIONETTE_BUDGET_SCALE`, else 1).
    pub budget_scale: Option<f64>,
    /// Check time budgets (default: only in release builds; debug timings mean nothing).
    pub enforce_timing: Option<bool>,
    /// Seconds without a finished tick that count as a hang, overriding every mode's `hang_s`.
    pub hang_s: Option<f64>,
    /// Test the harness: make modes fail on purpose.
    pub inject: Option<Inject>,
}

#[derive(Serialize, JsonSchema, Clone, Debug, Default)]
pub struct Percentiles {
    pub count: usize,
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

impl Percentiles {
    fn of(samples: &[f64]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        let mut s = samples.to_vec();
        s.sort_by(f64::total_cmp);
        let pct = |p: f64| s[((s.len() as f64 - 1.0) * p).round() as usize];
        let r = |x: f64| (x * 1000.0).round() / 1000.0;
        Self {
            count: s.len(),
            avg_ms: r(s.iter().sum::<f64>() / s.len() as f64),
            p50_ms: r(pct(0.5)),
            p99_ms: r(pct(0.99)),
            max_ms: r(*s.last().expect("not empty")),
        }
    }
}

#[derive(Serialize, JsonSchema, Clone, Debug, Default)]
pub struct Memory {
    /// What was measured: `resident` (process RSS) or the name of a custom probe.
    pub probe: String,
    pub start_mib: f64,
    pub after_warmup_mib: f64,
    pub end_mib: f64,
    pub peak_mib: f64,
    pub growth_mib: f64,
    pub limit_mib: f64,
}

#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct Failure {
    /// One of `panic`, `hang`, `nondeterminism`, `snapshot`, `budget`, `memory`,
    /// `script_error`, `error`.
    pub kind: String,
    /// Tick the failure happened at, if it belongs to one.
    pub tick: Option<u64>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backtrace: Option<String>,
}

#[derive(Serialize, JsonSchema, Clone, Debug, Default)]
pub struct ModeResult {
    pub format: String,
    pub mode: String,
    pub description: String,
    pub kind: String,
    pub scene: String,
    pub seed: u64,
    pub ticks: u64,
    pub passed: bool,
    /// Why the mode did not run (no GPU). Not a failure unless `MARIONETTE_REQUIRE_GPU=1`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    pub failures: Vec<Failure>,
    /// Ticks completed before the end or the failure.
    pub ticks_done: u64,
    pub state_hash: String,
    pub tick: Percentiles,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<Percentiles>,
    /// The GPU adapter of a render mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<String>,
    /// Whether `frame_budget_ms` was checked: only on a hardware GPU, since a software
    /// rasterizer (lavapipe, WARP) draws on the CPU and says nothing about frame time
    /// (decision 0023). Tick budgets are checked either way.
    pub frame_budget_checked: bool,
    pub tick_budget_ms: Option<f64>,
    pub frame_budget_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<Memory>,
    pub entities_start: usize,
    pub entities_end: usize,
    pub engine_events: usize,
    pub script_errors: usize,
    pub seconds: f64,
    /// Shell command that reruns exactly this mode, seed and length.
    pub reproduce: String,
    /// The input this run was fed (only when it failed), as a replayable events file body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_events: Option<Vec<InputEvent>>,
}

#[derive(Serialize, JsonSchema)]
pub struct RunOutput {
    pub suite: String,
    pub length: Length,
    pub passed: bool,
    pub timing_enforced: bool,
    pub budget_scale: f64,
    pub build: String,
    pub results: Vec<ModeResult>,
}

// ---------------------------------------------------------------------------------------------
// Memory probe

/// Reads the memory in use, in bytes; `None` if unavailable on this platform.
pub type MemoryProbe = fn() -> Option<u64>;

static PROBE: Mutex<Option<(&'static str, MemoryProbe)>> = Mutex::new(None);

/// Replaces the memory probe (default: process resident set size where the platform has one).
/// A binary with a counting allocator installs it here so every platform is measured.
pub fn set_memory_probe(name: &'static str, probe: MemoryProbe) {
    *PROBE.lock().unwrap_or_else(|e| e.into_inner()) = Some((name, probe));
}

fn probe() -> (&'static str, MemoryProbe) {
    PROBE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or(("resident", resident_bytes))
}

/// Process resident set size (Linux: `/proc/self/statm`). `None` elsewhere.
pub fn resident_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = text.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096)
}

fn mib(bytes: u64) -> f64 {
    (bytes as f64 / (1024.0 * 1024.0) * 100.0).round() / 100.0
}

// ---------------------------------------------------------------------------------------------
// Panics: a hook keeps the message and backtrace of panics on stability worker threads.

type PanicRecord = (String, String);
static PANICS: Mutex<BTreeMap<String, PanicRecord>> = Mutex::new(BTreeMap::new());

fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            match thread.name().filter(|n| n.starts_with(THREAD_PREFIX)) {
                Some(name) => {
                    let payload = info
                        .payload()
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| info.payload().downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "non-string panic payload".into());
                    let at = info
                        .location()
                        .map(|l| format!(" at {}:{}", l.file(), l.line()))
                        .unwrap_or_default();
                    let mut bt = std::backtrace::Backtrace::force_capture().to_string();
                    if bt.len() > 24_000 {
                        bt.truncate(24_000);
                        bt.push_str("\n...(truncated)");
                    }
                    PANICS
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(name.to_string(), (format!("{payload}{at}"), bt));
                }
                None => previous(info),
            }
        }));
    });
}

// ---------------------------------------------------------------------------------------------
// Random input

/// Seeded random input for `ticks` ticks over every device input in the action map: about one
/// change every six ticks; keys toggle, axes take values in steps of 0.1. Independent of the
/// simulation's own RNG, so feeding it does not change what the scene does with its seed.
pub fn random_events(map: &ActionMap, seed: u64, ticks: u64) -> Vec<InputEvent> {
    let mut inputs: Vec<&str> = map
        .actions
        .values()
        .flat_map(|a| a.bindings.iter().map(|b| b.input.as_str()))
        .collect();
    inputs.sort_unstable();
    inputs.dedup();
    if inputs.is_empty() {
        return Vec::new();
    }
    let mut rng = marionette_core::rng::SimRng::new(seed ^ 0x5717_ab11_7e57_0001);
    let mut held: BTreeMap<&str, bool> = BTreeMap::new();
    let mut out = Vec::new();
    for tick in 0..ticks {
        if !rng.next_u64().is_multiple_of(6) {
            continue;
        }
        let input = inputs[(rng.next_u64() % inputs.len() as u64) as usize];
        let value = if input.starts_with("axis:") {
            ((rng.next_u64() % 21) as f32 - 10.0) / 10.0
        } else {
            let h = held.entry(input).or_insert(false);
            *h = !*h;
            if *h { 1.0 } else { 0.0 }
        };
        out.push(InputEvent {
            tick,
            input: Some(input.to_string()),
            action: None,
            value,
        });
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Running

fn run(agent: &mut Agent, input: RunInput) -> Result<RunOutput, Errors> {
    let suite_path = input.suite.clone().unwrap_or_else(|| DEFAULT_SUITE.into());
    let suite = load_suite(agent, &suite_path)?;
    for name in &input.modes {
        if !suite.modes.iter().any(|m| &m.name == name) {
            let names: Vec<&str> = suite.modes.iter().map(|m| m.name.as_str()).collect();
            return Err(
                Diagnostic::error(errors::UNKNOWN_MODE, format!("no mode `{name}` in {suite_path}"))
                    .fix(format!("modes: {}", names.join(", ")))
                    .into(),
            );
        }
    }
    let budget_scale = input.budget_scale.unwrap_or_else(|| {
        std::env::var("MARIONETTE_BUDGET_SCALE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0)
    });
    let release = !cfg!(debug_assertions);
    let timing_enforced = input.enforce_timing.unwrap_or(release);
    let first_seed = input.seed.unwrap_or(suite.seed);
    let seeds = input.seeds.unwrap_or(match input.length {
        Length::Fast => 1,
        Length::Long => 3,
    });
    install_panic_hook();
    let mut results = Vec::new();
    for mode in &suite.modes {
        if !input.modes.is_empty() && !input.modes.contains(&mode.name) {
            continue;
        }
        let ticks = input.ticks.unwrap_or(match input.length {
            Length::Fast => mode.fast_ticks,
            Length::Long => mode.long_ticks,
        });
        let inject = input
            .inject
            .clone()
            .filter(|i| i.mode.as_ref().is_none_or(|m| m == &mode.name));
        let mut setup_error = None;
        // The caller's operation table is checked out while this operation runs, so setup
        // goes through a second agent on the same project with the same read-only setting.
        let mut setup_agent = (!mode.setup.is_empty())
            .then(|| Agent::with_engine(agent.root(), agent.read_only(), agent.engine().clone()));
        for s in &mode.setup {
            let helper = setup_agent.as_mut().expect("created when there is setup");
            if let Err(e) = helper.call(&s.op, s.input.clone()) {
                setup_error = Some(format!("setup operation `{}` failed: {e}", s.op));
                break;
            }
        }
        for k in 0..seeds.max(1) {
            let seed = first_seed + u64::from(k);
            let mut job_mode = mode.clone();
            job_mode.hang_s = input.hang_s.or(mode.hang_s);
            let job = Job {
                engine: agent.engine().clone(),
                mode: job_mode,
                root: agent.root().to_path_buf(),
                scene_path: agent.resolve(&mode.scene)?,
                input_path: mode.input.as_deref().map(|p| agent.resolve(p)).transpose()?,
                seed,
                ticks,
                budget_scale,
                timing_enforced,
                inject: inject.clone().unwrap_or_default(),
            };
            let mut result = match &setup_error {
                Some(message) => {
                    let mut r = job.empty_result();
                    r.failures.push(Failure {
                        kind: failure::ERROR.into(),
                        tick: None,
                        message: message.clone(),
                        backtrace: None,
                    });
                    r
                }
                None => supervise(job.clone()),
            };
            result.passed = result.failures.is_empty();
            result.reproduce = reproduce(&suite_path, &mode.name, seed, ticks);
            if result.passed {
                result.input_events = None;
            }
            results.push(result);
        }
    }
    Ok(RunOutput {
        suite: suite_path,
        length: input.length,
        passed: results.iter().all(|r| r.passed),
        timing_enforced,
        budget_scale,
        build: if release { "release" } else { "debug" }.into(),
        results,
    })
}

fn reproduce(suite: &str, mode: &str, seed: u64, ticks: u64) -> String {
    let suite = if suite == DEFAULT_SUITE {
        String::new()
    } else {
        format!(" --suite {suite}")
    };
    format!(
        "cargo run --release -q -p marionette-cli -- stability{suite} --mode {mode} --seed {seed} --ticks {ticks}"
    )
}

#[derive(Clone)]
struct Job {
    engine: crate::Engine,
    mode: Mode,
    root: PathBuf,
    scene_path: PathBuf,
    input_path: Option<PathBuf>,
    seed: u64,
    ticks: u64,
    budget_scale: f64,
    timing_enforced: bool,
    inject: Inject,
}

impl Job {
    fn empty_result(&self) -> ModeResult {
        ModeResult {
            format: REPORT_FORMAT.into(),
            mode: self.mode.name.clone(),
            description: self.mode.description.clone(),
            kind: format!("{:?}", self.mode.kind).to_lowercase(),
            scene: self.mode.scene.clone(),
            seed: self.seed,
            ticks: self.ticks,
            tick_budget_ms: self.mode.tick_budget_ms,
            frame_budget_ms: self.mode.frame_budget_ms,
            ..Default::default()
        }
    }
}

/// Shared between the worker and the supervisor.
struct Progress {
    ticks: AtomicU64,
    cancel: AtomicBool,
}

/// Runs a job on its own thread and watches it: a panic becomes a `panic` failure with its
/// backtrace, no progress for `hang_s` becomes a `hang` failure (the thread is left behind;
/// the CLI exits after reporting, so nothing outlives the process).
fn supervise(job: Job) -> ModeResult {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "{THREAD_PREFIX}{}-{}-{}",
        job.mode.name,
        job.seed,
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let progress = Arc::new(Progress {
        ticks: AtomicU64::new(0),
        cancel: AtomicBool::new(false),
    });
    let partial = Arc::new(Mutex::new(job.empty_result()));
    let (tx, rx) = mpsc::channel();
    let hang = Duration::from_secs_f64(job.mode.hang_s.unwrap_or(20.0));
    let worker = {
        let (job, progress, partial) = (job.clone(), progress.clone(), partial.clone());
        std::thread::Builder::new()
            .name(name.clone())
            .stack_size(64 << 20)
            .spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work(&job, &progress, &partial)
                }));
                let _ = tx.send(r.is_ok());
            })
    };
    let started = Instant::now();
    let mut finished = None;
    match worker {
        Err(e) => {
            let mut r = job.empty_result();
            r.failures.push(Failure {
                kind: failure::ERROR.into(),
                tick: None,
                message: format!("cannot start a worker thread: {e}"),
                backtrace: None,
            });
            return r;
        }
        Ok(handle) => {
            let mut last = (0, Instant::now());
            loop {
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(ok) => {
                        let _ = handle.join();
                        finished = Some(ok);
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        let _ = handle.join();
                        finished = Some(false);
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        let now = progress.ticks.load(Ordering::Relaxed);
                        if now != last.0 {
                            last = (now, Instant::now());
                        } else if last.1.elapsed() >= hang {
                            progress.cancel.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                }
            }
        }
    }
    let mut result = partial.lock().unwrap_or_else(|e| e.into_inner()).clone();
    result.seconds = (started.elapsed().as_secs_f64() * 100.0).round() / 100.0;
    let at = progress.ticks.load(Ordering::Relaxed);
    match finished {
        Some(true) => {}
        Some(false) => {
            let (message, backtrace) = PANICS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&name)
                .unwrap_or_else(|| ("panic without a message".into(), String::new()));
            result.ticks_done = at;
            result.failures.push(Failure {
                kind: failure::PANIC.into(),
                tick: Some(at),
                message,
                backtrace: Some(backtrace).filter(|b| !b.is_empty()),
            });
        }
        None => {
            result.ticks_done = at;
            result.failures.push(Failure {
                kind: failure::HANG.into(),
                tick: Some(at),
                message: format!(
                    "no tick finished for {:.1} s (stuck in or after tick {at}); the worker thread was abandoned",
                    hang.as_secs_f64()
                ),
                backtrace: None,
            });
        }
    }
    result
}

/// What one pass over the scene produced.
struct Pass {
    hash: String,
    snapshot: Option<SimSnapshot>,
}

fn fail(r: &Mutex<ModeResult>, kind: &str, tick: Option<u64>, message: String) {
    r.lock()
        .unwrap_or_else(|e| e.into_inner())
        .failures
        .push(Failure {
            kind: kind.into(),
            tick,
            message,
            backtrace: None,
        });
}

fn entities(sim: &mut Sim) -> usize {
    sim.world.query::<&SceneId>().iter(&sim.world).count()
}

fn load(job: &Job) -> Result<Sim, Errors> {
    job.engine.load_file(&job.scene_path, &job.mode.scene, job.seed)
}

fn inputs_for(job: &Job, sim: &Sim) -> Result<Vec<InputEvent>, Errors> {
    let mut events = Vec::new();
    if let Some(p) = &job.input_path {
        events.extend(marionette_input::load_events(
            p,
            job.mode.input.as_deref().unwrap_or(""),
        )?);
    }
    if job.mode.random_input
        && let Some(map) = sim.world.get_resource::<ActionMap>()
    {
        events.extend(random_events(map, job.seed, job.ticks));
    }
    events.sort_by_key(|e| e.tick);
    Ok(events)
}

fn require_gpu() -> bool {
    std::env::var("MARIONETTE_REQUIRE_GPU").as_deref() == Ok("1")
}

/// The worker: the measured run, then the replay and snapshot checks.
fn work(job: &Job, progress: &Progress, out: &Mutex<ModeResult>) {
    let r = match job.mode.kind {
        Kind::Sim | Kind::Render => work_sim(job, progress, out),
        Kind::Agent => work_agent(job, progress, out),
    };
    if let Err(e) = r {
        fail(out, failure::ERROR, None, e.to_string());
    }
}

struct Meter {
    ticks: Vec<f64>,
    frames: Vec<f64>,
    warmup: u64,
    probe: (&'static str, MemoryProbe),
    start: Option<u64>,
    after_warmup: Option<u64>,
    peak: u64,
    leak: Vec<Vec<u8>>,
    /// Frames are drawn by a software rasterizer: frame times are not checked.
    software_gpu: bool,
}

impl Meter {
    fn new(ticks: u64) -> Self {
        let probe = probe();
        let start = (probe.1)();
        Self {
            ticks: Vec::with_capacity(ticks as usize),
            frames: Vec::new(),
            warmup: (ticks / 5).max(1),
            probe,
            start,
            after_warmup: None,
            peak: start.unwrap_or(0),
            leak: Vec::new(),
            software_gpu: false,
        }
    }

    fn sample(&mut self, tick: u64, every: u64) {
        if !tick.is_multiple_of(every) && tick + 1 != self.warmup {
            return;
        }
        if let Some(b) = (self.probe.1)() {
            self.peak = self.peak.max(b);
            if tick + 1 >= self.warmup && self.after_warmup.is_none() {
                self.after_warmup = Some(b);
            }
        }
    }

    /// Writes timing and memory into the result and checks budgets and growth.
    fn finish(self, job: &Job, out: &Mutex<ModeResult>) {
        let skip = self.warmup.min(self.ticks.len() as u64 / 2) as usize;
        let tick = Percentiles::of(&self.ticks[skip.min(self.ticks.len())..]);
        let frame =
            (!self.frames.is_empty()).then(|| Percentiles::of(&self.frames[skip.min(self.frames.len())..]));
        let end = (self.probe.1)();
        let mut r = out.lock().unwrap_or_else(|e| e.into_inner());
        if job.timing_enforced {
            let s = job.budget_scale;
            if let Some(b) = job.mode.tick_budget_ms
                && tick.p99_ms > b * s
            {
                r.failures.push(Failure {
                    kind: failure::BUDGET.into(),
                    tick: None,
                    message: format!("tick p99 {} ms is over the budget {b} ms x {s}", tick.p99_ms),
                    backtrace: None,
                });
            }
            if let (Some(b), Some(f), false) = (job.mode.frame_budget_ms, &frame, self.software_gpu)
                && f.p99_ms > b * s
            {
                r.failures.push(Failure {
                    kind: failure::BUDGET.into(),
                    tick: None,
                    message: format!("frame p99 {} ms is over the budget {b} ms x {s}", f.p99_ms),
                    backtrace: None,
                });
            }
        }
        if let (Some(start), Some(warm), Some(end)) = (self.start, self.after_warmup, end) {
            let limit = job.mode.memory_growth_mib.unwrap_or(32.0);
            let growth = mib(end.saturating_sub(warm));
            if growth > limit {
                r.failures.push(Failure {
                    kind: failure::MEMORY.into(),
                    tick: None,
                    message: format!(
                        "memory ({}) grew {growth} MiB after warm-up (limit {limit} MiB): {} -> {} MiB",
                        self.probe.0,
                        mib(warm),
                        mib(end)
                    ),
                    backtrace: None,
                });
            }
            r.memory = Some(Memory {
                probe: self.probe.0.into(),
                start_mib: mib(start),
                after_warmup_mib: mib(warm),
                end_mib: mib(end),
                peak_mib: mib(self.peak.max(end)),
                growth_mib: growth,
                limit_mib: limit,
            });
        }
        r.frame_budget_checked = job.timing_enforced
            && job.mode.frame_budget_ms.is_some()
            && frame.is_some()
            && !self.software_gpu;
        r.tick = tick;
        r.frame = frame;
        drop(self.leak);
    }
}

fn inject_tick(job: &Job, tick: u64, progress: &Progress, meter: &mut Meter) {
    if job.inject.panic_at == Some(tick) {
        panic!("injected panic at tick {tick} (stability harness test)");
    }
    if job.inject.hang_at == Some(tick) {
        while !progress.cancel.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("injected hang cancelled");
    }
    if let Some(ms) = job.inject.slow_ms {
        std::thread::sleep(Duration::from_secs_f64(ms / 1000.0));
    }
    if let Some(kib) = job.inject.leak_kib {
        meter.leak.push(vec![1u8; (kib * 1024) as usize]);
    }
}

fn work_sim(job: &Job, progress: &Progress, out: &Mutex<ModeResult>) -> Result<(), Errors> {
    let mut sim = load(job)?;
    let events = inputs_for(job, &sim)?;
    if !events.is_empty() {
        marionette_input::feed(&mut sim.world, &events)?;
    }
    {
        let mut r = out.lock().unwrap_or_else(|e| e.into_inner());
        r.entities_start = entities(&mut sim);
        r.input_events = Some(events.clone());
    }
    // The renderer, for render modes.
    let mut render = None;
    if job.mode.kind == Kind::Render {
        match marionette_render::Gpu::headless() {
            Ok(gpu) => {
                let (w, h) = crate::observe::parse_size(job.mode.size.as_deref().unwrap_or("320x180"))?;
                let dir = job.scene_path.parent().map(Path::to_path_buf).unwrap_or_default();
                let off = marionette_render::Offscreen::new(&gpu, w, h, 1, dir);
                render = Some((gpu, off));
            }
            Err(e) if !require_gpu() => {
                out.lock().unwrap_or_else(|e| e.into_inner()).skipped = Some(format!(
                    "no GPU adapter ({e}); set MARIONETTE_REQUIRE_GPU=1 to make this a failure"
                ));
                return Ok(());
            }
            Err(e) => return Err(e),
        }
    }
    let views = job.mode.views.unwrap_or(1);
    let mut meter = Meter::new(job.ticks);
    if let Some((gpu, _)) = &render {
        let info = gpu.adapter.get_info();
        meter.software_gpu = format!("{:?}", info.device_type) == "Cpu";
        out.lock().unwrap_or_else(|e| e.into_inner()).gpu = Some(gpu.describe());
    }
    let sample_every = (job.ticks / 50).max(1);
    let half = job.ticks / 2;
    let mut snapshot = None;
    for t in 0..job.ticks {
        let start = Instant::now();
        inject_tick(job, t, progress, &mut meter);
        sim.step();
        meter.ticks.push(start.elapsed().as_secs_f64() * 1000.0);
        if let Some((gpu, off)) = render.as_mut() {
            let frame = marionette_render::extract(&sim.world);
            if views == 2 {
                // Both eyes of an emulated headset, 64 mm apart (decision 0025).
                off.render_eyes(gpu, &frame, None, 2, 0.064)?;
            } else {
                off.render(gpu, &frame, None)?;
            }
            meter.frames.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        if t + 1 == half && !job.mode.no_snapshot {
            snapshot = Some(sim.save_state());
        }
        meter.sample(t, sample_every);
        progress.ticks.store(t + 1, Ordering::Relaxed);
    }
    let main = Pass {
        hash: sim.state_hash(),
        snapshot,
    };
    {
        let mut r = out.lock().unwrap_or_else(|e| e.into_inner());
        r.ticks_done = job.ticks;
        r.state_hash = main.hash.clone();
        r.entities_end = entities(&mut sim);
        let log = sim.world.get_resource::<EventLog>();
        r.engine_events = log.map_or(0, |l| l.events.len());
        let errors: Vec<_> = log
            .into_iter()
            .flat_map(|l| l.events.iter())
            .filter(|e| e.kind == "script_error")
            .collect();
        r.script_errors = errors.len();
        if let Some(first) = errors.first().filter(|_| !job.mode.allow_script_errors) {
            r.failures.push(Failure {
                kind: failure::SCRIPT_ERROR.into(),
                tick: Some(first.tick),
                message: format!("{} script error(s); first: {}", errors.len(), first.detail),
                backtrace: None,
            });
        }
    }
    meter.finish(job, out);
    drop(render);
    drop(sim);

    if !job.mode.no_replay {
        let mut again = if job.inject.nondeterminism {
            // Same input, but the simulation's own RNG starts elsewhere.
            job.engine
                .load_file(&job.scene_path, &job.mode.scene, job.seed ^ 0xbad)?
        } else {
            load(job)?
        };
        let events = inputs_for(job, &again)?;
        if !events.is_empty() {
            marionette_input::feed(&mut again.world, &events)?;
        }
        for t in 0..job.ticks {
            again.step();
            progress.ticks.store(job.ticks + t + 1, Ordering::Relaxed);
        }
        let hash = again.state_hash();
        if hash != main.hash {
            fail(
                out,
                failure::NONDETERMINISM,
                None,
                format!(
                    "replay with the same scene, seed and input ended at {hash}, first run at {}",
                    main.hash
                ),
            );
        }
    }
    if let Some(snap) = main.snapshot {
        let mut resumed = load(job)?;
        resumed.restore(&snap)?;
        let from = resumed.tick();
        for t in from..job.ticks {
            resumed.step();
            progress.ticks.store(2 * job.ticks + t + 1, Ordering::Relaxed);
        }
        let hash = resumed.state_hash();
        if hash != main.hash {
            fail(
                out,
                failure::SNAPSHOT,
                Some(snap.tick),
                format!(
                    "restoring the snapshot of tick {} and running on ended at {hash}, uninterrupted run at {}",
                    snap.tick, main.hash
                ),
            );
        }
    }
    Ok(())
}

/// An agent session: the steps an AI takes while building a game, repeated with seeded choices.
/// Returns the final state hash.
fn agent_pass(
    job: &Job,
    progress: &Progress,
    offset: u64,
    meter: Option<&mut Meter>,
    out: &Mutex<ModeResult>,
) -> Result<String, Errors> {
    let mut agent = Agent::with_engine(&job.root, false, job.engine.clone());
    let started = agent.call(
        "session_start",
        json!({ "scene": job.mode.scene, "seed": job.seed }),
    )?;
    let session = started["session"].as_str().unwrap_or_default().to_string();
    let map = agent
        .sessions
        .get(&session)
        .and_then(|s| s.sim.world.get_resource::<ActionMap>().cloned());
    let events = match (&map, job.mode.random_input) {
        (Some(m), true) => random_events(m, job.seed, job.ticks),
        _ => Vec::new(),
    };
    if offset == 0 {
        out.lock().unwrap_or_else(|e| e.into_inner()).input_events = Some(events.clone());
    }
    let mut rng = marionette_core::rng::SimRng::new(job.seed ^ 0xa6e7_0000_5e55_1011);
    let mut meter = meter;
    let mut created: Vec<u64> = Vec::new();
    let mut snapshotted = false;
    let mut tick = 0;
    let sample_every = (job.ticks / 50).max(1);
    while tick < job.ticks {
        let n = (1 + rng.next_u64() % 8).min(job.ticks - tick);
        let batch: Vec<&InputEvent> = events
            .iter()
            .filter(|e| e.tick >= tick && e.tick < tick + n)
            .collect();
        let start = Instant::now();
        agent.call(
            "session_step",
            json!({ "session": session, "ticks": n, "events": batch }),
        )?;
        let ms = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
        for t in tick..tick + n {
            if let Some(m) = meter.as_deref_mut() {
                inject_tick(job, t, progress, m);
                m.ticks.push(ms);
                m.sample(t, sample_every);
            }
            progress.ticks.store(offset + t + 1, Ordering::Relaxed);
        }
        tick += n;
        match rng.next_u64() % 8 {
            0 => {
                agent.call(
                    "observe_query",
                    json!({ "session": session, "with": ["Transform"], "select": ["Transform"] }),
                )?;
            }
            1 => {
                agent.call("observe_stats", json!({ "session": session }))?;
            }
            2 => {
                let r = agent.call(
                    "entity_create",
                    json!({ "session": session, "name": format!("stability-{tick}"), "components": {} }),
                )?;
                created.extend(r["id"].as_u64());
            }
            3 => {
                if let Some(id) = created.pop() {
                    agent.call("entity_delete", json!({ "session": session, "id": id }))?;
                }
            }
            4 if !snapshotted => {
                agent.call(
                    "session_snapshot",
                    json!({ "session": session, "name": "stability" }),
                )?;
                snapshotted = true;
            }
            5 => {
                agent.call("observe_events", json!({ "session": session }))?;
            }
            _ => {}
        }
    }
    let end = agent.call("session_step", json!({ "session": session, "ticks": 0 }))?;
    if offset == 0 {
        let s = agent.sessions.get_mut(&session).expect("session exists");
        let ents = entities(&mut s.sim);
        let mut r = out.lock().unwrap_or_else(|e| e.into_inner());
        r.entities_end = ents;
        r.engine_events = s.sim.events().events.len();
        r.ticks_done = job.ticks;
    }
    if snapshotted {
        // Restoring must work after a long run too.
        agent.call(
            "session_restore",
            json!({ "session": session, "snapshot": "stability" }),
        )?;
    }
    agent.call("session_stop", json!({ "session": session }))?;
    Ok(end["state_hash"].as_str().unwrap_or_default().to_string())
}

fn work_agent(job: &Job, progress: &Progress, out: &Mutex<ModeResult>) -> Result<(), Errors> {
    let mut meter = Meter::new(job.ticks);
    let hash = agent_pass(job, progress, 0, Some(&mut meter), out)?;
    out.lock().unwrap_or_else(|e| e.into_inner()).state_hash = hash.clone();
    meter.finish(job, out);
    if !job.mode.no_replay {
        let again = agent_pass(job, progress, job.ticks, None, out)?;
        if again != hash {
            fail(
                out,
                failure::NONDETERMINISM,
                None,
                format!("the same agent session replayed ended at {again}, first at {hash}"),
            );
        }
    }
    Ok(())
}

/// Every `*.scene.json` under `dir` (relative paths, `/` separators, sorted), skipping build
/// output and hidden folders. Used by the rule that every scene has a stability mode.
pub fn scene_files(root: &Path, dirs: &[&str]) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            if p.is_dir() {
                walk(root, &p, out);
            } else if name.ends_with(".scene.json")
                && let Ok(rel) = p.strip_prefix(root)
            {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    for d in dirs {
        walk(root, &root.join(d), &mut out);
    }
    out.sort();
    out
}
