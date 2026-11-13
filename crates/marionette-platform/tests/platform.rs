use marionette_platform::*;

const DEFS: &str = r#"{ "format": "marionette.achievements", "version": 1,
  "achievements": [
    { "id": "FIRST_BUILD", "name": "First build" },
    { "id": "BUILD_10", "name": "Builder", "stat": "built", "at": 10 } ],
  "stats": [ { "id": "built", "kind": "int", "max": 1000 }, { "id": "distance", "kind": "float" } ] }"#;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-platform-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn definitions_are_checked() {
    assert!(parse_definitions(DEFS).is_ok());
    let dup = DEFS.replace("BUILD_10", "FIRST_BUILD");
    assert_eq!(parse_definitions(&dup).unwrap_err().code, codes::DEFINITIONS);
    let bad_stat = DEFS.replace("\"stat\": \"built\"", "\"stat\": \"nope\"");
    assert!(
        parse_definitions(&bad_stat)
            .unwrap_err()
            .message
            .contains("undefined stat")
    );
    let bad_id = DEFS.replace("FIRST_BUILD", "first build");
    assert!(parse_definitions(&bad_id).is_err());
}

#[test]
fn tracker_unlocks_from_stats_and_persists() {
    let dir = tmp("tracker");
    let defs = parse_definitions(DEFS).unwrap();
    let mut t = Tracker::new(defs.clone(), OfflinePlatform::open(&dir).unwrap());
    assert!(t.unlock("FIRST_BUILD").unwrap());
    assert!(!t.unlock("FIRST_BUILD").unwrap(), "once");
    assert_eq!(t.unlock("NOPE").unwrap_err().code, codes::UNKNOWN);
    assert_eq!(t.add_stat("built", 4).unwrap(), Vec::<String>::new());
    assert_eq!(t.add_stat("built", 6).unwrap(), vec!["BUILD_10".to_string()]);
    assert_eq!(t.add_stat("built", 5000).unwrap(), Vec::<String>::new());
    assert_eq!(
        t.backend.stat("built").unwrap(),
        Some(StatValue::Int(1000)),
        "clamped"
    );
    assert_eq!(
        t.set_stat("distance", StatValue::Int(3)).unwrap_err().code,
        codes::KIND
    );
    t.store().unwrap();
    let again = OfflinePlatform::open(&dir).unwrap();
    assert!(again.is_unlocked("BUILD_10").unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_folder_sync_two_machines() {
    let root = tmp("sync");
    let cloud_dir = root.join("platform");
    let pc = root.join("pc");
    let deck = root.join("deck");
    std::fs::create_dir_all(&pc).unwrap();
    std::fs::create_dir_all(&deck).unwrap();
    let mut cloud = OfflinePlatform::open(&cloud_dir).unwrap();
    std::fs::write(pc.join("slot1.save"), b"pc v1").unwrap();
    std::fs::write(pc.join("notes.txt"), b"not synced").unwrap();
    let r = sync_dir(&pc, &[".save"], &mut cloud).unwrap();
    assert_eq!(r.uploaded, vec!["slot1.save"]);
    // Second machine gets it.
    let r = sync_dir(&deck, &[".save"], &mut cloud).unwrap();
    assert_eq!(r.downloaded, vec!["slot1.save"]);
    assert_eq!(std::fs::read(deck.join("slot1.save")).unwrap(), b"pc v1");
    // Deck plays on; PC picks it up.
    std::fs::write(deck.join("slot1.save"), b"deck v2").unwrap();
    assert_eq!(
        sync_dir(&deck, &[".save"], &mut cloud).unwrap().uploaded,
        vec!["slot1.save"]
    );
    assert_eq!(
        sync_dir(&pc, &[".save"], &mut cloud).unwrap().downloaded,
        vec!["slot1.save"]
    );
    assert_eq!(std::fs::read(pc.join("slot1.save")).unwrap(), b"deck v2");
    // Nothing changed: nothing moves.
    assert_eq!(
        sync_dir(&pc, &[".save"], &mut cloud).unwrap(),
        SyncReport::default()
    );
    // Both change offline: conflict, nothing lost.
    std::fs::write(pc.join("slot1.save"), b"pc v3").unwrap();
    std::fs::write(deck.join("slot1.save"), b"deck v3").unwrap();
    sync_dir(&deck, &[".save"], &mut cloud).unwrap();
    let r = sync_dir(&pc, &[".save"], &mut cloud).unwrap();
    assert_eq!(r.conflicts, vec!["slot1.save"]);
    assert_eq!(std::fs::read(pc.join("slot1.save")).unwrap(), b"pc v3");
    assert_eq!(
        std::fs::read(pc.join("slot1.save.cloud-conflict")).unwrap(),
        b"deck v3"
    );
    // Deleting a save deletes it everywhere.
    std::fs::remove_file(deck.join("slot1.save")).unwrap();
    std::fs::write(deck.join("slot2.save"), b"x").unwrap();
    let r = sync_dir(&deck, &[".save"], &mut cloud).unwrap();
    assert_eq!(r.deleted_cloud, vec!["slot1.save"]);
    // Cloud off: sync does nothing.
    cloud.set_cloud_enabled(false);
    assert_eq!(
        sync_dir(&pc, &[".save"], &mut cloud).unwrap(),
        SyncReport::default()
    );
    assert_eq!(cloud.write("x.save", b"").unwrap_err().code, codes::UNAVAILABLE);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn bad_cloud_names() {
    for n in ["", "a/b", "..", "c:\\x"] {
        assert_eq!(check_name(n).unwrap_err().code, codes::NAME);
    }
}
