//! Performance operations: generate benchmark scenes and time the simulation tick
//! (decision 0021). Timings are measurements of this machine and never feed back into a
//! simulation; the state hash they report is the same for any thread count.

use crate::{Agent, Op, op, write_atomic};
use marionette_core::{Diagnostic, Errors, scene};
use marionette_jobs::Jobs;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Instant;

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "perf_factory_scene",
            "Write a factory benchmark scene: `scale` x 34 production cells (7 machines, 6 belts, \
             128 belt slots each: miners, smelters, wire, circuits, a lab), plus a camera and a sun.",
            true,
            factory_scene,
        ),
        op(
            "perf_sim",
            "Time the simulation tick of a scene: run `warmup` ticks, then `ticks` timed ticks; \
             returns per-tick milliseconds (avg, p50, p99, max), the worker thread count and the state hash.",
            false,
            sim,
        ),
    ]
}

/// Production cells per unit of scale.
pub const CELLS_PER_SCALE: u32 = 34;
/// Belt slot spacing used by the generator, meters.
const SLOT_M: f32 = 0.25;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FactorySceneInput {
    /// Output scene file (relative to the project root).
    pub out: String,
    /// Size: 1 = 34 cells (238 machines, 204 belts, 4,352 slots). 1..=256.
    #[serde(default = "one")]
    pub scale: u32,
}

fn one() -> u32 {
    1
}

#[derive(Serialize, JsonSchema)]
pub struct FactorySceneOutput {
    pub scene: String,
    pub cells: u32,
    pub machines: u32,
    pub belts: u32,
    pub slots: u32,
    pub entities: u32,
}

fn color(r: f32, g: f32, b: f32) -> Value {
    json!({ "r": r, "g": g, "b": b, "a": 1.0 })
}

fn transform(x: f32, y: f32, z: f32, s: f32) -> Value {
    json!({
        "translation": { "x": x, "y": y, "z": z },
        "rotation": { "x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0 },
        "scale": { "x": s, "y": s, "z": s },
    })
}

fn recipes() -> Value {
    let amount = |item: &str, count: u32| json!({ "item": item, "count": count });
    let recipe = |id: &str, inputs: Vec<Value>, output: Value, ticks: u32| json!({ "id": id, "inputs": inputs, "output": output, "ticks": ticks });
    let look =
        |item: &str, c: Value| json!({ "item": item, "model": "primitive:cube", "color": c, "scale": 0.18 });
    json!({
        "recipes": [
            recipe("mine_iron", vec![], amount("iron_ore", 1), 30),
            recipe("mine_copper", vec![], amount("copper_ore", 1), 30),
            recipe("smelt_iron", vec![amount("iron_ore", 1)], amount("iron_plate", 1), 60),
            recipe("smelt_copper", vec![amount("copper_ore", 1)], amount("copper_plate", 1), 60),
            recipe("wire", vec![amount("copper_plate", 1)], amount("wire", 2), 30),
            recipe("circuit", vec![amount("iron_plate", 1), amount("wire", 2)], amount("circuit", 1), 60),
            recipe("research", vec![amount("circuit", 1)], amount("science", 0), 120),
        ],
        "looks": [
            look("iron_ore", color(0.35, 0.3, 0.3)),
            look("copper_ore", color(0.6, 0.35, 0.2)),
            look("iron_plate", color(0.75, 0.75, 0.8)),
            look("copper_plate", color(0.85, 0.5, 0.3)),
            look("wire", color(0.95, 0.6, 0.2)),
            look("circuit", color(0.2, 0.7, 0.3)),
            look("science", color(0.3, 0.4, 0.9)),
        ],
    })
}

/// The factory benchmark scene as JSON (not yet canonical).
pub fn factory_scene_json(scale: u32) -> (Value, FactorySceneOutput) {
    let cells = scale * CELLS_PER_SCALE;
    let side = (cells as f32).sqrt().ceil() as u32;
    let (cell_w, cell_d) = (16.0f32, 12.0f32);
    let mut entities = Vec::new();
    let (mut machines, mut belts, mut slots) = (0u32, 0u32, 0u32);
    let mid_x = side as f32 * cell_w * 0.5;
    let mid_z = side as f32 * cell_d * 0.5;
    entities.push(json!({ "id": 1, "name": "camera", "components": {
        "Camera": { "clear_color": color(0.45, 0.6, 0.8),
                    "projection": { "Perspective": { "fov_y_degrees": 70.0, "near": 0.1, "far": 2000.0 } } },
        "Transform": transform(mid_x, 25.0, -20.0, 1.0),
        "LookAt": { "target": { "x": mid_x, "y": 0.0, "z": mid_z * 0.6 }, "up": { "x": 0.0, "y": 1.0, "z": 0.0 } },
    }}));
    entities.push(json!({ "id": 2, "name": "sun", "components": {
        "DirectionalLight": { "color": color(1.0, 0.97, 0.9), "direction": { "x": -0.4, "y": -1.0, "z": -0.3 }, "illuminance": 1.0 },
    }}));
    // (name, recipe, local x, local z, color)
    type Place = (&'static str, &'static str, f32, f32, (f32, f32, f32));
    let layout: [Place; 7] = [
        ("miner_iron", "mine_iron", 0.0, 0.0, (0.4, 0.4, 0.45)),
        ("miner_copper", "mine_copper", 0.0, 8.0, (0.5, 0.35, 0.25)),
        ("smelter_iron", "smelt_iron", 4.0, 0.0, (0.7, 0.3, 0.2)),
        ("smelter_copper", "smelt_copper", 4.0, 8.0, (0.7, 0.4, 0.2)),
        ("wire", "wire", 8.0, 8.0, (0.8, 0.7, 0.2)),
        ("circuit", "circuit", 8.0, 0.0, (0.2, 0.6, 0.3)),
        ("lab", "research", 12.0, 0.0, (0.3, 0.4, 0.8)),
    ];
    let links: [(usize, usize); 6] = [(0, 2), (1, 3), (3, 4), (2, 5), (4, 5), (5, 6)];
    let mut next_id = 10u64;
    for c in 0..cells {
        let (ox, oz) = ((c % side) as f32 * cell_w, (c / side) as f32 * cell_d);
        let ids: Vec<u64> = (0..layout.len() as u64).map(|k| next_id + k).collect();
        for (k, (name, recipe, x, z, (r, g, b))) in layout.iter().enumerate() {
            entities.push(
                json!({ "id": ids[k], "name": format!("c{c}_{name}"), "components": {
                    "Transform": transform(ox + x, 0.75, oz + z, 1.5),
                    "Model": { "asset": "primitive:cube" },
                    "Material": { "base_color": color(*r, *g, *b), "metallic": 0.3, "roughness": 0.5 },
                    "FactoryMachine": { "recipe": recipe, "buffer": 8 },
                }}),
            );
            machines += 1;
        }
        next_id += layout.len() as u64;
        for (from, to) in links {
            let (a, b) = (layout[from], layout[to]);
            let dist = ((b.2 - a.2).powi(2) + (b.3 - a.3).powi(2)).sqrt();
            let n = (dist / SLOT_M).round().max(1.0) as u32;
            entities.push(json!({ "id": next_id, "components": {
                "FactoryBelt": { "from": ids[from], "to": ids[to], "slots": n, "step_milliticks": 4000 },
            }}));
            next_id += 1;
            belts += 1;
            slots += n;
        }
    }
    let count = entities.len() as u32;
    let scene = json!({
        "format": "marionette.scene",
        "version": 1,
        "resources": {
            "FactoryRecipes": recipes(),
            "AmbientLight": { "color": color(1.0, 1.0, 1.0), "intensity": 0.35 },
        },
        "entities": entities,
    });
    (
        scene,
        FactorySceneOutput {
            scene: String::new(),
            cells,
            machines,
            belts,
            slots,
            entities: count,
        },
    )
}

fn factory_scene(agent: &mut Agent, input: FactorySceneInput) -> Result<FactorySceneOutput, Errors> {
    if !(1..=256).contains(&input.scale) {
        return Err(
            Diagnostic::error(crate::codes::BAD_INPUT, "scale must be 1..=256")
                .fix("use scale 1 for a starting base, 4 for a large factory, 32 for a stress test")
                .into(),
        );
    }
    let (value, mut out) = factory_scene_json(input.scale);
    let text = scene::format_text(&value.to_string(), agent.registry())?;
    let path = agent.resolve(&input.out)?;
    write_atomic(&path, &input.out, text.as_bytes())?;
    out.scene = input.out;
    Ok(out)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SimInput {
    pub scene: String,
    #[serde(default)]
    pub seed: u64,
    /// Timed ticks (default 600).
    #[serde(default = "default_ticks")]
    pub ticks: u64,
    /// Untimed ticks first (default 60), so belts fill and caches settle.
    #[serde(default = "default_warmup")]
    pub warmup: u64,
    /// Worker threads for jobs (1 = inline). Default: one per core, or MARIONETTE_JOB_THREADS.
    pub threads: Option<usize>,
}

fn default_ticks() -> u64 {
    600
}
fn default_warmup() -> u64 {
    60
}

#[derive(Serialize, JsonSchema)]
pub struct SimOutput {
    pub scene: String,
    pub ticks: u64,
    pub threads: usize,
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    /// Load time of the scene (parse, validate, build module state), milliseconds.
    pub load_ms: f64,
    pub state_hash: String,
}

fn sim(agent: &mut Agent, input: SimInput) -> Result<SimOutput, Errors> {
    let started = Instant::now();
    let mut sim = crate::project::load_sim(agent, &input.scene, input.seed)?;
    let load_ms = started.elapsed().as_secs_f64() * 1000.0;
    let jobs = match input.threads {
        Some(n) => Jobs::with_threads(n),
        None => Jobs::shared(),
    };
    let threads = jobs.threads();
    sim.world.insert_resource(jobs);
    sim.run(input.warmup);
    let mut times = Vec::with_capacity(input.ticks as usize);
    for _ in 0..input.ticks {
        let t = Instant::now();
        sim.step();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let round = |x: f64| (x * 1000.0).round() / 1000.0;
    let avg = if times.is_empty() {
        0.0
    } else {
        times.iter().sum::<f64>() / times.len() as f64
    };
    times.sort_by(f64::total_cmp);
    let pct = |p: f64| {
        times
            .get(((times.len().max(1) - 1) as f64 * p).round() as usize)
            .copied()
            .unwrap_or(0.0)
    };
    Ok(SimOutput {
        scene: input.scene,
        ticks: input.ticks,
        threads,
        avg_ms: round(avg),
        p50_ms: round(pct(0.5)),
        p99_ms: round(pct(0.99)),
        max_ms: round(times.last().copied().unwrap_or(0.0)),
        load_ms: round(load_ms),
        state_hash: sim.state_hash(),
    })
}
