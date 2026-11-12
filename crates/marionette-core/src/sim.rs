//! Fixed-step simulation: a world, a schedule, a seeded RNG and a tick counter.
//!
//! Each [`Sim::step`] runs every system once with a fixed `dt`; nothing reads the wall
//! clock. Systems run on one thread in a fixed order.

use crate::builtin::{SimClock, add_builtin_systems};
use crate::error::Result;
use crate::registry::Registry;
use crate::rng::SimRng;
use crate::scene;
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::SingleThreadedExecutor;
use sha2::{Digest, Sha256};

pub struct Sim {
    pub world: World,
    schedule: Schedule,
    registry: Registry,
}

impl Sim {
    /// Loads scene text and prepares a simulation seeded with `seed`.
    pub fn from_text(text: &str, file: Option<&str>, registry: Registry, seed: u64) -> Result<Self> {
        let mut world = World::new();
        scene::load(text, file, &registry, &mut world)?;
        world.insert_resource(SimRng::new(seed));
        world.insert_resource(SimClock::default());
        Ok(Self {
            world,
            schedule: new_schedule(),
            registry,
        })
    }

    /// Replaces the world's contents with `text` (a scene), keeping the tick counter and
    /// the RNG state, so a live session can be edited and keep running deterministically.
    /// On error the current world is left unchanged.
    pub fn reload(&mut self, text: &str) -> Result<()> {
        let mut world = World::new();
        scene::load(text, None, &self.registry, &mut world)?;
        world.insert_resource(*self.world.resource::<SimRng>());
        world.insert_resource(*self.world.resource::<SimClock>());
        self.world = world;
        // Systems cache state per world, so a new world needs a fresh schedule.
        self.schedule = new_schedule();
        Ok(())
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn step(&mut self) {
        self.schedule.run(&mut self.world);
    }

    pub fn run(&mut self, ticks: u64) {
        for _ in 0..ticks {
            self.step();
        }
    }

    pub fn tick(&self) -> u64 {
        self.world.resource::<SimClock>().tick
    }

    /// Current state as a canonical scene.
    pub fn snapshot(&mut self) -> scene::Scene {
        scene::extract(&mut self.world, &self.registry)
    }

    /// SHA-256 (hex) of all simulation state: the canonical scene text plus the tick
    /// counter and RNG state.
    pub fn state_hash(&mut self) -> String {
        let text = self.snapshot().to_text();
        let mut h = Sha256::new();
        h.update(text.as_bytes());
        h.update(
            format!(
                "tick={}\nrng={}\n",
                self.tick(),
                self.world.resource::<SimRng>().state()
            )
            .as_bytes(),
        );
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// The simulation schedule: built-in systems in a fixed order, on one thread.
fn new_schedule() -> Schedule {
    let mut schedule = Schedule::default();
    schedule.set_executor(SingleThreadedExecutor::new());
    add_builtin_systems(&mut schedule);
    schedule
}
