//! Acceptance checks for plan line 2 (core). See docs/DESIGN.md.

use marionette_core::error::codes;
use marionette_core::{Sim, default_registry, scene, schema};

const EXAMPLE: &str = include_str!("../../../examples/drift.scene.json");

fn hash(seed: u64, ticks: u64) -> String {
    let mut sim = Sim::from_text(EXAMPLE, None, default_registry(), seed).expect("example loads");
    sim.run(ticks);
    assert_eq!(sim.tick(), ticks);
    sim.state_hash()
}

#[test]
fn same_seed_gives_identical_hash_after_1000_ticks() {
    assert_eq!(hash(42, 1000), hash(42, 1000));
}

#[test]
fn different_seed_gives_different_hash() {
    assert_ne!(hash(42, 1000), hash(43, 1000));
}

#[test]
fn simulation_actually_changes_state() {
    assert_ne!(hash(42, 0), hash(42, 1));
}

#[test]
fn load_then_save_is_byte_identical() {
    let mut sim = Sim::from_text(EXAMPLE, None, default_registry(), 0).unwrap();
    assert_eq!(sim.snapshot().to_text(), EXAMPLE);
}

#[test]
fn example_is_canonical_and_fmt_is_idempotent() {
    let registry = default_registry();
    let once = scene::format_text(EXAMPLE, &registry).unwrap();
    assert_eq!(once, EXAMPLE);
    assert_eq!(scene::format_text(&once, &registry).unwrap(), once);
}

#[test]
fn fmt_normalizes_numbers_and_order() {
    let text = r#"{"version":1,"format":"marionette.scene","entities":[
        {"id":2,"components":{"Position":{"z":0,"y":0.1,"x":1}}},
        {"id":1,"components":{}}]}"#;
    let out = scene::format_text(text, &default_registry()).unwrap();
    assert!(out.contains("\"y\": 0.1,"), "f32 printed in shortest form: {out}");
    assert!(out.contains("\"x\": 1.0,"));
    assert!(out.find("\"id\": 1").unwrap() < out.find("\"id\": 2").unwrap());
}

fn codes_of(text: &str) -> Vec<&'static str> {
    let registry = default_registry();
    match scene::parse(text).and_then(|s| scene::validate(&s, &registry, Some(text)).map(|_| ())) {
        Ok(()) => vec![],
        Err(e) => e.0.iter().map(|d| d.code).collect(),
    }
}

#[test]
fn validator_reports_errors_as_data() {
    let bad = EXAMPLE
        .replacen("\"Velocity\": {", "\"Velocty\": {", 1)
        .replace("\"target\": 2", "\"target\": 99")
        .replace("\"strength\": 0.5", "\"strength\": \"big\"")
        .replacen("\"gain\": 1.5", "\"gian\": 1.5", 1);
    let codes = codes_of(&bad);
    for expected in [
        codes::SCENE_UNKNOWN_COMPONENT,
        codes::SCENE_FIELD_TYPE,
        codes::SCENE_UNKNOWN_FIELD,
        codes::SCENE_MISSING_FIELD,
    ] {
        assert!(codes.contains(&expected), "missing {expected} in {codes:?}");
    }

    let dangling = EXAMPLE.replace("\"target\": 2", "\"target\": 99");
    assert_eq!(codes_of(&dangling), vec![codes::SCENE_DANGLING_REF]);

    let dup = EXAMPLE.replace("\"id\": 3,", "\"id\": 2,");
    assert_eq!(codes_of(&dup), vec![codes::SCENE_DUPLICATE_ID]);

    assert_eq!(codes_of("{ nope"), vec![codes::SCENE_PARSE]);
    assert_eq!(
        codes_of(&EXAMPLE.replace("\"version\": 1", "\"version\": 7")),
        vec![codes::SCENE_VERSION]
    );
}

#[test]
fn diagnostics_carry_location_entity_and_fix() {
    let bad = EXAMPLE.replacen("\"Velocity\": {", "\"Velocty\": {", 1);
    let registry = default_registry();
    let err = scene::validate(&scene::parse(&bad).unwrap(), &registry, Some(&bad))
        .err()
        .unwrap();
    let d = &err.0[0];
    assert_eq!(d.code, codes::SCENE_UNKNOWN_COMPONENT);
    assert_eq!(d.entity, Some(2));
    assert!(d.line.is_some());
    assert_eq!(d.fix.as_deref(), Some("did you mean `Velocity`?"));
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(json[0]["code"], "scene.unknown_component");
}

#[test]
fn schema_covers_every_registered_type() {
    let registry = default_registry();
    let s = schema::schema(&registry);
    assert_eq!(s["$schema"], "https://json-schema.org/draft/2020-12/schema");
    for entry in registry.entries() {
        assert!(s["$defs"][entry.name].is_object(), "no schema for {}", entry.name);
    }
    assert_eq!(s["$defs"]["Position"]["properties"]["x"]["type"], "number");
    assert_eq!(s["$defs"]["Follow"]["properties"]["target"]["type"], "integer");
    assert!(s["properties"]["resources"]["properties"]["FixedStep"].is_object());
    // Deterministic output.
    assert_eq!(s, schema::schema(&default_registry()));
}

#[test]
fn diagnostic_positions_are_exact() {
    let bad = EXAMPLE.replacen("\"gain\": 1.5", "\"gian\": 1.5", 1);
    let registry = default_registry();
    let err = scene::validate(&scene::parse(&bad).unwrap(), &registry, Some(&bad))
        .err()
        .unwrap();
    let d = err
        .0
        .iter()
        .find(|d| d.code == codes::SCENE_UNKNOWN_FIELD)
        .unwrap();
    let (line, col) = (d.line.unwrap(), d.column.unwrap());
    let text_line = bad.lines().nth(line - 1).unwrap();
    assert_eq!(&text_line[col - 1..col + 5], "\"gian\"");
}

#[test]
fn render_data_components_are_registered() {
    let registry = default_registry();
    for name in [
        "Transform",
        "LookAt",
        "Camera",
        "Model",
        "Material",
        "Sprite",
        "SpriteRegion",
        "DirectionalLight",
        "AmbientLight",
    ] {
        assert!(registry.get(name).is_some(), "{name} not registered");
    }
}

#[test]
fn reload_keeps_tick_and_rng_and_keeps_running() {
    let mut a = Sim::from_text(EXAMPLE, None, default_registry(), 9).unwrap();
    let mut b = Sim::from_text(EXAMPLE, None, default_registry(), 9).unwrap();
    a.run(10);
    b.run(10);
    let text = a.snapshot().to_text();
    a.reload(&text).unwrap();
    assert_eq!(a.tick(), 10);
    a.run(10);
    b.run(10);
    assert_eq!(
        a.state_hash(),
        b.state_hash(),
        "reloading the same state changes nothing"
    );
}
