//! End-to-end checks of the agent interface (plan line 5): the same sequence through the CLI
//! (`--json` only) and through the MCP server over stdio must succeed and agree on the state
//! hash. Also: read-only mode and refused edits.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_marionette");
const SCENE: &str = "scenes/e2e.scene.json";

fn gpu_required() -> bool {
    std::env::var("MARIONETTE_REQUIRE_GPU").as_deref() == Ok("1")
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The operation calls of the scenario, as (name, input). Same list for CLI and MCP.
fn steps() -> Vec<(&'static str, Value)> {
    let t = |x: f64, y: f64, z: f64| {
        json!({ "translation": { "x": x, "y": y, "z": z }, "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
                "scale": { "x": 1, "y": 1, "z": 1 } })
    };
    let color = |r: f64, g: f64, b: f64| json!({ "r": r, "g": g, "b": b, "a": 1 });
    vec![
        ("scene_create", json!({ "scene": SCENE })),
        (
            "entity_create",
            json!({ "scene": SCENE, "name": "camera", "components": {
            "Transform": t(0.0, 3.0, 8.0),
            "LookAt": { "target": { "x": 0, "y": 0.5, "z": 0 }, "up": { "x": 0, "y": 1, "z": 0 } },
            "Camera": { "projection": { "Perspective": { "fov_y_degrees": 50, "near": 0.1, "far": 100 } },
                        "clear_color": color(0.3, 0.4, 0.6) } } }),
        ),
        (
            "entity_create",
            json!({ "scene": SCENE, "name": "sun", "components": {
            "DirectionalLight": { "direction": { "x": -0.3, "y": -1, "z": -0.4 }, "color": color(1.0, 1.0, 1.0),
                                  "illuminance": 1.0 } } }),
        ),
        (
            "entity_create",
            json!({ "scene": SCENE, "name": "crate", "components": {
            "Transform": t(-1.5, 0.5, 0.0), "Model": { "asset": "primitive:cube" } } }),
        ),
        (
            "entity_create",
            json!({ "scene": SCENE, "name": "mover", "components": {
            "Transform": t(0.0, 0.5, 0.0), "Model": { "asset": "primitive:sphere" },
            "Position": { "x": 0, "y": 0.5, "z": 0 }, "Velocity": { "x": 1, "y": 0, "z": 0 },
            "RandomImpulse": { "strength": 0.1 } } }),
        ),
        ("project_validate", json!({})),
        ("scene_run", json!({ "scene": SCENE, "seed": 1, "ticks": 100 })),
        (
            "observe_query",
            json!({ "scene": SCENE, "seed": 1, "ticks": 100, "with": ["Velocity"], "select": ["Position"] }),
        ),
        (
            "observe_screenshot",
            json!({ "scene": SCENE, "seed": 1, "ticks": 100, "size": "320x180" }),
        ),
        ("test_run", json!({})),
    ]
}

/// Checks every step's result and returns the state hash after 100 ticks.
fn check(results: &[(String, Value)], root: &Path) -> String {
    let get = |name: &str| {
        &results
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("no {name}"))
            .1
    };
    let ids: Vec<u64> = results
        .iter()
        .filter(|(n, _)| n == "entity_create")
        .map(|(_, v)| v["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, vec![1, 2, 3, 4]);
    assert_eq!(
        results.iter().find(|(n, _)| n == "entity_create").unwrap().1["diff"]["added"],
        json!([1])
    );
    assert_eq!(get("project_validate")["ok"], true, "{}", get("project_validate"));
    let hash = get("scene_run")["state_hash"].as_str().unwrap().to_string();
    let q = get("observe_query");
    assert_eq!(q["total"], 1);
    assert_eq!(q["items"][0]["id"], 4);
    let x = q["items"][0]["values"]["Position"]["x"].as_f64().unwrap();
    assert!(x > 1.0, "mover moved: x = {x}");
    if let Some((_, shot)) = results.iter().find(|(n, _)| n == "observe_screenshot") {
        let path = root.join(shot["path"].as_str().unwrap());
        assert!(
            std::fs::metadata(&path).unwrap().len() > 100,
            "{}",
            path.display()
        );
        assert_eq!(shot["state_hash"], hash.as_str());
    }
    let tests = get("test_run");
    assert_eq!(tests["failed"], 0, "{tests}");
    assert_eq!(tests["passed"], 1);
    hash
}

// ---------- CLI ----------

fn cli(root: &Path, extra: &[&str], name: &str, input: &Value) -> (bool, Value) {
    let out = Command::new(BIN)
        .args(["--project", root.to_str().unwrap()])
        .args(extra)
        .args(["op", name, &input.to_string(), "--json"])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{name}: stdout not JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    (out.status.success(), v)
}

fn init_project(tag: &str) -> PathBuf {
    let base = temp_dir(tag);
    let (ok, v) = cli(&base, &[], "project_init", &json!({ "path": "game" }));
    assert!(ok, "{v}");
    assert!(
        v["result"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "PROTOCOL.md")
    );
    base.join("game")
}

fn no_gpu(v: &Value) -> bool {
    v["diagnostics"][0]["code"] == "render.no_adapter" && !gpu_required()
}

fn run_cli_sequence(tag: &str) -> String {
    let root = init_project(tag);
    assert!(root.join("PROTOCOL.md").exists() && root.join("AGENTS.md").exists());
    let mut results = Vec::new();
    for (name, input) in steps() {
        let (ok, v) = cli(&root, &[], name, &input);
        if name == "observe_screenshot" && !ok && no_gpu(&v) {
            eprintln!("skipping screenshot: no GPU");
            continue;
        }
        assert!(ok, "{name} failed: {v}");
        results.push((name.to_string(), v["result"].clone()));
    }
    check(&results, &root)
}

#[test]
fn cli_only_sequence_succeeds_and_is_deterministic() {
    let a = run_cli_sequence("cli-a");
    let b = run_cli_sequence("cli-b");
    assert_eq!(a, b, "state hash differs between two CLI runs");
}

#[test]
fn invalid_edits_are_refused_and_leave_the_file_alone() {
    let root = init_project("bad-edit");
    let scene = "scenes/main.scene.json";
    let before = std::fs::read(root.join(scene)).unwrap();

    let (ok, v) = cli(
        &root,
        &[],
        "component_set",
        &json!({ "scene": scene, "id": 4, "component": "Velocty",
        "value": { "x": 1, "y": 0, "z": 0 } }),
    );
    assert!(!ok);
    let d = &v["diagnostics"][0];
    assert_eq!(d["code"], "scene.unknown_component");
    assert_eq!(d["entity"], 4);
    assert_eq!(d["pointer"], "/entities/3/components/Velocty");
    assert_eq!(d["fix"], "did you mean `Velocity`?");

    let (ok, v) = cli(
        &root,
        &[],
        "component_set",
        &json!({ "scene": scene, "id": 4, "component": "Velocity",
        "value": { "x": "fast", "y": 0, "z": 0 } }),
    );
    assert!(!ok);
    let d = &v["diagnostics"][0];
    assert_eq!(d["code"], "scene.field_type");
    assert_eq!(d["pointer"], "/entities/3/components/Velocity");
    assert!(d["message"].as_str().unwrap().contains("expected f32"), "{d}");

    let (ok, v) = cli(&root, &[], "entity_delete", &json!({ "scene": scene, "id": 99 }));
    assert!(!ok);
    assert_eq!(v["diagnostics"][0]["code"], "agent.no_entity");

    assert_eq!(
        std::fs::read(root.join(scene)).unwrap(),
        before,
        "refused edits must not touch the file"
    );
}

#[test]
fn cli_read_only_refuses_mutations() {
    let root = init_project("ro-cli");
    let (ok, v) = cli(
        &root,
        &["--read-only"],
        "entity_delete",
        &json!({ "scene": "scenes/main.scene.json", "id": 4 }),
    );
    assert!(!ok);
    assert_eq!(v["diagnostics"][0]["code"], "agent.read_only");
    let (ok, _) = cli(&root, &["--read-only"], "project_validate", &json!({}));
    assert!(ok, "read-only operations still work");
}

// ---------- MCP over stdio ----------

struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Mcp {
    fn start(root: &Path, read_only: bool) -> Self {
        Self::start_with(root, read_only, &[])
    }

    /// `extra` goes after `mcp` (e.g. `--tools core --lean`).
    fn start_with(root: &Path, read_only: bool, extra: &[&str]) -> Self {
        let mut cmd = Command::new(BIN);
        cmd.args(["--project", root.to_str().unwrap()]);
        if read_only {
            cmd.arg("--read-only");
        }
        let mut child = cmd
            .arg("mcp")
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut m = Mcp {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        let init = m.request(
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "marionette-test", "version": "0" }
            }),
        );
        assert_eq!(init["serverInfo"]["name"], "marionette", "{init}");
        assert!(init["instructions"].as_str().unwrap().contains("PROTOCOL.md"));
        m.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        m
    }

    fn send(&mut self, msg: Value) {
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Sends a request and returns its `result` (panics on a JSON-RPC error).
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let mut line = String::new();
            assert!(
                self.stdout.read_line(&mut line).unwrap() > 0,
                "server closed stdout"
            );
            let msg: Value = serde_json::from_str(&line).unwrap();
            if msg["id"] == id {
                assert!(msg.get("error").is_none(), "{method}: {msg}");
                return msg["result"].clone();
            }
        }
    }

    /// Calls a tool; returns (is_error, structured content).
    fn call(&mut self, name: &str, arguments: Value) -> (bool, Value) {
        let r = self.request("tools/call", json!({ "name": name, "arguments": arguments }));
        (r["isError"] == true, r["structuredContent"].clone())
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn mcp_sequence_matches_cli_state_hash() {
    let cli_hash = run_cli_sequence("mcp-ref");
    let base = temp_dir("mcp");
    let mut m = Mcp::start(&base, false);
    let tools = m.request("tools/list", json!({}));
    let names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for expected in [
        "project_init",
        "entity_create",
        "session_step",
        "observe_query",
        "observe_screenshot",
        "test_run",
    ] {
        assert!(names.contains(&expected), "missing tool {expected}: {names:?}");
    }
    let (err, v) = m.call("project_init", json!({ "path": "game" }));
    assert!(!err, "{v}");
    drop(m);

    // Serve the new project and run the same sequence through MCP tools.
    let root = base.join("game");
    let mut m = Mcp::start(&root, false);
    let mut results = Vec::new();
    for (name, input) in steps() {
        let (err, v) = m.call(name, input);
        if name == "observe_screenshot" && err && no_gpu(&v) {
            continue;
        }
        assert!(!err, "{name} failed: {v}");
        results.push((name.to_string(), v));
    }
    let mcp_hash = check(&results, &root);
    assert_eq!(mcp_hash, cli_hash, "MCP and CLI disagree on the state hash");

    // Live session: start, step 100, observe, edit the live world, step again.
    let (err, s) = m.call("session_start", json!({ "scene": SCENE, "seed": 1 }));
    assert!(!err, "{s}");
    let id = s["session"].as_str().unwrap().to_string();
    let (_, stepped) = m.call("session_step", json!({ "session": id, "ticks": 100 }));
    assert_eq!(
        stepped["state_hash"],
        cli_hash.as_str(),
        "live session agrees with scene_run"
    );
    let (err, e) = m.call("observe_entity", json!({ "session": id, "id": 4 }));
    assert!(
        !err && e["components"]["Position"]["x"].as_f64().unwrap() > 1.0,
        "{e}"
    );
    let (err, edit) = m.call(
        "component_set",
        json!({ "session": id, "id": 4, "component": "Velocity",
        "value": { "x": 0, "y": 0, "z": 0 } }),
    );
    assert!(!err, "{edit}");
    assert_eq!(edit["diff"]["changed"][0]["modified"], json!(["Velocity"]));
    let (_, stats) = m.call("observe_stats", json!({ "session": id }));
    assert_eq!(stats["tick"], 100);
    let (_, logs) = m.call("observe_logs", json!({ "session": id }));
    assert!(logs["entries"].as_array().unwrap().len() >= 3, "{logs}");
    let file_before = std::fs::read(root.join(SCENE)).unwrap();
    m.call("session_step", json!({ "session": id, "ticks": 10 }));
    assert_eq!(
        std::fs::read(root.join(SCENE)).unwrap(),
        file_before,
        "session edits do not touch the file"
    );
}

#[test]
fn mcp_read_only_refuses_mutations_with_structured_error() {
    let root = init_project("ro-mcp");
    let mut m = Mcp::start(&root, true);
    let tools = m.request("tools/list", json!({}));
    let names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        !names.contains(&"entity_create") && names.contains(&"observe_query"),
        "{names:?}"
    );
    let (err, v) = m.call(
        "entity_create",
        json!({ "scene": "scenes/main.scene.json", "components": {} }),
    );
    assert!(err);
    assert_eq!(v["ok"], false);
    assert_eq!(v["diagnostics"][0]["code"], "agent.read_only");
    let (err, v) = m.call(
        "scene_run",
        json!({ "scene": "scenes/main.scene.json", "seed": 1, "ticks": 10 }),
    );
    assert!(!err, "{v}");
}

/// Focus 2 (decision 0061): a client that only edits, runs and observes scenes can list just
/// those tools, without output schemas, and the listing shrinks to a fraction.
#[test]
fn mcp_tool_groups_and_lean_listing_shrink_the_context() {
    let root = init_project("mcp-lean");
    let full = Mcp::start(&root, false).request("tools/list", json!({}));
    let mut m = Mcp::start_with(&root, false, &["--tools", "core", "--lean"]);
    let lean = m.request("tools/list", json!({}));
    let names: Vec<&str> = lean["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    let core = [
        "project",
        "scene",
        "entity",
        "component",
        "resource",
        "session",
        "observe",
        "test",
        "batch",
    ];
    assert!(
        names.contains(&"entity_create") && names.contains(&"observe_query"),
        "{names:?}"
    );
    assert!(
        names.iter().all(|n| core.contains(&n.split('_').next().unwrap())),
        "{names:?}"
    );
    assert!(
        lean["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t.get("outputSchema").is_none())
    );
    let (full_len, lean_len) = (full.to_string().len(), lean.to_string().len());
    println!("tools/list bytes: all {full_len}, core lean {lean_len}");
    assert!(
        lean_len * 3 < full_len,
        "core lean listing {lean_len} B vs all {full_len} B"
    );

    // Tools left out are refused, listed ones work.
    let (err, v) = m.call("pool_list", json!({}));
    assert!(err);
    assert_eq!(v["diagnostics"][0]["code"], "agent.not_enabled");
    let (err, v) = m.call(
        "scene_run",
        json!({ "scene": "scenes/main.scene.json", "seed": 1, "ticks": 10 }),
    );
    assert!(!err, "{v}");

    // Unknown groups stop the server with a message naming the real ones.
    let out = Command::new(BIN)
        .args(["--project", root.to_str().unwrap(), "mcp", "--tools", "nosuch"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("observe"));
}

#[test]
fn ops_brief_is_small_and_filters_by_group() {
    let root = init_project("ops-brief");
    let run = |args: &[&str]| {
        let out = Command::new(BIN)
            .args(["--project", root.to_str().unwrap(), "--json", "ops"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    };
    let (full, brief) = (run(&[]), run(&["--brief"]));
    println!("ops bytes: full {}, brief {}", full.len(), brief.len());
    assert!(brief.len() * 10 < full.len());
    let pool: Value = serde_json::from_str(&run(&["--brief", "--tools", "pool"])).unwrap();
    let names: Vec<&str> = pool["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap())
        .collect();
    assert!(
        !names.is_empty() && names.iter().all(|n| n.starts_with("pool_")),
        "{names:?}"
    );
}
