//! One synth voice and the plain-data [`Patch`] that describes it. A patch has no heap data
//! (it is `Copy`), so a note-on carries its whole sound through the lock-free queue and the
//! audio thread never allocates.

use crate::dsp::{Adsr, Biquad, Envelope, FilterKind, Osc, Rng, Wave, pan_gains};
use serde::{Deserialize, Serialize};

/// Control values (filter, vibrato) refresh every this many samples; per-sample work stays
/// small. Counted per voice, so output does not depend on how the host splits blocks.
const CONTROL: u32 = 32;

/// How a voice sounds. Every field has a default, so a patch in JSON can name only what it
/// changes: `{"wave":"saw","filter":"low_pass","cutoff":800}`.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(default)]
pub struct Patch {
    pub wave: Wave,
    /// Second oscillator (detuned layer, octave, noise).
    pub wave2: Wave,
    /// Detune of the second oscillator in cents.
    pub detune_cents: f32,
    /// 0..1 how much of the second oscillator is mixed in.
    pub mix2: f32,
    /// 0..1 white noise added to the voice.
    pub noise: f32,
    pub filter: FilterKind,
    /// Filter cutoff in hertz.
    pub cutoff: f32,
    /// Filter resonance (Q); 0.7 is flat.
    pub resonance: f32,
    /// Octaves the filter envelope opens the cutoff by (negative closes it).
    pub filter_env: f32,
    /// 0..1: how much the cutoff follows the note's pitch (1 = one octave per octave).
    pub key_track: f32,
    pub amp: Adsr,
    pub filter_adsr: Adsr,
    pub vibrato_hz: f32,
    /// Vibrato depth in cents.
    pub vibrato_cents: f32,
    /// -1 left .. 1 right.
    pub pan: f32,
    pub gain: f32,
}

impl Default for Patch {
    fn default() -> Self {
        Self {
            wave: Wave::Sine,
            wave2: Wave::Off,
            detune_cents: 7.0,
            mix2: 0.0,
            noise: 0.0,
            filter: FilterKind::Off,
            cutoff: 2000.0,
            resonance: 0.7,
            filter_env: 0.0,
            key_track: 0.0,
            amp: Adsr::default(),
            filter_adsr: Adsr {
                attack: 0.005,
                decay: 0.2,
                sustain: 0.3,
                release: 0.2,
            },
            vibrato_hz: 5.0,
            vibrato_cents: 0.0,
            pan: 0.0,
            gain: 0.5,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Voice {
    pub id: u32,
    pub active: bool,
    /// Order the voice was started in (oldest is stolen first).
    pub born: u64,
    patch: Patch,
    freq: f32,
    velocity: f32,
    o1: Osc,
    o2: Osc,
    rng: Rng,
    filter: Biquad,
    amp_env: Envelope,
    f_env: Envelope,
    lfo: f32,
    n: u32,
    /// Filter envelope level the coefficients were last computed for.
    f_done: f32,
    // Per-control-block values.
    inc1: f32,
    inc2: f32,
    pan_l: f32,
    pan_r: f32,
}

impl Voice {
    pub fn idle() -> Self {
        Self {
            id: 0,
            active: false,
            born: 0,
            patch: Patch::default(),
            freq: 440.0,
            velocity: 1.0,
            o1: Osc::default(),
            o2: Osc::default(),
            rng: Rng::new(1),
            filter: Biquad::default(),
            amp_env: Envelope::default(),
            f_env: Envelope::default(),
            lfo: 0.0,
            n: 0,
            f_done: -1.0,
            inc1: 0.0,
            inc2: 0.0,
            pan_l: 1.0,
            pan_r: 1.0,
        }
    }

    pub fn start(&mut self, id: u32, born: u64, freq: f32, velocity: f32, patch: Patch) {
        *self = Self::idle();
        self.id = id;
        self.active = true;
        self.born = born;
        self.patch = patch;
        self.freq = freq.max(1.0);
        self.velocity = velocity.clamp(0.0, 1.0);
        // Seed per note, so a note sounds the same wherever it lands in the stream.
        self.rng = Rng::new(id.wrapping_mul(2_654_435_761).wrapping_add(0x1234_5678));
        self.amp_env.gate_on();
        self.f_env.gate_on();
    }

    pub fn release(&mut self) {
        self.amp_env.gate_off();
        self.f_env.gate_off();
    }

    pub fn releasing(&self) -> bool {
        self.amp_env.is_releasing()
    }

    /// Loudness hint for stealing (releasing quiet voices go first).
    pub fn level(&self) -> f32 {
        self.amp_env.level
    }

    fn control(&mut self, rate: f32) {
        let p = &self.patch;
        let vib = if p.vibrato_cents != 0.0 {
            (self.lfo * std::f32::consts::TAU).sin() * p.vibrato_cents
        } else {
            0.0
        };
        let f = self.freq * (vib / 1200.0).exp2();
        self.inc1 = (f / rate).min(0.49);
        self.inc2 = (f * ((p.detune_cents + 0.0) / 1200.0).exp2() / rate).min(0.49);
        // Recompute the filter only when its envelope has moved enough to matter.
        if p.filter != FilterKind::Off && (self.f_env.level - self.f_done).abs() > 2e-3 {
            self.f_done = self.f_env.level;
            let track = (self.freq / 440.0).max(1e-3).powf(p.key_track);
            let cut = p.cutoff * track * (p.filter_env * self.f_env.level).exp2();
            self.filter.set(p.filter, cut, p.resonance, rate);
        }
        let (l, r) = pan_gains(p.pan);
        self.pan_l = l;
        self.pan_r = r;
    }

    /// Adds `out.len()` frames of this voice to `out`.
    pub fn render_add(&mut self, out: &mut [[f32; 2]], rate: f32) {
        let gain = self.patch.gain * self.velocity;
        let mix2 = self.patch.mix2;
        let noise = self.patch.noise;
        let one = 1.0 - mix2 * 0.5;
        let lfo_inc = self.patch.vibrato_hz / rate;
        for frame in out.iter_mut() {
            if self.n.is_multiple_of(CONTROL) {
                self.control(rate);
            }
            self.n = self.n.wrapping_add(1);
            self.lfo = (self.lfo + lfo_inc).fract();
            let mut s = self.o1.next(self.patch.wave, self.inc1, &mut self.rng) * one;
            if mix2 > 0.0 {
                s += self.o2.next(self.patch.wave2, self.inc2, &mut self.rng) * mix2 * 0.5;
            }
            if noise > 0.0 {
                s += self.rng.sample() * noise;
            }
            if self.patch.filter != FilterKind::Off {
                s = self.filter.process(s);
            }
            let env = self.amp_env.next(&self.patch.amp, rate);
            self.f_env.next(&self.patch.filter_adsr, rate);
            let s = s * env * gain;
            frame[0] += s * self.pan_l;
            frame[1] += s * self.pan_r;
            if self.amp_env.is_idle() {
                self.active = false;
                break;
            }
        }
    }
}
