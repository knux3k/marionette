//! Synth operations (decision 0220) through the operation table: render a score to numbers and
//! a WAV, with no sound card. Sample hashes are not pinned across platforms (the maths library
//! differs); block-size independence is proved in `crates/marionette-synth/tests`.

use marionette_agent::Agent;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn call(a: &mut Agent, op: &str, input: Value) -> Value {
    a.call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} {input} failed: {e}"))
}

#[test]
fn example_score_renders_clean() {
    let dir = std::env::temp_dir().join(format!("marionette-synth-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(
        repo().join("examples/synth-demo.score.json"),
        dir.join("demo.score.json"),
    )
    .unwrap();
    let mut a = Agent::new(&dir, false);
    let r = call(
        &mut a,
        "synth_render",
        json!({ "file": "demo.score.json", "wav": "out/demo.wav", "envelope_ms": 500 }),
    );
    let rep = &r["report"];
    assert_eq!(rep["non_finite"], 0);
    assert_eq!(rep["clipped"], 0);
    assert!(
        rep["peak"].as_f64().unwrap() > 0.1 && rep["peak"].as_f64().unwrap() <= 1.0,
        "{rep}"
    );
    assert!(
        rep["first_sound_frame"].as_u64().unwrap() < 100,
        "the first note starts at once (a pad fades in from silence): {rep}"
    );
    assert!(rep["dc"].as_f64().unwrap().abs() < 0.01, "{rep}");
    // Master gain 0 at 3 s: the last second is silent, the first half-second is not.
    let env = r["envelope"].as_array().unwrap();
    assert_eq!(env.len(), 8);
    assert!(env[0].as_f64().unwrap() > 0.01);
    assert!(env[7].as_f64().unwrap() < 0.001, "{env:?}");
    let wav = std::fs::read(dir.join("out/demo.wav")).unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(wav.len(), 44 + 48_000 * 4 * 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_score_is_a_diagnostic_and_patches_list() {
    let mut a = Agent::new(std::env::temp_dir(), false);
    let e = a
        .call(
            "synth_render",
            json!({ "score": { "notes": [{ "note": 60, "patch": "nope" }] } }),
        )
        .unwrap_err();
    assert!(e.to_string().contains("unknown patch"), "{e}");
    let p = call(&mut a, "synth_patches", json!({}));
    assert_eq!(p["presets"].as_array().unwrap().len(), 7);
    assert_eq!(p["patches"]["pad"]["wave"], "saw");
}
