//! `marionette xr`: run a scene on a headset through OpenXR, or probe for one (decision 0190).

use marionette_agent::{Engine, engine};
use marionette_core::{Errors, Sim};
use marionette_openxr::{XrHooks, XrOptions};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub struct XrRun<'a> {
    pub engine: &'a Engine,
    pub path: &'a str,
    pub seed: u64,
    pub camera: Option<u64>,
    pub msaa: u32,
    pub frames: Option<u64>,
    pub ticks: Option<u64>,
    pub scale: f32,
    pub input: Option<&'a str>,
    pub record: Option<&'a str>,
    pub mute: bool,
}

struct Sound {
    player: Option<engine::SoundPlayer>,
    problems: Vec<String>,
    last: Option<std::time::Instant>,
}

impl XrHooks for Sound {
    fn head(&mut self, position: [f32; 3], rotation: [f32; 4]) {
        if let Some(p) = self.player.as_mut() {
            p.set_head(Some((position, rotation)));
        }
    }
    fn after_ticks(&mut self, sim: &mut Sim) {
        let Some(p) = self.player.as_mut() else { return };
        let now = std::time::Instant::now();
        let dt = self.last.map_or(0.0, |t| (now - t).as_secs_f32());
        self.last = Some(now);
        let mut problems = p.play_new(sim.events());
        problems.extend(p.update(&mut sim.world, dt));
        for m in problems {
            if self.problems.len() < 20 && !self.problems.contains(&m) {
                self.problems.push(m);
            }
        }
    }
}

pub fn probe() -> Result<Value, Errors> {
    let found = marionette_openxr::probe()?;
    Ok(json!({ "ok": true, "command": "xr", "probe": found }))
}

pub fn run(r: XrRun) -> Result<Value, Errors> {
    let dir = Path::new(r.path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut sim = r.engine.load_file(Path::new(r.path), r.path, r.seed)?;
    if let Some(f) = r.input {
        engine::feed_file(&mut sim, Path::new(f), f)?;
    }
    let mut hooks = Sound {
        player: None,
        problems: Vec::new(),
        last: None,
    };
    if !r.mute {
        match engine::SoundPlayer::new(dir.clone()) {
            Ok(p) => hooks.player = Some(p),
            Err(m) => hooks.problems.push(m),
        }
    }
    let opts = XrOptions {
        camera: r.camera,
        sample_count: r.msaa,
        max_frames: r.frames,
        max_ticks: r.ticks,
        resolution_scale: r.scale,
        base_dir: dir,
        app_name: format!("Marionette - {}", r.path),
    };
    let (sim, report) = marionette_openxr::run(sim, opts, &mut hooks).map_err(|e| e.with_file(r.path))?;
    let recorded = match r.record {
        Some(out) => Some(engine::save_recording(&sim, Path::new(out), out)?),
        None => None,
    };
    let mut sim = sim;
    let mut out = json!({
        "ok": true,
        "command": "xr",
        "file": r.path,
        "seed": r.seed,
        "state_hash": sim.state_hash(),
        "headset": report,
        "problems": hooks.problems,
    });
    if let Some(n) = recorded {
        out["recorded_events"] = json!(n);
    }
    Ok(out)
}
