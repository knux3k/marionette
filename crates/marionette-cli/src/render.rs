//! CLI commands that use the renderer directly: bench and the windowed run.
//! (Screenshots go through the shared `observe_screenshot` operation.)

use marionette_agent::{Engine, engine};
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

/// Loads a scene with the running engine's modules, as `scene_run` does.
fn load(engine: &Engine, path: &str, seed: u64) -> Result<Sim, Errors> {
    engine.load_file(Path::new(path), path, seed)
}

/// Typical interpupillary distance in meters, for the stereo benchmark.
const IPD_M: f32 = 0.064;

/// Options of `marionette bench`.
pub struct BenchOptions<'a> {
    pub engine: &'a Engine,
    pub path: &'a str,
    pub frames: u32,
    pub seed: u64,
    pub camera: Option<u64>,
    pub size: &'a str,
    pub msaa: u32,
    pub eyes: u32,
    pub headset: bool,
    pub warmup: u32,
}

/// Head turn for headset emulation: yaw in radians at frame `i` (90 Hz, 2 s period).
/// Measurement only; the simulation never sees it.
fn head_yaw(i: u32) -> f32 {
    let t = i as f32 / 90.0;
    25f32.to_radians() * (t * std::f32::consts::PI).sin()
}

pub fn bench(o: BenchOptions) -> Result<Value, Errors> {
    let BenchOptions {
        engine: o_engine,
        path,
        frames,
        seed,
        camera,
        size,
        msaa,
        eyes,
        headset,
        warmup,
    } = o;
    let (width, height) = parse_size(size)?;
    check_msaa(msaa)?;
    if frames == 0 {
        return Err(Diagnostic::error("cli.bad_frames", "--frames must be at least 1").into());
    }
    if !(1..=marionette_render::MAX_VIEWS as u32).contains(&eyes) {
        return Err(Diagnostic::error(
            "cli.bad_eyes",
            format!("--eyes must be 1..{}", marionette_render::MAX_VIEWS),
        )
        .fix("use --eyes 2 for a stereo headset")
        .into());
    }
    let mut sim = load(o_engine, path, seed)?;
    let gpu = Gpu::headless()?;
    let mut offscreen = Offscreen::new(&gpu, width, height, msaa, scene_dir(path));
    // Warm-up: load assets, compile pipelines, settle clocks. Not measured.
    for _ in 0..warmup {
        sim.step();
        offscreen
            .render_eyes(&gpu, &extract(&sim.world), camera, eyes, IPD_M)
            .map_err(|e| e.with_file(path))?;
    }
    let n = frames as usize;
    let (mut times, mut sims, mut extracts, mut records, mut waits) = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    let mut stats = marionette_render::RenderStats::default();
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
    for i in 0..frames {
        let start = Instant::now();
        sim.step();
        let stepped = Instant::now();
        let mut frame = extract(&sim.world);
        if headset {
            let turn = glam::Mat4::from_rotation_y(head_yaw(i));
            for c in &mut frame.cameras {
                c.world *= turn;
            }
        }
        let extracted = Instant::now();
        stats = offscreen.render_eyes(&gpu, &frame, camera, eyes, IPD_M)?;
        times.push(ms(start.elapsed()));
        sims.push(ms(stepped - start));
        extracts.push(ms(extracted - stepped));
        records.push(offscreen.last_timing.record_ms);
        waits.push(offscreen.last_timing.gpu_wait_ms);
    }
    let round = |x: f64| (x * 1000.0).round() / 1000.0;
    let pct = |v: &[f64], p: f64| {
        let mut sorted = v.to_vec();
        sorted.sort_by(f64::total_cmp);
        round(sorted[((sorted.len() as f64 - 1.0) * p).round() as usize])
    };
    let avg = times.iter().sum::<f64>() / times.len() as f64;
    let max = times.iter().copied().fold(0.0, f64::max);
    const BUDGET_MS: f64 = 1000.0 / 90.0;
    Ok(json!({
        "ok": true,
        "command": "bench",
        "file": path,
        "gpu": gpu.describe(),
        "width": width,
        "height": height,
        "msaa": msaa,
        "eyes": eyes,
        "headset": headset,
        "frames": frames,
        "avg_ms": round(avg),
        "p50_ms": pct(&times, 0.5),
        "p99_ms": pct(&times, 0.99),
        "max_ms": round(max),
        "phases_p50_ms": {
            "sim": pct(&sims, 0.5),
            "extract": pct(&extracts, 0.5),
            "record": pct(&records, 0.5),
            "gpu_wait": pct(&waits, 0.5),
        },
        "render": stats,
        "budget_ms": round(BUDGET_MS),
        "within_budget": max <= BUDGET_MS,
        "method": "offscreen; per frame: 1 sim tick + extract + render all eyes + wait for GPU idle (serialized, so conservative)",
    }))
}

/// Live input and sound for the window. Input problems are kept and reported at the end.
struct Live {
    player: Option<engine::SoundPlayer>,
    problems: Vec<String>,
    #[cfg(feature = "save")]
    saves: Option<Saves>,
}

/// Saving in the window (decision 0076): autosave, `save_request` events, F5 and F9.
#[cfg(feature = "save")]
struct Saves {
    writer: marionette_save::SaveWriter,
    auto: marionette_save::Autosaver,
    written: u64,
    last: Option<marionette_save::WriteResult>,
}

#[cfg(feature = "save")]
impl Saves {
    const QUICK: &str = "quick";

    fn new(engine: &Engine, sim: &Sim, dir: &str) -> Result<Self, Errors> {
        let store = marionette_save::SaveStore::open(dir)?;
        let registry = std::sync::Arc::new(marionette_core::module::registry_with(&engine.modules()));
        Ok(Self {
            auto: marionette_save::Autosaver::new(sim, &store),
            writer: marionette_save::SaveWriter::new(store, registry),
            written: 0,
            last: None,
        })
    }

    fn quick_save(&self, sim: &mut Sim) -> Result<(), Errors> {
        let info = marionette_save::SaveInfo {
            label: "Quick save".into(),
            meta: Value::Null,
        };
        self.writer
            .submit(Self::QUICK, marionette_save::Pending::capture(sim, info))
    }

    fn quick_load(&mut self, sim: &mut Sim) -> Result<(), Errors> {
        self.writer.flush();
        marionette_save::load(sim, self.writer.store(), Self::QUICK)?;
        self.auto = marionette_save::Autosaver::new(sim, self.writer.store());
        Ok(())
    }

    /// Queues due autosaves and collects finished writes; returns write errors.
    fn after_ticks(&mut self, sim: &mut Sim) -> Vec<String> {
        let mut problems = Vec::new();
        if let Err(e) = self.auto.after_step(sim, &self.writer) {
            problems.push(e.to_string());
        }
        for r in self.writer.take_results() {
            match &r.error {
                Some(e) => problems.push(format!("save {}: {e}", r.slot)),
                None => self.written += 1,
            }
            self.last = Some(r);
        }
        problems
    }

    fn finish(mut self) -> Value {
        self.writer.flush();
        let rest = self.writer.take_results();
        self.written += rest.iter().filter(|r| r.error.is_none()).count() as u64;
        let last = rest.into_iter().last().or(self.last.take());
        json!({ "written": self.written, "last": last })
    }
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
        #[cfg(feature = "save")]
        if value > 0.0
            && let Some(saves) = self.saves.as_mut()
        {
            let r = match input {
                "key:F5" => saves.quick_save(sim),
                "key:F9" => saves.quick_load(sim),
                _ => Ok(()),
            };
            self.note(r);
        }
        let tick = sim.tick();
        let r = engine::queue_live(sim, tick, input, false, value);
        self.note(r);
    }
    fn action(&mut self, sim: &mut Sim, tick: u64, action: &str, value: f32) {
        let r = engine::queue_live(sim, tick, action, true, value);
        self.note(r);
    }
    fn after_ticks(&mut self, sim: &mut Sim) {
        #[cfg(feature = "save")]
        if let Some(saves) = self.saves.as_mut() {
            for m in saves.after_ticks(sim) {
                if self.problems.len() < 20 {
                    self.problems.push(m);
                }
            }
        }
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
    pub engine: &'a Engine,
    pub path: &'a str,
    pub ticks: Option<u64>,
    pub seed: u64,
    pub camera: Option<u64>,
    pub size: &'a str,
    pub input: Option<&'a str>,
    pub record: Option<&'a str>,
    pub mute: bool,
    /// Save folder (autosave, F5 and F9); `None` saves nothing.
    pub saves: Option<&'a str>,
}

pub fn windowed(run: WindowedRun) -> Result<Value, Errors> {
    let WindowedRun {
        engine: o_engine,
        path,
        ticks,
        seed,
        camera,
        size,
        input,
        record,
        mute,
        saves,
    } = run;
    let (width, height) = parse_size(size)?;
    let mut sim = load(o_engine, path, seed)?;
    if let Some(f) = input {
        engine::feed_file(&mut sim, Path::new(f), f)?;
    }
    let mut live = Live {
        player: None,
        problems: Vec::new(),
        #[cfg(feature = "save")]
        saves: match saves {
            Some(dir) => Some(Saves::new(o_engine, &sim, dir)?),
            None => None,
        },
    };
    #[cfg(not(feature = "save"))]
    if saves.is_some() {
        return Err(Diagnostic::error("cli.no_saves", "this build has no save games")
            .fix("build with the `save` feature")
            .into());
    }
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
    #[cfg(feature = "save")]
    let saved = live.saves.take().map(Saves::finish);
    #[cfg(not(feature = "save"))]
    let saved: Option<Value> = None;
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
    if let Some(s) = saved {
        out["saves"] = s;
    }
    Ok(out)
}
