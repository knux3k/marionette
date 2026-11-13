//! The engine as the agent runs it: core, the built-in modules (input, physics, audio,
//! script, factory, animation, job schedules) and the operation table, plus whatever a game adds.
//!
//! A game adds its own Rust modules and operations without editing the engine (decision
//! 0062):
//!
//! ```ignore
//! let engine = Engine::builder().module(MyModule).op(op("my_op", "...", false, my_op)).build()?;
//! marionette_cli::main_with(engine) // or Agent::with_engine(root, read_only, engine)
//! ```

use crate::{Agent, Op, codes, write_atomic};
use marionette_anim::AnimModule;
use marionette_audio::AudioModule;
use marionette_core::module::{Module, Modules, registry_with};
use marionette_core::{Diagnostic, Errors, Registry, Sim};
use marionette_factory::FactoryModule;
use marionette_input::{InputEvent, InputModule};
use marionette_physics::PhysicsModule;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, OnceLock};

/// A built engine: its modules in a fixed order, the type registry they make, and the
/// operation table. Cheap to clone (shared).
#[derive(Clone)]
pub struct Engine(Arc<Inner>);

struct Inner {
    modules: Modules,
    registry: Registry,
    ops: Arc<Vec<Op>>,
}

/// One slot of the module list. The script module needs the finished registry (scripts reach
/// every component by name, the game's too), so it is made in [`EngineBuilder::build`].
enum Part {
    Module(Box<dyn Module>),
    #[cfg(feature = "script")]
    Script,
}

/// Collects modules and operations for an [`Engine`]. Modules run in the order added, after
/// the built-in ones; within a [`marionette_core::Stage`] their systems run after the built-in
/// systems of that stage.
pub struct EngineBuilder {
    parts: Vec<Part>,
    ops: Vec<Op>,
}

impl EngineBuilder {
    /// The built-in modules and operations compiled into this build (see the cargo features of
    /// `marionette-agent`).
    pub fn standard() -> Self {
        Self {
            parts: vec![
                Part::Module(Box::new(InputModule)),
                Part::Module(Box::new(PhysicsModule)),
                Part::Module(Box::new(AudioModule)),
                #[cfg(feature = "script")]
                Part::Script,
                Part::Module(Box::new(FactoryModule)),
                Part::Module(Box::new(AnimModule)),
                #[cfg(feature = "ui3d")]
                Part::Module(Box::new(marionette_ui3d::Ui3dModule)),
                #[cfg(feature = "jobs-ts")]
                Part::Module(Box::new(marionette_jobs_ts::JobsModule)),
                #[cfg(feature = "netplay")]
                Part::Module(Box::new(marionette_net::NetModule)),
            ],
            ops: crate::builtin_ops(),
        }
    }

    /// Adds a simulation module (components, resources, systems, snapshot state).
    pub fn module(mut self, module: impl Module) -> Self {
        self.parts.push(Part::Module(Box::new(module)));
        self
    }

    /// Adds one operation (make it with [`crate::op`]).
    pub fn op(mut self, op: Op) -> Self {
        self.ops.push(op);
        self
    }

    /// Adds several operations.
    pub fn ops(mut self, ops: impl IntoIterator<Item = Op>) -> Self {
        self.ops.extend(ops);
        self
    }

    /// Checks names (module names and operation names must be unique; operation names are
    /// `lower_snake_case`, and the part before the first `_` is the group) and builds.
    pub fn build(self) -> Result<Engine, Errors> {
        let mut problems = Vec::new();
        let mut seen = BTreeSet::new();
        for part in &self.parts {
            if let Part::Module(m) = part
                && !seen.insert(m.name())
            {
                problems.push(
                    Diagnostic::error(codes::DUPLICATE, format!("two modules are named `{}`", m.name()))
                        .fix("give each module a unique `Module::name` (it keys snapshot data)"),
                );
            }
        }
        let mut names = BTreeSet::new();
        for op in &self.ops {
            let name = op.info.name;
            let well_formed = name.starts_with(|c: char| c.is_ascii_lowercase())
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !well_formed {
                problems.push(
                    Diagnostic::error(
                        codes::BAD_NAME,
                        format!("operation name `{name}` is not lower_snake_case"),
                    )
                    .fix("name it `group_verb`, for example `orbit_set`"),
                );
            }
            if !names.insert(name) {
                problems.push(
                    Diagnostic::error(codes::DUPLICATE, format!("two operations are named `{name}`"))
                        .fix("rename the game's operation; `marionette ops --brief` lists the engine's"),
                );
            }
        }
        if !problems.is_empty() {
            return Err(Errors(problems));
        }
        let parts = self.parts;
        let modules: Modules = Arc::new_cyclic(|_weak: &std::sync::Weak<Vec<Box<dyn Module>>>| {
            parts
                .into_iter()
                .map(|part| match part {
                    Part::Module(m) => m,
                    #[cfg(feature = "script")]
                    Part::Script => {
                        // A weak handle: the module list owns the script module, so a strong
                        // one would be a reference cycle that never frees.
                        let weak = _weak.clone();
                        Box::new(marionette_script::ScriptModule::new(move || {
                            registry_with(&weak.upgrade().expect("the engine outlives its simulations"))
                        })) as Box<dyn Module>
                    }
                })
                .collect::<Vec<_>>()
        });
        let registry = registry_with(&modules);
        Ok(Engine(Arc::new(Inner {
            modules,
            registry,
            ops: Arc::new(self.ops),
        })))
    }
}

impl Engine {
    /// The built-in engine (everything compiled into this build, nothing a game added).
    pub fn standard() -> Engine {
        static STANDARD: OnceLock<Engine> = OnceLock::new();
        STANDARD
            .get_or_init(|| {
                EngineBuilder::standard()
                    .build()
                    .expect("built-in module and operation names are unique")
            })
            .clone()
    }

    /// Starts from the built-in engine; add a game's modules and operations, then `build`.
    pub fn builder() -> EngineBuilder {
        EngineBuilder::standard()
    }

    /// The modules every simulation of this engine runs with, in a fixed order.
    pub fn modules(&self) -> Modules {
        self.0.modules.clone()
    }

    /// Every component and resource type a scene may contain.
    pub fn registry(&self) -> &Registry {
        &self.0.registry
    }

    pub(crate) fn ops(&self) -> Arc<Vec<Op>> {
        self.0.ops.clone()
    }

    /// Loads scene text. Relative asset paths resolve against `dir`.
    pub fn load_text(&self, text: &str, shown: Option<&str>, seed: u64, dir: &Path) -> Result<Sim, Errors> {
        Sim::with_modules(text, shown, seed, self.modules(), dir.to_path_buf())
    }

    /// Loads a scene file. Relative asset paths resolve against its directory.
    pub fn load_file(&self, path: &Path, shown: &str, seed: u64) -> Result<Sim, Errors> {
        let text = crate::read_text(path, shown)?;
        self.load_text(&text, Some(shown), seed, path.parent().unwrap_or(Path::new("")))
    }
}

/// The built-in engine's modules ([`Engine::standard`]).
pub fn modules() -> Modules {
    Engine::standard().modules()
}

/// The built-in engine's registry ([`Engine::standard`]).
pub fn registry() -> Registry {
    registry_with(&modules())
}

/// Queues an input file (script or recording) and extra events into a fresh simulation.
pub(crate) fn feed(
    agent: &Agent,
    sim: &mut Sim,
    file: Option<&str>,
    events: &[InputEvent],
) -> Result<(), Errors> {
    let mut all = Vec::new();
    if let Some(f) = file {
        all.extend(marionette_input::load_events(&agent.resolve(f)?, f)?);
    }
    all.extend_from_slice(events);
    if !all.is_empty() {
        marionette_input::feed(&mut sim.world, &all)?;
    }
    Ok(())
}

/// Writes the simulation's input recording; returns the number of events.
pub(crate) fn write_recording(agent: &Agent, sim: &Sim, out: &str) -> Result<usize, Errors> {
    let file = marionette_input::recording(&sim.world);
    write_atomic(&agent.resolve(out)?, out, file.to_text().as_bytes())?;
    Ok(file.events.len())
}

/// Plays `sound` events through the default audio device (windowed mode only).
pub use marionette_audio::Player as SoundPlayer;
/// The synth host and the synth crate (decision 0220).
pub use marionette_audio::{SynthHost, synth};

/// Loads a scene file with the built-in engine ([`Engine::load_file`] for a game's engine).
pub fn load_file(path: &Path, shown: &str, seed: u64) -> Result<Sim, Errors> {
    Engine::standard().load_file(path, shown, seed)
}

/// Queues one live input change for `tick`: a device input (`key:KeyW`) or, with
/// `direct`, an action set directly (a UI button).
pub fn queue_live(sim: &mut Sim, tick: u64, name: &str, direct: bool, value: f32) -> Result<(), Errors> {
    let event = InputEvent {
        tick,
        input: (!direct).then(|| name.to_string()),
        action: direct.then(|| name.to_string()),
        value,
    };
    marionette_input::feed(&mut sim.world, &[event])
}

/// Queues the events of an input file (script or recording) into a fresh simulation.
pub fn feed_file(sim: &mut Sim, path: &std::path::Path, shown: &str) -> Result<(), Errors> {
    let events = marionette_input::load_events(path, shown)?;
    marionette_input::feed(&mut sim.world, &events)
}

/// Writes the simulation's input recording to `path` (atomically); returns the event count.
pub fn save_recording(sim: &Sim, path: &std::path::Path, shown: &str) -> Result<usize, Errors> {
    let file = marionette_input::recording(&sim.world);
    write_atomic(path, shown, file.to_text().as_bytes())?;
    Ok(file.events.len())
}
