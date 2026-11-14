//! The factory benchmark scene and the `perf_factory_scene` operation (decision 0023): a
//! grid of production cells (miners, smelters, wire, circuits, a lab) joined by belts. The
//! engine's `perf_suite` runs it as `sim/factory-*` and `render/factory-*` when the engine it
//! runs in has this operation (decision 0270).

use marionette_agent::{Agent, Op, codes, op};
use marionette_core::{Diagnostic, Errors, scene};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// This crate's operations.
pub fn ops() -> Vec<Op> {
    vec![op(
        "perf_factory_scene",
        "Write a factory benchmark scene: `scale` x 34 production cells (7 machines, 6 belts, \
         128 belt slots each: miners, smelters, wire, circuits, a lab), plus a camera and a sun.",
        true,
        factory_scene,
    )]
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
        return Err(Diagnostic::error(codes::BAD_INPUT, "scale must be 1..=256")
            .fix("use scale 1 for a starting base, 4 for a large factory, 32 for a stress test")
            .into());
    }
    let (value, mut out) = factory_scene_json(input.scale);
    let text = scene::format_text(&value.to_string(), agent.registry())?;
    agent.write_file(&input.out, text.as_bytes())?;
    out.scene = input.out;
    Ok(out)
}
