//! Packaging without a build (decision 0140): what goes into each proof game's package, and
//! the zip writer. The full build per target runs in the CI workflow `package`.

use marionette_package::{read_game, zip};
use std::path::Path;

fn games() -> Vec<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../games");
    let mut out: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("marionette.json").is_file())
        .collect();
    out.sort();
    out
}

#[test]
fn every_proof_game_has_a_package_plan_without_tests_or_recipes() {
    let games = games();
    assert_eq!(games.len(), 6, "the six proof games");
    for dir in games {
        let g = read_game(&dir).unwrap();
        assert!(g.files.contains(&g.entry), "{}: entry in files", g.name);
        assert!(g.files.contains(&"marionette.json".to_string()));
        for f in &g.files {
            assert!(!f.starts_with("tests/"), "{}: {f}", g.name);
            assert!(
                !f.ends_with(".recipe.json") && !f.ends_with(".approval.json"),
                "{}: {f}",
                g.name
            );
            assert!(f != "build.ops.json" && f != "providers.json", "{}: {f}", g.name);
        }
        // Every model and image a scene names is kept.
        let scene = std::fs::read_to_string(dir.join(&g.entry)).unwrap();
        for f in g.files.iter().chain(&g.left_out) {
            let name = f.rsplit('/').next().unwrap();
            if scene.contains(name) {
                assert!(g.files.contains(f), "{}: named asset {f} left out", g.name);
            }
        }
        // The proof games script in TypeScript and use no job TypeScript.
        assert_eq!(g.features, vec!["script".to_string()], "{}", g.name);
    }
}

#[test]
fn job_typescript_is_detected_and_unreferenced_assets_are_listed() {
    let tmp = marionette_host::temp_dir("package-plan").unwrap();
    let d = tmp.path();
    std::fs::create_dir_all(d.join("scenes")).unwrap();
    std::fs::create_dir_all(d.join("assets")).unwrap();
    std::fs::write(
        d.join("marionette.json"),
        r#"{"format":"marionette.project","name":"t","version":1}"#,
    )
    .unwrap();
    std::fs::write(
        d.join("scenes/main.scene.json"),
        r#"{"JobSchedule":{},"model":"../assets/used.glb"}"#,
    )
    .unwrap();
    std::fs::write(d.join("assets/used.glb"), b"x").unwrap();
    std::fs::write(d.join("assets/spare.png"), b"x").unwrap();
    std::fs::write(d.join("assets/spare.provenance.json"), b"{}").unwrap();
    let g = read_game(d).unwrap();
    assert_eq!(g.features, vec!["jobs-ts".to_string()]);
    assert_eq!(
        g.left_out,
        vec![
            "assets/spare.png".to_string(),
            "assets/spare.provenance.json".to_string()
        ]
    );
    assert!(g.files.contains(&"assets/used.glb".to_string()));
    std::fs::write(
        d.join("marionette.json"),
        r#"{"name":"t","package":{"keep_unreferenced":true,"entry":"scenes/missing.json"}}"#,
    )
    .unwrap();
    let e = read_game(d).unwrap_err();
    assert_eq!(e.0[0].code, marionette_package::codes::GAME);
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

#[test]
fn zips_are_reproducible_and_list_every_file() {
    let tmp = marionette_host::temp_dir("package-zip").unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(src.join("a/b")).unwrap();
    std::fs::write(src.join("a/b/c.txt"), "hello hello hello").unwrap();
    std::fs::write(src.join("top.bin"), [0u8, 1, 2, 3]).unwrap();
    let (one, two) = (tmp.path().join("1.zip"), tmp.path().join("2.zip"));
    zip::write_dir(&src, "game", &one).unwrap();
    zip::write_dir(&src, "game", &two).unwrap();
    let bytes = std::fs::read(&one).unwrap();
    assert_eq!(bytes, std::fs::read(&two).unwrap(), "same folder, same bytes");
    assert_eq!(&bytes[..4], b"PK\x03\x04");
    let end = bytes.len() - 22;
    assert_eq!(&bytes[end..end + 4], b"PK\x05\x06");
    assert_eq!(u16_at(&bytes, end + 10), 2, "two entries");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("game/a/b/c.txt") && text.contains("game/top.bin"));
}
