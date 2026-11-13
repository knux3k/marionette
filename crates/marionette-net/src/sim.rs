//! Running lockstep frames on a [`Sim`]: inputs become per-player actions, the roster
//! becomes the `NetPlayers` resource, and snapshots travel as bytes.
//!
//! Player `n`'s action `jump` is the action `p<n>.jump` in the sim (scripts read
//! `engine.action("p1.jump")`). Each peer reads its own devices through its local action
//! map and hands the values to the session; the sim only ever sees the stamped frames.

use crate::lockstep::{Frame, LockstepConfig, RosterEvent};
use crate::wire::{Reader, Writer};
use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_core::module::{Module, SimSnapshot};
use marionette_core::{Registry, Sim};
use marionette_input::InputEvent;

/// The players in a networked session, by slot (0 is the host). Part of the state hash, so a
/// peer that disagrees about who is playing is caught as a desync.
#[derive(Resource, Reflect, Debug, Clone, PartialEq, Default)]
pub struct NetPlayers {
    /// Occupied slots, ascending.
    pub slots: Vec<u32>,
}

/// Registers [`NetPlayers`]. Add it to the engine of a networked game
/// (`Engine::builder().module(NetModule)`).
pub struct NetModule;

impl Module for NetModule {
    fn name(&self) -> &'static str {
        "net"
    }
    fn register(&self, r: &mut Registry) {
        r.register_resource::<NetPlayers>();
    }
    fn add_systems(&self, _schedule: &mut Schedule) {}
}

/// The sim name of player `slot`'s `action`.
pub fn player_action(slot: u8, action: &str) -> String {
    format!("p{slot}.{action}")
}

/// Queues `frame` on `sim` for its current tick. Call before [`Sim::step`]. The frame's tick
/// must be the sim's tick.
pub fn apply_frame(sim: &mut Sim, cfg: &LockstepConfig, frame: &Frame) -> marionette_core::Result<()> {
    debug_assert_eq!(frame.tick, sim.tick(), "frame for another tick");
    let tick = sim.tick();
    let mut events = Vec::new();
    let mut players = sim
        .world
        .get_resource::<NetPlayers>()
        .cloned()
        .unwrap_or_default();
    for r in &frame.roster {
        match *r {
            RosterEvent::Join { slot, .. } => {
                if !players.slots.contains(&(slot as u32)) {
                    players.slots.push(slot as u32);
                }
            }
            RosterEvent::Leave { slot } => {
                players.slots.retain(|s| *s != slot as u32);
                // A player who leaves lets go of everything.
                for a in &cfg.actions {
                    events.push(InputEvent {
                        tick,
                        input: None,
                        action: Some(player_action(slot, a)),
                        value: 0.0,
                    });
                }
            }
        }
    }
    players.slots.sort_unstable();
    for &(slot, a, v) in &frame.inputs {
        if let Some(name) = cfg.actions.get(a as usize) {
            events.push(InputEvent {
                tick,
                input: None,
                action: Some(player_action(slot, name)),
                value: v,
            });
        }
    }
    if !frame.roster.is_empty() {
        sim.world.insert_resource(players);
    }
    if !events.is_empty() {
        marionette_input::feed(&mut sim.world, &events)?;
    }
    Ok(())
}

/// A snapshot of `sim` as bytes (for late joins and resyncs).
pub fn encode_snapshot(sim: &mut Sim) -> Vec<u8> {
    let s = sim.save_state();
    let mut w = Writer::new();
    w.u64(s.tick).u64(s.rng).bytes(s.scene.as_bytes());
    w.u32(s.modules.len() as u32);
    for (name, b) in &s.modules {
        w.bytes(name.as_bytes()).bytes(b);
    }
    w.buf
}

/// Restores bytes from [`encode_snapshot`] into `sim` (same engine and scene). Returns false
/// when the bytes cannot be read or restored.
pub fn restore_snapshot(sim: &mut Sim, bytes: &[u8]) -> bool {
    let mut r = Reader::new(bytes);
    let read = |r: &mut Reader| -> Option<SimSnapshot> {
        let tick = r.u64()?;
        let rng = r.u64()?;
        let scene = String::from_utf8(r.bytes()?.to_vec()).ok()?;
        let mut modules = std::collections::BTreeMap::new();
        for _ in 0..r.u32()? {
            let name = std::str::from_utf8(r.bytes()?).ok()?.to_string();
            let data = r.bytes()?.to_vec();
            // Module names are `&'static str`; take the matching one from this sim.
            if let Some(m) = sim.modules().iter().find(|m| m.name() == name) {
                modules.insert(m.name(), data);
            }
        }
        Some(SimSnapshot {
            scene,
            tick,
            rng,
            modules,
        })
    };
    match read(&mut r) {
        Some(s) => sim.restore(&s).is_ok(),
        None => false,
    }
}
