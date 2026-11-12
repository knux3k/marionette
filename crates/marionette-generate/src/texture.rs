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
    /// The flat background was made transparent (`remove_background`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub background_removed: bool,
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
    let mut background_removed = false;
    if output == OutputKind::Texture {
        let opts = options.cloned().unwrap_or_default();
        let [tw, th] = opts.resize.unwrap_or([floor_pow2(w), floor_pow2(h)]);
        if [tw, th] != [w, h] {
            rgba = resample(&rgba, w, h, tw, th);
            (w, h) = (tw, th);
            resized = true;
        }
        if opts.remove_background {
            remove_background(&mut rgba, w, h);
            background_removed = true;
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
            background_removed,
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
    // Vertical: lines are columns, pixels advance by one row. `resample_axis` writes output
    // pixel (row `x`, column `l`) at `x * tw + l`, which is already row-major.
    resample_axis(&horiz, tw, h, th, 4, tw as usize * 4)
}

/// Distance from the key colour (largest channel difference) at which a pixel starts to show,
/// and at which it is fully opaque.
const KEY_LO: u32 = 48;
const KEY_HI: u32 = 112;

/// Makes a flat background transparent. The key is the mean colour of the four corner pixels.
/// A pixel's opacity grows from 0 at distance `KEY_LO` from the key to full at `KEY_HI`;
/// partly transparent pixels get the key's share taken out of their colour (so edges do not
/// keep a coloured fringe), and fully transparent pixels become transparent black. Integer
/// arithmetic only.
pub fn remove_background(rgba: &mut [u8], w: u32, h: u32) {
    if w == 0 || h == 0 {
        return;
    }
    let (w, h) = (w as usize, h as usize);
    let corners = [0, w - 1, (h - 1) * w, h * w - 1];
    let mut key = [0u32; 3];
    for c in 0..3 {
        key[c] = (corners.iter().map(|&i| rgba[i * 4 + c] as u32).sum::<u32>() + 2) / 4;
    }
    for px in rgba.as_chunks_mut::<4>().0 {
        let d = (0..3).map(|c| (px[c] as u32).abs_diff(key[c])).max().unwrap_or(0);
        let a = if d <= KEY_LO {
            0
        } else if d >= KEY_HI {
            255
        } else {
            (d - KEY_LO) * 255 / (KEY_HI - KEY_LO)
        };
        if a == 0 {
            px.copy_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        if a < 255 {
            // c = a*fg + (255-a)*key  =>  fg = (255*c - (255-a)*key) / a
            for c in 0..3 {
                let num = (255 * px[c] as u32).saturating_sub((255 - a) * key[c]);
                px[c] = ((num + a / 2) / a).min(255) as u8;
            }
        }
        px[3] = ((px[3] as u32 * a + 127) / 255) as u8;
    }
    unmix_edge_spill(rgba, w, h, key);
}

/// Width in pixels of the band along transparent areas where `unmix_edge_spill` works.
const SPILL_BAND: usize = 3;

/// Smallest key share (of 255) that `unmix_edge_spill` treats as spill.
const SPILL_MIN: u32 = 32;

/// A dark subject blended with a bright key is far from the key on the largest channel and so
/// stays opaque, keeping a coloured outline. Near transparent pixels, this estimates how much of
/// the key a pixel holds from its spill (how far the key's strong channels rise above its weak
/// ones, compared with the key itself), takes that share out of the colour and the opacity.
/// Pixels deeper inside keep their colour, so key-coloured detail in the subject survives.
fn unmix_edge_spill(rgba: &mut [u8], w: usize, h: usize, key: [u32; 3]) {
    let strong: Vec<usize> = (0..3).filter(|&c| key[c] >= 128).collect();
    let weak: Vec<usize> = (0..3).filter(|&c| key[c] < 128).collect();
    if strong.is_empty() || weak.is_empty() {
        return;
    }
    let spill_of = |px: &[u32; 3]| -> u32 {
        let lo = strong.iter().map(|&c| px[c]).min().unwrap_or(0);
        let hi = weak.iter().map(|&c| px[c]).max().unwrap_or(0);
        lo.saturating_sub(hi)
    };
    let key_spill = spill_of(&key);
    if key_spill == 0 {
        return;
    }
    // Distance (in pixels, Chebyshev) to the nearest transparent pixel, capped at the band.
    let transparent: Vec<bool> = rgba.as_chunks::<4>().0.iter().map(|p| p[3] == 0).collect();
    let near_edge = |x: usize, y: usize| -> bool {
        let (x0, x1) = (x.saturating_sub(SPILL_BAND), (x + SPILL_BAND).min(w - 1));
        let (y0, y1) = (y.saturating_sub(SPILL_BAND), (y + SPILL_BAND).min(h - 1));
        (y0..=y1).any(|yy| (x0..=x1).any(|xx| transparent[yy * w + xx]))
    };
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            if transparent[y * w + x] || !near_edge(x, y) {
                continue;
            }
            let c = [rgba[i] as u32, rgba[i + 1] as u32, rgba[i + 2] as u32];
            // Share of key, 0..=255.
            let f = (spill_of(&c) * 255 / key_spill).min(255);
            // A slight lean toward the key is the subject's own colour, not spill.
            if f < SPILL_MIN {
                continue;
            }
            let keep = 255 - f;
            let a = (rgba[i + 3] as u32 * keep + 127) / 255;
            if a == 0 {
                rgba[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            // c = keep*fg + f*key  =>  fg = (255*c - f*key) / keep
            for ch in 0..3 {
                let num = (255 * c[ch]).saturating_sub(f * key[ch]);
                rgba[i + ch] = ((num + keep / 2) / keep).min(255) as u8;
            }
            rgba[i + 3] = a as u8;
        }
    }
}

/// Seam blending with four copies: the original and the copies shifted half a width, half a
/// height and both (wrapping). Weights are separable: `wx` goes from 0 at the left and right
/// borders to 1 at the center column, `wy` likewise for rows; the original gets `wx*wy`, the
/// x-shifted copy `(1-wx)*wy`, the y-shifted copy `wx*(1-wy)` and the doubly shifted copy
/// `(1-wx)*(1-wy)`. Each shifted copy has its own seam on a center line, exactly where its
/// weight is zero, and at the borders only copies whose neighbours across the wrap are adjacent
/// original pixels remain: the result repeats with no seam at the borders or the center.
/// Integer weights (0..=256 per axis), so results are exact on every machine.
pub fn make_tileable(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as u64, h as u64);
    let mut out = vec![0u8; rgba.len()];
    // 0 at a border, 256 at the center.
    let edge = |v: u64, n: u64| -> u64 {
        let d = v.min(n - 1 - v) * 2 * 256;
        (d / n.max(2).saturating_sub(1)).min(256)
    };
    let at = |x: u64, y: u64| ((y * w + x) * 4) as usize;
    for y in 0..h {
        let wy = edge(y, h);
        let sy = (y + h / 2) % h;
        for x in 0..w {
            let wx = edge(x, w);
            let sx = (x + w / 2) % w;
            let copies = [
                (at(x, y), wx * wy),
                (at(sx, y), (256 - wx) * wy),
                (at(x, sy), wx * (256 - wy)),
                (at(sx, sy), (256 - wx) * (256 - wy)),
            ];
            let a = at(x, y);
            for c in 0..4 {
                let v: u64 = copies.iter().map(|&(i, k)| rgba[i + c] as u64 * k).sum();
                out[a + c] = ((v + 32768) / 65536) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_backgrounds_become_transparent_and_lose_their_tint_at_edges() {
        // 4x1: magenta, magenta-ish edge blend, orange, magenta.
        let k = [250u8, 0, 200, 255];
        let fg = [255u8, 140, 0, 255];
        // A 50% blend of fg and key.
        let mid = [252u8, 70, 100, 255];
        let mut img = [k, mid, fg, k].concat();
        remove_background(&mut img, 4, 1);
        assert_eq!(&img[0..4], &[0, 0, 0, 0], "key colour is transparent");
        assert_eq!(&img[8..12], &fg, "far colours are untouched");
        assert_eq!(&img[12..16], &[0, 0, 0, 0]);
        // The blend is opaque enough to show and its colour moves toward fg, away from the key.
        assert!(img[7] > 0, "{:?}", &img[4..8]);
        assert!(img[6] < mid[2] && img[5] >= mid[1], "{:?}", &img[4..8]);
        // Deterministic.
        let mut again = [k, mid, fg, k].concat();
        remove_background(&mut again, 4, 1);
        assert_eq!(img, again);
    }

    #[test]
    fn dark_outlines_blended_with_the_key_lose_their_tint() {
        let k = [250u8, 0, 200, 255];
        let dark = [20u8, 10, 40, 255];
        // Half dark, half key: far from the key on red, so the first pass keeps it opaque.
        let blend = [135u8, 5, 120, 255];
        let mut img = [k, blend, dark, dark, k].concat();
        remove_background(&mut img, 5, 1);
        let px = &img[4..8];
        assert!(px[3] > 0 && px[3] < 200, "edge is partly transparent: {px:?}");
        assert!(px[0] < 70 && px[2] < 90, "edge loses the magenta: {px:?}");
        assert_eq!(&img[8..12], &dark, "subject colours are untouched");
    }

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
    fn tileable_has_no_seam_at_the_center_either() {
        // A diagonal ramp wraps hard in both directions; every neighbour step, across the
        // wrap and through the center lines, stays small.
        let (w, h) = (64u32, 64u32);
        let mut img = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = ((x + y) * 2) as u8;
                img.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let t = make_tileable(&img, w, h);
        let px = |x: u32, y: u32| t[((y * w + x) * 4) as usize] as i32;
        let mut worst = 0;
        for y in 0..h {
            for x in 0..w {
                worst = worst
                    .max((px(x, y) - px((x + 1) % w, y)).abs())
                    .max((px(x, y) - px(x, (y + 1) % h)).abs());
            }
        }
        assert!(worst <= 12, "largest neighbour step {worst}");
    }

    #[test]
    fn resampling_keeps_orientation() {
        // 2 x 4, every pixel distinct: value = 10 * row + column.
        let (w, h) = (2u32, 4u32);
        let mut img = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = (10 * y + x) as u8;
                img.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let px = |t: &[u8], tw: u32, x: u32, y: u32| t[((y * tw + x) * 4) as usize];
        // Height only: rows 0-1 and 2-3 average; columns stay apart.
        let t = resample(&img, w, h, 2, 2);
        assert_eq!(
            [px(&t, 2, 0, 0), px(&t, 2, 1, 0), px(&t, 2, 0, 1), px(&t, 2, 1, 1)],
            [5, 6, 25, 26]
        );
        // Both axes, to a wide image: the top stays on top.
        let t = resample(&img, w, h, 4, 2);
        assert!(px(&t, 4, 0, 0) < px(&t, 4, 0, 1));
        assert!(px(&t, 4, 0, 0) <= px(&t, 4, 3, 0));
    }

    #[test]
    fn floor_pow2_rounds_down() {
        assert_eq!(floor_pow2(1024), 1024);
        assert_eq!(floor_pow2(1000), 512);
        assert_eq!(floor_pow2(17), 16);
        assert_eq!(floor_pow2(3), 16);
    }
}
