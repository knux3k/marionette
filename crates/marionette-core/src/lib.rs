//! Marionette engine core: ECS world, schema registry, text scene format and validator,
//! fixed-step loop with seeded RNG, and errors as data. No graphics, no network.
//!
//! The ECS is [`bevy_ecs`] used standalone; type descriptions come from [`bevy_reflect`].

pub mod actions;
pub mod builtin;
pub mod error;
pub mod jobdata;
pub mod json_span;
pub mod mesh;
pub mod module;
pub mod person_only;
pub mod registry;
pub mod rng;
pub mod scene;
pub mod schema;
pub mod sim;
pub mod spatial;
pub mod ui;

pub use error::{Diagnostic, Errors, Result, Severity};
pub use module::{EngineEvent, EventLog, Module, Modules, SceneDir, SimSnapshot, Stage};
pub use registry::{Kind, Registry};
pub use scene::Scene;
pub use sim::Sim;

/// A registry with every built-in type.
pub fn default_registry() -> Registry {
    let mut r = Registry::new();
    builtin::register_builtins(&mut r);
    r
}
