//! The seed set: assets made for the project by code in this file, so anyone can rebuild them
//! byte for byte (`cargo run -p marionette-pool --example make_seed -- <pool folder>`). The
//! engine repository keeps them in `pool/`; the assets repository starts from them. The test
//! `seed_pool_matches_its_generator` rebuilds the set and compares it with `pool/`.

use crate::admit::{AddReport, Candidate, add};
use crate::{Origin, Pool, PoolKind, Provenance};
use marionette_core::Errors;

const GENERATOR: &str = "crates/marionette-pool/src/seed.rs";

/// SplitMix64, as the engine's simulation RNG (decision 0003).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&f(x, y));
        }
    }
    marionette_assets::encode_png(w, h, &px)
}

/// A 1 m prototype grid: light tiles, darker every 4th line.
fn grid() -> Vec<u8> {
    image(256, 256, |x, y| {
        let major = x % 64 < 2 || y % 64 < 2;
        let minor = x % 16 == 0 || y % 16 == 0;
        let tile = ((x / 64) + (y / 64)) % 2 == 0;
        let base: u8 = if tile { 200 } else { 186 };
        let v = if major {
            90
        } else if minor {
            base - 40
        } else {
            base
        };
        [v, v, v.saturating_add(8), 255]
    })
}

/// Running-bond bricks with mortar, each brick a slightly different red.
fn bricks() -> Vec<u8> {
    let mut rng = Rng(0xb51c);
    let shades: Vec<i32> = (0..64).map(|_| (rng.next() % 41) as i32 - 20).collect();
    image(256, 256, move |x, y| {
        let row = y / 32;
        let off = if row % 2 == 0 { 0 } else { 32 };
        let col = (x + off) / 64;
        let mortar = y % 32 < 3 || (x + off) % 64 < 3;
        if mortar {
            return [178, 172, 160, 255];
        }
        let s = shades[((row * 4 + col) % 64) as usize];
        [
            (150 + s).clamp(0, 255) as u8,
            (62 + s / 2).clamp(0, 255) as u8,
            (48 + s / 3).clamp(0, 255) as u8,
            255,
        ]
    })
}

/// Tileable value noise (four octaves), gray.
fn noise() -> Vec<u8> {
    let mut rng = Rng(0x0a15e);
    let size = 256u32;
    let lattice: Vec<f32> = (0..32 * 32).map(|_| (rng.next() % 1000) as f32 / 999.0).collect();
    let value = move |x: f32, y: f32, period: u32| -> f32 {
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let s = |t: f32| t * t * (3.0 - 2.0 * t);
        let at = |i: u32, j: u32| lattice[((j % period) * 32 + (i % period)) as usize];
        let a = at(x0, y0) + (at(x0 + 1, y0) - at(x0, y0)) * s(fx);
        let b = at(x0, y0 + 1) + (at(x0 + 1, y0 + 1) - at(x0, y0 + 1)) * s(fx);
        a + (b - a) * s(fy)
    };
    image(size, size, move |x, y| {
        let mut v = 0.0;
        let mut amp = 0.5;
        for period in [4u32, 8, 16, 32] {
            let scale = period as f32 / size as f32;
            v += value(x as f32 * scale, y as f32 * scale, period) * amp;
            amp *= 0.5;
        }
        let g = (v / 0.9375 * 255.0).round().clamp(0.0, 255.0) as u8;
        [g, g, g, 255]
    })
}

/// A short rising blip: 16-bit mono WAV, 44.1 kHz, 0.12 s.
fn blip() -> Vec<u8> {
    let rate = 44_100u32;
    let n = rate * 12 / 100;
    let mut samples = Vec::with_capacity(n as usize * 2);
    let mut phase = 0.0f64;
    for i in 0..n {
        let t = i as f64 / n as f64;
        let freq = 660.0 + 440.0 * t;
        phase += freq / rate as f64;
        let env = (1.0 - t) * (1.0 - t) * (t * 40.0).min(1.0);
        // Triangle wave: no transcendental functions, identical everywhere.
        let p = phase.fract();
        let tri = if p < 0.5 { 4.0 * p - 1.0 } else { 3.0 - 4.0 * p };
        let v = (tri * env * 0.6 * 32767.0).round() as i16;
        samples.extend_from_slice(&v.to_le_bytes());
    }
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    w.extend_from_slice(&samples);
    w
}

fn made(
    kind: PoolKind,
    id: &str,
    name: &str,
    tags: &[&str],
    method: &str,
    file: &str,
    bytes: Vec<u8>,
) -> Candidate {
    Candidate {
        kind,
        name: name.into(),
        id: Some(id.into()),
        tags: tags.iter().map(|t| t.to_string()).collect(),
        source: Provenance {
            origin: Origin::Made,
            library: None,
            page: None,
            source_id: None,
            author: Some("Marionette Contributors".into()),
            retrieved: None,
            downloads: Vec::new(),
            method: Some(method.into()),
            generator: Some(GENERATOR.into()),
        },
        proof: Vec::new(),
        files: vec![(file.into(), bytes)],
        main: Some(file.into()),
        notes: Vec::new(),
        origin_path: None,
        generated_dirs: Vec::new(),
    }
}

/// Every seed asset, in a fixed order.
pub fn candidates() -> Vec<Candidate> {
    vec![
        made(
            PoolKind::Texture,
            "prototype-grid",
            "Prototype grid",
            &["grid", "prototype", "floor", "wall", "measure"],
            "computed in code: 256 px, 4 x 4 checkered tiles, dark lines every 64 px and light lines every 16 px",
            "texture.png",
            grid(),
        ),
        made(
            PoolKind::Texture,
            "red-bricks",
            "Red bricks",
            &["brick", "bricks", "wall", "red", "masonry"],
            "computed in code: running bond, per-brick shade from a seeded SplitMix64",
            "texture.png",
            bricks(),
        ),
        made(
            PoolKind::Texture,
            "gray-noise",
            "Gray value noise",
            &["noise", "gray", "grey", "grunge", "height", "tileable"],
            "computed in code: tileable four-octave value noise from a seeded SplitMix64",
            "texture.png",
            noise(),
        ),
        made(
            PoolKind::Model,
            "wooden-crate",
            "Wooden crate",
            &["crate", "box", "prop", "wood", "cube"],
            "computed in code: a 1 m box with one brown PBR material (marionette-providers box_glb)",
            "model.glb",
            marionette_providers::glb::box_glb([0.5, 0.5, 0.5], [0.42, 0.24, 0.11, 1.0], "Wooden crate"),
        ),
        made(
            PoolKind::Sound,
            "blip",
            "Blip",
            &["blip", "ui", "pickup", "beep", "sfx"],
            "computed in code: rising triangle-wave blip, 0.12 s, 16-bit 44.1 kHz WAV",
            "sound.wav",
            blip(),
        ),
    ]
}

/// Adds the seed set to `pool`.
pub fn build(pool: &Pool) -> Result<Vec<AddReport>, Errors> {
    candidates()
        .into_iter()
        .map(|c| add(pool, c, None, false))
        .collect()
}
