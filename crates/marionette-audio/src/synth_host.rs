//! Runs a `marionette-synth` [`Synth`] inside kira's audio callback (decision 0220), so
//! procedural and streamed audio reach the real device. The synth is one kira sound; the game
//! talks to it only through the lock-free [`SynthHandle`]. The adapter times every callback and
//! records the longest one and how many took longer than the audio they made, which is the
//! underrun signal a device run reports.

use crate::player::Player;
use kira::sound::{Sound, SoundData};
use kira::{Frame, info::Info};
use marionette_synth::{Synth, SynthHandle, create};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::Instant;

/// Frames rendered per inner step (the callback buffer is split into pieces this big).
const SCRATCH: usize = 1024;

struct SynthSound {
    synth: Synth,
    scratch: Vec<[f32; 2]>,
    closed: Arc<AtomicBool>,
}

impl Sound for SynthSound {
    fn process(&mut self, out: &mut [Frame], dt: f64, _info: &Info) {
        let rate = (1.0 / dt).round() as u32;
        if rate != self.synth.rate() && rate > 0 {
            self.synth.set_rate(rate);
        }
        let started = Instant::now();
        for chunk in out.chunks_mut(SCRATCH) {
            let buf = &mut self.scratch[..chunk.len()];
            self.synth.render(buf);
            for (o, f) in chunk.iter_mut().zip(buf.iter()) {
                *o = Frame {
                    left: f[0],
                    right: f[1],
                };
            }
        }
        let took = started.elapsed();
        let stats = self.synth.stats();
        stats.max_block_us.fetch_max(took.as_micros() as u64, Relaxed);
        let budget = out.len() as f64 * dt;
        stats.buffer_frames.store(out.len() as u32, Relaxed);
        stats
            .max_load_permille
            .fetch_max((took.as_secs_f64() / budget * 1000.0) as u32, Relaxed);
        if took.as_secs_f64() > budget {
            stats.slow_blocks.fetch_add(1, Relaxed);
        }
    }

    fn finished(&self) -> bool {
        self.closed.load(Relaxed)
    }
}

struct SynthData {
    synth: Synth,
    closed: Arc<AtomicBool>,
}

impl SoundData for SynthData {
    type Error = std::convert::Infallible;
    type Handle = ();
    fn into_sound(self) -> Result<(Box<dyn Sound>, ()), Self::Error> {
        Ok((
            Box::new(SynthSound {
                synth: self.synth,
                scratch: vec![[0.0; 2]; SCRATCH],
                closed: self.closed,
            }),
            (),
        ))
    }
}

/// A synth playing on the device. Dropping it ends the sound.
pub struct SynthHost {
    pub handle: SynthHandle,
    closed: Arc<AtomicBool>,
}

impl Drop for SynthHost {
    fn drop(&mut self) {
        self.closed.store(true, Relaxed);
    }
}

impl Player {
    /// Starts a synth with `voices` of polyphony on the audio device. The rate is learned from
    /// the device on the first callback (`handle.stats().rate()` follows it; 48 kHz until then).
    pub fn start_synth(&mut self, voices: usize) -> Result<SynthHost, String> {
        let (synth, handle) = create(48_000, voices);
        let closed = Arc::new(AtomicBool::new(false));
        self.manager_mut()
            .play(SynthData {
                synth,
                closed: closed.clone(),
            })
            .map_err(|e| format!("cannot start the synth: {e}"))?;
        Ok(SynthHost { handle, closed })
    }
}
