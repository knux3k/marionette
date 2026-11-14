//! The shared performance suite (decision 0100): one fixed set of cases that every session
//! runs the same way, written to one report format, and a comparison of two reports that
//! says what got faster or slower and whether any output changed.
//!
//! A case is a scene run for a fixed number of ticks (`sim`), the same plus rendering each
//! tick offscreen (`render`), or the time to start the engine (`startup`). Every case records
//! the outputs that must not change when only speed changes: the state hash after the run
//! and, for render cases, the SHA-256 of the pixels of a fixed final frame. `perf_compare`
//! checks those before it reports any timing.
//!
//! Timings are measurements of this machine; nothing here feeds back into a simulation.

use crate::{Agent, Op, op, read_text, sha256_hex, write_atomic};
use marionette_core::{Diagnostic, Errors, Sim};
use marionette_render::{Offscreen, extract};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "perf_suite",
            "Run the shared performance suite (decision 0100): fixed sim, render and startup cases; \
             per case load time, tick and frame percentiles (CPU phases and GPU wait), heap, \
             allocations per tick, state hash and image hash. Writes the full report as JSON \
             (default .marionette/perf/<label>.json) and returns a one-line summary per case. \
             Compare two reports with `perf_compare`.",
            true,
            suite,
        ),
        op(
            "perf_compare",
            "Compare two `perf_suite` reports: first whether outputs are identical (state hashes; \
             image hashes when the GPU is the same), then the timing and memory changes beyond \
             `threshold_pct`, largest first.",
            false,
            compare,
        ),
    ]
}

/// Report format; bump when fields change meaning.
pub const FORMAT: &str = "marionette-perf/1";
/// Suite version; bump when cases change, so reports of different suites are not compared
/// case by case without a warning.
pub const SUITE: &str = "core/1";

// ---------------------------------------------------------------------------------------------
// Cases

#[derive(Clone, Copy)]
enum Source {
    File(&'static str),
    /// A scene written by an operation that takes `{ "out": <path>, "scale": <n> }`. A game
    /// or example adds the operation (the factory example adds `perf_factory_scene`,
    /// decision 0270); in an engine without it the case is skipped.
    Generated {
        op: &'static str,
        scale: u32,
    },
}

#[derive(Clone, Copy)]
struct RenderSpec {
    width: u32,
    height: u32,
    eyes: u32,
    msaa: u32,
    /// Turn the head as `bench --headset` does.
    headset: bool,
}

#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    source: Source,
    /// Timed ticks (or frames) at full length; quick length uses a tenth.
    ticks: u64,
    warmup: u64,
    render: Option<RenderSpec>,
}

const fn sim(name: &'static str, source: Source, ticks: u64) -> Case {
    Case {
        name,
        source,
        ticks,
        warmup: 60,
        render: None,
    }
}

const HD: RenderSpec = RenderSpec {
    width: 1920,
    height: 1080,
    eyes: 1,
    msaa: 4,
    headset: false,
};
const HEADSET: RenderSpec = RenderSpec {
    width: 2160,
    height: 2160,
    eyes: 2,
    msaa: 4,
    headset: true,
};

const fn render(name: &'static str, source: Source, frames: u64, spec: RenderSpec) -> Case {
    Case {
        name,
        source,
        ticks: frames,
        warmup: 30,
        render: Some(spec),
    }
}

const FACTORY_4: Source = Source::Generated {
    op: "perf_factory_scene",
    scale: 4,
};
const FACTORY_32: Source = Source::Generated {
    op: "perf_factory_scene",
    scale: 32,
};

/// The suite. Order is the report order. Cover each engine system once: core logic, physics
/// 2D and 3D, scripts, animation, job TypeScript, the factory module at two sizes, the six
/// proof games, and rendering of plain, opt-in looks, many instances, skinned models and a
/// headset.
const CASES: &[Case] = &[
    sim("sim/drift", Source::File("examples/drift.scene.json"), 1000),
    sim(
        "sim/physics-3d",
        Source::File("examples/playground3d.scene.json"),
        1000,
    ),
    sim(
        "sim/physics-2d",
        Source::File("examples/playground2d.scene.json"),
        1000,
    ),
    sim("sim/scripts", Source::File("examples/scripted.scene.json"), 1000),
    sim("sim/spawner", Source::File("examples/spawner.scene.json"), 1000),
    sim(
        "sim/animation",
        Source::File("examples/animation.scene.json"),
        1000,
    ),
    sim("sim/job-swarm", Source::File("examples/swarm.scene.json"), 600),
    sim("sim/factory-4x", FACTORY_4, 1200),
    sim("sim/factory-32x", FACTORY_32, 300),
    sim(
        "sim/game-lantern-keeper",
        Source::File("games/lantern-keeper/scenes/main.scene.json"),
        600,
    ),
    sim(
        "sim/game-ember-run",
        Source::File("games/ember-run/scenes/main.scene.json"),
        600,
    ),
    sim(
        "sim/game-cloudstep",
        Source::File("games/cloudstep/scenes/main.scene.json"),
        600,
    ),
    sim(
        "sim/game-crystal-ward",
        Source::File("games/crystal-ward/scenes/main.scene.json"),
        600,
    ),
    sim(
        "sim/game-dust-circuit",
        Source::File("games/dust-circuit/scenes/main.scene.json"),
        600,
    ),
    sim(
        "sim/game-signal-yard",
        Source::File("games/signal-yard/scenes/main.scene.json"),
        600,
    ),
    render(
        "render/showcase-1080p",
        Source::File("examples/showcase.scene.json"),
        300,
        HD,
    ),
    render(
        "render/looks-1080p",
        Source::File("examples/looks.scene.json"),
        300,
        HD,
    ),
    render(
        "render/lighting-1080p",
        Source::File("examples/lighting.scene.json"),
        300,
        HD,
    ),
    render(
        "render/sprites-1080p",
        Source::File("examples/sprites.scene.json"),
        300,
        HD,
    ),
    render(
        "render/animation-1080p",
        Source::File("examples/animation.scene.json"),
        300,
        HD,
    ),
    render(
        "render/job-swarm-1080p",
        Source::File("examples/swarm.scene.json"),
        300,
        HD,
    ),
    render(
        "render/game-crystal-ward-1080p",
        Source::File("games/crystal-ward/scenes/main.scene.json"),
        300,
        HD,
    ),
    render("render/factory-4x-headset", FACTORY_4, 300, HEADSET),
    render("render/factory-32x-headset", FACTORY_32, 120, HEADSET),
];

// ---------------------------------------------------------------------------------------------
// Report

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub count: usize,
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

/// Milliseconds to 0.1 microsecond, so ticks of a few microseconds keep their digits.
fn r3(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

impl Stats {
    fn of(samples: &[f64]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        let mut s = samples.to_vec();
        s.sort_by(f64::total_cmp);
        let pct = |p: f64| s[((s.len() as f64 - 1.0) * p).round() as usize];
        Self {
            count: s.len(),
            avg_ms: r3(s.iter().sum::<f64>() / s.len() as f64),
            p50_ms: r3(pct(0.5)),
            p99_ms: r3(pct(0.99)),
            max_ms: r3(s[s.len() - 1]),
        }
    }
}

/// Wall-clock split of a rendered frame. `gpu_wait` is the time from submit until the GPU
/// is idle, so it is a lower bound of GPU time for the part not overlapped with recording.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct FrameStats {
    pub total: Stats,
    pub sim: Stats,
    pub extract: Stats,
    pub record: Stats,
    pub gpu_wait: Stats,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct DrawStats {
    pub views: u32,
    pub draw_calls: u32,
    pub instances: u32,
    pub triangles: u64,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct CaseResult {
    pub name: String,
    /// `sim`, `render` or `startup`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scene: String,
    /// Why the case did not run (no GPU, feature not built, scene missing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Scene load (parse, validate, build module state), median over repeats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_ms: Option<f64>,
    /// Ticks per repeat (frames for a render case).
    #[serde(default)]
    pub ticks: u64,
    /// Repeats run (at least `repeats`, more until `min_case_s` of timed ticks).
    #[serde(default)]
    pub runs: u32,
    /// Tick time over all repeats (sim cases), or the startup time (startup cases).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick: Option<Stats>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<FrameStats>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draws: Option<DrawStats>,
    /// Heap allocations per timed tick (or frame), counted heap only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allocs_per_tick: Option<f64>,
    /// Counted heap after loading (MiB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heap_after_load_mib: Option<f64>,
    /// Highest counted heap during the case (MiB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heap_peak_mib: Option<f64>,
    /// Counted heap change over the timed ticks (MiB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heap_growth_mib: Option<f64>,
    /// Spread of the per-repeat average between repeats, percent of the median.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise_pct: Option<f64>,
    /// State hash after the run; the same in every repeat (otherwise `error`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_hash: Option<String>,
    /// SHA-256 of the RGBA pixels of the final frame (camera without head turn).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_sha256: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct BuildInfo {
    /// `release` or `debug` (debug assertions).
    pub profile: String,
    /// Engine cargo features built in.
    pub features: Vec<String>,
    /// CPU features the build may use without a runtime check (shows `target-cpu`).
    pub target_features: Vec<String>,
    pub os: String,
    pub arch: String,
    /// Commit of the project folder, if it is a git checkout.
    pub commit: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct MachineInfo {
    /// CPU model name as the OS reports it.
    pub cpu: String,
    pub logical_cores: usize,
    /// Worker threads the job pool uses.
    pub job_threads: usize,
    /// GPU adapter and backend, if one was used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<String>,
    /// `DiscreteGpu`, `IntegratedGpu`, `Cpu` (a software rasterizer), ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_kind: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct Report {
    pub format: String,
    pub suite: String,
    pub label: String,
    /// `full` or `quick`.
    pub length: String,
    pub repeats: u32,
    pub build: BuildInfo,
    pub machine: MachineInfo,
    pub started_unix_s: u64,
    pub seconds: f64,
    #[serde(default)]
    pub conditions: Conditions,
    pub cases: Vec<CaseResult>,
}

/// What else was going on while the suite ran.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct Conditions {
    /// The lock file held during the run, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock: Option<String>,
    /// Whole-machine CPU busy share just before the run (other processes), percent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_busy_before_pct: Option<f64>,
    /// Highest whole-machine CPU busy share in the pauses between cases (this process is
    /// idle then, so it is other processes), percent. Above about 10, timings are suspect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_busy_between_max_pct: Option<f64>,
    /// Lock holders seen while waiting.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waited_for: Vec<String>,
}

/// Holds the machine's benchmark lock; removes the file when dropped (also on a panic) if it
/// is still ours.
struct BenchLock {
    path: PathBuf,
    mine: String,
}

impl Drop for BenchLock {
    fn drop(&mut self) {
        if std::fs::read_to_string(&self.path).is_ok_and(|t| t == self.mine) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn busy_error(what: String) -> Errors {
    Diagnostic::error("perf.machine_busy", what)
        .fix("wait for the other run to finish, or pass `wait_s` to wait for it")
        .into()
}

/// Takes the lock (if any) and checks the CPU is quiet, waiting up to `wait_s`.
fn take_machine(
    lock: Option<PathBuf>,
    owner: &str,
    wait_s: u64,
    max_busy: f64,
    min_idle_s: u64,
    conditions: &mut Conditions,
) -> Result<Option<BenchLock>, Errors> {
    let deadline = Instant::now() + std::time::Duration::from_secs(wait_s);
    let pause = || std::thread::sleep(std::time::Duration::from_secs(30));
    loop {
        let guard = match &lock {
            None => None,
            Some(path) => {
                let mine = format!("{owner} {} pid {}\n", unix_now(), std::process::id());
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                {
                    Ok(mut f) => {
                        use std::io::Write;
                        f.write_all(mine.as_bytes())
                            .map_err(|e| busy_error(format!("cannot write lock: {e}")))?;
                        Some(BenchLock {
                            path: path.clone(),
                            mine,
                        })
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        let holder = std::fs::read_to_string(path)
                            .unwrap_or_default()
                            .trim()
                            .to_string();
                        if !conditions.waited_for.contains(&holder) && conditions.waited_for.len() < 16 {
                            conditions.waited_for.push(holder.clone());
                        }
                        if Instant::now() >= deadline {
                            return Err(busy_error(format!(
                                "benchmark lock {} is held: {holder}",
                                path.display()
                            )));
                        }
                        pause();
                        continue;
                    }
                    Err(e) => return Err(busy_error(format!("cannot create lock {}: {e}", path.display()))),
                }
            }
        };
        if min_idle_s > 0
            && let Some(idle) = marionette_host::machine::user_idle()
            && idle.as_secs() < min_idle_s
        {
            drop(guard);
            if Instant::now() >= deadline {
                return Err(busy_error(format!(
                    "the desktop was used {} s ago (need {min_idle_s} s idle)",
                    idle.as_secs()
                )));
            }
            pause();
            continue;
        }
        let busy = marionette_host::machine::cpu_busy_pct(std::time::Duration::from_secs(1));
        conditions.cpu_busy_before_pct = busy.map(r2);
        match busy {
            Some(b) if b > max_busy && max_busy < 100.0 => {
                drop(guard);
                if Instant::now() >= deadline {
                    return Err(busy_error(format!(
                        "other processes keep {b:.0}% of the CPU busy (limit {max_busy}%)"
                    )));
                }
                pause();
            }
            _ => {
                conditions.lock = lock.as_ref().map(|p| p.display().to_string());
                return Ok(guard);
            }
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------
// perf_suite

#[derive(Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct SuiteInput {
    /// Report file (relative to the project root). Default `.marionette/perf/<label>.json`.
    pub out: Option<String>,
    /// Name of this run, e.g. `baseline` or `lto-fat`. Default `<commit>-<profile>`.
    pub label: Option<String>,
    /// `full` (default) or `quick` (a tenth of the ticks, one repeat: a smoke test).
    pub length: Option<String>,
    /// Runs of each case; timings pool all runs. Default 3 (`quick`: 1).
    pub repeats: Option<u32>,
    /// After `repeats`, run a case again until this many seconds of timed ticks are pooled
    /// (at most 100 runs), so cases of a few microseconds per tick get a stable p99.
    /// Default 2 (`quick`: 0).
    pub min_case_s: Option<f64>,
    /// Only cases whose name starts with one of these (`sim/`, `render/factory`).
    #[serde(default)]
    pub only: Vec<String>,
    /// Extra scenes of a game to run as sim cases (and render cases if `render_extra`).
    #[serde(default)]
    pub extra_scenes: Vec<String>,
    /// Also render the extra scenes at 1920x1080.
    #[serde(default)]
    pub render_extra: bool,
    /// Skip render cases (default: run them when a GPU adapter exists).
    #[serde(default)]
    pub no_gpu: bool,
    #[serde(default)]
    pub seed: u64,
    /// Lock file shared by every benchmark on this machine (several agents share one PC).
    /// The suite creates it before measuring and removes it after, and refuses to start while
    /// another run holds it. Default: env `MARIONETTE_BENCH_LOCK`; none if unset.
    pub lock: Option<String>,
    /// Who holds the lock (an area name such as `raytracing`); written into the lock file.
    /// Default: the label.
    pub lock_owner: Option<String>,
    /// Wait up to this many seconds for a held lock or a busy CPU, checking every 30 s
    /// (default 0: refuse at once).
    #[serde(default)]
    pub wait_s: u64,
    /// Refuse to start unless nobody has used keyboard or mouse for this many seconds
    /// (Windows only; elsewhere not checked). Default 0. On a shared desktop use 300.
    #[serde(default)]
    pub min_idle_s: u64,
    /// Refuse to start when other processes keep more than this share of all cores busy
    /// (default 15 percent; 100 turns the check off).
    pub max_busy_pct: Option<f64>,
}

#[derive(Serialize, JsonSchema)]
pub struct SuiteOutput {
    pub report: String,
    pub machine: String,
    pub build: String,
    pub seconds: f64,
    /// Lock held and other processes' CPU load before and between cases.
    pub conditions: String,
    /// One line per case: name, then the main numbers (ms) and the short output hash.
    pub cases: Vec<String>,
    pub errors: usize,
    pub skipped: usize,
}

fn mib(bytes: u64) -> f64 {
    r2(bytes as f64 / (1024.0 * 1024.0))
}

fn heap() -> marionette_host::alloc::AllocStats {
    marionette_host::alloc::stats()
}

/// `true` when the binary counts allocations (the `marionette` binary does).
fn counted() -> bool {
    heap().total_allocs > 0
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Spread of the per-repeat averages, in percent of their median.
fn noise(avgs: &mut [f64]) -> Option<f64> {
    if avgs.len() < 2 {
        return None;
    }
    let m = median(avgs);
    (m > 0.0).then(|| r2((avgs[avgs.len() - 1] - avgs[0]) / m * 100.0))
}

fn build_info(root: &Path) -> BuildInfo {
    let target_features = [
        ("sse4.2", cfg!(target_feature = "sse4.2")),
        ("avx", cfg!(target_feature = "avx")),
        ("avx2", cfg!(target_feature = "avx2")),
        ("fma", cfg!(target_feature = "fma")),
        ("bmi2", cfg!(target_feature = "bmi2")),
        ("avx512f", cfg!(target_feature = "avx512f")),
        ("neon", cfg!(target_feature = "neon")),
    ]
    .into_iter()
    .filter(|(_, on)| *on)
    .map(|(n, _)| n.to_string())
    .collect();
    BuildInfo {
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
        .into(),
        features: crate::FEATURES.iter().map(|s| s.to_string()).collect(),
        target_features,
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        commit: git_commit(root).unwrap_or_else(|| "unknown".into()),
    }
}

/// Reads the checked-out commit without running git.
fn git_commit(root: &Path) -> Option<String> {
    let mut git = root.join(".git");
    if git.is_file() {
        // A worktree: `.git` names the real folder.
        let text = std::fs::read_to_string(&git).ok()?;
        git = PathBuf::from(text.strip_prefix("gitdir:")?.trim());
    }
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    let Some(name) = head.strip_prefix("ref:").map(str::trim) else {
        return Some(head.chars().take(12).collect());
    };
    // A worktree keeps refs in the common folder.
    let common = std::fs::read_to_string(git.join("commondir"))
        .ok()
        .map(|c| git.join(c.trim()))
        .unwrap_or_else(|| git.clone());
    for dir in [&git, &common] {
        if let Ok(id) = std::fs::read_to_string(dir.join(name)) {
            return Some(id.trim().chars().take(12).collect());
        }
        if let Ok(packed) = std::fs::read_to_string(dir.join("packed-refs")) {
            for line in packed.lines() {
                if let Some(id) = line.strip_suffix(name).map(str::trim)
                    && line.ends_with(&format!(" {name}"))
                {
                    return Some(id.chars().take(12).collect());
                }
            }
        }
    }
    None
}

fn summary_line(c: &CaseResult) -> String {
    if let Some(why) = &c.skipped {
        return format!("{} skipped: {why}", c.name);
    }
    if let Some(e) = &c.error {
        return format!("{} ERROR: {e}", c.name);
    }
    let mut s = c.name.clone();
    if let Some(l) = c.load_ms {
        s += &format!(" load {l}");
    }
    if let Some(t) = &c.tick {
        s += &format!(" tick p50 {} p99 {} max {}", t.p50_ms, t.p99_ms, t.max_ms);
    }
    if let Some(f) = &c.frame {
        s += &format!(
            " frame p50 {} p99 {} (record {} gpu {})",
            f.total.p50_ms, f.total.p99_ms, f.record.p50_ms, f.gpu_wait.p50_ms
        );
    }
    if let Some(a) = c.allocs_per_tick {
        s += &format!(" allocs/tick {a}");
    }
    if let Some(h) = c.heap_after_load_mib {
        s += &format!(" heap {h} MiB");
    }
    if c.runs > 0 {
        s += &format!(" runs {}", c.runs);
    }
    if let Some(n) = c.noise_pct {
        s += &format!(" noise {n}%");
    }
    if let Some(h) = &c.state_hash {
        s += &format!(" state {}", &h[..h.len().min(12)]);
    }
    if let Some(h) = &c.image_sha256 {
        s += &format!(" image {}", &h[..h.len().min(12)]);
    }
    s
}

fn suite(agent: &mut Agent, input: SuiteInput) -> Result<SuiteOutput, Errors> {
    let started = Instant::now();
    let started_unix_s = unix_now();
    let quick = match input.length.as_deref().unwrap_or("full") {
        "full" => false,
        "quick" => true,
        other => {
            return Err(
                Diagnostic::error(crate::codes::BAD_INPUT, format!("unknown length `{other}`"))
                    .fix("use `full` or `quick`")
                    .into(),
            );
        }
    };
    let repeats = input.repeats.unwrap_or(if quick { 1 } else { 3 }).clamp(1, 20);
    let root = agent.resolve(".")?;
    let build = build_info(&root);
    let label = input
        .label
        .clone()
        .unwrap_or_else(|| format!("{}-{}", build.commit, build.profile));
    if label.is_empty()
        || !label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    {
        return Err(Diagnostic::error(
            crate::codes::BAD_NAME,
            format!("label `{label}` is not a plain name"),
        )
        .fix("use letters, digits, `-`, `_` or `.`")
        .into());
    }
    let out = input
        .out
        .clone()
        .unwrap_or_else(|| format!(".marionette/perf/{label}.json"));
    let out_path = agent.resolve(&out)?;
    let lock = input
        .lock
        .clone()
        .or_else(|| std::env::var("MARIONETTE_BENCH_LOCK").ok())
        .filter(|l| !l.is_empty())
        .map(|l| agent.resolve(&l))
        .transpose()?;
    let mut conditions = Conditions::default();
    let owner = input.lock_owner.clone().unwrap_or_else(|| label.clone());
    let _lock = take_machine(
        lock,
        &owner,
        input.wait_s,
        input.max_busy_pct.unwrap_or(15.0),
        input.min_idle_s,
        &mut conditions,
    )?;

    let mut cases: Vec<Case> = CASES
        .iter()
        .copied()
        .filter(|c| input.only.is_empty() || input.only.iter().any(|p| c.name.starts_with(p.as_str())))
        .collect();
    // Extra scenes of a game: leaked names live as long as the process; the list is small.
    for scene in &input.extra_scenes {
        let stem = Path::new(scene)
            .file_name()
            .map_or("scene".into(), |n| n.to_string_lossy().replace(".scene.json", ""));
        let file: &'static str = Box::leak(scene.clone().into_boxed_str());
        let name: &'static str = Box::leak(format!("extra/sim/{stem}").into_boxed_str());
        cases.push(sim(name, Source::File(file), 600));
        if input.render_extra {
            let name: &'static str = Box::leak(format!("extra/render/{stem}-1080p").into_boxed_str());
            cases.push(render(name, Source::File(file), 300, HD));
        }
    }

    // Generated scenes go to a temp folder that is removed with the process.
    let temp = marionette_host::temp_dir("perf-suite").map_err(|e| {
        Errors::from(Diagnostic::error(
            marionette_core::error::codes::IO_WRITE,
            format!("cannot make a temp folder: {e}"),
        ))
    })?;
    let mut machine = MachineInfo {
        cpu: marionette_host::machine::cpu_model(),
        logical_cores: std::thread::available_parallelism().map_or(1, |n| n.get()),
        job_threads: marionette_jobs::Jobs::shared().threads(),
        gpu: None,
        gpu_kind: None,
    };

    let mut results = Vec::new();
    if input.only.is_empty()
        || input
            .only
            .iter()
            .any(|p| "startup/".starts_with(p.as_str()) || p.starts_with("startup"))
    {
        results.push(startup_engine(repeats));
        results.push(startup_process(&root, repeats));
    }
    let gpu_ok = !input.no_gpu && agent.gpu().is_ok();
    if gpu_ok && let Ok(gpu) = agent.gpu() {
        let info = gpu.adapter.get_info();
        machine.gpu = Some(gpu.describe());
        machine.gpu_kind = Some(format!("{:?}", info.device_type));
    }
    let mut between_max: Option<f64> = None;
    for case in &cases {
        // Others' load while this process is idle, so a disturbed run shows in the report.
        if !quick
            && let Some(b) = marionette_host::machine::cpu_busy_pct(std::time::Duration::from_millis(300))
        {
            between_max = Some(between_max.map_or(b, |m: f64| m.max(b)));
        }
        let mut result = CaseResult {
            name: case.name.into(),
            kind: if case.render.is_some() { "render" } else { "sim" }.into(),
            ..Default::default()
        };
        let path = match case.source {
            Source::File(f) => {
                result.scene = f.into();
                let p = agent.resolve(f)?;
                if !p.exists() {
                    result.skipped = Some("scene file not found in this project".into());
                    results.push(result);
                    continue;
                }
                p
            }
            Source::Generated { op, scale } => {
                result.scene = format!("{op} scale {scale}");
                if !agent.ops.iter().any(|o| o.info.name == op) {
                    result.skipped = Some(format!(
                        "operation `{op}` is not in this engine (the example or game that adds it runs this case)"
                    ));
                    results.push(result);
                    continue;
                }
                let p = temp.path().join(format!("{op}-{scale}.scene.json"));
                if !p.exists() {
                    let out = p.to_string_lossy().to_string();
                    agent.call(op, serde_json::json!({ "out": out, "scale": scale }))?;
                }
                p
            }
        };
        if case.render.is_some() && !gpu_ok {
            result.skipped = Some(
                if input.no_gpu {
                    "no_gpu set"
                } else {
                    "no GPU adapter"
                }
                .into(),
            );
            results.push(result);
            continue;
        }
        let ticks = if quick {
            (case.ticks / 10).max(10)
        } else {
            case.ticks
        };
        let warmup = if quick {
            (case.warmup / 4).max(5)
        } else {
            case.warmup
        };
        result.ticks = ticks;
        let runs = Runs {
            min: repeats,
            min_s: input.min_case_s.unwrap_or(if quick { 0.0 } else { 2.0 }).max(0.0),
        };
        let run = match case.render {
            None => run_sim(agent, &path, input.seed, ticks, warmup, runs, &mut result),
            Some(spec) => run_render(agent, &path, input.seed, ticks, warmup, runs, spec, &mut result),
        };
        if let Err(e) = run {
            let msg =
                e.0.first()
                    .map_or("unknown error".into(), |d| format!("{}: {}", d.code, d.message));
            // A scene that needs a feature this build left out is not an engine error.
            if msg.contains("unknown component")
                || msg.contains("unknown resource")
                || msg.contains("not_enabled")
            {
                result.skipped = Some(format!("needs an engine part this build leaves out ({msg})"));
            } else {
                result.error = Some(msg);
            }
        }
        results.push(result);
    }

    conditions.cpu_busy_between_max_pct = between_max.map(r2);
    let report = Report {
        format: FORMAT.into(),
        suite: SUITE.into(),
        label,
        length: if quick { "quick" } else { "full" }.into(),
        repeats,
        build: build.clone(),
        machine: machine.clone(),
        started_unix_s,
        seconds: r2(started.elapsed().as_secs_f64()),
        conditions,
        cases: results,
    };
    let text = serde_json::to_string_pretty(&report).expect("report serializes") + "\n";
    write_atomic(&out_path, &out, text.as_bytes())?;
    Ok(SuiteOutput {
        report: out,
        machine: format!(
            "{}, {} logical cores, {} job threads{}",
            machine.cpu,
            machine.logical_cores,
            machine.job_threads,
            machine
                .gpu
                .as_ref()
                .map(|g| format!(", {g} [{}]", machine.gpu_kind.clone().unwrap_or_default()))
                .unwrap_or_default()
        ),
        build: format!(
            "{} {} {} [{}] target features [{}]",
            build.profile,
            build.os,
            build.arch,
            build.features.join(","),
            build.target_features.join(",")
        ),
        seconds: report.seconds,
        conditions: format!(
            "lock {}, others' CPU busy before {}%, between cases max {}%",
            report.conditions.lock.as_deref().unwrap_or("none"),
            report
                .conditions
                .cpu_busy_before_pct
                .map_or("?".into(), |b| b.to_string()),
            report
                .conditions
                .cpu_busy_between_max_pct
                .map_or("-".into(), |b| b.to_string()),
        ),
        errors: report.cases.iter().filter(|c| c.error.is_some()).count(),
        skipped: report.cases.iter().filter(|c| c.skipped.is_some()).count(),
        cases: report.cases.iter().map(summary_line).collect(),
    })
}

/// Building the standard engine from scratch (registry, modules, operation table with
/// schemas) in this process.
fn startup_engine(repeats: u32) -> CaseResult {
    let n = (repeats * 5) as usize;
    let mut times = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        let engine = crate::Engine::builder().build();
        std::hint::black_box(&engine);
        times.push(ms(t.elapsed()));
    }
    CaseResult {
        name: "startup/engine".into(),
        kind: "startup".into(),
        ticks: n as u64,
        tick: Some(Stats::of(&times)),
        ..Default::default()
    }
}

/// A new `marionette` process that loads the drift example, runs one tick and exits: what a
/// person or an agent waits for on every command.
fn startup_process(root: &Path, repeats: u32) -> CaseResult {
    let mut result = CaseResult {
        name: "startup/process".into(),
        kind: "startup".into(),
        scene: "examples/drift.scene.json".into(),
        ..Default::default()
    };
    let exe = std::env::current_exe().ok();
    let is_marionette = exe
        .as_ref()
        .and_then(|e| e.file_stem())
        .is_some_and(|s| s == "marionette");
    let scene = root.join("examples/drift.scene.json");
    let (Some(exe), true, true) = (exe, is_marionette, scene.exists()) else {
        result.skipped = Some("needs the marionette binary and examples/drift.scene.json".into());
        return result;
    };
    let n = (repeats * 3) as usize;
    let mut times = Vec::with_capacity(n);
    for _ in 0..n {
        let mut cmd = std::process::Command::new(&exe);
        cmd.current_dir(root)
            .args(["run", "examples/drift.scene.json", "--headless", "--ticks", "1"]);
        let t = Instant::now();
        match marionette_host::output(cmd) {
            Ok(o) if o.status.success() => times.push(ms(t.elapsed())),
            Ok(o) => {
                result.error = Some(format!("exit status {}", o.status));
                return result;
            }
            Err(e) => {
                result.error = Some(format!("cannot start: {e}"));
                return result;
            }
        }
    }
    result.ticks = n as u64;
    result.tick = Some(Stats::of(&times));
    result
}

fn load(agent: &Agent, path: &Path, seed: u64) -> Result<(Sim, f64), Errors> {
    let t = Instant::now();
    let sim = agent.engine().load_file(path, &path.to_string_lossy(), seed)?;
    Ok((sim, ms(t.elapsed())))
}

fn check_hash(result: &mut CaseResult, hash: String) {
    match &result.state_hash {
        Some(h) if *h != hash => {
            result.error = Some(format!("state hash differs between repeats ({h} vs {hash})"));
        }
        _ => result.state_hash = Some(hash),
    }
}

fn run_sim(
    agent: &Agent,
    path: &Path,
    seed: u64,
    ticks: u64,
    warmup: u64,
    repeats: Runs,
    result: &mut CaseResult,
) -> Result<(), Errors> {
    let mut times = Vec::with_capacity((ticks * repeats.min as u64) as usize);
    let (mut loads, mut avgs, mut heaps, mut allocs, mut growth) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    marionette_host::alloc::reset_peak();
    let mut done = 0;
    while repeats.more(done, &times) {
        done += 1;
        let (mut sim, load_ms) = load(agent, path, seed)?;
        loads.push(load_ms);
        heaps.push(heap().live_bytes);
        sim.run(warmup);
        let before = heap();
        let start = times.len();
        for _ in 0..ticks {
            let t = Instant::now();
            sim.step();
            times.push(ms(t.elapsed()));
        }
        let after = heap();
        allocs.push((after.total_allocs - before.total_allocs) as f64 / ticks as f64);
        growth.push(after.live_bytes as f64 - before.live_bytes as f64);
        avgs.push(times[start..].iter().sum::<f64>() / ticks as f64);
        check_hash(result, sim.state_hash());
    }
    result.runs = done;
    result.load_ms = Some(r3(median(&mut loads)));
    result.tick = Some(Stats::of(&times));
    result.noise_pct = noise(&mut avgs);
    if counted() {
        result.allocs_per_tick = Some(r2(median(&mut allocs)));
        result.heap_after_load_mib = Some(mib(heaps[heaps.len() / 2]));
        result.heap_peak_mib = Some(mib(heap().peak_bytes));
        result.heap_growth_mib = Some(r2(median(&mut growth) / (1024.0 * 1024.0)));
    }
    Ok(())
}

/// How often a case runs: at least `min` times, then again until `min_s` seconds of timed
/// ticks are pooled (at most [`MAX_RUNS`]), so short cases get enough samples for a stable p99.
#[derive(Clone, Copy)]
struct Runs {
    min: u32,
    min_s: f64,
}

/// Upper bound of runs per case.
const MAX_RUNS: u32 = 100;

impl Runs {
    fn more(&self, done: u32, timed_ms: &[f64]) -> bool {
        done < self.min || (done < MAX_RUNS && timed_ms.iter().sum::<f64>() / 1000.0 < self.min_s)
    }
}

/// Typical interpupillary distance, meters (as `bench`).
const IPD_M: f32 = 0.064;

/// Head yaw for headset emulation at frame `i` (90 Hz, +-25 degrees, 2 s period), as `bench`.
fn head_yaw(i: u64) -> f32 {
    let t = i as f32 / 90.0;
    25f32.to_radians() * (t * std::f32::consts::PI).sin()
}

#[allow(clippy::too_many_arguments)]
fn run_render(
    agent: &mut Agent,
    path: &Path,
    seed: u64,
    frames: u64,
    warmup: u64,
    repeats: Runs,
    spec: RenderSpec,
    result: &mut CaseResult,
) -> Result<(), Errors> {
    let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let shown = path.to_string_lossy().to_string();
    let n = (frames * repeats.min as u64) as usize;
    let (mut total, mut sims, mut extracts, mut records, mut waits) = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    let (mut loads, mut avgs, mut allocs) = (Vec::new(), Vec::new(), Vec::new());
    let mut draws = DrawStats::default();
    marionette_host::alloc::reset_peak();
    let mut done = 0;
    while repeats.more(done, &total) {
        done += 1;
        let (mut sim, load_ms) = load(agent, path, seed)?;
        loads.push(load_ms);
        let gpu = agent.gpu()?;
        let mut off = Offscreen::new(gpu, spec.width, spec.height, spec.msaa, base.clone());
        // Warm-up: decode assets, compile pipelines, settle clocks. Not timed.
        for _ in 0..warmup {
            sim.step();
            off.render_eyes(gpu, &extract(&sim.world), None, spec.eyes, IPD_M)
                .map_err(|e| e.with_file(&shown))?;
        }
        let before = heap();
        let start = total.len();
        for i in 0..frames {
            let t0 = Instant::now();
            sim.step();
            let t1 = Instant::now();
            let mut frame = extract(&sim.world);
            if spec.headset {
                let turn = marionette_assets::glam::Mat4::from_rotation_y(head_yaw(i));
                for c in &mut frame.cameras {
                    c.world *= turn;
                }
            }
            let t2 = Instant::now();
            let stats = off.render_eyes(gpu, &frame, None, spec.eyes, IPD_M)?;
            total.push(ms(t0.elapsed()));
            sims.push(ms(t1 - t0));
            extracts.push(ms(t2 - t1));
            records.push(off.last_timing.record_ms);
            waits.push(off.last_timing.gpu_wait_ms);
            draws = DrawStats {
                views: stats.views,
                draw_calls: stats.draw_calls,
                instances: stats.instances,
                triangles: stats.triangles,
            };
        }
        let after = heap();
        allocs.push((after.total_allocs - before.total_allocs) as f64 / frames as f64);
        avgs.push(total[start..].iter().sum::<f64>() / frames as f64);
        // Fixed final image: this tick's world, the camera as the scene has it.
        off.render_eyes(gpu, &extract(&sim.world), None, spec.eyes, IPD_M)?;
        let pixels = off.read_pixels(gpu)?;
        let image = sha256_hex(&pixels);
        match &result.image_sha256 {
            Some(h) if *h != image => {
                result.error = Some(format!("image differs between repeats ({h} vs {image})"));
            }
            _ => result.image_sha256 = Some(image),
        }
        check_hash(result, sim.state_hash());
    }
    result.runs = done;
    result.load_ms = Some(r3(median(&mut loads)));
    result.frame = Some(FrameStats {
        total: Stats::of(&total),
        sim: Stats::of(&sims),
        extract: Stats::of(&extracts),
        record: Stats::of(&records),
        gpu_wait: Stats::of(&waits),
    });
    result.draws = Some(draws);
    result.noise_pct = noise(&mut avgs);
    if counted() {
        result.allocs_per_tick = Some(r2(median(&mut allocs)));
        result.heap_peak_mib = Some(mib(heap().peak_bytes));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// perf_compare

/// One report or several runs of the same build.
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Reports {
    One(String),
    Many(Vec<String>),
}

impl Reports {
    fn paths(&self) -> Vec<&str> {
        match self {
            Reports::One(p) => vec![p.as_str()],
            Reports::Many(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompareInput {
    /// The earlier report, or several runs of the earlier build (relative to the project
    /// root). Several runs per side, ideally alternated A B A B, give a change only when the
    /// two sides' ranges do not overlap.
    pub base: Reports,
    /// The later report, or several runs of the later build.
    pub new: Reports,
    /// Report changes larger than this, percent (default 5).
    pub threshold_pct: Option<f64>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct Change {
    pub case: String,
    pub metric: String,
    /// Median over the base runs.
    pub base: f64,
    /// Median over the new runs.
    pub new: f64,
    /// Negative is faster or smaller.
    pub change_pct: f64,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct CompareOutput {
    /// Every state hash equal, and every image hash equal when both ran on the same GPU.
    pub outputs_identical: bool,
    pub output_mismatches: Vec<String>,
    pub same_machine: bool,
    pub same_gpu: bool,
    /// Runs read per side.
    pub runs: (usize, usize),
    /// Notes on things that make the comparison weaker (different suite, length, machine).
    pub warnings: Vec<String>,
    /// Changes beyond the threshold (and outside the run-to-run range when several runs are
    /// given), largest first.
    pub changes: Vec<Change>,
    /// Metrics compared that stayed within the threshold or the run-to-run range.
    pub within_threshold: usize,
    /// Cases only in one side.
    pub unmatched: Vec<String>,
}

fn read_report(agent: &Agent, path: &str) -> Result<Report, Errors> {
    let text = read_text(&agent.resolve(path)?, path)?;
    let report: Report = serde_json::from_str(&text).map_err(|e| {
        Errors::from(
            Diagnostic::error(
                crate::codes::BAD_INPUT,
                format!("{path} is not a perf_suite report: {e}"),
            )
            .file(path),
        )
    })?;
    if report.format != FORMAT {
        return Err(Diagnostic::error(
            crate::codes::BAD_INPUT,
            format!(
                "{path} has format `{}`, this engine reads `{FORMAT}`",
                report.format
            ),
        )
        .file(path)
        .fix("rerun perf_suite with this engine for both reports")
        .into());
    }
    Ok(report)
}

/// The numbers compared per case, with the smallest change that counts: short times and
/// tails (p99) move by noise alone.
fn metrics(c: &CaseResult) -> Vec<(&'static str, f64, f64)> {
    let mut m = Vec::new();
    if let Some(l) = c.load_ms {
        m.push(("load_ms", l, 0.05));
    }
    if let Some(t) = &c.tick {
        m.push(("tick.avg_ms", t.avg_ms, 0.01));
        m.push(("tick.p50_ms", t.p50_ms, 0.01));
        m.push(("tick.p99_ms", t.p99_ms, 0.05));
    }
    if let Some(f) = &c.frame {
        m.push(("frame.p50_ms", f.total.p50_ms, 0.02));
        m.push(("frame.p99_ms", f.total.p99_ms, 0.1));
        m.push(("frame.extract.p50_ms", f.extract.p50_ms, 0.01));
        m.push(("frame.record.p50_ms", f.record.p50_ms, 0.01));
        m.push(("frame.gpu_wait.p50_ms", f.gpu_wait.p50_ms, 0.02));
    }
    if let Some(a) = c.allocs_per_tick {
        m.push(("allocs_per_tick", a, 1.0));
    }
    if let Some(h) = c.heap_after_load_mib {
        m.push(("heap_after_load_mib", h, 0.1));
    }
    m
}

fn compare(agent: &mut Agent, input: CompareInput) -> Result<CompareOutput, Errors> {
    let read = |r: &Reports| -> Result<Vec<Report>, Errors> {
        r.paths().into_iter().map(|p| read_report(agent, p)).collect()
    };
    let base = read(&input.base)?;
    let new = read(&input.new)?;
    if base.is_empty() || new.is_empty() {
        return Err(Diagnostic::error(crate::codes::BAD_INPUT, "give at least one report per side").into());
    }
    Ok(compare_reports(&base, &new, input.threshold_pct.unwrap_or(5.0)))
}

fn median_of(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    median(&mut s)
}

/// Compares runs of two builds (also used by tests). Outputs are compared between every
/// report; timings by the median over each side's runs.
pub fn compare_reports(base: &[Report], new: &[Report], threshold_pct: f64) -> CompareOutput {
    let mut warnings = Vec::new();
    let all: Vec<&Report> = base.iter().chain(new).collect();
    let first = all[0];
    if all.iter().any(|r| r.suite != first.suite) {
        warnings.push("reports come from different suite versions".into());
    }
    if all.iter().any(|r| r.length != first.length) {
        warnings.push("reports have different lengths (full and quick)".into());
    }
    let same_machine = all.iter().all(|r| {
        r.machine.cpu == first.machine.cpu
            && r.machine.logical_cores == first.machine.logical_cores
            && r.build.os == first.build.os
    });
    if !same_machine {
        warnings.push("different machines: timings are not comparable".into());
    }
    for r in &all {
        if let Some(b) = r.conditions.cpu_busy_between_max_pct.filter(|b| *b > 10.0) {
            warnings.push(format!(
                "{}: other processes used up to {b}% of the CPU during the run",
                r.label
            ));
        }
    }
    let same_gpu = first.machine.gpu.is_some() && all.iter().all(|r| r.machine.gpu == first.machine.gpu);
    if all.iter().any(|r| r.build.profile != first.build.profile) {
        warnings.push("reports mix release and debug builds".into());
    }
    let (mut mismatches, mut changes, mut within, mut unmatched) = (Vec::new(), Vec::new(), 0, Vec::new());
    let names: Vec<&str> = first.cases.iter().map(|c| c.name.as_str()).collect();
    for name in &names {
        let find = |side: &[Report]| -> Vec<CaseResult> {
            side.iter()
                .filter_map(|r| r.cases.iter().find(|c| c.name == *name).cloned())
                .collect()
        };
        let (b, n) = (find(base), find(new));
        if b.len() != base.len() || n.len() != new.len() {
            unmatched.push(format!("{name} (not in every report)"));
            continue;
        }
        let reference = &b[0];
        for c in b.iter().chain(&n) {
            if let (Some(x), Some(y)) = (&reference.state_hash, &c.state_hash)
                && x != y
            {
                mismatches.push(format!("{name}: state hash {} vs {}", &x[..12], &y[..12]));
            }
            if same_gpu
                && let (Some(x), Some(y)) = (&reference.image_sha256, &c.image_sha256)
                && x != y
            {
                mismatches.push(format!("{name}: image {} vs {}", &x[..12], &y[..12]));
            }
            if let Some(e) = &c.error {
                mismatches.push(format!("{name}: error {e}"));
            }
        }
        for (metric, _, min_abs) in metrics(reference) {
            let values = |side: &[CaseResult]| -> Vec<f64> {
                side.iter()
                    .filter_map(|c| {
                        metrics(c)
                            .into_iter()
                            .find(|(m, _, _)| *m == metric)
                            .map(|(_, v, _)| v)
                    })
                    .collect()
            };
            let (bv, nv) = (values(&b), values(&n));
            if bv.len() != b.len() || nv.len() != n.len() {
                continue;
            }
            let (bm, nm) = (median_of(&bv), median_of(&nv));
            let pct = if bm > 0.0 { (nm - bm) / bm * 100.0 } else { 0.0 };
            let range = |v: &[f64]| {
                (
                    v.iter().copied().fold(f64::MAX, f64::min),
                    v.iter().copied().fold(f64::MIN, f64::max),
                )
            };
            let ((blo, bhi), (nlo, nhi)) = (range(&bv), range(&nv));
            let apart = bv.len() < 2 || nv.len() < 2 || nlo > bhi || nhi < blo;
            if pct.abs() > threshold_pct && (nm - bm).abs() >= min_abs && apart {
                changes.push(Change {
                    case: name.to_string(),
                    metric: metric.into(),
                    base: bm,
                    new: nm,
                    change_pct: r2(pct),
                });
            } else {
                within += 1;
            }
        }
    }
    for r in all.iter().skip(1) {
        for c in &r.cases {
            let label = format!("{} (not in every report)", c.name);
            if !names.contains(&c.name.as_str()) && !unmatched.contains(&label) {
                unmatched.push(label);
            }
        }
    }
    mismatches.sort();
    mismatches.dedup();
    changes.sort_by(|a, b| b.change_pct.abs().total_cmp(&a.change_pct.abs()));
    CompareOutput {
        outputs_identical: mismatches.is_empty(),
        output_mismatches: mismatches,
        same_machine,
        same_gpu,
        runs: (base.len(), new.len()),
        warnings,
        changes,
        within_threshold: within,
        unmatched,
    }
}
