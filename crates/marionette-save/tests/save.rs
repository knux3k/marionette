//! Save games: exact continuation, crash safety, migration, autosave (decision 0076).

use marionette_core::module::{Module, Modules};
use marionette_core::{Errors, Sim};
use marionette_save::format::SaveFile;
use marionette_save::{
    Autosaver, Change, Migrations, Pending, SaveFormat, SaveInfo, SaveModule, SaveStore, SaveWriter, codes,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-save-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn scene(format: Option<serde_json::Value>) -> String {
    let mut resources = json!({ "FixedStep": { "dt": 0.016666668 } });
    if let Some(f) = format {
        resources["SaveFormat"] = f;
    }
    json!({
        "format": "marionette.scene", "version": 1, "resources": resources,
        "entities": [
            { "id": 1, "name": "a", "components": {
                "Position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "Velocity": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "RandomImpulse": { "strength": 0.5 } } },
            { "id": 2, "name": "b", "components": {
                "Position": { "x": 1.0, "y": 2.0, "z": 3.0 },
                "Velocity": { "x": 0.1, "y": 0.0, "z": 0.0 },
                "Follow": { "gain": 1.5, "target": 1 } } }
        ]
    })
    .to_string()
}

fn format(game: &str, version: u32, every: u64, slots: u32, migrations: &str) -> serde_json::Value {
    json!({ "game": game, "version": version, "autosave_every": every,
            "autosave_slots": slots, "migrations": migrations })
}

fn sim_with(text: &str, dir: &Path, module: SaveModule) -> Result<Sim, Errors> {
    let modules: Modules = Arc::new(vec![Box::new(module) as Box<dyn Module>]);
    Sim::with_modules(text, None, 7, modules, dir.to_path_buf())
}

fn sim(text: &str, dir: &Path) -> Sim {
    sim_with(text, dir, SaveModule::default()).unwrap()
}

fn save(sim: &mut Sim, store: &SaveStore, slot: &str) {
    let file = Pending::capture(sim, SaveInfo::default())
        .into_file(sim.registry(), 0)
        .unwrap();
    store.write(slot, &file).unwrap();
}

#[test]
fn a_loaded_save_continues_exactly_as_if_never_saved() {
    let d = temp_dir("exact");
    let store = SaveStore::open(d.join("saves")).unwrap();
    let text = scene(Some(format("g", 1, 0, 3, "")));
    let mut a = sim(&text, &d);
    a.run(50);
    save(&mut a, &store, "slot-1");
    a.run(50);
    let mut b = sim(&text, &d);
    let loaded = marionette_save::load(&mut b, &store, "slot-1").unwrap();
    assert_eq!((loaded.tick, loaded.exact, loaded.recovered), (50, true, None));
    b.run(50);
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn the_writer_thread_writes_what_save_state_holds_and_drop_finishes_every_save() {
    let d = temp_dir("writer");
    let store = SaveStore::open(d.join("saves")).unwrap();
    let text = scene(None);
    let mut s = sim(&text, &d);
    let registry = Arc::new(marionette_core::module::registry_with(s.modules()));
    let writer = SaveWriter::new(store.clone(), registry).unwrap();
    let mut expected = Vec::new();
    for i in 0..10 {
        s.run(3);
        expected.push(s.save_state().scene);
        writer
            .submit(&format!("s{i}"), Pending::capture(&mut s, SaveInfo::default()))
            .unwrap();
    }
    drop(writer);
    for (i, scene) in expected.iter().enumerate() {
        let (file, recovered) = store.read(&format!("s{i}")).unwrap();
        assert!(recovered.is_none());
        let (snap, _) = file.into_snapshot(&["save"]).unwrap();
        assert_eq!(&snap.scene, scene, "slot s{i}");
        assert_eq!(snap.tick, 3 * (i as u64 + 1));
    }
}

#[test]
fn a_newer_save_of_a_waiting_slot_replaces_the_waiting_one() {
    let d = temp_dir("replace");
    let store = SaveStore::open(d.join("saves")).unwrap();
    let mut s = sim(&scene(None), &d);
    let registry = Arc::new(marionette_core::module::registry_with(s.modules()));
    let writer = SaveWriter::new(store.clone(), registry).unwrap();
    for _ in 0..50 {
        s.step();
        writer
            .submit("auto-0", Pending::capture(&mut s, SaveInfo::default()))
            .unwrap();
    }
    writer.flush();
    let results = writer.take_results();
    assert_eq!(results.len() as u64 + writer.replaced(), 50);
    assert!(results.iter().all(|r| r.error.is_none()));
    assert_eq!(store.read("auto-0").unwrap().0.header.tick, 50);
}

#[test]
fn a_damaged_or_cut_off_slot_loads_its_backup() {
    let d = temp_dir("damaged");
    let store = SaveStore::open(d.join("saves")).unwrap();
    let mut s = sim(&scene(None), &d);
    s.run(10);
    save(&mut s, &store, "main");
    s.run(10);
    save(&mut s, &store, "main");
    let path = store.path("main");
    let good = std::fs::read(&path).unwrap();

    // Cut off (power loss in the middle of a copy).
    std::fs::write(&path, &good[..good.len() / 2]).unwrap();
    let (file, recovered) = store.read("main").unwrap();
    assert_eq!(file.header.tick, 10);
    assert!(recovered.unwrap().contains("truncated"));

    // One flipped byte in the body.
    let mut bad = good.clone();
    let last = bad.len() - 5;
    bad[last] ^= 0x20;
    std::fs::write(&path, &bad).unwrap();
    let (file, recovered) = store.read("main").unwrap();
    assert_eq!(file.header.tick, 10);
    assert!(recovered.unwrap().contains("checksum"));

    // No backup either: a clear error, never a half-loaded world.
    std::fs::remove_file(store.backup_path("main")).unwrap();
    let e = store.read("main").unwrap_err();
    assert_eq!(e.0[0].code, codes::CORRUPT);
    let e = store.read("never").unwrap_err();
    assert_eq!(e.0[0].code, codes::NOT_FOUND);
}

#[test]
fn a_crash_between_the_two_renames_leaves_the_old_save_loadable_and_the_next_start_cleans_up() {
    let d = temp_dir("crash");
    let dir = d.join("saves");
    let store = SaveStore::open(&dir).unwrap();
    let mut s = sim(&scene(None), &d);
    s.run(10);
    save(&mut s, &store, "main");
    // The state a crash after "old save -> backup" and before "temp -> save" leaves.
    std::fs::rename(store.path("main"), store.backup_path("main")).unwrap();
    let tmp = dir.join(".main.99999.tmp");
    std::fs::write(&tmp, b"half written").unwrap();
    let (file, recovered) = store.read("main").unwrap();
    assert_eq!(file.header.tick, 10);
    assert!(recovered.is_some());
    assert_eq!(store.sweep(Duration::ZERO), 1);
    assert!(!tmp.exists());
}

#[test]
fn slot_menus_read_only_headers_and_bad_names_and_large_meta_are_refused() {
    let d = temp_dir("list");
    let store = SaveStore::open(d.join("saves")).unwrap();
    let mut s = sim(&scene(Some(format("g", 2, 0, 3, ""))), &d);
    s.run(5);
    let info = SaveInfo {
        label: "Gate".into(),
        meta: json!({ "chapter": 3 }),
    };
    let file = Pending::capture(&mut s, info).into_file(s.registry(), 1).unwrap();
    store.write("slot-1", &file).unwrap();
    std::fs::write(store.path("broken"), b"nope").unwrap();
    let list = store.list().unwrap();
    assert_eq!(list.len(), 2);
    assert!(list[0].problem.is_some() && list[0].slot == "broken");
    let h = list[1].header.as_ref().unwrap();
    assert_eq!(
        (h.game.as_str(), h.version, h.tick, h.label.as_str()),
        ("g", 2, 5, "Gate")
    );
    assert_eq!(h.meta["chapter"], 3);

    for bad in ["", "Slot", "../x", "a b", "-x", &"x".repeat(49)] {
        assert_eq!(
            store.write(bad, &file).unwrap_err().0[0].code,
            codes::BAD_SLOT,
            "{bad}"
        );
    }
    let big = SaveInfo {
        label: String::new(),
        meta: json!("x".repeat(20_000)),
    };
    let e = Pending::capture(&mut s, big)
        .into_file(s.registry(), 0)
        .unwrap_err();
    assert_eq!(e.0[0].code, codes::META);
    assert!(store.delete("slot-1").unwrap());
    assert!(!store.delete("slot-1").unwrap());
}

/// A version-1 save written by hand: its entity has `Speed` (later renamed `Velocity`) and
/// lacks `Position.z` (added in version 3).
fn old_save() -> SaveFile {
    let text = json!({
        "format": "marionette.scene", "version": 1,
        "resources": { "FixedStep": { "dt": 0.016666668 }, "SaveFormat": format("g", 1, 0, 3, "") },
        "entities": [ { "id": 1, "components": {
            "Position": { "x": 1.0, "y": 2.0 },
            "Speed": { "x": 0.5, "y": 0.0, "z": 0.0 } } } ]
    })
    .to_string();
    let snap = marionette_core::SimSnapshot {
        scene: text,
        tick: 40,
        rng: 9,
        modules: Default::default(),
    };
    let f = SaveFormat {
        game: "g".into(),
        version: 1,
        ..SaveFormat::default()
    };
    SaveFile::new(&snap, &f, SaveInfo::default(), 0).unwrap()
}

#[test]
fn old_saves_migrate_one_version_at_a_time_with_data_and_rust_steps() {
    let d = temp_dir("migrate");
    let store = SaveStore::open(d.join("saves")).unwrap();
    store.write("old", &old_save()).unwrap();
    let migrations = Migrations::new()
        .data(
            1,
            vec![Change::RenameType {
                from: "Speed".into(),
                to: "Velocity".into(),
            }],
        )
        .rust(2, |scene| {
            for e in &mut scene.entities {
                if let Some(p) = e.components.get_mut("Position") {
                    p["z"] = json!(-1.0);
                }
            }
            Ok(())
        });
    let text = scene(Some(format("g", 3, 0, 3, "")));
    let mut s = sim_with(&text, &d, SaveModule::with(migrations)).unwrap();
    let loaded = marionette_save::load(&mut s, &store, "old").unwrap();
    assert_eq!(
        (loaded.from_version, loaded.to_version, loaded.exact),
        (1, 3, false)
    );
    assert_eq!(s.tick(), 40);
    let now = s.snapshot();
    let e = &now.entities[0].components;
    assert_eq!(e["Velocity"]["x"], 0.5);
    assert_eq!(e["Position"]["z"], -1.0);
    // The running game's format replaced the one in the save.
    assert_eq!(now.resources["SaveFormat"]["version"], 3);
}

#[test]
fn the_migrations_file_named_by_the_scene_is_used() {
    let d = temp_dir("migfile");
    let store = SaveStore::open(d.join("saves")).unwrap();
    store.write("old", &old_save()).unwrap();
    std::fs::write(
        d.join("game.migrations.json"),
        json!({ "steps": [
            { "from": 1, "changes": [ { "op": "rename_type", "from": "Speed", "to": "Velocity" } ] },
            { "from": 2, "changes": [ { "op": "set_field", "on": "Position", "field": "z", "value": 4.0 } ] }
        ] })
        .to_string(),
    )
    .unwrap();
    let text = scene(Some(format("g", 3, 0, 3, "game.migrations.json")));
    let mut s = sim(&text, &d);
    marionette_save::load(&mut s, &store, "old").unwrap();
    assert_eq!(s.snapshot().entities[0].components["Position"]["z"], 4.0);

    // A missing file is a load error naming the field to fix.
    let bad = scene(Some(format("g", 3, 0, 3, "missing.json")));
    let e = sim_with(&bad, &d, SaveModule::default()).err().unwrap();
    assert_eq!(e.0[0].code, codes::IO);
}

#[test]
fn saves_of_another_game_or_a_newer_version_are_refused_and_the_world_is_untouched() {
    let d = temp_dir("refuse");
    let store = SaveStore::open(d.join("saves")).unwrap();
    let mut newer = sim(&scene(Some(format("g", 5, 0, 3, ""))), &d);
    save(&mut newer, &store, "newer");
    let mut other = sim(&scene(Some(format("h", 1, 0, 3, ""))), &d);
    save(&mut other, &store, "other");

    let mut s = sim(&scene(Some(format("g", 2, 0, 3, ""))), &d);
    s.run(3);
    let before = s.state_hash();
    let e = marionette_save::load(&mut s, &store, "newer").unwrap_err();
    assert_eq!(e.0[0].code, codes::NEWER);
    let e = marionette_save::load(&mut s, &store, "other").unwrap_err();
    assert_eq!(e.0[0].code, codes::OTHER_GAME);
    // A version-2 game has no step from 1 for this save's `Speed`: validation fails.
    store.write("old", &old_save()).unwrap();
    assert!(marionette_save::load(&mut s, &store, "old").is_err());
    assert_eq!(s.state_hash(), before);
}

#[test]
fn autosave_rotates_its_slots_and_save_requests_name_their_own() {
    let d = temp_dir("auto");
    let store = SaveStore::open(d.join("saves")).unwrap();
    let mut s = sim(&scene(Some(format("g", 1, 10, 2, ""))), &d);
    let registry = Arc::new(marionette_core::module::registry_with(s.modules()));
    let writer = SaveWriter::new(store.clone(), registry).unwrap();
    let mut auto = Autosaver::new(&s, &store);
    let mut queued = Vec::new();
    for t in 0..35 {
        s.step();
        if t == 14 {
            let tick = s.tick();
            s.world.resource_mut::<marionette_core::EventLog>().push(
                tick,
                "save_request",
                None,
                None,
                json!({ "slot": "checkpoint", "label": "Gate" }),
            );
        }
        queued.extend(auto.after_step(&mut s, &writer).unwrap());
    }
    drop(writer);
    assert_eq!(queued, ["auto-0", "checkpoint", "auto-1", "auto-0"]);
    assert_eq!(store.read("auto-0").unwrap().0.header.tick, 30);
    assert_eq!(store.read("auto-1").unwrap().0.header.tick, 20);
    let cp = store.read("checkpoint").unwrap().0.header;
    assert_eq!((cp.tick, cp.label.as_str()), (15, "Gate"));
}
