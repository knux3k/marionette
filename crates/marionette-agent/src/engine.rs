//! The engine as the agent runs it: core plus the input, physics, audio, script and factory
//! modules.

use crate::{Agent, write_atomic};
use marionette_audio::AudioModule;
use marionette_core::module::{Modules, registry_with};
use marionette_core::{Errors, Registry, Sim};
use marionette_factory::FactoryModule;
use marionette_input::{InputEvent, InputModule};
use marionette_physics::PhysicsModule;
use marionette_script::ScriptModule;
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
                Box::new(ScriptModule::new(registry)),
                Box::new(FactoryModule),
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

/// Plays `sound` events through the default audio device (windowed mode only).
pub use marionette_audio::Player as SoundPlayer;

/// Loads a scene file with every engine module (for the window and the benchmark, which
/// run outside the operation table). Relative asset paths resolve against its directory.
pub fn load_file(path: &std::path::Path, shown: &str, seed: u64) -> Result<Sim, Errors> {
    let text = crate::read_text(path, shown)?;
    let dir = path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    Sim::with_modules(&text, Some(shown), seed, modules(), dir)
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
