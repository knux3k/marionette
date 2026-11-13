//! Check audio as numbers (an agent cannot listen): level, clipping, clicks, pitch, spectrum and
//! WAV output. Works on any rendered buffer, from the offline renderer or a test.

use serde::Serialize;

/// What a rendered buffer contains.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Report {
    pub frames: usize,
    pub rate: u32,
    pub seconds: f64,
    /// Largest absolute sample.
    pub peak: f32,
    pub rms: f32,
    /// Mean of all samples (a DC offset wastes headroom and thumps on start and stop).
    pub dc: f32,
    /// Samples at or beyond full scale.
    pub clipped: usize,
    /// NaN or infinite samples (always a bug).
    pub non_finite: usize,
    /// Frames quieter than 1e-5 on both channels.
    pub silent_frames: usize,
    pub first_sound_frame: Option<usize>,
    /// Largest jump between neighbouring samples on one channel. A click is a jump far
    /// bigger than the signal's own slope.
    pub max_step: f32,
    /// Frequency with the most energy in the loudest second (None if silent).
    pub dominant_hz: Option<f32>,
}

/// Analyzes stereo frames.
pub fn analyze(frames: &[[f32; 2]], rate: u32) -> Report {
    let n = frames.len();
    let (mut peak, mut sum, mut sq) = (0.0f32, 0.0f64, 0.0f64);
    let (mut clipped, mut bad, mut silent) = (0, 0, 0);
    let mut first = None;
    let mut max_step = 0.0f32;
    for (i, f) in frames.iter().enumerate() {
        let mut quiet = true;
        for c in 0..2 {
            let x = f[c];
            if !x.is_finite() {
                bad += 1;
                continue;
            }
            peak = peak.max(x.abs());
            sum += f64::from(x);
            sq += f64::from(x) * f64::from(x);
            if x.abs() >= 1.0 {
                clipped += 1;
            }
            if x.abs() >= 1e-5 {
                quiet = false;
            }
            if i > 0 && frames[i - 1][c].is_finite() {
                max_step = max_step.max((x - frames[i - 1][c]).abs());
            }
        }
        if quiet {
            silent += 1;
        } else if first.is_none() {
            first = Some(i);
        }
    }
    let count = (n * 2).max(1) as f64;
    let mono: Vec<f32> = frames.iter().map(|f| (f[0] + f[1]) * 0.5).collect();
    Report {
        frames: n,
        rate,
        seconds: n as f64 / f64::from(rate.max(1)),
        peak,
        rms: (sq / count).sqrt() as f32,
        dc: (sum / count) as f32,
        clipped,
        non_finite: bad,
        silent_frames: silent,
        first_sound_frame: first,
        max_step,
        dominant_hz: if peak > 1e-4 {
            dominant_hz(&mono, rate)
        } else {
            None
        },
    }
}

/// In-place radix-2 FFT (length must be a power of two).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f32::consts::TAU / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
        }
        len <<= 1;
    }
}

/// Magnitude spectrum (bins 0..N/2) of up to `size` samples (a power of two) from `start`,
/// Hann windowed. Bin `k` is `k * rate / size` hertz.
pub fn spectrum(mono: &[f32], start: usize, size: usize) -> Vec<f32> {
    let size = size.next_power_of_two();
    let mut re = vec![0.0f32; size];
    let mut im = vec![0.0f32; size];
    for (i, r) in re.iter_mut().enumerate() {
        if let Some(&x) = mono.get(start + i) {
            let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / size as f32).cos();
            *r = x * w;
        }
    }
    fft(&mut re, &mut im);
    (0..size / 2)
        .map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt() / size as f32)
        .collect()
}

/// Strongest frequency in the loudest 16384-sample window.
pub fn dominant_hz(mono: &[f32], rate: u32) -> Option<f32> {
    const W: usize = 16_384;
    // Loudest window by sum of squares, stepping half a window.
    let mut best = (0usize, 0.0f64);
    let mut s = 0;
    while s < mono.len().max(1) {
        let e: f64 = mono[s..(s + W).min(mono.len())]
            .iter()
            .map(|x| f64::from(*x) * f64::from(*x))
            .sum();
        if e > best.1 {
            best = (s, e);
        }
        s += W / 2;
    }
    if best.1 <= 0.0 {
        return None;
    }
    let sp = spectrum(mono, best.0, W);
    let (k, _) = sp.iter().enumerate().skip(1).max_by(|a, b| a.1.total_cmp(b.1))?;
    // Parabolic interpolation around the peak.
    let (a, b, c) = (sp[k.saturating_sub(1)], sp[k], sp[(k + 1).min(sp.len() - 1)]);
    let denom = a - 2.0 * b + c;
    let off = if denom.abs() > 1e-12 {
        0.5 * (a - c) / denom
    } else {
        0.0
    };
    Some((k as f32 + off) * rate as f32 / W as f32)
}

/// 16-bit stereo PCM WAV bytes.
pub fn wav_stereo_bytes(frames: &[[f32; 2]], rate: u32) -> Vec<u8> {
    let data_len = (frames.len() * 4) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for f in frames {
        for c in f {
            b.extend_from_slice(&((c.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
        }
    }
    b
}

/// Stable hash of a rendered buffer's bits (FNV-1a), to pin "this score renders this audio".
pub fn hash_frames(frames: &[[f32; 2]]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for f in frames {
        for c in f {
            for byte in c.to_bits().to_le_bytes() {
                h ^= u64::from(byte);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    format!("{h:016x}")
}
