//! Environment lighting and reflections (decision 0066), built on the CPU once per change.
//!
//! The source is an equirectangular image (2:1, `u` turns around +y starting at -z, `v` runs
//! from straight up to straight down), either a PNG or a sky made from two colors. It is
//! resampled to [`BASE_W`] x [`BASE_W`]/2 and stored with mip levels blurred for growing
//! roughness (level `k` of `n` is roughness `k / (n - 1)`), so the shader picks the level
//! that matches a surface. Diffuse light comes from nine spherical harmonics coefficients.
//! Cost: about 1 ms in a release build, once; the texture is 350 KB.

use glam::Vec3;
use std::f32::consts::PI;

/// Width of the finest level (height is half).
pub(crate) const BASE_W: usize = 256;
/// Mip levels: 256x128 down to 8x4.
pub(crate) const LEVELS: usize = 6;

/// Linear RGB, row-major, `w` x `h`.
#[derive(Clone)]
pub(crate) struct EnvImage {
    pub w: usize,
    pub h: usize,
    pub px: Vec<Vec3>,
}

/// What the environment comes from (the cache key).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Source {
    /// An image path, relative to the scene directory.
    Image(String),
    /// A sky made from `sky` above, `ground` below.
    Sky { sky: [f32; 3], ground: [f32; 3] },
}

/// Prefiltered levels (finest first) and irradiance coefficients (already divided by PI,
/// rgb in xyz).
pub(crate) struct Prefiltered {
    pub levels: Vec<EnvImage>,
    pub sh: [[f32; 4]; 9],
}

/// Direction of the texel center (x, y) of a `w` x `h` equirectangular image.
fn direction(x: usize, y: usize, w: usize, h: usize) -> Vec3 {
    let u = (x as f32 + 0.5) / w as f32;
    let v = (y as f32 + 0.5) / h as f32;
    let phi = (u - 0.5) * 2.0 * PI;
    let theta = v * PI;
    Vec3::new(theta.sin() * phi.sin(), theta.cos(), -theta.sin() * phi.cos())
}

/// A sky: `sky` overhead, brighter toward the horizon, `ground` below with a soft edge.
pub(crate) fn sky_image(sky: Vec3, ground: Vec3) -> EnvImage {
    let (w, h) = (BASE_W, BASE_W / 2);
    let horizon = sky * 0.6 + Vec3::splat(0.4) * sky.max_element().max(0.05) + ground * 0.15;
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let d = direction(x, y, w, h);
            let c = if d.y >= 0.0 {
                horizon.lerp(sky, d.y.sqrt())
            } else {
                horizon.lerp(ground, (-d.y * 6.0).min(1.0))
            };
            px.push(c);
        }
    }
    EnvImage { w, h, px }
}

/// An RGBA8 image (sRGB) resampled to the base size, bilinear, wrapping around in `u`.
pub(crate) fn from_rgba8(width: u32, height: u32, rgba: &[u8], srgb: bool) -> EnvImage {
    let (sw, sh) = (width.max(1) as usize, height.max(1) as usize);
    let to_linear = |c: u8| {
        let c = c as f32 / 255.0;
        if !srgb {
            c
        } else if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let src: Vec<Vec3> = rgba
        .chunks_exact(4)
        .map(|p| Vec3::new(to_linear(p[0]), to_linear(p[1]), to_linear(p[2])))
        .collect();
    let (w, h) = (BASE_W, BASE_W / 2);
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let fx = (x as f32 + 0.5) / w as f32 * sw as f32 - 0.5;
            let fy = ((y as f32 + 0.5) / h as f32 * sh as f32 - 0.5).clamp(0.0, (sh - 1) as f32);
            let (x0, y0) = (fx.floor(), fy.floor());
            let (tx, ty) = (fx - x0, fy - y0);
            let at = |xx: f32, yy: f32| {
                let xi = (xx as i64).rem_euclid(sw as i64) as usize;
                let yi = (yy as usize).min(sh - 1);
                src.get(yi * sw + xi).copied().unwrap_or(Vec3::ZERO)
            };
            let top = at(x0, y0).lerp(at(x0 + 1.0, y0), tx);
            let bottom = at(x0, y0 + 1.0).lerp(at(x0 + 1.0, y0 + 1.0), tx);
            px.push(top.lerp(bottom, ty));
        }
    }
    EnvImage { w, h, px }
}

/// Angular spread (radians, one standard deviation) of the blur for `roughness`.
fn spread(roughness: f32) -> f32 {
    let a = roughness * roughness;
    a * 1.1
}

/// Half size, 2x2 box.
fn downsample(img: &EnvImage) -> EnvImage {
    let (w, h) = ((img.w / 2).max(1), (img.h / 2).max(1));
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let mut c = Vec3::ZERO;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = ((2 * x + dx).min(img.w - 1), (2 * y + dy).min(img.h - 1));
                c += img.px[sy * img.w + sx];
            }
            px.push(c * 0.25);
        }
    }
    EnvImage { w, h, px }
}

/// Gaussian blur of `sigma` radians on the sphere, approximated in the image: vertical in
/// rows, horizontal in columns widened by 1 / cos(latitude), wrapping around in `u`.
fn blur(img: &EnvImage, sigma: f32) -> EnvImage {
    if sigma <= 0.0 {
        return img.clone();
    }
    let (w, h) = (img.w, img.h);
    let kernel = |s: f32, max: usize| -> Vec<f32> {
        let r = ((3.0 * s).ceil() as usize).clamp(1, max);
        let k: Vec<f32> = (0..=2 * r)
            .map(|i| {
                let d = i as f32 - r as f32;
                (-d * d / (2.0 * s * s).max(1e-6)).exp()
            })
            .collect();
        let sum: f32 = k.iter().sum();
        k.into_iter().map(|v| v / sum).collect()
    };
    // Horizontal pass, per row.
    let mut tmp = vec![Vec3::ZERO; w * h];
    for y in 0..h {
        let lat = ((y as f32 + 0.5) / h as f32 - 0.5) * PI;
        let s_px = sigma / (2.0 * PI / w as f32) / lat.cos().max(0.05);
        let k = kernel(s_px, w / 2);
        let r = (k.len() / 2) as i64;
        for x in 0..w {
            let mut c = Vec3::ZERO;
            for (i, kv) in k.iter().enumerate() {
                let sx = (x as i64 + i as i64 - r).rem_euclid(w as i64) as usize;
                c += img.px[y * w + sx] * *kv;
            }
            tmp[y * w + x] = c;
        }
    }
    // Vertical pass, clamped at the poles.
    let s_px = sigma / (PI / h as f32);
    let k = kernel(s_px, h);
    let r = (k.len() / 2) as i64;
    let mut px = vec![Vec3::ZERO; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut c = Vec3::ZERO;
            for (i, kv) in k.iter().enumerate() {
                let sy = (y as i64 + i as i64 - r).clamp(0, h as i64 - 1) as usize;
                c += tmp[sy * w + x] * *kv;
            }
            px[y * w + x] = c;
        }
    }
    EnvImage { w, h, px }
}

/// Mip levels blurred for roughness 0, 1/(n-1), ... 1, and irradiance coefficients.
pub(crate) fn prefilter(base: EnvImage) -> Prefiltered {
    let mut levels = vec![base];
    for k in 1..LEVELS {
        let prev = levels.last().expect("base level");
        let r0 = (k - 1) as f32 / (LEVELS - 1) as f32;
        let r1 = k as f32 / (LEVELS - 1) as f32;
        // Each level adds only the blur the previous one lacks (variances add).
        let sigma = (spread(r1).powi(2) - spread(r0).powi(2)).max(0.0).sqrt();
        let next = blur(&downsample(prev), sigma);
        levels.push(next);
    }
    let sh = irradiance(&levels[2]);
    Prefiltered { levels, sh }
}

/// SH9 projection of the radiance, convolved with the cosine lobe and divided by PI, so
/// evaluating it at a normal gives the light a white diffuse surface reflects.
fn irradiance(img: &EnvImage) -> [[f32; 4]; 9] {
    let mut c = [Vec3::ZERO; 9];
    let (w, h) = (img.w, img.h);
    for y in 0..h {
        let theta = (y as f32 + 0.5) / h as f32 * PI;
        let d_omega = (2.0 * PI / w as f32) * (PI / h as f32) * theta.sin();
        for x in 0..w {
            let d = direction(x, y, w, h);
            let l = img.px[y * w + x] * d_omega;
            let basis = [
                0.282095,
                0.488603 * d.y,
                0.488603 * d.z,
                0.488603 * d.x,
                1.092548 * d.x * d.y,
                1.092548 * d.y * d.z,
                0.315392 * (3.0 * d.z * d.z - 1.0),
                1.092548 * d.x * d.z,
                0.546274 * (d.x * d.x - d.y * d.y),
            ];
            for (ci, b) in c.iter_mut().zip(basis) {
                *ci += l * b;
            }
        }
    }
    // Cosine lobe per band (PI, 2PI/3, PI/4), then / PI.
    let band = [1.0, 2.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0, 0.25, 0.25, 0.25, 0.25, 0.25];
    let mut out = [[0.0; 4]; 9];
    for i in 0..9 {
        out[i] = (c[i] * band[i]).extend(0.0).to_array();
    }
    out
}

/// Half-precision bits of `v` (round to nearest even; overflow saturates to the largest
/// finite value, so a bright image never turns into infinities).
pub(crate) fn f16_bits(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let v = v.abs();
    if v.is_nan() {
        return sign | 0x7e00;
    }
    if v >= 65504.0 {
        return sign | 0x7bff;
    }
    if v < 6.103_515_6e-5 {
        // Subnormal: value / 2^-24, rounded.
        return sign | (v / 5.960_464_5e-8).round_ties_even() as u16;
    }
    let b = v.to_bits();
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let mant = b & 0x7f_ffff;
    let mut h = ((exp as u32) << 10) | (mant >> 13);
    let rest = mant & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && (h & 1) == 1) {
        h += 1;
    }
    sign | h as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats_round_trip_simple_values() {
        assert_eq!(f16_bits(0.0), 0);
        assert_eq!(f16_bits(1.0), 0x3c00);
        assert_eq!(f16_bits(-2.0), 0xc000);
        assert_eq!(f16_bits(0.5), 0x3800);
        assert_eq!(f16_bits(1e9), 0x7bff);
        assert_eq!(f16_bits(65504.0), 0x7bff);
    }

    #[test]
    fn uniform_environment_gives_its_color_as_diffuse_light() {
        let img = EnvImage {
            w: 64,
            h: 32,
            px: vec![Vec3::splat(0.5); 64 * 32],
        };
        let sh = irradiance(&img);
        // Only the constant band: 0.282095 * c0 is the light at every normal.
        let l = sh[0][0] * 0.282095;
        assert!((l - 0.5).abs() < 0.01, "{l}");
        assert!(sh[1..].iter().all(|c| c[0].abs() < 0.01));
    }

    #[test]
    fn sky_levels_blur_toward_the_average() {
        let p = prefilter(sky_image(Vec3::new(0.3, 0.5, 0.9), Vec3::new(0.2, 0.15, 0.1)));
        assert_eq!(p.levels.len(), LEVELS);
        assert_eq!((p.levels[LEVELS - 1].w, p.levels[LEVELS - 1].h), (8, 4));
        // Top row of the roughest level is darker than the sharp zenith less than the
        // ground is: blur mixes sky and ground.
        let top = |img: &EnvImage| img.px[0].z;
        let bottom = |img: &EnvImage| img.px[img.px.len() - 1].z;
        let (sharp, rough) = (&p.levels[0], &p.levels[LEVELS - 1]);
        assert!(top(rough) - bottom(rough) < top(sharp) - bottom(sharp));
        // Up-facing surfaces get more light than down-facing ones.
        let up = p.sh[0][2] * 0.282095 + p.sh[1][2] * 0.488603;
        let down = p.sh[0][2] * 0.282095 - p.sh[1][2] * 0.488603;
        assert!(up > down);
    }

    #[test]
    fn directions_cover_the_sphere() {
        let up = direction(0, 0, 256, 128);
        assert!(up.y > 0.99);
        // Center of the image looks toward -z.
        let fwd = direction(128, 64, 256, 128);
        assert!(fwd.z < -0.99, "{fwd}");
    }
}
