//! Sounds a game makes in code, and synth commands from game systems (decision 0221).
//!
//! - [`SoundBank`]: named mono sample buffers. A game fills it (at load, or on a worker and then
//!   inserted) and plays them as `bank:<name>` from an `AudioSource` or a `sound` event, exactly
//!   like a file or a built-in sound. The windowed [`crate::Player`] turns each new buffer into
//!   kira data once; headless runs only log. Bounded: [`BANK_SAMPLES`] in all.
//! - [`SynthOutbox`]: synth [`Command`]s queued by game systems (which have only the world, not
//!   the device). The windowed player starts a synth host the first time the outbox has
//!   something in it and forwards every command each frame. Bounded: [`OUTBOX`] waiting.
//!
//! Ownership: the bank owns each buffer as an `Arc<[f32]>` (one heap block, shared read-only
//! with the player's converted copy until the name is replaced). The outbox owns its commands
//! until the player moves them into the lock-free queue of the audio thread.

use bevy_ecs::prelude::*;
use marionette_synth::Command;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Samples held by a [`SoundBank`] at most (64 MB of `f32`), so a runaway game cannot grow it
/// without bound (MEMORY-AND-CRASHES.md).
pub const BANK_SAMPLES: usize = 16 * 1024 * 1024;
/// Commands waiting in a [`SynthOutbox`] at most; past it new ones are dropped and counted.
pub const OUTBOX: usize = 512;

/// One sound of the bank: mono samples at `rate` hertz.
#[derive(Debug, Clone)]
pub struct BankSound {
    pub rate: u32,
    pub samples: Arc<[f32]>,
}

/// Named sounds made by game code, played as `bank:<name>` (decision 0221).
#[derive(Resource, Debug, Default)]
pub struct SoundBank {
    sounds: BTreeMap<String, BankSound>,
    total: usize,
    version: u64,
}

impl SoundBank {
    /// Adds or replaces `name`. Refused (nothing changes) when the bank would pass
    /// [`BANK_SAMPLES`], the name is empty or the rate is zero.
    pub fn insert(&mut self, name: &str, rate: u32, samples: Arc<[f32]>) -> Result<(), String> {
        if name.is_empty() || rate == 0 {
            return Err("a bank sound needs a name and a sample rate".into());
        }
        let old = self.sounds.get(name).map_or(0, |s| s.samples.len());
        let total = self.total - old + samples.len();
        if total > BANK_SAMPLES {
            return Err(format!(
                "the sound bank is full ({BANK_SAMPLES} samples); `{name}` was not added"
            ));
        }
        self.total = total;
        self.sounds.insert(name.to_string(), BankSound { rate, samples });
        self.version += 1;
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let Some(s) = self.sounds.remove(name) else {
            return false;
        };
        self.total -= s.samples.len();
        self.version += 1;
        true
    }

    pub fn get(&self, name: &str) -> Option<&BankSound> {
        self.sounds.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.sounds.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.sounds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sounds.is_empty()
    }

    /// Samples held in all.
    pub fn samples(&self) -> usize {
        self.total
    }

    /// Changes on every insert or remove (the player converts only when it moves).
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &BankSound)> {
        self.sounds.iter().map(|(n, s)| (n.as_str(), s))
    }
}

/// Synth commands from game systems, played by the windowed player's synth (decision 0221).
/// Headless runs have no device: commands wait (bounded) and are counted as dropped past
/// [`OUTBOX`]; check [`SynthOutbox::live`] to skip making sound nobody hears.
#[derive(Resource)]
pub struct SynthOutbox {
    /// A command can hold a generator, which is `Send` but not `Sync`; a resource must be both.
    /// Only `&mut self` methods touch it (`Mutex::get_mut`), so it is never actually locked.
    commands: Mutex<Vec<Command>>,
    waiting: usize,
    /// Polyphony of the synth the player starts (read once, at start).
    pub voices: usize,
    /// Commands lost because the outbox was full.
    pub dropped: u64,
    /// Commands the synth refused (its queue was full).
    pub refused: u64,
    /// Set by the player once its synth runs: someone is listening.
    pub live: bool,
    /// The synth's sample rate and timeline position after the last frame (0 until live), for
    /// scheduling `at` frames ahead.
    pub rate: u32,
    pub now: u64,
}

impl Default for SynthOutbox {
    fn default() -> Self {
        Self {
            commands: Mutex::new(Vec::new()),
            waiting: 0,
            voices: 32,
            dropped: 0,
            refused: 0,
            live: false,
            rate: 0,
            now: 0,
        }
    }
}

impl SynthOutbox {
    /// Queues a command; false (and counted) when the outbox is full.
    pub fn push(&mut self, c: Command) -> bool {
        if self.waiting >= OUTBOX {
            self.dropped += 1;
            return false;
        }
        self.list().push(c);
        self.waiting += 1;
        true
    }

    pub fn len(&self) -> usize {
        self.waiting
    }

    pub fn is_empty(&self) -> bool {
        self.waiting == 0
    }

    /// Takes every waiting command, oldest first.
    pub fn take(&mut self) -> Vec<Command> {
        self.waiting = 0;
        std::mem::take(self.list())
    }

    fn list(&mut self) -> &mut Vec<Command> {
        // Poisoning needs a panic while locked, and nothing here locks; take the data anyway.
        self.commands.get_mut().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bank_counts_and_refuses_past_its_bound() {
        let mut b = SoundBank::default();
        b.insert("a", 44_100, Arc::from(vec![0.0; 100])).unwrap();
        b.insert("a", 44_100, Arc::from(vec![0.0; 50])).unwrap();
        assert_eq!((b.len(), b.samples(), b.version()), (1, 50, 2));
        let big: Arc<[f32]> = Arc::from(vec![0.0; BANK_SAMPLES]);
        assert!(b.insert("b", 44_100, big).is_err());
        assert_eq!(b.samples(), 50);
        assert!(b.insert("", 44_100, Arc::from(vec![0.0; 1])).is_err());
        assert!(b.remove("a"));
        assert_eq!((b.len(), b.samples()), (0, 0));
    }

    #[test]
    fn outbox_is_bounded() {
        let mut o = SynthOutbox::default();
        for _ in 0..OUTBOX + 3 {
            o.push(Command::AllOff { at: 0 });
        }
        assert_eq!((o.len(), o.dropped), (OUTBOX, 3));
        assert_eq!(o.take().len(), OUTBOX);
        assert!(o.is_empty());
    }
}
