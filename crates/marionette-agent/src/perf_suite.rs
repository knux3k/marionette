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
use marionette_core::{Diagnostic, Errors, Sim, scene};
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
    /// The generated factory benchmark scene at this scale (`perf_factory_scene`).
    Factory(u32),
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
    Case { name, source, ticks, warmup: 60, render: None }
}

const HD: RenderSpec = RenderSpec { width: 1920, height: 1080, eyes: 1, msaa: 4, headset: false };
const HEADSET: RenderSpec = RenderSpec { width: 2160, height: 2160, eyes: 2, msaa: 4, headset: true };

const fn render(name: &'static str, source: Source, frames: u64, spec: RenderSpec) -> Case {
    Case { name, source, ticks: frames, warmup: 30, render: Some(spec) }
}

/// The suite. Order is the report order. Cover each engine system once: core logic, physics
/// 2D and 3D, scripts, animation, job TypeScript, the factory module at two sizes, the six
/// proof games, and rendering of plain, opt-in looks, many instances, skinned models and a
/// headset.
const CASES: &[Case] = &[
    sim("sim/drift", Source::File("examples/drift.scene.json"), 1000),
    sim("sim/physics-3d", Source::File("examples/playground3d.scene.json"), 1000),
    sim("sim/physics-2d", Source::File("examples/playground2d.scene.json"), 1000),
    sim("sim/scripts", Source::File("examples/scripted.scene.json"), 1000),
    sim("sim/spawner", Source::File("examples/spawner.scene.json"), 1000),
    sim("sim/animation", Source::File("examples/animation.scene.json"), 1000),
    sim("sim/job-swarm", Source::File("examples/swarm.scene.json"), 600),
    sim("sim/factory-4x", Source::Factory(4), 1200),
    sim("sim/factory-32x", Source::Factory(32), 300),
    sim("sim/game-lantern-keeper", Source::File("games/lantern-keeper/scenes/main.scene.json"), 600),
    sim("sim/game-ember-run", Source::File("games/ember-run/scenes/main.scene.json"), 600),
    sim("sim/game-cloudstep", Source::File("games/cloudstep/scenes/main.scene.json"), 600),
    sim("sim/game-crystal-ward", Source::File("games/crystal-ward/scenes/main.scene.json"), 600),
    sim("sim/game-dust-circuit", Source::File("games/dust-circuit/scenes/main.scene.json"), 600),
    sim("sim/game-signal-yard", Source::File("games/signal-yard/scenes/main.scene.json"), 600),
    render("render/showcase-1080p", Source::File("examples/showcase.scene.json"), 300, HD),
    render("render/looks-1080p", Source::File("examples/looks.scene.json"), 300, HD),
    render("render/lighting-1080p", Source::File("examples/lighting.scene.json"), 300, HD),
    render("render/sprites-1080p", Source::File("examples/sprites.scene.json"), 300, HD),
    render("render/animation-1080p", Source::File("examples/animation.scene.json"), 300, HD),
    render("render/job-swarm-1080p", Source::File("examples/swarm.scene.json"), 300, HD),
    render("render/game-crystal-ward-1080p", Source::File("games/crystal-ward/scenes/main.scene.json"), 300, HD),
    render("render/factory-4x-headset", Source::Factory(4), 300, HEADSET),
    render("render/factory-32x-headset", Source::Factory(32), 120, HEADSET),
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
    pub cases: Vec<CaseResult>,
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
}

#[derive(Serialize, JsonSchema)]
pub struct SuiteOutput {
    pub report: String,
    pub machine: String,
    pub build: String,
    pub seconds: f64,
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
        profile: if cfg!(debug_assertions) { "debug" } else { "release" }.into(),
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
    let started_unix_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let quick = match input.length.as_deref().unwrap_or("full") {
        "full" => false,
        "quick" => true,
        other => {
            return Err(Diagnostic::error(crate::codes::BAD_INPUT, format!("unknown length `{other}`"))
                .fix("use `full` or `quick`")
                .into());
        }
    };
    let repeats = input.repeats.unwrap_or(if quick { 1 } else { 3 }).clamp(1, 20);
    let root = agent.resolve(".")?;
    let build = build_info(&root);
    let label = input
        .label
        .clone()
        .unwrap_or_else(|| format!("{}-{}", build.commit, build.profile));
    if label.is_empty() || !label.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
        return Err(Diagnostic::error(crate::codes::BAD_NAME, format!("label `{label}` is not a plain name"))
            .fix("use letters, digits, `-`, `_` or `.`")
            .into());
    }
    let out = input
        .out
        .clone()
        .unwrap_or_else(|| format!(".marionette/perf/{label}.json"));
    let out_path = agent.resolve(&out)?;

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

    // Generated factory scenes go to a temp folder that is removed with the process.
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
    if input.only.is_empty() || input.only.iter().any(|p| "startup/".starts_with(p.as_str()) || p.starts_with("startup")) {
        results.push(startup_engine(repeats));
        results.push(startup_process(&root, repeats));
    }
    let gpu_ok = !input.no_gpu && agent.gpu().is_ok();
    if gpu_ok && let Ok(gpu) = agent.gpu() {
        let info = gpu.adapter.get_info();
        machine.gpu = Some(gpu.describe());
        machine.gpu_kind = Some(format!("{:?}", info.device_type));
    }
    for case in &cases {
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
            Source::Factory(scale) => {
                result.scene = format!("perf_factory_scene scale {scale}");
                let p = temp.path().join(format!("factory-{scale}x.scene.json"));
                if !p.exists() {
                    let (value, _) = crate::perf::factory_scene_json(scale);
                    let text = scene::format_text(&value.to_string(), agent.registry())?;
                    write_atomic(&p, &result.scene, text.as_bytes())?;
                }
                p
            }
        };
        if case.render.is_some() && !gpu_ok {
            result.skipped = Some(if input.no_gpu { "no_gpu set" } else { "no GPU adapter" }.into());
            results.push(result);
            continue;
        }
        let ticks = if quick { (case.ticks / 10).max(10) } else { case.ticks };
        let warmup = if quick { (case.warmup / 4).max(5) } else { case.warmup };
        result.ticks = ticks;
        let run = match case.render {
            None => run_sim(agent, &path, input.seed, ticks, warmup, repeats, &mut result),
            Some(spec) => run_render(agent, &path, input.seed, ticks, warmup, repeats, spec, &mut result),
        };
        if let Err(e) = run {
            let msg = e.0.first().map_or("unknown error".into(), |d| format!("{}: {}", d.code, d.message));
            // A scene that needs a feature this build left out is not an engine error.
            if msg.contains("unknown component") || msg.contains("unknown resource") || msg.contains("not_enabled") {
                result.skipped = Some(format!("needs an engine part this build leaves out ({msg})"));
            } else {
                result.error = Some(msg);
            }
        }
        results.push(result);
    }

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
    let sim = agent
        .engine()
        .load_file(path, &path.to_string_lossy(), seed)?;
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
    repeats: u32,
    result: &mut CaseResult,
) -> Result<(), Errors> {
    let mut times = Vec::with_capacity((ticks * repeats as u64) as usize);
    let (mut loads, mut avgs, mut heaps, mut allocs, mut growth) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    marionette_host::alloc::reset_peak();
    for _ in 0..repeats {
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
    repeats: u32,
    spec: RenderSpec,
    result: &mut CaseResult,
) -> Result<(), Errors> {
    let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let shown = path.to_string_lossy().to_string();
    let n = (frames * repeats as u64) as usize;
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
    for _ in 0..repeats {
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

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompareInput {
    /// The earlier report (relative to the project root).
    pub base: String,
    /// The later report.
    pub new: String,
    /// Report changes larger than this, percent (default 5).
    pub threshold_pct: Option<f64>,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct Change {
    pub case: String,
    pub metric: String,
    pub base: f64,
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
    /// Notes on things that make the comparison weaker (different suite, length, machine).
    pub warnings: Vec<String>,
    /// Changes beyond the threshold, largest first.
    pub changes: Vec<Change>,
    /// Metrics compared that stayed within the threshold.
    pub within_threshold: usize,
    /// Cases only in one of the two reports.
    pub unmatched: Vec<String>,
}

fn read_report(agent: &Agent, path: &str) -> Result<Report, Errors> {
    let text = read_text(&agent.resolve(path)?, path)?;
    let report: Report = serde_json::from_str(&text).map_err(|e| {
        Errors::from(
            Diagnostic::error(crate::codes::BAD_INPUT, format!("{path} is not a perf_suite report: {e}"))
                .file(path),
        )
    })?;
    if report.format != FORMAT {
        return Err(Diagnostic::error(
            crate::codes::BAD_INPUT,
            format!("{path} has format `{}`, this engine reads `{FORMAT}`", report.format),
        )
        .file(path)
        .fix("rerun perf_suite with this engine for both reports")
        .into());
    }
    Ok(report)
}

/// The numbers compared per case. Times under 0.02 ms change by noise alone, so a change
/// must also be at least 0.01 ms (or 1 allocation, or 0.1 MiB) to count.
fn metrics(c: &CaseResult) -> Vec<(&'static str, f64, f64)> {
    let mut m = Vec::new();
    if let Some(l) = c.load_ms {
        m.push(("load_ms", l, 0.05));
    }
    if let Some(t) = &c.tick {
        m.push(("tick.avg_ms", t.avg_ms, 0.01));
        m.push(("tick.p50_ms", t.p50_ms, 0.01));
        m.push(("tick.p99_ms", t.p99_ms, 0.01));
    }
    if let Some(f) = &c.frame {
        m.push(("frame.p50_ms", f.total.p50_ms, 0.01));
        m.push(("frame.p99_ms", f.total.p99_ms, 0.01));
        m.push(("frame.extract.p50_ms", f.extract.p50_ms, 0.01));
        m.push(("frame.record.p50_ms", f.record.p50_ms, 0.01));
        m.push(("frame.gpu_wait.p50_ms", f.gpu_wait.p50_ms, 0.01));
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
    let base = read_report(agent, &input.base)?;
    let new = read_report(agent, &input.new)?;
    Ok(compare_reports(&base, &new, input.threshold_pct.unwrap_or(5.0)))
}

/// Compares two reports (also used by tests).
pub fn compare_reports(base: &Report, new: &Report, threshold_pct: f64) -> CompareOutput {
    let mut warnings = Vec::new();
    if base.suite != new.suite {
        warnings.push(format!("different suites: {} vs {}", base.suite, new.suite));
    }
    if base.length != new.length {
        warnings.push(format!("different lengths: {} vs {}", base.length, new.length));
    }
    let same_machine = base.machine.cpu == new.machine.cpu
        && base.machine.logical_cores == new.machine.logical_cores
        && base.build.os == new.build.os;
    if !same_machine {
        warnings.push("different machines: timings are not comparable".into());
    }
    let same_gpu = base.machine.gpu.is_some() && base.machine.gpu == new.machine.gpu;
    if base.build.profile != new.build.profile {
        warnings.push(format!("different profiles: {} vs {}", base.build.profile, new.build.profile));
    }
    let (mut mismatches, mut changes, mut within, mut unmatched) = (Vec::new(), Vec::new(), 0, Vec::new());
    for b in &base.cases {
        let Some(n) = new.cases.iter().find(|n| n.name == b.name) else {
            unmatched.push(format!("{} (only in base)", b.name));
            continue;
        };
        if let (Some(x), Some(y)) = (&b.state_hash, &n.state_hash)
            && x != y
        {
            mismatches.push(format!("{}: state hash {} vs {}", b.name, &x[..12], &y[..12]));
        }
        if same_gpu
            && let (Some(x), Some(y)) = (&b.image_sha256, &n.image_sha256)
            && x != y
        {
            mismatches.push(format!("{}: image {} vs {}", b.name, &x[..12], &y[..12]));
        }
        for e in [&b.error, &n.error].into_iter().flatten() {
            mismatches.push(format!("{}: error {e}", b.name));
        }
        let nm = metrics(n);
        for (metric, bv, min_abs) in metrics(b) {
            let Some(&(_, nv, _)) = nm.iter().find(|(m, _, _)| *m == metric) else {
                continue;
            };
            let pct = if bv > 0.0 { (nv - bv) / bv * 100.0 } else { 0.0 };
            if pct.abs() > threshold_pct && (nv - bv).abs() >= min_abs {
                changes.push(Change {
                    case: b.name.clone(),
                    metric: metric.into(),
                    base: bv,
                    new: nv,
                    change_pct: r2(pct),
                });
            } else {
                within += 1;
            }
        }
    }
    for n in &new.cases {
        if !base.cases.iter().any(|b| b.name == n.name) {
            unmatched.push(format!("{} (only in new)", n.name));
        }
    }
    changes.sort_by(|a, b| b.change_pct.abs().total_cmp(&a.change_pct.abs()));
    CompareOutput {
        outputs_identical: mismatches.is_empty(),
        output_mismatches: mismatches,
        same_machine,
        same_gpu,
        warnings,
        changes,
        within_threshold: within,
        unmatched,
    }
}
