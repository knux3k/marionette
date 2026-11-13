//! Job TypeScript from scenes and scripts (decision 0070): `JobSchedule` steps run every tick
//! over `JobData` arrays, scripts call `engine.jobs`, `JobDraw` draws the arrays, and problems
//! come back as data (load diagnostics, `job_error` events, script exceptions). GPU checks
//! skip without an adapter unless MARIONETTE_REQUIRE_GPU=1; MARIONETTE_UPDATE_REFERENCES=1
//! rewrites `examples/reference/swarm.png`.
#![cfg(all(feature = "jobs-ts", feature = "script"))]

use marionette_agent::Agent;
use marionette_core::jobdata::{JobData, JobValues, draw_job_data};
use marionette_core::scene::SceneId;
use marionette_core::{Errors, Sim};
use marionette_jobs::Jobs;
use marionette_render::assets::{decode_png, encode_png};
use marionette_render::compare::{REFERENCE_THRESHOLD, diff, within_reference_tolerance};
use marionette_render::{Gpu, Offscreen, extract};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// `examples/swarm.scene.json`, 600 ticks, seed 1 (4,096 particles stepped by a job, seeded
/// and measured by a script). A change here changes job or script results: say so in the
/// commit and in docs/ai/PROOFS.md.
const SWARM_HASH: &str = "f39ab44c85fbb75a1edcf1182caf6249ad8b069953c616d238dacf4f49286f33";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent
        .call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} {input} failed: {e}"))
}

fn swarm(threads: usize) -> Sim {
    let mut sim = Agent::new(root(), true)
        .load_sim("examples/swarm.scene.json", 1)
        .expect("the swarm example loads");
    sim.world.insert_resource(Jobs::with_threads(threads));
    sim
}

fn array<'w>(sim: &'w mut Sim, name: &str) -> &'w JobValues {
    let mut q = sim.world.query::<&JobData>();
    q.iter(&sim.world).next().unwrap().get(name).unwrap()
}

#[test]
fn swarm_hash_is_pinned_and_independent_of_thread_count() {
    let mut hashes = Vec::new();
    for threads in [1, 2, 4] {
        let mut sim = swarm(threads);
        sim.run(600);
        hashes.push(sim.state_hash());
    }
    assert_eq!(hashes[0], hashes[1]);
    assert_eq!(hashes[0], hashes[2]);
    assert_eq!(hashes[0], SWARM_HASH);
}

#[test]
fn script_seeds_the_arrays_and_the_schedule_moves_them() {
    let mut sim = swarm(2);
    sim.step();
    assert_eq!(array(&mut sim, "px").len(), 4096);
    let before = array(&mut sim, "px").get_f64(5);
    sim.run(10);
    assert_ne!(array(&mut sim, "px").get_f64(5), before);
    // The script reports energy once a second, from a job run (`engine.jobs.sum`).
    sim.run(110);
    let energy: Vec<f64> = sim
        .events()
        .events
        .iter()
        .filter(|e| e.kind == "swarm_energy")
        .map(|e| e.detail["energy"].as_f64().unwrap())
        .collect();
    assert_eq!(energy.len(), 3, "ticks 0, 60 and 120");
    assert!(energy.iter().all(|e| *e > 0.0));
    // One instanced draw of 4,096 cubes.
    let mut draws = Vec::new();
    draw_job_data(&sim.world, 0.0, &mut draws);
    assert_eq!(draws.len(), 1);
    assert_eq!(draws[0].asset, "primitive:cube");
    assert_eq!(draws[0].transforms.len(), 4096);
    assert_eq!(
        draws[0].transforms[5][9] as f64,
        array(&mut sim, "px").get_f64(5) as f32 as f64
    );
}

#[test]
fn snapshot_and_restore_continue_identically() {
    let mut sim = swarm(4);
    sim.run(100);
    let snap = sim.save_state();
    sim.run(100);
    let straight = sim.state_hash();
    sim.restore(&snap).unwrap();
    sim.run(100);
    assert_eq!(sim.state_hash(), straight);
}

/// A temporary project with one job file and a scene built from `entity` (id 1).
fn project(job: &str, entity: Value, script: Option<&str>) -> marionette_host::TempDir {
    let d = marionette_host::temp_dir("job-scenes").unwrap();
    std::fs::write(d.path().join("t.job.ts"), job).unwrap();
    if let Some(s) = script {
        std::fs::write(d.path().join("t.ts"), s).unwrap();
    }
    let scene = json!({
        "format": "marionette.scene", "version": 1,
        "entities": [{ "id": 1, "name": "e", "components": entity }],
    });
    std::fs::write(d.path().join("t.scene.json"), scene.to_string()).unwrap();
    d
}

fn load(d: &marionette_host::TempDir) -> Result<Sim, Errors> {
    Agent::new(d.path(), true).load_sim("t.scene.json", 1)
}

const ADD: &str = "export function add(a: Float32Array, b: Float32Array, k: number): void {\n  for (let i = 0; i < a.length; i++) {\n    a[i] = a[i] + b[i] * k;\n  }\n}\nexport function at(a: Int32Array, i: i32): i32 {\n  return a[i];\n}\ntype i32 = number;\n";

fn data(arrays: Value) -> Value {
    json!({ "arrays": arrays })
}

fn step(function: &str, args: &[&str]) -> Value {
    json!({ "steps": [{ "path": "t.job.ts", "function": function, "args": args, "chunk": 2 }] })
}

#[test]
fn a_scheduled_step_runs_every_tick_in_place() {
    let d = project(
        ADD,
        json!({
            "JobData": data(json!([
                { "name": "a", "values": { "F32": [0.0, 1.0, 2.0] } },
                { "name": "b", "values": { "F32": [1.0, 1.0, 1.0] } },
            ])),
            "JobSchedule": step("add", &["a", "in:b", "0.5"]),
        }),
        None,
    );
    let mut sim = load(&d).unwrap();
    sim.run(4);
    assert_eq!(
        array(&mut sim, "a"),
        &JobValues::F32(vec![2.0, 3.0, 4.0]),
        "four ticks of a += b * 0.5"
    );
    // Arrays are scene data: written by snapshots and part of the state hash.
    assert!(sim.snapshot().to_text().contains("\"JobData\""));
}

#[test]
fn schedule_problems_fail_the_load_with_diagnostics() {
    let cases = [
        (
            step("add", &["a", "missing", "1"]),
            "jobts.args",
            "no job array `missing`",
        ),
        (
            step("add", &["a", "a2", "1"]),
            "jobts.args",
            "is Float32Array but job array `a2` is Int32Array",
        ),
        (step("add", &["a", "b"]), "jobts.args", "takes 3 arguments"),
        (
            step("add", &["a", "b", "true"]),
            "jobts.args",
            "does not fit parameter `k: number`",
        ),
        (step("nope", &[]), "jobts.no_function", "`nope`"),
    ];
    for (schedule, code, text) in cases {
        let d = project(
            ADD,
            json!({
                "JobData": data(json!([
                    { "name": "a", "values": { "F32": [0.0] } },
                    { "name": "b", "values": { "F32": [0.0] } },
                    { "name": "a2", "values": { "I32": [0] } },
                ])),
                "JobSchedule": schedule,
            }),
            None,
        );
        let errs = load(&d).err().expect("load fails");
        let d0 = &errs.0[0];
        assert_eq!(d0.code, code, "{d0:?}");
        assert!(d0.message.contains(text), "{} lacks {text}", d0.message);
        assert_eq!(d0.entity, Some(1));
        assert_eq!(d0.component.as_deref(), Some("JobSchedule"));
    }
    let d = project(
        "export function f(: void {}",
        json!({ "JobSchedule": step("f", &[]) }),
        None,
    );
    assert_eq!(load(&d).err().unwrap().0[0].code, "jobts.syntax");
}

#[test]
fn a_failing_step_records_an_event_and_the_tick_goes_on() {
    let d = project(
        ADD,
        json!({
            "JobData": data(json!([{ "name": "n", "values": { "I32": [1, 2] } }])),
            "JobSchedule": step("at", &["in:n", "7"]),
        }),
        None,
    );
    let mut sim = load(&d).unwrap();
    sim.run(3);
    assert_eq!(sim.tick(), 3);
    let errors: Vec<_> = sim
        .events()
        .events
        .iter()
        .filter(|e| e.kind == "job_error")
        .collect();
    assert_eq!(errors.len(), 3);
    assert_eq!(errors[0].entity, Some(1));
    assert_eq!(errors[0].detail["code"], "jobts.trap");
}

const SCRIPT: &str = r#"
export function update(self: Entity): void {
  if (engine.tick !== 0) return;
  // A script's own typed array, changed in place; a job array of the entity, read only.
  const mine = new Float32Array([1, 2, 3, 4]);
  engine.jobs.run("t.job.ts", "add", [mine, engine.jobs.read("b"), 2], { chunk: 3 });
  engine.emit("mine", Array.from(mine));
  // Job arrays: create, run in place, read back a copy.
  engine.jobs.set("c", new Float32Array([10, 20, 30, 40]));
  engine.jobs.run("t.job.ts", "add", ["c", engine.jobs.read(mine), -1]);
  engine.emit("c", Array.from(engine.jobs.get("c")!));
  engine.emit("sum", engine.jobs.sum("t.job.ts", "at", [engine.jobs.whole(new Int32Array([5, 6])), 1]));
  try {
    engine.jobs.run("t.job.ts", "add", ["c", "nope", 0.5]);
  } catch (e) {
    engine.emit("caught", String((e as Error).message));
  }
}
"#;

#[test]
fn scripts_run_jobs_on_their_own_arrays_and_on_job_data() {
    let d = project(
        ADD,
        json!({
            "JobData": data(json!([{ "name": "b", "values": { "F32": [1.0, 1.0, 1.0, 1.0] } }])),
            "Script": { "path": "t.ts" },
        }),
        Some(SCRIPT),
    );
    let mut sim = load(&d).unwrap();
    sim.step();
    let ev = |kind: &str| -> Value {
        sim.events()
            .events
            .iter()
            .find(|e| e.kind == kind)
            .unwrap_or_else(|| panic!("no {kind} event: {:?}", sim.events().events))
            .detail
            .clone()
    };
    assert_eq!(ev("mine"), json!([3, 4, 5, 6]));
    assert_eq!(ev("c"), json!([7, 16, 25, 34]));
    assert_eq!(ev("sum"), json!(6));
    let caught = ev("caught");
    let caught = caught.as_str().unwrap();
    assert!(caught.starts_with("jobts.args"), "{caught}");
    // The entity's id is its scene id.
    let mut q = sim.world.query::<(&SceneId, &JobData)>();
    let (id, d) = q.iter(&sim.world).next().unwrap();
    assert_eq!((id.0, d.names()), (1, vec!["b", "c"]));
}

#[test]
fn jobs_reload_swaps_a_changed_file_in_a_live_session() {
    let d = project(
        ADD,
        json!({
            "JobData": data(json!([
                { "name": "a", "values": { "F32": [0.0] } },
                { "name": "b", "values": { "F32": [1.0] } },
            ])),
            "JobSchedule": step("add", &["a", "in:b", "1"]),
        }),
        None,
    );
    let mut agent = Agent::new(d.path(), false);
    let s = call(
        &mut agent,
        "session_start",
        json!({ "scene": "t.scene.json", "seed": 1 }),
    )["session"]
        .as_str()
        .unwrap()
        .to_string();
    call(&mut agent, "session_step", json!({ "session": s, "ticks": 2 }));
    std::fs::write(
        d.path().join("t.job.ts"),
        ADD.replace("b[i] * k", "b[i] * k * 10.0"),
    )
    .unwrap();
    let r = call(&mut agent, "jobs_reload", json!({ "session": s }));
    assert_eq!(r["files"][0]["status"], "reloaded");
    call(&mut agent, "session_step", json!({ "session": s, "ticks": 1 }));
    let sim = agent.session_sim(&s).unwrap();
    assert_eq!(array(sim, "a"), &JobValues::F32(vec![12.0]));
    // A broken file keeps the last good version and reports why.
    std::fs::write(d.path().join("t.job.ts"), "export function add(").unwrap();
    let r = call(&mut agent, "jobs_reload", json!({ "session": s }));
    assert_eq!(r["files"][0]["status"], "failed");
    assert_eq!(r["files"][0]["errors"][0]["code"], "jobts.syntax");
}

fn gpu() -> Option<Gpu> {
    match Gpu::headless() {
        Ok(g) => Some(g),
        Err(e) if std::env::var("MARIONETTE_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipping GPU test: {e}");
            None
        }
        Err(e) => panic!("GPU required but unavailable: {e}"),
    }
}

#[test]
fn swarm_draws_as_one_instanced_batch_and_matches_the_reference() {
    let Some(gpu) = gpu() else { return };
    let size = (320, 180);
    let mut sim = swarm(4);
    sim.run(600);
    let mut off = Offscreen::new(&gpu, size.0, size.1, 4, root().join("examples"));
    let stats = off.render(&gpu, &extract(&sim.world), None).unwrap();
    // Ground and 4,096 cubes: one draw each in the shadow and main passes, however many
    // particles.
    assert_eq!((stats.draw_calls, stats.instances), (4, 4097), "{stats:?}");
    let pixels = off.read_pixels(&gpu).unwrap();
    let reference = root().join("examples/reference/swarm.png");
    if std::env::var("MARIONETTE_UPDATE_REFERENCES").as_deref() == Ok("1") {
        std::fs::write(&reference, encode_png(size.0, size.1, &pixels)).unwrap();
    }
    let expected = decode_png(&std::fs::read(&reference).unwrap()).unwrap();
    let d = diff(&pixels, &expected.rgba, REFERENCE_THRESHOLD);
    assert!(within_reference_tolerance(&d), "swarm.png: {d:?}");
}
