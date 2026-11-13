//! Release-mode cost of the synth: how many sustained voices fit in a real-time budget.
//! `cargo run --release -p marionette-synth --example bench`
use marionette_synth::*;
use std::time::Instant;

fn main() {
    for voices in [16usize, 64, 128, 256, 512] {
        for preset in ["pad", "bass", "lead"] {
            let (mut synth, mut h) = create(48_000, voices);
            for i in 0..voices {
                h.note_on(
                    0,
                    i as u32,
                    80.0 + i as f32 * 7.0,
                    1.0,
                    Patch::preset(preset).unwrap(),
                );
            }
            let mut out = vec![[0.0f32; 2]; 480];
            synth.render(&mut out);
            let blocks = 400; // 4 s of audio
            let t = Instant::now();
            for _ in 0..blocks {
                synth.render(&mut out);
            }
            let secs = t.elapsed().as_secs_f64();
            println!(
                "{voices:4} voices {preset:6}: {:6.2}% of one core ({:.1} us per 10 ms block)",
                secs / (blocks as f64 * 0.01) * 100.0,
                secs / blocks as f64 * 1e6
            );
        }
    }
}
