//! Mipmap chains for model textures (decision 0280), built on the CPU so the result is the
//! same on every GPU and can be made off the frame (the background model loader calls it).
//!
//! Each level halves the one above with a 2x2 box filter. Colors are averaged in linear
//! light and weighted by alpha, so cut-out edges do not darken; normal maps are averaged as
//! vectors and renormalized; data textures (metallic-roughness) are averaged as numbers.
//! Cut-out textures keep their alpha coverage at the cutoff on every level, so foliage does
//! not thin out with distance.

use crate::Image;

/// How a texture's texels are averaged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MipKind {
    /// sRGB color with alpha. `coverage`: the alpha cutoff of a cut-out material using it,
    /// whose coverage every level keeps.
    Color { coverage: Option<f32> },
    /// Linear numbers per channel (metallic-roughness, masks).
    Linear,
    /// Tangent-space normal map (xyz in 0..255 for -1..1).
    Normal,
}

/// Number of levels of a full chain for a `width` x `height` image (1 for 1x1).
pub fn level_count(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

/// The levels below level 0, largest first: RGBA8 texels, each level half the size of the
/// one above (rounded down, at least 1), down to 1x1.
pub fn chain(image: &Image, kind: MipKind) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let (mut w, mut h) = (image.width.max(1), image.height.max(1));
    if image.rgba.len() < (w * h * 4) as usize {
        return out;
    }
    let lut = srgb_to_linear_table();
    let coverage = match kind {
        MipKind::Color { coverage: Some(c) } => Some((c, alpha_coverage(&image.rgba, c, 1.0))),
        _ => None,
    };
    let mut src: &[u8] = &image.rgba;
    let mut owned: Vec<u8>;
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut dst = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let xs = [(2 * x).min(w - 1), (2 * x + 1).min(w - 1)];
                let ys = [(2 * y).min(h - 1), (2 * y + 1).min(h - 1)];
                let texels = [
                    texel(src, w, xs[0], ys[0]),
                    texel(src, w, xs[1], ys[0]),
                    texel(src, w, xs[0], ys[1]),
                    texel(src, w, xs[1], ys[1]),
                ];
                let o = ((y * nw + x) * 4) as usize;
                dst[o..o + 4].copy_from_slice(&match kind {
                    MipKind::Color { .. } if image.srgb => average_color(&texels, &lut),
                    MipKind::Color { .. } | MipKind::Linear => average_linear(&texels),
                    MipKind::Normal => average_normal(&texels),
                });
            }
        }
        if let Some((cutoff, target)) = coverage {
            keep_coverage(&mut dst, cutoff, target);
        }
        out.push(dst);
        owned = out.last().cloned().unwrap_or_default();
        src = &owned;
        (w, h) = (nw, nh);
    }
    out
}

fn texel(src: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [src[i], src[i + 1], src[i + 2], src[i + 3]]
}

fn srgb_to_linear_table() -> [f32; 256] {
    let mut t = [0.0; 256];
    for (i, v) in t.iter_mut().enumerate() {
        let c = i as f32 / 255.0;
        *v = if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        };
    }
    t
}

fn linear_to_srgb8(l: f32) -> u8 {
    let l = l.clamp(0.0, 1.0);
    let c = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0 + 0.5) as u8
}

/// Alpha-weighted average in linear light; alpha itself averages linearly.
fn average_color(t: &[[u8; 4]; 4], lut: &[f32; 256]) -> [u8; 4] {
    let mut rgb = [0.0f32; 3];
    let mut plain = [0.0f32; 3];
    let mut alpha = 0.0f32;
    for p in t {
        let a = p[3] as f32 / 255.0;
        for c in 0..3 {
            rgb[c] += lut[p[c] as usize] * a;
            plain[c] += lut[p[c] as usize];
        }
        alpha += a;
    }
    let color = |c: usize| {
        if alpha > 0.0 {
            rgb[c] / alpha
        } else {
            plain[c] / 4.0
        }
    };
    [
        linear_to_srgb8(color(0)),
        linear_to_srgb8(color(1)),
        linear_to_srgb8(color(2)),
        (alpha / 4.0 * 255.0 + 0.5) as u8,
    ]
}

fn average_linear(t: &[[u8; 4]; 4]) -> [u8; 4] {
    let mut out = [0u8; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let sum: u32 = t.iter().map(|p| p[c] as u32).sum();
        *o = ((sum + 2) / 4) as u8;
    }
    out
}

fn average_normal(t: &[[u8; 4]; 4]) -> [u8; 4] {
    let mut n = [0.0f32; 3];
    for p in t {
        for c in 0..3 {
            n[c] += p[c] as f32 / 127.5 - 1.0;
        }
    }
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    let n = if len > 1e-6 {
        n.map(|v| v / len)
    } else {
        [0.0, 0.0, 1.0]
    };
    let alpha: u32 = t.iter().map(|p| p[3] as u32).sum();
    [
        ((n[0] + 1.0) * 127.5 + 0.5).clamp(0.0, 255.0) as u8,
        ((n[1] + 1.0) * 127.5 + 0.5).clamp(0.0, 255.0) as u8,
        ((n[2] + 1.0) * 127.5 + 0.5).clamp(0.0, 255.0) as u8,
        ((alpha + 2) / 4) as u8,
    ]
}

/// Fraction of texels whose alpha, scaled by `scale`, reaches `cutoff`.
fn alpha_coverage(rgba: &[u8], cutoff: f32, scale: f32) -> f32 {
    let n = rgba.len() / 4;
    if n == 0 {
        return 0.0;
    }
    let hit = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| (p[3] as f32 / 255.0 * scale).min(1.0) >= cutoff)
        .count();
    hit as f32 / n as f32
}

/// Scales a level's alpha so the share of texels at or above `cutoff` matches `target`
/// (the full-size image's share), found by bisection on the scale.
fn keep_coverage(level: &mut [u8], cutoff: f32, target: f32) {
    let (mut lo, mut hi) = (0.0f32, 4.0f32);
    for _ in 0..16 {
        let mid = 0.5 * (lo + hi);
        if alpha_coverage(level, cutoff, mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let scale = 0.5 * (lo + hi);
    for p in level.as_chunks_mut::<4>().0.iter_mut() {
        p[3] = (p[3] as f32 * scale).round().clamp(0.0, 255.0) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4], srgb: bool) -> Image {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&f(x, y));
            }
        }
        Image {
            width: w,
            height: h,
            rgba,
            srgb,
        }
    }

    #[test]
    fn chain_halves_down_to_one_texel() {
        let img = image(8, 2, |_, _| [10, 20, 30, 255], false);
        let levels = chain(&img, MipKind::Linear);
        assert_eq!(level_count(8, 2), 4);
        assert_eq!(levels.len(), 3);
        assert_eq!(levels.iter().map(Vec::len).collect::<Vec<_>>(), [16, 8, 4]);
        assert_eq!(levels[2], [10, 20, 30, 255]);
        assert_eq!(level_count(1, 1), 1);
        assert!(chain(&image(1, 1, |_, _| [0; 4], false), MipKind::Linear).is_empty());
    }

    #[test]
    fn colors_average_in_linear_light() {
        // Black and white checker: the linear mean is 0.5, which is about 188 in sRGB (not 128).
        let img = image(
            2,
            2,
            |x, y| {
                if (x + y) % 2 == 0 {
                    [255; 4]
                } else {
                    [0, 0, 0, 255]
                }
            },
            true,
        );
        let c = chain(&img, MipKind::Color { coverage: None });
        assert!((186..=190).contains(&c[0][0]), "{:?}", c[0]);
        let d = chain(&Image { srgb: false, ..img }, MipKind::Linear);
        assert_eq!(d[0][0], 128);
    }

    #[test]
    fn transparent_texels_do_not_darken_colors() {
        let img = image(
            2,
            2,
            |x, _| if x == 0 { [255, 0, 0, 255] } else { [0, 0, 0, 0] },
            true,
        );
        let c = chain(&img, MipKind::Color { coverage: None });
        assert_eq!(&c[0][..3], &[255, 0, 0]);
        assert_eq!(c[0][3], 128);
    }

    #[test]
    fn normals_stay_unit_length() {
        // Two normals leaning opposite ways average to straight up.
        let img = image(
            2,
            1,
            |x, _| {
                if x == 0 {
                    [218, 128, 218, 255]
                } else {
                    [37, 128, 218, 255]
                }
            },
            false,
        );
        let c = chain(&img, MipKind::Normal);
        assert_eq!(&c[0][..3], &[128, 128, 255]);
    }

    #[test]
    fn cut_outs_keep_their_coverage() {
        // Speckled alpha mostly under the 0.5 cutoff: plain averaging pulls every texel toward
        // the mean (under the cutoff), so the cut-out would thin out with distance.
        let mut seed = 12345u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            ((seed >> 16) % 179) as u8
        };
        let alphas: Vec<u8> = (0..256).map(|_| noise()).collect();
        let img = image(16, 16, |x, y| [0, 200, 0, alphas[(y * 16 + x) as usize]], true);
        let want = alpha_coverage(&img.rgba, 0.5, 1.0);
        let plain = chain(&img, MipKind::Color { coverage: None });
        let kept = chain(&img, MipKind::Color { coverage: Some(0.5) });
        for (p, k) in plain.iter().zip(&kept).take(2) {
            assert!(alpha_coverage(p, 0.5, 1.0) < want - 0.1, "{want}");
            assert!((alpha_coverage(k, 0.5, 1.0) - want).abs() < 0.1, "{want}");
        }
    }
}
