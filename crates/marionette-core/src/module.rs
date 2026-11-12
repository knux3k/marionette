//! Engine modules: how optional parts (input, physics, audio) plug into the simulation
//! without the core depending on them.
//!
//! A module registers its component and resource types, adds systems to one of the fixed
//! [`Stage`]s, builds its runtime state after a scene is loaded, and can save and restore
//! that state for snapshots. Stages run in order every tick, on one thread.

use crate::error::Result;
use crate::registry::Registry;
use bevy_ecs::prelude::*;
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

/// The fixed order of work inside one tick.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Turn input events into action state.
    Input,
    /// Game scripts (`marionette-script`): read actions, change components.
    Script,
    /// Game logic and built-in systems (random impulses, follow, integrate).
    Logic,
    /// Physics: character controllers, the physics step, writing results back.
    Physics,
    /// Effects that read the finished tick: sounds, events.
    Effects,
    /// Bookkeeping (the tick counter).
    End,
}

pub trait Module: Send + Sync + 'static {
    /// Stable name, used as the key of this module's snapshot data.
    fn name(&self) -> &'static str;
    /// Register component and resource types (they become part of the schema).
    fn register(&self, _registry: &mut Registry) {}
    /// Add systems to the schedule, each in a [`Stage`].
    fn add_systems(&self, _schedule: &mut Schedule) {}
    /// Called after a scene is loaded into `world` (also after reload and restore). Build
    /// runtime state from components here. [`SceneDir`] is available.
    fn init(&self, _world: &mut World) -> Result<()> {
        Ok(())
    }
    /// Runtime state that components do not capture (for snapshots).
    fn save(&self, _world: &World) -> Option<Vec<u8>> {
        None
    }
    /// Restores what [`Module::save`] produced. Called after [`Module::init`].
    fn load(&self, _world: &mut World, _bytes: &[u8]) -> Result<()> {
        Ok(())
    }
}

/// The set of modules a simulation runs with.
pub type Modules = Arc<Vec<Box<dyn Module>>>;

/// The directory of the scene file; relative asset paths (models, input maps, sounds)
/// resolve against it.
#[derive(Resource, Debug, Clone, Default)]
pub struct SceneDir(pub PathBuf);

/// Something that happened during a tick (a collision, a jump, a sound). Not simulation
/// state: it is a record for observation and tests, and is not part of the state hash.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EngineEvent {
    pub tick: u64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<u64>,
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub detail: serde_json::Value,
}

/// Recent events, oldest first. Bounded so long runs keep steady memory (decision 0029): a
/// ring of at most `EventLog::KEEP` events; each push past that drops the oldest event and
/// counts it in `dropped`. The trim depends only on the event count, so it is
/// deterministic. Readers that keep a position use absolute positions: event `i` of
/// `events` has absolute position `dropped + i`.
#[derive(Resource, Debug, Clone, Default)]
pub struct EventLog {
    pub events: VecDeque<EngineEvent>,
    /// Events removed from the front so far.
    pub dropped: u64,
}

impl EventLog {
    /// Events kept. Far more than one tick produces; about 10 MB of typical events.
    pub const KEEP: usize = 16_384;

    pub fn push(
        &mut self,
        tick: u64,
        kind: &str,
        entity: Option<u64>,
        other: Option<u64>,
        detail: serde_json::Value,
    ) {
        if self.events.len() >= Self::KEEP {
            self.events.pop_front();
            self.dropped += 1;
        }
        self.events.push_back(EngineEvent {
            tick,
            kind: kind.into(),
            entity,
            other,
            detail,
        });
    }

    /// Absolute position after the last event (events ever pushed).
    pub fn end(&self) -> u64 {
        self.dropped + self.events.len() as u64
    }

    /// Events from absolute position `from` on (all held events if `from` was dropped).
    pub fn since(&self, from: u64) -> impl Iterator<Item = &EngineEvent> {
        let skip = from.saturating_sub(self.dropped).min(self.events.len() as u64) as usize;
        self.events.range(skip..)
    }

    /// Events of one tick, found from the end (cheap for the current or last tick).
    pub fn of_tick(&self, tick: u64) -> impl Iterator<Item = &EngineEvent> {
        let start = self
            .events
            .iter()
            .rposition(|e| e.tick < tick)
            .map_or(0, |i| i + 1);
        self.events.range(start..).filter(move |e| e.tick == tick)
    }
}

/// A complete, restorable copy of a simulation's state.
#[derive(Debug, Clone)]
pub struct SimSnapshot {
    /// Canonical scene text of the world (components and registered resources).
    pub scene: String,
    pub tick: u64,
    pub rng: u64,
    /// Per-module runtime state, keyed by module name.
    pub modules: BTreeMap<&'static str, Vec<u8>>,
}

/// A registry with the built-in types plus every module's types.
pub fn registry_with(modules: &[Box<dyn Module>]) -> Registry {
    let mut r = crate::default_registry();
    for m in modules {
        m.register(&mut r);
    }
    r
}

#[cfg(test)]
mod event_log_tests {
    use super::EventLog;

    #[test]
    fn the_event_log_is_bounded_and_positions_stay_absolute() {
        let mut log = EventLog::default();
        let n = 5 * EventLog::KEEP as u64 + 17;
        for i in 0..n {
            log.push(i / 4, "e", Some(i), None, serde_json::Value::Null);
            assert!(log.events.len() <= EventLog::KEEP);
        }
        assert_eq!(log.end(), n);
        assert_eq!(log.dropped + log.events.len() as u64, n);
        // Absolute positions: the last three events, and an old position falls back to all.
        let tail: Vec<u64> = log.since(n - 3).map(|e| e.entity.unwrap()).collect();
        assert_eq!(tail, [n - 3, n - 2, n - 1]);
        assert_eq!(log.since(0).count(), log.events.len());
        assert_eq!(log.since(n + 10).count(), 0);
        // One tick's events, found from the end.
        let last_tick = (n - 1) / 4;
        let ids: Vec<u64> = log.of_tick(last_tick).map(|e| e.entity.unwrap()).collect();
        assert_eq!(ids, ((last_tick * 4)..n).collect::<Vec<_>>());
    }
}
