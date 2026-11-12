//! Image post-processing, in integer arithmetic only so the result is the same byte for byte
//! on every platform: decode (any PNG color type to RGBA8), optional box resampling to a new
//! size, optional seam blending so the texture tiles, and a canonical re-encode (which also
//! drops any metadata chunks the provider embedded).

use crate::codes;
use crate::recipe::{OutputKind, TextureOptions};
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::Serialize;

/// What post-processing did to an image.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ImageReport {
    /// Size of the provider's image.
    pub source_size: [u32; 2],
    /// Size of the written PNG.
    pub size: [u32; 2],
    pub resized: bool,
    pub tileable: bool,
}

fn post_err(msg: impl Into<String>) -> Errors {
    Diagnostic::error(codes::POSTPROCESS, msg)
        .fix("regenerate the asset; if it keeps failing, report the provider's output format")
        .into()
}

/// Largest power of two not above `v` (at least 16).
fn floor_pow2(v: u32) -> u32 {
    (1u32 << (31 - v.max(16).leading_zeros())).max(16)
}

/// Post-processes a generated PNG for an image or texture recipe.
pub fn process(
    png: &[u8],
    output: OutputKind,
    options: Option<&TextureOptions>,
) -> Result<(Vec<u8>, ImageReport), Errors> {
    let img = marionette_assets::decode_png(png)
        .map_err(|e| post_err(format!("the image cannot be decoded: {e}")))?;
    let source_size = [img.width, img.height];
    let mut rgba = img.rgba;
    let (mut w, mut h) = (img.width, img.height);
    let mut resized = false;
    let mut tileable = false;
    if output == OutputKind::Texture {
        let opts = options.cloned().unwrap_or_default();
        let [tw, th] = opts.resize.unwrap_or([floor_pow2(w), floor_pow2(h)]);
        if [tw, th] != [w, h] {
            rgba = resample(&rgba, w, h, tw, th);
            (w, h) = (tw, th);
            resized = true;
        }
        if opts.tileable {
            rgba = make_tileable(&rgba, w, h);
            tileable = true;
        }
    }
    Ok((
        marionette_assets::encode_png(w, h, &rgba),
        ImageReport {
            source_size,
            size: [w, h],
            resized,
            tileable,
        },
    ))
}

/// Box resampling along one axis with exact integer weights. Source pixel `i` covers
/// `[i * dst, (i + 1) * dst)` and destination pixel `x` covers `[x * src, (x + 1) * src)` on a
/// common integer grid; each destination pixel is the overlap-weighted mean (rounded).
fn resample_axis(
    rgba: &[u8],
    lines: u32,
    src: u32,
    dst: u32,
    stride_line: usize,
    stride_px: usize,
) -> Vec<u8> {
    // Output layout: same line/pixel strides, with `dst` pixels per line.
    let (src, dst) = (src as u64, dst as u64);
    let mut out = vec![0u8; lines as usize * dst as usize * 4];
    let out_line = |l: usize, x: usize| -> usize {
        if stride_px == 4 {
            (l * dst as usize + x) * 4
        } else {
            (x * lines as usize + l) * 4
        }
    };
    for l in 0..lines as usize {
        for x in 0..dst {
            let (lo, hi) = (x * src, (x + 1) * src);
            let mut acc = [0u64; 4];
            let first = lo / dst;
            let last = (hi - 1) / dst;
            for i in first..=last {
                let (a, b) = (i * dst, (i + 1) * dst);
                let wgt = hi.min(b) - lo.max(a);
                let p = l * stride_line + i as usize * stride_px;
                for c in 0..4 {
                    acc[c] += rgba[p + c] as u64 * wgt;
                }
            }
            let o = out_line(l, x as usize);
            for c in 0..4 {
                out[o + c] = ((acc[c] + src / 2) / src) as u8;
            }
        }
    }
    out
}

/// Resamples an RGBA8 image to `tw` x `th` (horizontal pass, then vertical).
pub fn resample(rgba: &[u8], w: u32, h: u32, tw: u32, th: u32) -> Vec<u8> {
    // Horizontal: lines are rows, pixels advance by 4 bytes. Result is tw x h, row-major.
    let horiz = if tw == w {
        rgba.to_vec()
    } else {
        resample_axis(rgba, h, w, tw, w as usize * 4, 4)
    };
    if th == h {
        return horiz;
    }
    // Vertical: lines are columns, pixels advance by one row. `resample_axis` writes the
    // result column-major for this case; transpose back to row-major.
    let cols = resample_axis(&horiz, tw, h, th, 4, tw as usize * 4);
    let mut out = vec![0u8; (tw * th * 4) as usize];
    for x in 0..tw as usize {
        for y in 0..th as usize {
            let s = (x * th as usize + y) * 4;
            let d = (y * tw as usize + x) * 4;
            out[d..d + 4].copy_from_slice(&cols[s..s + 4]);
        }
    }
    out
}

/// Seam blending: each pixel is mixed with the pixel half the image away (wrapping). The mix
/// weight goes from all-shifted at the borders to all-original at the center, so the borders
/// show the shifted copy, whose left and right (and top and bottom) neighbours are adjacent
/// pixels of the original: the result repeats without a seam.
pub fn make_tileable(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as u64, h as u64);
    let mut out = vec![0u8; rgba.len()];
    // Weight of the original, 0..=256: distance to the nearest border, relative to half size.
    let edge = |v: u64, n: u64| -> u64 {
        let d = v.min(n - 1 - v) * 2 * 256;
        (d / n.max(2).saturating_sub(1)).min(256)
    };
    for y in 0..h {
        for x in 0..w {
            let t = edge(x, w).min(edge(y, h));
            let a = ((y * w + x) * 4) as usize;
            let (sx, sy) = ((x + w / 2) % w, (y + h / 2) % h);
            let b = ((sy * w + sx) * 4) as usize;
            for c in 0..4 {
                let v = rgba[a + c] as u64 * t + rgba[b + c] as u64 * (256 - t);
                out[a + c] = ((v + 128) / 256) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
        c.iter().copied().cycle().take((w * h * 4) as usize).collect()
    }

    #[test]
    fn resampling_keeps_solid_colors_and_averages_exactly() {
        let img = solid(30, 20, [10, 200, 30, 255]);
        assert_eq!(resample(&img, 30, 20, 16, 16), solid(16, 16, [10, 200, 30, 255]));
        assert_eq!(resample(&img, 30, 20, 64, 32), solid(64, 32, [10, 200, 30, 255]));
        // Two pixels, black and white, to one: the mean (rounded).
        let two = [0, 0, 0, 255, 255, 255, 255, 255];
        assert_eq!(resample(&two, 2, 1, 1, 1), vec![128, 128, 128, 255]);
        // Non-square vertical pass keeps row-major order.
        let col = [
            0, 0, 0, 255, 100, 100, 100, 255, 200, 200, 200, 255, 50, 50, 50, 255,
        ];
        assert_eq!(
            resample(&col, 1, 4, 1, 2),
            vec![50, 50, 50, 255, 125, 125, 125, 255]
        );
    }

    #[test]
    fn tileable_borders_continue_across_the_wrap() {
        // A horizontal ramp has a hard seam (255 next to 0) when repeated.
        let (w, h) = (64u32, 8u32);
        let mut img = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                let v = (x * 4) as u8;
                img.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let t = make_tileable(&img, w, h);
        let px = |x: u32, y: u32| t[((y * w + x) * 4) as usize] as i32;
        // Across the wrap the step is as small as between ordinary neighbours.
        for y in 0..h {
            assert!((px(w - 1, y) - px(0, y)).abs() <= 8, "seam at row {y}");
        }
        assert_eq!(t, make_tileable(&img, w, h));
    }

    #[test]
    fn floor_pow2_rounds_down() {
        assert_eq!(floor_pow2(1024), 1024);
        assert_eq!(floor_pow2(1000), 512);
        assert_eq!(floor_pow2(17), 16);
        assert_eq!(floor_pow2(3), 16);
    }
}
