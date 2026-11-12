//! The engine as the agent runs it: core plus the input, physics and audio modules.

use crate::{Agent, write_atomic};
use marionette_audio::AudioModule;
use marionette_core::module::{Modules, registry_with};
use marionette_core::{Errors, Registry, Sim};
use marionette_input::{InputEvent, InputModule};
use marionette_physics::PhysicsModule;
use std::sync::{Arc, OnceLock};

/// The modules every simulation runs with, in a fixed order.
pub fn modules() -> Modules {
    static MODULES: OnceLock<Modules> = OnceLock::new();
    MODULES
        .get_or_init(|| {
            Arc::new(vec![
                Box::new(InputModule),
                Box::new(PhysicsModule),
                Box::new(AudioModule),
            ])
        })
        .clone()
}

/// Every component and resource type a scene may contain.
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
