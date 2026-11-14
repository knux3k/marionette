//! Whole sessions over the loopback network: every peer must end in the host's state.

use marionette_core::Sim;
use marionette_core::module::Module;
use marionette_input::InputModule;
use marionette_net::{LinkConditions, NetModule, SimulateOptions, simulate};
use std::path::PathBuf;
use std::sync::Arc;

fn make(seed: u64) -> Result<Sim, String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let text = std::fs::read_to_string(root.join("drift.scene.json")).map_err(|e| e.to_string())?;
    let modules: Vec<Box<dyn Module>> = vec![Box::new(InputModule), Box::new(NetModule)];
    Sim::with_modules(&text, Some("drift.scene.json"), seed, Arc::new(modules), root)
        .map_err(|e| e.to_string())
}

#[test]
fn four_players_bad_network_stay_in_step() {
    let opts = SimulateOptions {
        players: 4,
        ticks: 600,
        seed: 5,
        link: LinkConditions {
            latency_ms: 80.0,
            jitter_ms: 40.0,
            loss: 0.10,
            duplicate: 0.05,
        },
        hash_interval: 30,
        input_change: 0.1,
        ..SimulateOptions::default()
    };
    let r = simulate(&opts, &make).unwrap();
    assert!(r.all_equal, "{r:#?}");
    assert!(
        r.hash_checks >= 3 * 600 / 30 - 3,
        "hashes were compared: {}",
        r.hash_checks
    );
    assert!(r.desyncs.is_empty());
    for p in &r.peers {
        assert!(p.max_behind < 40, "lag stays bounded: {p:?}");
    }
    // Same inputs: same run, byte for byte.
    assert_eq!(simulate(&opts, &make).unwrap(), r);
}

#[test]
fn late_join_and_drop() {
    let opts = SimulateOptions {
        players: 3,
        ticks: 400,
        seed: 9,
        late_join_at: Some(150),
        drop_slot: Some(2),
        drop_at: Some(100),
        ..SimulateOptions::default()
    };
    let r = simulate(&opts, &make).unwrap();
    assert!(r.all_equal, "{r:#?}");
    assert_eq!(r.peers.len(), 3);
    assert!(r.peers[1].final_hash.is_none(), "dropped peer");
    let joined = &r.peers[2];
    assert_eq!(joined.final_hash.as_deref(), Some(r.host_hash.as_str()));
    // The joiner took the slot freed by the dropped player once the host timed it out, or a
    // new one: either way it is a real slot.
    assert!(joined.slot < 8);
}

#[test]
fn a_desync_is_reported() {
    // Each peer loads with its own seed: the drift scene diverges at once.
    let seeds = std::sync::atomic::AtomicU64::new(0);
    let make_bad = |_seed: u64| make(seeds.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    let opts = SimulateOptions {
        players: 2,
        ticks: 200,
        hash_interval: 20,
        ..SimulateOptions::default()
    };
    let r = simulate(&opts, &make_bad).unwrap();
    assert!(!r.all_equal);
    assert!(!r.desyncs.is_empty());
    assert_eq!(r.desyncs[0], (1, 20), "first check catches it, with the slot");
}
