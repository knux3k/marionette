//! Runs a whole networked session in one process: a host and clients, each with its own
//! copy of the sim, over a [`LoopbackNet`] with chosen latency, jitter, loss and
//! duplication, on virtual time. Every peer makes seeded random input. The result says
//! whether every copy ended in the same state, plus stalls, lag and traffic. This is the
//! `net_simulate` operation and the CI proof that a scene is safe to network.

use crate::lockstep::{Client, Host, LockstepConfig, SessionEvent};
use crate::loopback::{LinkConditions, LoopbackNet, LoopbackSocket, Mix};
use crate::reliable::Endpoint;
use crate::sim::{apply_frame, encode_snapshot, restore_snapshot};
use crate::transport::{Micros, Transport};
use marionette_core::Sim;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const STEP: Micros = 16_667;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SimulateOptions {
    /// Players at the start, including the host (1..=8).
    pub players: u8,
    /// Ticks the host runs.
    pub ticks: u64,
    /// Seed of the sims, the inputs and the network.
    pub seed: u64,
    pub link: LinkConditions,
    /// Compare state hashes every this many ticks (0 = never).
    pub hash_interval: u64,
    /// Actions that travel. Empty: the scene's action map, else `move_x`, `move_y`, `jump`.
    pub actions: Vec<String>,
    /// Chance per tick per action that a player changes it (to -1, 0 or 1).
    pub input_change: f64,
    /// A player who joins at this host tick (gets a snapshot).
    pub late_join_at: Option<u64>,
    /// A client slot whose game vanishes at `drop_at` (host frees the slot after a time-out).
    pub drop_slot: Option<u8>,
    pub drop_at: Option<u64>,
}

impl Default for SimulateOptions {
    fn default() -> Self {
        Self {
            players: 2,
            ticks: 600,
            seed: 1,
            link: LinkConditions {
                latency_ms: 40.0,
                jitter_ms: 10.0,
                loss: 0.02,
                duplicate: 0.0,
            },
            hash_interval: 60,
            actions: Vec::new(),
            input_change: 0.05,
            late_join_at: None,
            drop_slot: None,
            drop_at: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PeerReport {
    pub slot: u8,
    /// Fixed steps where a welcomed client had no frame to run.
    pub stalls: u64,
    /// Most ticks this peer was behind the host.
    pub max_behind: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub resends: u64,
    /// Hash at the host's final tick (absent for a dropped peer).
    pub final_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SimulateReport {
    /// Every peer still playing ended in the host's state.
    pub all_equal: bool,
    pub final_tick: u64,
    pub host_hash: String,
    /// (slot, tick) of each hash mismatch the session itself reported.
    pub desyncs: Vec<(u8, u64)>,
    /// Hash comparisons made by the session protocol.
    pub hash_checks: u64,
    pub actions: Vec<String>,
    pub peers: Vec<PeerReport>,
    /// Host bytes sent per tick, average.
    pub host_bytes_per_tick: f64,
    /// Virtual seconds of play.
    pub seconds: f64,
}

struct Peer {
    t: Endpoint<LoopbackSocket>,
    client: Client,
    sim: Option<Sim>,
    slot: u8,
    rng: Mix,
    values: BTreeMap<String, f32>,
    stalls: u64,
    max_behind: u64,
    gone: bool,
}

fn random_values(rng: &mut Mix, actions: &[String], p: f64, values: &mut BTreeMap<String, f32>) {
    for a in actions {
        if rng.unit() < p {
            let v = match rng.next() % 3 {
                0 => -1.0,
                1 => 0.0,
                _ => 1.0,
            };
            values.insert(a.clone(), v);
        }
    }
}

/// Runs the session. `make_sim` loads a fresh copy of the scene with the given seed.
pub fn simulate(
    opts: &SimulateOptions,
    make_sim: &dyn Fn(u64) -> Result<Sim, String>,
) -> Result<SimulateReport, String> {
    if !(1..=8).contains(&opts.players) {
        return Err("players must be 1 to 8".into());
    }
    let mut host_sim = make_sim(opts.seed)?;
    let actions = if opts.actions.is_empty() {
        host_sim
            .world
            .get_resource::<marionette_input::ActionMap>()
            .map(|m| m.actions.keys().cloned().collect::<Vec<_>>())
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| vec!["jump".into(), "move_x".into(), "move_y".into()])
    } else {
        opts.actions.clone()
    };
    let cfg = LockstepConfig {
        actions: actions.clone(),
        hash_interval: opts.hash_interval,
        timeout_us: 3_000_000,
        max_players: 8,
        late_join: true,
    };
    let net = LoopbackNet::new(opts.seed ^ 0x6e65_7473_696d, opts.link);
    let mut host_t = Endpoint::new(net.endpoint());
    let host_id = host_t.local();
    let mut peers: Vec<Peer> = Vec::new();
    let new_peer = |net: &LoopbackNet, slot_hint: u8| {
        let t = Endpoint::new(net.endpoint());
        Peer {
            t,
            client: Client::new(cfg.clone(), host_id),
            sim: None,
            slot: slot_hint,
            rng: Mix(opts.seed ^ (0x1000 + slot_hint as u64)),
            values: BTreeMap::new(),
            stalls: 0,
            max_behind: 0,
            gone: false,
        }
    };
    for i in 1..opts.players {
        peers.push(new_peer(&net, i));
    }
    let ids: Vec<_> = peers.iter().map(|p| p.t.local()).collect();
    let start = host_sim.tick();
    let end = start + opts.ticks;
    let mut host = Host::new(cfg.clone(), host_id, start, &ids);
    let mut host_rng = Mix(opts.seed ^ 0x1000);
    let mut host_values = BTreeMap::new();
    let mut desyncs = Vec::new();
    let mut hash_checks = 0u64;
    let mut iterations = 0u64;
    let limit = opts.ticks + 20_000;
    loop {
        iterations += 1;
        if iterations > limit {
            return Err(format!(
                "clients did not catch up within {} extra steps (network too lossy?)",
                limit - opts.ticks
            ));
        }
        net.advance(STEP);
        let now = net.now();
        host.poll(&mut host_t, now);
        if host.next_tick() < end {
            let tick = host.next_tick();
            if opts.late_join_at == Some(tick - start) {
                peers.push(new_peer(&net, 100 + peers.len() as u8));
            }
            if opts.drop_at == Some(tick - start)
                && let Some(p) = peers.iter_mut().find(|p| Some(p.slot) == opts.drop_slot)
            {
                p.gone = true;
                net.set_down(p.t.local(), true);
            }
            random_values(&mut host_rng, &actions, opts.input_change, &mut host_values);
            host.set_local(&host_values);
            let frame = host.next_frame();
            apply_frame(&mut host_sim, &cfg, &frame).map_err(|e| e.to_string())?;
            host_sim.step();
            if host.should_hash(host_sim.tick()) {
                let h = host_sim.state_hash();
                host.record_hash(host_sim.tick(), h.as_bytes());
            }
            if host.needs_snapshot() {
                let s = encode_snapshot(&mut host_sim);
                host.provide_snapshot(&s);
            }
        }
        host.flush(&mut host_t);
        while let Some(e) = host.event() {
            if let SessionEvent::Desync { slot, tick } = e {
                desyncs.push((slot, tick));
            }
        }
        let host_tick = host.next_tick();
        let mut done = host_tick >= end;
        for p in peers.iter_mut().filter(|p| !p.gone) {
            p.client.poll(&mut p.t, now);
            while let Some(e) = p.client.event() {
                if let SessionEvent::Welcomed {
                    slot,
                    start_tick,
                    snapshot,
                } = e
                {
                    p.slot = slot;
                    let mut sim = make_sim(opts.seed)?;
                    if let Some(s) = snapshot
                        && !restore_snapshot(&mut sim, &s)
                    {
                        return Err("a joining peer could not restore the snapshot".into());
                    }
                    if sim.tick() != start_tick {
                        return Err(format!(
                            "welcome for tick {start_tick} but the sim is at {}",
                            sim.tick()
                        ));
                    }
                    p.sim = Some(sim);
                }
            }
            if let Some(sim) = p.sim.as_mut() {
                random_values(&mut p.rng, &actions, opts.input_change, &mut p.values);
                p.client.set_local(&p.values);
                let due = p.client.frames_due();
                if due == 0 && p.client.next_tick() < host_tick {
                    p.stalls += 1;
                }
                for _ in 0..due {
                    let Some(f) = p.client.next_frame() else { break };
                    apply_frame(sim, &cfg, &f).map_err(|e| e.to_string())?;
                    sim.step();
                    if p.client.should_hash(sim.tick()) {
                        let h = sim.state_hash();
                        p.client.record_hash(sim.tick(), h.as_bytes());
                        hash_checks += 1;
                    }
                }
                p.max_behind = p.max_behind.max(host_tick.saturating_sub(p.client.next_tick()));
            }
            p.client.flush(&mut p.t);
            if p.sim.as_ref().map(|s| s.tick()) != Some(end) {
                done = false;
            }
        }
        if done {
            // A few more steps so the last hashes reach the host.
            for _ in 0..60 {
                net.advance(STEP);
                let now = net.now();
                for p in peers.iter_mut().filter(|p| !p.gone) {
                    p.client.poll(&mut p.t, now);
                    p.client.flush(&mut p.t);
                }
                host.poll(&mut host_t, now);
                host.flush(&mut host_t);
            }
            while let Some(e) = host.event() {
                if let SessionEvent::Desync { slot, tick } = e {
                    desyncs.push((slot, tick));
                }
            }
            break;
        }
    }
    let host_hash = host_sim.state_hash();
    let mut all_equal = true;
    let mut reports = Vec::new();
    for p in peers.iter_mut() {
        let final_hash = if p.gone {
            None
        } else {
            p.sim.as_mut().map(|s| s.state_hash())
        };
        if !p.gone && final_hash.as_deref() != Some(host_hash.as_str()) {
            all_equal = false;
        }
        let s = p.t.stats();
        reports.push(PeerReport {
            slot: p.slot,
            stalls: p.stalls,
            max_behind: p.max_behind,
            bytes_sent: s.bytes_sent,
            bytes_received: s.bytes_received,
            resends: s.resends,
            final_hash,
        });
    }
    all_equal &= desyncs.is_empty();
    Ok(SimulateReport {
        all_equal,
        final_tick: end,
        host_hash,
        desyncs,
        hash_checks,
        actions,
        peers: reports,
        host_bytes_per_tick: host_t.stats().bytes_sent as f64 / opts.ticks.max(1) as f64,
        seconds: net.now() as f64 / 1e6,
    })
}
