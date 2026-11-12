//! The example game's module and operation, through the library and through its own binary.

use marionette_agent::{Agent, Engine, Op, codes, op};
use marionette_core::Module;
use marionette_core::builtin::Position;
use marionette_example_rust_module::{SpringModule, engine};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn bob(sim: &mut marionette_core::Sim) -> Position {
    *sim.world.query::<&Position>().single(&sim.world).unwrap()
}

#[test]
fn the_game_module_runs_deterministically_and_settles_at_the_anchor() {
    let e = engine().unwrap();
    assert!(e.registry().get("Spring").is_some());
    let path = dir().join("spring.scene.json");
    let mut a = e.load_file(&path, "spring", 1).unwrap();
    let mut b = e.load_file(&path, "spring", 1).unwrap();
    a.run(1200);
    b.run(1200);
    assert_eq!(a.state_hash(), b.state_hash());
    let p = bob(&mut a);
    assert!(
        p.x.abs() < 0.01 && (p.y - 1.0).abs() < 0.01 && p.z.abs() < 0.01,
        "{p:?}"
    );
}

#[test]
fn the_built_in_engine_does_not_know_the_game_component() {
    let path = dir().join("spring.scene.json");
    let Err(err) = Engine::standard().load_file(&path, "spring", 1) else {
        panic!("loaded")
    };
    assert!(err.to_string().contains("Spring"), "{err}");
}

#[test]
fn the_game_operation_changes_a_live_session() {
    let mut agent = Agent::with_engine(dir(), false, engine().unwrap());
    assert!(agent.operations().any(|o| o.name == "spring_set"));
    let s = agent
        .call(
            "session_start",
            json!({ "scene": "spring.scene.json", "seed": 1 }),
        )
        .unwrap();
    let id = s["session"].as_str().unwrap().to_string();
    let out = agent
        .call(
            "spring_set",
            json!({ "session": id, "entity": 1, "stiffness": 0.0, "damping": 0.0 }),
        )
        .unwrap();
    assert_eq!(out, json!({ "entity": 1, "stiffness": 0.0, "damping": 0.0 }));
    // With no pull the entity keeps its (zero) velocity and stays put.
    agent
        .call("session_step", json!({ "session": id, "ticks": 60 }))
        .unwrap();
    let p = bob(agent.session_sim(&id).unwrap());
    assert_eq!((p.x, p.y, p.z), (3.0, 2.0, -1.0));
    let err = agent
        .call("spring_set", json!({ "session": id, "entity": 9 }))
        .unwrap_err();
    assert_eq!(err.0[0].code, codes::NO_ENTITY);
    // Read-only agents refuse it like any mutating operation.
    let mut ro = Agent::with_engine(dir(), true, engine().unwrap());
    let err = ro
        .call("spring_set", json!({ "session": "s1", "entity": 1 }))
        .unwrap_err();
    assert_eq!(err.0[0].code, codes::READ_ONLY);
}

struct Named(&'static str);
impl Module for Named {
    fn name(&self) -> &'static str {
        self.0
    }
}

fn noop(_: &mut Agent, _: marionette_agent::session::SessionInput) -> Result<Value, marionette_core::Errors> {
    Ok(Value::Null)
}

fn named_op(name: &'static str) -> Op {
    op(name, "test", false, noop)
}

#[test]
fn clashing_or_badly_named_parts_are_refused() {
    let dup_module = Engine::builder()
        .module(SpringModule)
        .module(Named("spring"))
        .build();
    let dup_builtin = Engine::builder().module(Named("physics")).build();
    let dup_op = Engine::builder().op(named_op("scene_run")).build();
    let bad_name = Engine::builder().op(named_op("Spring-Set")).build();
    for (what, r) in [("module", dup_module), ("builtin", dup_builtin), ("op", dup_op)] {
        let e = r.err().unwrap_or_else(|| panic!("{what} accepted"));
        assert_eq!(e.0[0].code, codes::DUPLICATE, "{what}");
    }
    assert_eq!(bad_name.err().unwrap().0[0].code, codes::BAD_NAME);
}

fn game(args: &[&str]) -> (bool, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_spring-game"))
        .args(["--json", "--project"])
        .arg(dir())
        .args(args)
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
    (out.status.success(), v)
}

#[test]
fn the_game_binary_has_every_command_with_the_game_parts() {
    let (ok, v) = game(&[
        "run",
        "spring.scene.json",
        "--headless",
        "--ticks",
        "120",
        "--seed",
        "1",
    ]);
    assert!(ok, "{v}");
    let (ok, again) = game(&[
        "run",
        "spring.scene.json",
        "--headless",
        "--ticks",
        "120",
        "--seed",
        "1",
    ]);
    assert!(ok);
    assert_eq!(v["state_hash"], again["state_hash"]);
    let (ok, v) = game(&["ops"]);
    assert!(ok);
    let names: Vec<&str> = v["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"spring_set") && names.contains(&"scene_run"));
    // Optional parts are in only when their cargo feature is. This crate asks for `script`
    // alone; a workspace-wide build unifies features, so CI also tests this crate on its own.
    let features = marionette_agent::FEATURES;
    for (feature, op) in [
        ("pool", "asset_search"),
        ("import", "library_info"),
        ("generate", "generate_recipe"),
        ("port", "port_scan"),
        ("jobs-ts", "jobs_check"),
    ] {
        assert_eq!(names.contains(&op), features.contains(&feature), "{feature}");
    }
    assert!(names.iter().any(|n| n.starts_with("script_")));
    assert!(Path::new(env!("CARGO_BIN_EXE_spring-game")).exists());
}
