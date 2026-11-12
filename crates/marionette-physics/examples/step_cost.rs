//! Wall-clock cost of the physics tick on a pile of dynamic boxes (measurement tool, not
//! part of any simulation): `cargo run --release -p marionette-physics --example step_cost -- 400`.
//! Used to measure Rapier's `enhanced-determinism` cost (decision 0012).

use marionette_core::Sim;
use marionette_physics::PhysicsModule;
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let boxes: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(400);
    let t = |x: f64, y: f64, z: f64, s: [f64; 3]| {
        json!({ "translation": { "x": x, "y": y, "z": z }, "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
                "scale": { "x": s[0], "y": s[1], "z": s[2] } })
    };
    let collider = json!({ "shape": { "Cuboid": { "half_extents": { "x": 0.5, "y": 0.5, "z": 0.5 } } },
                           "friction": 0.6, "restitution": 0, "density": 1, "sensor": false });
    let mut entities = vec![json!({ "id": 1, "components": {
        "Transform": t(0.0, -0.5, 0.0, [40.0, 1.0, 40.0]), "Collider": collider } })];
    let side = (boxes as f64).cbrt().ceil() as u64;
    for i in 0..boxes {
        let (x, y, z) = (i % side, i / (side * side), (i / side) % side);
        entities.push(json!({ "id": 100 + i, "components": {
            "Transform": t(x as f64 * 1.1 - side as f64 * 0.55, 2.0 + y as f64 * 1.3, z as f64 * 1.1 - side as f64 * 0.55, [1.0; 3]),
            "RigidBody": { "kind": "Dynamic", "linear_damping": 0, "angular_damping": 0, "gravity_scale": 1, "lock_rotations": false },
            "Collider": collider, "Velocity": { "x": 0, "y": 0, "z": 0 }, "AngularVelocity": { "x": 0, "y": 0, "z": 0 } } }));
    }
    let text = json!({ "format": "marionette.scene", "version": 1,
        "resources": { "PhysicsSettings": { "mode": "Dim3", "gravity": { "x": 0, "y": -9.81, "z": 0 } } },
        "entities": entities })
    .to_string();
    let modules = Arc::new(vec![Box::new(PhysicsModule) as Box<dyn marionette_core::Module>]);
    let mut sim = Sim::with_modules(&text, None, 0, modules, Default::default()).expect("scene loads");
    let mut times = Vec::new();
    for _ in 0..600 {
        let start = Instant::now();
        sim.step();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let avg = times.iter().sum::<f64>() / times.len() as f64;
    // The pile falls and collapses during the first ticks, then bodies go to sleep.
    let active = times[..120].iter().sum::<f64>() / 120.0;
    times.sort_by(f64::total_cmp);
    println!(
        "{boxes} boxes, 600 ticks: first 120 avg {:.3} ms, all avg {:.3} ms, p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms, state {}",
        active,
        avg,
        times[300],
        times[593],
        times[599],
        &sim.state_hash()[..16]
    );
}
