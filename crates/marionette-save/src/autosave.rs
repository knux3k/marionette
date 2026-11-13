//! Autosave: every `SaveFormat.autosave_every` ticks and whenever game code asks.
//!
//! Game code asks by recording a `save_request` event (scripts: `engine.emit("save_request")`
//! or `engine.emit("save_request", JSON.stringify({ slot: "checkpoint", label: "Gate" }))`;
//! Rust systems: push the same event to the `EventLog`). Ask at safe moments (a checkpoint,
//! a level change), not mid-action. Events are records, not state, so asking changes no
//! state hash. A request without a slot is an autosave.
//!
//! Autosaves rotate through `auto-0 .. auto-(n-1)`, overwriting the oldest, so an autosave
//! never replaces a slot the player saved by hand. The runner (window loop or a game's own
//! binary) calls [`Autosaver::after_step`] after each [`Sim::step`]; only the copy happens on
//! that thread.

use crate::store::{SaveStore, check_slot};
use crate::writer::SaveWriter;
use crate::{Pending, SaveInfo, format_of};
use marionette_core::{Errors, Sim};

/// Event kind that asks for a save.
pub const REQUEST: &str = "save_request";

pub struct Autosaver {
    next_due: Option<u64>,
    next_slot: u32,
    events_seen: u64,
}

/// Slot name of autosave `i`.
pub fn auto_slot(i: u32) -> String {
    format!("auto-{i}")
}

impl Autosaver {
    /// Starts after `sim`'s current tick; picks the oldest (or a missing) autosave slot
    /// in `store` to write first.
    pub fn new(sim: &Sim, store: &SaveStore) -> Self {
        let f = format_of(&sim.world);
        let slots = f.autosave_slots.max(1);
        let next_slot = (0..slots)
            .min_by_key(|i| {
                store
                    .info(&auto_slot(*i))
                    .header
                    .map_or((0, 0), |h| (1, h.saved_unix))
            })
            .unwrap_or(0);
        Self {
            next_due: (f.autosave_every > 0).then(|| sim.tick() + f.autosave_every),
            next_slot,
            events_seen: sim.events().end(),
        }
    }

    /// Queues the saves due after this tick: one per requested slot, plus one autosave if
    /// the interval came round or a request named no slot. Returns the slots queued.
    pub fn after_step(&mut self, sim: &mut Sim, writer: &SaveWriter) -> Result<Vec<String>, Errors> {
        let mut wanted: Vec<(String, String)> = Vec::new();
        let mut auto: Option<String> = None;
        for e in sim.events().since(self.events_seen) {
            if e.kind != REQUEST {
                continue;
            }
            let label = e.detail["label"].as_str().unwrap_or_default().to_string();
            match e.detail["slot"].as_str() {
                Some(slot) if !wanted.iter().any(|(s, _)| s == slot) => {
                    check_slot(slot)?;
                    wanted.push((slot.to_string(), label));
                }
                Some(_) => {}
                None => auto = Some(label),
            }
        }
        self.events_seen = sim.events().end();
        let f = format_of(&sim.world);
        let tick = sim.tick();
        if auto.is_none() && self.next_due.is_some_and(|due| tick >= due) {
            auto = Some(String::new());
        }
        if let Some(label) = auto {
            let slots = f.autosave_slots.max(1);
            let slot = auto_slot(self.next_slot % slots);
            self.next_slot = (self.next_slot + 1) % slots;
            wanted.push((slot, label));
            self.next_due = (f.autosave_every > 0).then(|| tick + f.autosave_every);
        }
        // One copy per slot (two slots in one tick is rare; the copy is the cheap part).
        let mut queued = Vec::new();
        for (slot, label) in wanted {
            let info = SaveInfo {
                label,
                meta: serde_json::Value::Null,
            };
            writer.submit(&slot, Pending::capture(sim, info))?;
            queued.push(slot);
        }
        Ok(queued)
    }
}
