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
use std::collections::BTreeMap;
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

/// A ray hit: the entity, distance along the ray, hit point and surface normal.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct RayHit {
    pub entity: u64,
    pub distance: f32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

/// Which colliders a ray may hit.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RayFilter {
    /// Skip this entity's collider (the shooter itself).
    pub exclude: Option<u64>,
    /// Skip sensors (trigger volumes).
    pub skip_sensors: bool,
}

/// A ray query offered by a module (physics) to code that must not depend on it (scripts).
/// The module inserts this resource in its `init`; it is not simulation state. Arguments:
/// origin, direction (need not be unit length), maximum distance, filter.
#[derive(Resource, Clone, Copy)]
pub struct RayCaster(pub RayFn);

/// The signature of a [`RayCaster`]: world, origin, direction, maximum distance, filter.
pub type RayFn = fn(&World, [f32; 3], [f32; 3], f32, RayFilter) -> Option<RayHit>;

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

/// All events so far, in the order they happened.
#[derive(Resource, Debug, Clone, Default)]
pub struct EventLog {
    pub events: Vec<EngineEvent>,
}

impl EventLog {
    pub fn push(
        &mut self,
        tick: u64,
        kind: &str,
        entity: Option<u64>,
        other: Option<u64>,
        detail: serde_json::Value,
    ) {
        self.events.push(EngineEvent {
            tick,
            kind: kind.into(),
            entity,
            other,
            detail,
        });
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
