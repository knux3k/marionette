//! Small DSP blocks. All are plain structs: no heap, no locks, so they are safe in an audio
//! callback. Everything is deterministic (no clock, no global state), so the same input gives
//! the same samples on every machine.

use serde::{Deserialize, Serialize};
use std::f32::consts::{PI, TAU};

/// xorshift32 noise: cheap, allocation free and repeatable from its seed.
#[derive(Debug, Clone, Copy)]
pub struct Rng(pub u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(if seed == 0 { 0x9E37_79B9 } else { seed })
    }
    /// Next value in -1..1.
    #[inline]
    pub fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 * (2.0 / (1u32 << 24) as f32) - 1.0
    }
}

/// Oscillator shapes. `Saw`, `Square` and `Triangle` are band limited (polyBLEP), so high notes
/// do not alias into harsh noise.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Wave {
    #[default]
    Sine,
    Saw,
    Square,
    Triangle,
    /// White noise (ignores pitch).
    Noise,
    /// Silence: turns an oscillator off.
    Off,
}

#[inline]
fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// Phase accumulator oscillator.
#[derive(Debug, Clone, Copy, Default)]
pub struct Osc {
    pub phase: f32,
    /// Triangle integrator state.
    tri: f32,
}

impl Osc {
    /// One sample. `inc` is frequency / sample rate (0..0.5).
    #[inline]
    pub fn next(&mut self, wave: Wave, inc: f32, rng: &mut Rng) -> f32 {
        let p = self.phase;
        let out = match wave {
            Wave::Off => 0.0,
            Wave::Noise => rng.next(),
            Wave::Sine => (p * TAU).sin(),
            Wave::Saw => 2.0 * p - 1.0 - blep(p, inc),
            Wave::Square => {
                let naive = if p < 0.5 { 1.0 } else { -1.0 };
                naive + blep(p, inc) - blep((p + 0.5).fract(), inc)
            }
            Wave::Triangle => {
                // Integrated band-limited square: leaky so it never drifts.
                let naive = if p < 0.5 { 1.0 } else { -1.0 };
                let sq = naive + blep(p, inc) - blep((p + 0.5).fract(), inc);
                self.tri = inc * 4.0 * sq + (1.0 - inc * 0.5) * self.tri;
                self.tri
            }
        };
        self.phase += inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        out
    }
}

/// Filter response.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FilterKind {
    /// No filter.
    #[default]
    Off,
    LowPass,
    HighPass,
    BandPass,
}

/// RBJ biquad, transposed direct form II.
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Default for Biquad {
    fn default() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: 0.0, z2: 0.0 }
    }
}

impl Biquad {
    /// Sets the response (keeps the state, so sweeping does not click). `q` 0.5 is flat,
    /// larger rings at the cutoff.
    pub fn set(&mut self, kind: FilterKind, cutoff: f32, q: f32, rate: f32) {
        let fc = cutoff.clamp(10.0, rate * 0.45);
        let w = TAU * fc / rate;
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q.max(0.1));
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = match kind {
            FilterKind::Off => (a0, 0.0, 0.0),
            FilterKind::LowPass => ((1.0 - c) * 0.5, 1.0 - c, (1.0 - c) * 0.5),
            FilterKind::HighPass => ((1.0 + c) * 0.5, -(1.0 + c), (1.0 + c) * 0.5),
            FilterKind::BandPass => (alpha, 0.0, -alpha),
        };
        let inv = 1.0 / a0;
        self.b0 = b0 * inv;
        self.b1 = b1 * inv;
        self.b2 = b2 * inv;
        self.a1 = -2.0 * c * inv;
        self.a2 = (1.0 - alpha) * inv;
        if kind == FilterKind::Off {
            self.a1 = 0.0;
            self.a2 = 0.0;
        }
    }
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// Attack, decay, sustain, release in seconds and level 0..1.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(default)]
pub struct Adsr {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl Default for Adsr {
    fn default() -> Self {
        Self { attack: 0.005, decay: 0.1, sustain: 0.8, release: 0.15 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Release,
}

/// Envelope generator: linear attack, exponential decay and release (-60 dB over the time).
#[derive(Debug, Clone, Copy)]
pub struct Envelope {
    stage: Stage,
    pub level: f32,
}

impl Default for Envelope {
    fn default() -> Self {
        Self { stage: Stage::Idle, level: 0.0 }
    }
}

#[inline]
fn coef(secs: f32, rate: f32) -> f32 {
    if secs <= 1e-4 { 0.0 } else { (-6.9 / (secs * rate)).exp() }
}

impl Envelope {
    pub fn gate_on(&mut self) {
        self.stage = Stage::Attack;
    }
    pub fn gate_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }
    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }
    pub fn is_releasing(&self) -> bool {
        self.stage == Stage::Release
    }
    /// Next level.
    #[inline]
    pub fn next(&mut self, p: &Adsr, rate: f32) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                if p.attack <= 1e-4 {
                    self.level = 1.0;
                } else {
                    self.level += 1.0 / (p.attack * rate);
                }
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                let c = coef(p.decay, rate);
                self.level = p.sustain + (self.level - p.sustain) * c;
                // A decay to silence ends the note (and avoids denormal numbers, which are slow).
                if p.sustain <= 0.0 && self.level < 1e-4 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
            Stage::Release => {
                self.level *= coef(p.release, rate);
                if self.level < 1e-4 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

/// Moves a value toward a target without steps (parameter smoothing).
#[derive(Debug, Clone, Copy)]
pub struct Smooth {
    pub value: f32,
}

impl Smooth {
    #[inline]
    pub fn toward(&mut self, target: f32, k: f32) -> f32 {
        self.value += (target - self.value) * k;
        self.value
    }
}

/// Constant-power pan law, `pan` -1 left .. 1 right: returns (left, right) gains.
#[inline]
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * 0.25 * PI;
    (a.cos(), a.sin())
}

/// Frequency of a MIDI note (69 = 440 Hz).
pub fn midi_hz(note: f32) -> f32 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}
