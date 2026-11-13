//! The synth engine. Two halves joined by a lock-free queue (`rtrb`):
//!
//! - [`SynthHandle`], on any ordinary thread (the game): sends [`Command`]s.
//! - [`Synth`], on the audio thread: drains the queue, starts and stops voices at the exact
//!   frame each command names, mixes voices and [`Generator`]s into the output.
//!
//! Real-time rules for everything `Synth::render` touches: no allocation, no locks, no
//! blocking, no I/O. Memory is reserved in `create`; a full queue, command table or voice pool
//! drops or steals and counts it in [`Stats`], never grows.

use crate::voice::{Patch, Voice};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};

/// Slots for [`Generator`]s (game code that makes its own samples).
pub const GENERATOR_SLOTS: usize = 16;
/// Commands waiting for their frame (scheduled ahead) at most.
const PENDING: usize = 1024;
/// Queue between game and audio thread.
const QUEUE: usize = 2048;

/// Game code that generates audio. `render` runs on the audio thread, so it must not allocate,
/// lock or block; preallocate everything in `new`. Add into `out` (do not overwrite it).
/// `frame` is the synth timeline position of `out[0]`.
pub trait Generator: Send {
    fn render(&mut self, out: &mut [[f32; 2]], frame: u64, rate: f32);
    /// True when it has nothing more to play; the slot is freed (the box goes back to the game
    /// thread to be dropped there).
    fn finished(&self) -> bool {
        false
    }
}

/// Something for the audio thread to do at `at` (a frame of the synth timeline; 0 or a past
/// frame means now).
pub enum Command {
    /// Start a note. `id` names it for `NoteOff`; reusing an id retriggers.
    NoteOn { at: u64, id: u32, freq: f32, velocity: f32, patch: Patch },
    /// Release every voice with this id.
    NoteOff { at: u64, id: u32 },
    AllOff { at: u64 },
    /// Master gain, glided so it does not click.
    Master { at: u64, gain: f32 },
    /// Put a generator in a slot (replacing and returning the old one).
    Install { slot: u8, generator: Box<dyn Generator> },
    Remove { slot: u8 },
}

impl Command {
    fn at(&self) -> u64 {
        match self {
            Command::NoteOn { at, .. }
            | Command::NoteOff { at, .. }
            | Command::AllOff { at }
            | Command::Master { at, .. } => *at,
            Command::Install { .. } | Command::Remove { .. } => 0,
        }
    }
}

/// Counters shared with the game thread (atomics: reading them costs the audio thread nothing).
#[derive(Debug, Default)]
pub struct Stats {
    /// Frames rendered so far: the synth timeline.
    pub frame: AtomicU64,
    /// Sample rate in use.
    pub rate: AtomicU32,
    pub voices: AtomicU32,
    /// Voices cut off to make room.
    pub stolen: AtomicU64,
    /// Commands lost because the table was full.
    pub dropped: AtomicU64,
    /// Commands that arrived after their frame and ran late.
    pub late: AtomicU64,
    /// Samples whose mixed level went past full scale before limiting.
    pub clipped: AtomicU64,
    /// Peak level since the last `take_peak`, as f32 bits.
    pub peak: AtomicU32,
    /// Callbacks that took longer than the audio they produced (set by the host adapter).
    pub slow_blocks: AtomicU64,
    /// Longest callback in microseconds (set by the host adapter).
    pub max_block_us: AtomicU64,
    /// Stream reads that found no data ready (silence was played).
    pub underruns: AtomicU64,
    /// Generators that could not be handed back because the return queue was full.
    pub trash_overflow: AtomicU64,
}

impl Stats {
    pub fn rate(&self) -> u32 {
        self.rate.load(Relaxed)
    }
    pub fn now(&self) -> u64 {
        self.frame.load(Relaxed)
    }
    /// Peak since last call, then reset.
    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak.swap(0, Relaxed))
    }
}

/// The game-thread half.
pub struct SynthHandle {
    tx: Producer<Command>,
    trash: Consumer<Box<dyn Generator>>,
    stats: Arc<Stats>,
}

impl SynthHandle {
    pub fn stats(&self) -> &Arc<Stats> {
        &self.stats
    }
    /// Sends a command. False when the queue is full (nothing was sent).
    pub fn send(&mut self, c: Command) -> bool {
        self.free_trash();
        self.tx.push(c).is_ok()
    }
    /// Frames the audio thread has rendered so far.
    pub fn now(&self) -> u64 {
        self.stats.now()
    }
    /// Frames in `secs` seconds at the current rate.
    pub fn frames(&self, secs: f64) -> u64 {
        (secs * f64::from(self.stats.rate())).round().max(0.0) as u64
    }
    pub fn note_on(&mut self, at: u64, id: u32, freq: f32, velocity: f32, patch: Patch) -> bool {
        self.send(Command::NoteOn { at, id, freq, velocity, patch })
    }
    pub fn note_off(&mut self, at: u64, id: u32) -> bool {
        self.send(Command::NoteOff { at, id })
    }
    pub fn all_off(&mut self, at: u64) -> bool {
        self.send(Command::AllOff { at })
    }
    pub fn master(&mut self, at: u64, gain: f32) -> bool {
        self.send(Command::Master { at, gain })
    }
    pub fn install(&mut self, slot: u8, generator: Box<dyn Generator>) -> bool {
        self.send(Command::Install { slot, generator })
    }
    pub fn remove(&mut self, slot: u8) -> bool {
        self.send(Command::Remove { slot })
    }
    /// Drops generators the audio thread has finished with (done on every `send`; call it
    /// yourself if you send rarely).
    pub fn free_trash(&mut self) {
        while self.trash.pop().is_ok() {}
    }
}

/// The audio-thread half.
pub struct Synth {
    rate: f32,
    frame: u64,
    rx: Consumer<Command>,
    trash: Producer<Box<dyn Generator>>,
    stats: Arc<Stats>,
    voices: Box<[Voice]>,
    pending: Vec<Command>,
    generators: Vec<Option<Box<dyn Generator>>>,
    master: f32,
    master_target: f32,
    born: u64,
}

/// A synth and its handle. `voices` is the polyphony limit (the pool is allocated here, once).
pub fn create(rate: u32, voices: usize) -> (Synth, SynthHandle) {
    let (tx, rx) = RingBuffer::new(QUEUE);
    let (trash_tx, trash_rx) = RingBuffer::new(GENERATOR_SLOTS * 2);
    let stats = Arc::new(Stats::default());
    stats.rate.store(rate, Relaxed);
    let synth = Synth {
        rate: rate as f32,
        frame: 0,
        rx,
        trash: trash_tx,
        stats: stats.clone(),
        voices: vec![Voice::idle(); voices.max(1)].into_boxed_slice(),
        pending: Vec::with_capacity(PENDING),
        generators: (0..GENERATOR_SLOTS).map(|_| None).collect(),
        master: 1.0,
        master_target: 1.0,
        born: 0,
    };
    (synth, SynthHandle { tx, trash: trash_rx, stats })
}

impl Synth {
    pub fn stats(&self) -> &Arc<Stats> {
        &self.stats
    }
    pub fn rate(&self) -> u32 {
        self.rate as u32
    }
    pub fn frame(&self) -> u64 {
        self.frame
    }
    /// Changes the sample rate (the host device may differ from the one asked for). Voices
    /// pick it up at their next control step.
    pub fn set_rate(&mut self, rate: u32) {
        self.rate = rate as f32;
        self.stats.rate.store(rate, Relaxed);
    }
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    fn insert(&mut self, c: Command) {
        if self.pending.len() == PENDING {
            self.stats.dropped.fetch_add(1, Relaxed);
            return;
        }
        let at = c.at();
        let i = self.pending.partition_point(|p| p.at() <= at);
        self.pending.insert(i, c);
    }

    fn drain_queue(&mut self) {
        while let Ok(c) = self.rx.pop() {
            match c {
                Command::Install { slot, generator } => {
                    let s = usize::from(slot);
                    if s < self.generators.len() {
                        if let Some(old) = self.generators[s].replace(generator) {
                            self.give_back(old);
                        }
                    } else {
                        self.give_back(generator);
                    }
                }
                Command::Remove { slot } => {
                    if let Some(old) = self.generators.get_mut(usize::from(slot)).and_then(Option::take) {
                        self.give_back(old);
                    }
                }
                other => self.insert(other),
            }
        }
    }

    fn give_back(&mut self, g: Box<dyn Generator>) {
        if let Err(rtrb::PushError::Full(g)) = self.trash.push(g) {
            // Return queue full (the game stopped draining it): dropping here is the only
            // option left. Counted, and rare by construction (32 slots of room).
            self.stats.trash_overflow.fetch_add(1, Relaxed);
            drop(g);
        }
    }

    fn apply(&mut self, c: Command) {
        match c {
            Command::NoteOn { at, id, freq, velocity, patch } => {
                if at < self.frame {
                    self.stats.late.fetch_add(1, Relaxed);
                }
                let born = self.born;
                self.born += 1;
                // Free voice, else a retrigger of the same id, else steal: releasing and
                // quiet first, then oldest.
                let slot = self
                    .voices
                    .iter()
                    .position(|v| !v.active)
                    .or_else(|| self.voices.iter().position(|v| v.id == id))
                    .unwrap_or_else(|| {
                        self.stats.stolen.fetch_add(1, Relaxed);
                        let mut best = 0;
                        let mut best_key = (false, f32::MAX, u64::MAX);
                        for (i, v) in self.voices.iter().enumerate() {
                            // Ordering: releasing voices first (true sorts after false, so
                            // flip), then lower level, then older.
                            let key = (!v.releasing(), v.level(), v.born);
                            if key < best_key {
                                best_key = key;
                                best = i;
                            }
                        }
                        best
                    });
                self.voices[slot].start(id, born, freq, velocity, patch);
            }
            Command::NoteOff { id, .. } => {
                for v in self.voices.iter_mut().filter(|v| v.active && v.id == id) {
                    v.release();
                }
            }
            Command::AllOff { .. } => {
                for v in self.voices.iter_mut().filter(|v| v.active) {
                    v.release();
                }
            }
            Command::Master { gain, .. } => self.master_target = gain.max(0.0),
            Command::Install { .. } | Command::Remove { .. } => {}
        }
    }

    fn render_voices(&mut self, out: &mut [[f32; 2]]) {
        let rate = self.rate;
        for v in self.voices.iter_mut().filter(|v| v.active) {
            v.render_add(out, rate);
        }
    }

    /// Renders `out.len()` frames (overwrites `out`). Commands run at their exact frame.
    pub fn render(&mut self, out: &mut [[f32; 2]]) {
        self.drain_queue();
        for f in out.iter_mut() {
            *f = [0.0; 2];
        }
        let end = self.frame + out.len() as u64;
        let mut done = 0usize;
        loop {
            // Run everything due at or before the current position.
            while self.pending.first().is_some_and(|c| c.at() <= self.frame) {
                let c = self.pending.remove(0);
                self.apply(c);
            }
            let next = self.pending.first().map_or(end, |c| c.at().min(end));
            let n = (next - self.frame) as usize;
            if n == 0 {
                break;
            }
            self.render_voices(&mut out[done..done + n]);
            done += n;
            self.frame += n as u64;
            if self.frame >= end {
                break;
            }
        }
        // Generators, over the whole block.
        let start = end - out.len() as u64;
        let rate = self.rate;
        for s in 0..self.generators.len() {
            let finished = match self.generators[s].as_mut() {
                Some(g) => {
                    g.render(out, start, rate);
                    g.finished()
                }
                None => false,
            };
            if finished {
                if let Some(g) = self.generators[s].take() {
                    self.give_back(g);
                }
            }
        }
        self.finish(out);
    }

    /// Master gain, soft limiter and stats.
    fn finish(&mut self, out: &mut [[f32; 2]]) {
        let k = 1.0 - (-1.0 / (0.01 * self.rate)).exp(); // 10 ms glide
        let (mut peak, mut clipped) = (0.0f32, 0u64);
        for f in out.iter_mut() {
            self.master += (self.master_target - self.master) * k;
            for c in f.iter_mut() {
                let x = *c * self.master;
                let a = x.abs();
                if a > 1.0 {
                    clipped += 1;
                }
                *c = soft_limit(x);
                peak = peak.max(c.abs());
            }
        }
        if clipped > 0 {
            self.stats.clipped.fetch_add(clipped, Relaxed);
        }
        self.stats.frame.store(self.frame, Relaxed);
        self.stats.voices.store(self.active_voices() as u32, Relaxed);
        // fetch_max on f32 bits works for non-negative floats.
        self.stats.peak.fetch_max(peak.to_bits(), Relaxed);
    }
}

/// Identity below 0.8, then a smooth knee that never exceeds 1.
#[inline]
pub fn soft_limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.8 {
        x
    } else {
        x.signum() * (0.8 + 0.2 * ((a - 0.8) / 0.2).tanh())
    }
}

/// Plain copy of [`Stats`] at one moment (for reports).
#[derive(serde::Serialize, Debug, Clone, PartialEq)]
pub struct StatsSnapshot {
    pub frame: u64,
    pub rate: u32,
    pub voices: u32,
    pub stolen: u64,
    pub dropped: u64,
    pub late: u64,
    pub clipped: u64,
    pub peak: f32,
    pub slow_blocks: u64,
    pub max_block_us: u64,
    pub underruns: u64,
}

impl StatsSnapshot {
    pub fn of(s: &Stats) -> Self {
        Self {
            frame: s.frame.load(Relaxed),
            rate: s.rate.load(Relaxed),
            voices: s.voices.load(Relaxed),
            stolen: s.stolen.load(Relaxed),
            dropped: s.dropped.load(Relaxed),
            late: s.late.load(Relaxed),
            clipped: s.clipped.load(Relaxed),
            peak: f32::from_bits(s.peak.load(Relaxed)),
            slow_blocks: s.slow_blocks.load(Relaxed),
            max_block_us: s.max_block_us.load(Relaxed),
            underruns: s.underruns.load(Relaxed),
        }
    }
}
