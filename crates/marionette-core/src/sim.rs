//! Fixed-step simulation: a world, a schedule, a seeded RNG and a tick counter.
//!
//! Each [`Sim::step`] runs every system once with a fixed `dt`; nothing reads the wall
//! clock. Systems run on one thread in a fixed order of [`Stage`]s.

use crate::builtin::{SimClock, add_builtin_systems};
use crate::error::Result;
use crate::module::{EventLog, Modules, SceneDir, SimSnapshot, Stage, registry_with};
use crate::registry::Registry;
use crate::rng::SimRng;
use crate::scene;
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::SingleThreadedExecutor;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;

pub struct Sim {
    pub world: World,
    schedule: Schedule,
    registry: Registry,
    modules: Modules,
    scene_dir: PathBuf,
}

impl Sim {
    /// Loads scene text with only the built-in systems, seeded with `seed`.
    pub fn from_text(text: &str, file: Option<&str>, registry: Registry, seed: u64) -> Result<Self> {
        let mut sim = Self {
            world: World::new(),
            schedule: Schedule::default(),
            registry,
            modules: Arc::new(Vec::new()),
            scene_dir: PathBuf::new(),
        };
        sim.world = sim.new_world(text, file, SimRng::new(seed), SimClock::default())?;
        sim.schedule = sim.new_schedule();
        Ok(sim)
    }

    /// Loads scene text with `modules` (input, physics, audio...). Relative asset paths
    /// resolve against `scene_dir`.
    pub fn with_modules(
        text: &str,
        file: Option<&str>,
        seed: u64,
        modules: Modules,
        scene_dir: PathBuf,
    ) -> Result<Self> {
        let mut sim = Self {
            world: World::new(),
            schedule: Schedule::default(),
            registry: registry_with(&modules),
            modules,
            scene_dir,
        };
        sim.world = sim.new_world(text, file, SimRng::new(seed), SimClock::default())?;
        sim.schedule = sim.new_schedule();
        Ok(sim)
    }

    fn new_world(&self, text: &str, file: Option<&str>, rng: SimRng, clock: SimClock) -> Result<World> {
        let mut world = World::new();
        scene::load(text, file, &self.registry, &mut world)?;
        world.insert_resource(rng);
        world.insert_resource(clock);
        world.insert_resource(SceneDir(self.scene_dir.clone()));
        world.insert_resource(EventLog::default());
        for m in self.modules.iter() {
            m.init(&mut world).map_err(|e| match file {
                Some(f) => e.with_file(f),
                None => e,
            })?;
        }
        Ok(world)
    }

    /// The schedule: stages in order, built-in systems, then each module's systems.
    fn new_schedule(&self) -> Schedule {
        let mut schedule = Schedule::default();
        schedule.set_executor(SingleThreadedExecutor::new());
        schedule.configure_sets(
            (
                Stage::Input,
                Stage::Script,
                Stage::Logic,
                Stage::Physics,
                Stage::Effects,
                Stage::End,
            )
                .chain(),
        );
        add_builtin_systems(&mut schedule);
        for m in self.modules.iter() {
            m.add_systems(&mut schedule);
        }
        schedule
    }

    /// Replaces the world's contents with `text` (a scene), keeping the tick counter, the
    /// RNG state and the event log, so a live session can be edited and keep running
    /// deterministically. On error the current world is left unchanged.
    pub fn reload(&mut self, text: &str) -> Result<()> {
        let rng = *self.world.resource::<SimRng>();
        let clock = *self.world.resource::<SimClock>();
        let mut world = self.new_world(text, None, rng, clock)?;
        world.insert_resource(self.world.resource::<EventLog>().clone());
        self.world = world;
        // Systems cache state per world, so a new world needs a fresh schedule.
        self.schedule = self.new_schedule();
        Ok(())
    }

    /// A complete copy of the current state (see [`Sim::restore`]).
    pub fn save_state(&mut self) -> SimSnapshot {
        let modules = self
            .modules
            .iter()
            .filter_map(|m| m.save(&self.world).map(|b| (m.name(), b)))
            .collect();
        SimSnapshot {
            scene: self.snapshot().to_text(),
            tick: self.tick(),
            rng: self.world.resource::<SimRng>().state(),
            modules,
        }
    }

    /// Returns to a snapshot taken with [`Sim::save_state`]. Continuing from here gives the
    /// same results as continuing from the moment the snapshot was taken.
    pub fn restore(&mut self, snap: &SimSnapshot) -> Result<()> {
        let mut world = self.new_world(
            &snap.scene,
            None,
            SimRng::new(snap.rng),
            SimClock { tick: snap.tick },
        )?;
        for m in self.modules.iter() {
            if let Some(bytes) = snap.modules.get(m.name()) {
                m.load(&mut world, bytes)?;
            }
        }
        world.insert_resource(self.world.resource::<EventLog>().clone());
        self.world = world;
        self.schedule = self.new_schedule();
        Ok(())
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn modules(&self) -> &Modules {
        &self.modules
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

    /// Events recorded so far (collisions, jumps, sounds...).
    pub fn events(&self) -> &EventLog {
        self.world.resource::<EventLog>()
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
