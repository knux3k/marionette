//! CLI commands that use the renderer directly: bench and the windowed run.
//! (Screenshots go through the shared `observe_screenshot` operation.)

use marionette_agent::engine;
use marionette_core::{Diagnostic, Errors, Sim};
use marionette_render::window::{WindowHooks, WindowOptions, run_windowed};
use marionette_render::{Gpu, Offscreen, extract};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn parse_size(size: &str) -> Result<(u32, u32), Errors> {
    let parsed = size
        .split_once(['x', 'X'])
        .and_then(|(w, h)| Some((w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?)))
        .filter(|&(w, h)| (1..=8192).contains(&w) && (1..=8192).contains(&h));
    parsed.ok_or_else(|| {
        Diagnostic::error("cli.bad_size", format!("invalid size `{size}`"))
            .fix("use WIDTHxHEIGHT with each side 1..8192, e.g. 1280x720")
            .into()
    })
}

fn check_msaa(msaa: u32) -> Result<(), Errors> {
    if matches!(msaa, 1 | 4) {
        Ok(())
    } else {
        Err(
            Diagnostic::error("cli.bad_msaa", format!("unsupported MSAA sample count {msaa}"))
                .fix("use --msaa 1 or --msaa 4 (the counts every WebGPU device supports)")
                .into(),
        )
    }
}

/// Directory that relative asset paths in the scene are resolved against.
fn scene_dir(path: &str) -> PathBuf {
    Path::new(path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// Loads a scene with every engine module (input, physics, audio), as `scene_run` does.
fn load(path: &str, seed: u64) -> Result<Sim, Errors> {
    engine::load_file(Path::new(path), path, seed)
}

pub fn bench(
    path: &str,
    frames: u32,
    seed: u64,
    camera: Option<u64>,
    size: &str,
    msaa: u32,
) -> Result<Value, Errors> {
    let (width, height) = parse_size(size)?;
    check_msaa(msaa)?;
    if frames == 0 {
        return Err(Diagnostic::error("cli.bad_frames", "--frames must be at least 1").into());
    }
    let mut sim = load(path, seed)?;
    let gpu = Gpu::headless()?;
    let mut offscreen = Offscreen::new(&gpu, width, height, msaa, scene_dir(path));
    // Warm-up: load assets, compile pipelines, settle clocks. Not measured.
    for _ in 0..30 {
        sim.step();
        offscreen
            .render(&gpu, &extract(&sim.world), camera)
            .map_err(|e| e.with_file(path))?;
    }
    let mut times = Vec::with_capacity(frames as usize);
    for _ in 0..frames {
        let start = Instant::now();
        sim.step();
        let frame = extract(&sim.world);
        offscreen.render(&gpu, &frame, camera)?;
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let avg = times.iter().sum::<f64>() / times.len() as f64;
    let mut sorted = times.clone();
    sorted.sort_by(f64::total_cmp);
    let pct = |p: f64| sorted[((sorted.len() as f64 - 1.0) * p).round() as usize];
    let round = |x: f64| (x * 1000.0).round() / 1000.0;
    const BUDGET_MS: f64 = 1000.0 / 90.0;
    Ok(json!({
        "ok": true,
        "command": "bench",
        "file": path,
        "gpu": gpu.describe(),
        "width": width,
        "height": height,
        "msaa": msaa,
        "frames": frames,
        "avg_ms": round(avg),
        "p50_ms": round(pct(0.5)),
        "p99_ms": round(pct(0.99)),
        "max_ms": round(*sorted.last().expect("frames >= 1")),
        "budget_ms": round(BUDGET_MS),
        "within_budget": sorted.last().is_some_and(|&m| m <= BUDGET_MS),
        "method": "offscreen; per frame: 1 sim tick + extract + render + wait for GPU idle (serialized, so conservative)",
    }))
}

/// Live input and sound for the window. Input problems are kept and reported at the end.
struct Live {
    player: Option<engine::SoundPlayer>,
    problems: Vec<String>,
}

impl Live {
    fn note(&mut self, r: Result<(), Errors>) {
        if let Err(e) = r
            && self.problems.len() < 20
        {
            self.problems.push(e.to_string());
        }
    }
}

impl WindowHooks for Live {
    fn device(&mut self, sim: &mut Sim, input: &str, value: f32) {
        let tick = sim.tick();
        let r = engine::queue_live(sim, tick, input, false, value);
        self.note(r);
    }
    fn action(&mut self, sim: &mut Sim, tick: u64, action: &str, value: f32) {
        let r = engine::queue_live(sim, tick, action, true, value);
        self.note(r);
    }
    fn after_ticks(&mut self, sim: &mut Sim) {
        if let Some(p) = self.player.as_mut() {
            for m in p.play_new(sim.events()) {
                if self.problems.len() < 20 && !self.problems.contains(&m) {
                    self.problems.push(m);
                }
            }
        }
    }
}

/// Options of the windowed run beyond the shared ones.
pub struct WindowedRun<'a> {
    pub path: &'a str,
    pub ticks: Option<u64>,
    pub seed: u64,
    pub camera: Option<u64>,
    pub size: &'a str,
    pub input: Option<&'a str>,
    pub record: Option<&'a str>,
    pub mute: bool,
}

pub fn windowed(run: WindowedRun) -> Result<Value, Errors> {
    let WindowedRun {
        path,
        ticks,
        seed,
        camera,
        size,
        input,
        record,
        mute,
    } = run;
    let (width, height) = parse_size(size)?;
    let mut sim = load(path, seed)?;
    if let Some(f) = input {
        engine::feed_file(&mut sim, Path::new(f), f)?;
    }
    let mut live = Live {
        player: None,
        problems: Vec::new(),
    };
    if !mute {
        match engine::SoundPlayer::new(scene_dir(path)) {
            Ok(p) => live.player = Some(p),
            Err(m) => live.problems.push(m),
        }
    }
    let opts = WindowOptions {
        title: format!("Marionette - {path}"),
        width,
        height,
        camera,
        sample_count: 4,
        max_ticks: ticks,
        base_dir: scene_dir(path),
    };
    let mut sim = run_windowed(sim, opts, &mut live).map_err(|e| e.with_file(path))?;
    let recorded = match record {
        Some(out) => Some(engine::save_recording(&sim, Path::new(out), out)?),
        None => None,
    };
    let mut out = json!({
        "ok": true,
        "command": "run",
        "file": path,
        "windowed": true,
        "ticks": sim.tick(),
        "seed": seed,
        "state_hash": sim.state_hash(),
        "problems": live.problems,
    });
    if let Some(n) = recorded {
        out["recorded_events"] = json!(n);
    }
    Ok(out)
}
