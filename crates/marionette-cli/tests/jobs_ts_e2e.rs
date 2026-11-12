//! Job TypeScript through the operation table (the code the CLI and MCP server run): check a
//! file, run a function on JSON data with any thread count, report errors as data.

use marionette_agent::Agent;
use serde_json::{Value, json};

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent
        .call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} {input} failed: {e}"))
}

fn repo() -> Agent {
    Agent::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."), true)
}

#[test]
fn jobs_check_lists_functions() {
    let r = call(
        &mut repo(),
        "jobs_check",
        json!({"path": "examples/jobs/particles.job.ts"}),
    );
    assert_eq!(r["ok"], true);
    let names: Vec<&str> = r["functions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["drag", "step", "energy"]);
    assert_eq!(r["functions"][1]["params"][0]["type"], "Float32Array");
}

#[test]
fn jobs_check_reports_errors_as_data() {
    let d = std::env::temp_dir().join(format!("marionette-jobs-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("bad.job.ts"),
        "export function f(a: Float32Array): void {\n  a[0] = {x: 1};\n}\n",
    )
    .unwrap();
    let r = call(
        &mut Agent::new(&d, true),
        "jobs_check",
        json!({"path": "bad.job.ts"}),
    );
    assert_eq!(r["ok"], false);
    let d0 = &r["diagnostics"][0];
    assert_eq!(
        (d0["code"].as_str(), d0["line"].as_u64(), d0["column"].as_u64()),
        (Some("jobts.unsupported"), Some(2), Some(10))
    );
    assert!(d0["fix"].as_str().unwrap().contains("typed array"));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn jobs_run_is_the_same_on_any_thread_count() {
    let input = |threads: usize| {
        let v: Vec<f64> = (0..100_000).map(|i| (i % 97) as f64 * 0.25 - 12.0).collect();
        json!({
            "path": "examples/jobs/particles.job.ts",
            "function": "step",
            "args": [{"f32": v}, {"f32": v}, {"f32": v}, {"f32": v}, {"f32": v}, {"f32": v}, {"number": 0.016}, {"number": 0.001}],
            "chunk": 1000,
            "threads": threads,
        })
    };
    let one = call(&mut repo(), "jobs_run", input(1));
    let four = call(&mut repo(), "jobs_run", input(4));
    assert_eq!(one["chunks"], 100);
    assert_eq!(one["arrays"], four["arrays"]);
    assert_eq!(one["arrays"].as_object().unwrap().len(), 6);
}

#[test]
fn jobs_run_names_the_exported_functions_when_one_is_missing() {
    let e = repo()
        .call(
            "jobs_run",
            json!({"path": "examples/jobs/particles.job.ts", "function": "nope", "args": []}),
        )
        .unwrap_err();
    assert_eq!(e.0[0].code, "jobts.no_function");
}
