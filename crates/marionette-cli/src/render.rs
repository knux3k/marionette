//! CLI commands that use the renderer: screenshot, bench, windowed run.

use crate::read;
use marionette_core::error::codes;
use marionette_core::{Diagnostic, Errors, Sim, default_registry};
use marionette_render::assets::encode_png;
use marionette_render::window::{WindowOptions, run_windowed};
use marionette_render::{Gpu, Offscreen, extract};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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

fn load(path: &str, seed: u64) -> Result<Sim, Errors> {
    let text = read(path)?;
    Sim::from_text(&text, Some(path), default_registry(), seed)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn screenshot(
    path: &str,
    ticks: u64,
    seed: u64,
    camera: Option<u64>,
    size: &str,
    out: &str,
    msaa: u32,
) -> Result<Value, Errors> {
    let (width, height) = parse_size(size)?;
    check_msaa(msaa)?;
    let mut sim = load(path, seed)?;
    sim.run(ticks);
    let gpu = Gpu::headless()?;
    let mut offscreen = Offscreen::new(&gpu, width, height, msaa, scene_dir(path));
    let frame = extract(&sim.world);
    offscreen
        .render(&gpu, &frame, camera)
        .map_err(|e| e.with_file(path))?;
    let pixels = offscreen.read_pixels(&gpu)?;
    let png = encode_png(width, height, &pixels);
    std::fs::write(out, &png).map_err(|e| {
        Errors::from(Diagnostic::error(codes::IO_WRITE, format!("cannot write {out}: {e}")).file(out))
    })?;
    Ok(json!({
        "ok": true,
        "command": "screenshot",
        "file": path,
        "out": out,
        "width": width,
        "height": height,
        "camera": camera,
        "ticks": ticks,
        "seed": seed,
        "msaa": msaa,
        "gpu": gpu.describe(),
        "image_sha256": sha256_hex(&png),
        "state_hash": sim.state_hash(),
    }))
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

pub fn windowed(
    path: &str,
    ticks: Option<u64>,
    seed: u64,
    camera: Option<u64>,
    size: &str,
) -> Result<Value, Errors> {
    let (width, height) = parse_size(size)?;
    let sim = load(path, seed)?;
    let opts = WindowOptions {
        title: format!("Marionette - {path}"),
        width,
        height,
        camera,
        sample_count: 4,
        max_ticks: ticks,
        base_dir: scene_dir(path),
    };
    let mut sim = run_windowed(sim, opts).map_err(|e| e.with_file(path))?;
    Ok(json!({
        "ok": true,
        "command": "run",
        "file": path,
        "windowed": true,
        "ticks": sim.tick(),
        "seed": seed,
        "state_hash": sim.state_hash(),
    }))
}
