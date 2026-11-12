//! Example game code: a Rust module and an operation of the game's own, plugged into the
//! engine without editing it (decision 0062). Copy this crate to start a game with Rust code.
//!
//! - [`SpringModule`] registers the [`Spring`] component and a `Stage::Logic` system.
//! - [`ops`] adds the `spring_set` operation (CLI `op` and MCP tool alike).
//! - [`engine`] puts both on top of the built-in engine; `src/main.rs` runs it as a
//!   `marionette`-style binary.

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_agent::{Agent, Engine, Op, codes, op};
use marionette_core::builtin::{FixedStep, Position, Velocity};
use marionette_core::module::Stage;
use marionette_core::scene::SceneId;
use marionette_core::spatial::Vec3;
use marionette_core::{Diagnostic, Errors, Module, Registry};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Pulls the entity's `Velocity` toward `anchor` like a damped spring. Needs `Position` and
/// `Velocity`.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    /// The point the entity is pulled to, in meters.
    pub anchor: Vec3,
    /// Pull per meter of distance, per second squared.
    pub stiffness: f32,
    /// Velocity lost per second, as a fraction (0 keeps bouncing).
    pub damping: f32,
}

/// The game's module. Its name keys its snapshot data and must be unique in the engine.
pub struct SpringModule;

impl Module for SpringModule {
    fn name(&self) -> &'static str {
        "spring"
    }

    fn register(&self, r: &mut Registry) {
        r.register_component::<Spring>();
    }

    fn add_systems(&self, schedule: &mut Schedule) {
        schedule.add_systems(spring.in_set(Stage::Logic));
    }
}

/// Plain `+ - * /` only, so the result is the same on every platform (ARCHITECTURE.md,
/// determinism). Each entity only reads its own components, so query order does not matter.
fn spring(step: Option<Res<FixedStep>>, mut q: Query<(&Spring, &Position, &mut Velocity)>) {
    let dt = step.map_or(1.0 / 60.0, |s| s.dt);
    for (s, p, mut v) in &mut q {
        v.x += ((s.anchor.x - p.x) * s.stiffness - v.x * s.damping) * dt;
        v.y += ((s.anchor.y - p.y) * s.stiffness - v.y * s.damping) * dt;
        v.z += ((s.anchor.z - p.z) * s.stiffness - v.z * s.damping) * dt;
    }
}

/// Input of `spring_set`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetInput {
    /// A live session (`session_start`).
    pub session: String,
    /// Scene id of an entity with a `Spring`.
    pub entity: u64,
    /// New stiffness (unchanged when absent).
    pub stiffness: Option<f32>,
    /// New damping (unchanged when absent).
    pub damping: Option<f32>,
}

/// Output of `spring_set`: the spring after the change.
#[derive(Serialize, JsonSchema)]
pub struct SetOutput {
    pub entity: u64,
    pub stiffness: f32,
    pub damping: f32,
}

fn set(agent: &mut Agent, input: SetInput) -> Result<SetOutput, Errors> {
    let sim = agent.session_sim(&input.session)?;
    let mut q = sim.world.query::<(&SceneId, &mut Spring)>();
    let Some((_, mut s)) = q.iter_mut(&mut sim.world).find(|(id, _)| id.0 == input.entity) else {
        return Err(
            Diagnostic::error(codes::NO_ENTITY, format!("entity {} has no Spring", input.entity))
                .fix("pick an entity with a Spring component (observe_tree lists them)")
                .into(),
        );
    };
    if let Some(k) = input.stiffness {
        s.stiffness = k;
    }
    if let Some(d) = input.damping {
        s.damping = d;
    }
    Ok(SetOutput {
        entity: input.entity,
        stiffness: s.stiffness,
        damping: s.damping,
    })
}

/// The game's operations. Name them `group_verb`; the group (`spring`) is how MCP clients
/// pick tools.
pub fn ops() -> Vec<Op> {
    vec![op(
        "spring_set",
        "Changes the stiffness or damping of an entity's Spring in a live session.",
        true,
        set,
    )]
}

/// The built-in engine plus this game's module and operations.
pub fn engine() -> Result<Engine, Errors> {
    Engine::builder().module(SpringModule).ops(ops()).build()
}
