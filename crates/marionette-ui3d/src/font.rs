//! Distance-field font atlas. One atlas image serves text at every size and distance (the
//! shader smooths the edge over about one screen pixel), and a solid texel in it serves every
//! rectangle, so a whole world of panels draws with one texture.
//!
//! Two sources: a TrueType/OpenType file given by the game (`UiFontConfig`), or the small
//! built-in 5x7 font the 2D UI already uses (capitals only), so the engine ships no extra font.

use marionette_assets::Image;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Characters baked from a font file: ASCII, Latin-1 and a few common symbols.
fn charset() -> impl Iterator<Item = char> {
    (32u32..=126)
        .chain(160..=255)
        .chain([0x2022, 0x2013, 0x2014, 0x2026, 0x2192, 0x2212, 0x00D7])
        .filter_map(char::from_u32)
}

/// Where a glyph sits. Rectangles are in em units relative to the pen position on the
/// baseline (x right, y up); `uv` is `[u0, v0, u1, v1]` in the atlas (v0 is the bottom edge).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub advance: f32,
}

#[derive(Debug, Clone)]
pub struct FontData {
    pub glyphs: BTreeMap<char, Glyph>,
    /// Distance from one baseline to the next, in em.
    pub line_height: f32,
    /// Baseline to the top of a line, in em.
    pub ascent: f32,
    /// Centre of a solid area of the atlas (value 1.0 everywhere around it).
    pub white_uv: [f32; 2],
    /// Name of the atlas for `WorldQuads::image` (`generated:<key>`).
    pub image_name: String,
    pub atlas: Arc<Image>,
}

impl FontData {
    pub fn glyph(&self, c: char) -> &Glyph {
        self.glyphs
            .get(&c)
            .or_else(|| self.glyphs.get(&'?'))
            .or_else(|| self.glyphs.get(&' '))
            .expect("every font has a space")
    }

    /// The built-in font, drawn from the 2D UI's bitmap font.
    pub fn builtin() -> Self {
        use marionette_assets::font::{CELL_H, CELL_W, atlas, cell};
        const SCALE: u32 = 8;
        const SPREAD: u32 = 8;
        let src = atlas();
        let em = (CELL_H * SCALE) as f32;
        let mut bakes = Vec::new();
        for c in (32u8..=126).map(char::from) {
            let (gx, gy) = cell(c);
            let (w, h) = (CELL_W * SCALE + 2 * SPREAD, CELL_H * SCALE + 2 * SPREAD);
            let mut cov = vec![0u8; (w * h) as usize];
            for y in 0..CELL_H * SCALE {
                for x in 0..CELL_W * SCALE {
                    let (sx, sy) = (gx + x / SCALE, gy + y / SCALE);
                    let a = src.rgba[((sy * src.width + sx) * 4 + 3) as usize];
                    cov[((y + SPREAD) * w + x + SPREAD) as usize] = a;
                }
            }
            // The cell is 8 rows with the glyph in the top 7: the baseline is 7 rows down.
            let baseline = 7.0 * SCALE as f32;
            bakes.push(Bake {
                c,
                cov,
                w,
                h,
                x0: -(SPREAD as f32) / em,
                y0: (baseline - (CELL_H * SCALE + SPREAD) as f32) / em,
                advance: CELL_W as f32 / CELL_H as f32,
                px_per_em: em,
            });
        }
        pack(bakes, SPREAD, 1.25, 7.0 / 8.0 + 0.1)
    }

    /// A font from TrueType or OpenType bytes, baked at 48 pixels per em.
    pub fn from_ttf(bytes: &[u8]) -> Result<Self, String> {
        const PX: f32 = 48.0;
        const SPREAD: u32 = 6;
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(|e| format!("cannot read font: {e}"))?;
        let lm = font
            .horizontal_line_metrics(PX)
            .ok_or("font has no horizontal line metrics")?;
        let mut bakes = Vec::new();
        for c in charset() {
            if c != ' ' && font.lookup_glyph_index(c) == 0 {
                continue;
            }
            let (m, bitmap) = font.rasterize(c, PX);
            let (w, h) = (m.width as u32 + 2 * SPREAD, m.height as u32 + 2 * SPREAD);
            let mut cov = vec![0u8; (w * h) as usize];
            for y in 0..m.height {
                for x in 0..m.width {
                    cov[(y + SPREAD as usize) * w as usize + x + SPREAD as usize] = bitmap[y * m.width + x];
                }
            }
            bakes.push(Bake {
                c,
                cov,
                w,
                h,
                x0: (m.xmin as f32 - SPREAD as f32) / PX,
                y0: (m.ymin as f32 - SPREAD as f32) / PX,
                advance: m.advance_width / PX,
                px_per_em: PX,
            });
        }
        if bakes.is_empty() {
            return Err("font has none of the needed characters".into());
        }
        Ok(pack(bakes, SPREAD, lm.new_line_size / PX, lm.ascent / PX))
    }
}

struct Bake {
    c: char,
    /// Coverage 0..255 including `SPREAD` pixels of empty border.
    cov: Vec<u8>,
    w: u32,
    h: u32,
    /// Em position of the bitmap's bottom-left corner relative to the pen.
    x0: f32,
    y0: f32,
    advance: f32,
    px_per_em: f32,
}

const ATLAS_W: u32 = 1024;

fn pack(bakes: Vec<Bake>, spread: u32, line_height: f32, ascent: f32) -> FontData {
    // Shelf packing: left to right, a new row when the width is used up. One pixel between cells.
    let mut place = Vec::with_capacity(bakes.len() + 1);
    let (mut x, mut y, mut row_h) = (1u32, 1u32, 0u32);
    let white = 8u32;
    let mut sizes: Vec<(u32, u32)> = vec![(white, white)];
    sizes.extend(bakes.iter().map(|b| (b.w, b.h)));
    for (w, h) in &sizes {
        if x + w + 1 > ATLAS_W {
            x = 1;
            y += row_h + 1;
            row_h = 0;
        }
        place.push((x, y));
        x += w + 1;
        row_h = row_h.max(*h);
    }
    let height = (y + row_h + 1).next_power_of_two();
    let mut sdf = vec![0u8; (ATLAS_W * height) as usize];
    let (wx, wy) = place[0];
    for yy in 0..white {
        for xx in 0..white {
            sdf[((wy + yy) * ATLAS_W + wx + xx) as usize] = 255;
        }
    }
    let mut glyphs = BTreeMap::new();
    for (b, (px, py)) in bakes.iter().zip(place.iter().skip(1)) {
        let field = sdf_from_coverage(&b.cov, b.w as usize, b.h as usize, spread as f32);
        for yy in 0..b.h {
            let dst = ((py + yy) * ATLAS_W + px) as usize;
            let src = (yy * b.w) as usize;
            sdf[dst..dst + b.w as usize].copy_from_slice(&field[src..src + b.w as usize]);
        }
        // The bitmap's top row is the glyph's highest, so uv v grows downward while y grows up.
        let (aw, ah) = (ATLAS_W as f32, height as f32);
        glyphs.insert(
            b.c,
            Glyph {
                rect: [
                    b.x0,
                    b.y0,
                    b.x0 + b.w as f32 / b.px_per_em,
                    b.y0 + b.h as f32 / b.px_per_em,
                ],
                uv: [
                    *px as f32 / aw,
                    (*py + b.h) as f32 / ah,
                    (*px + b.w) as f32 / aw,
                    *py as f32 / ah,
                ],
                advance: b.advance,
            },
        );
    }
    let mut rgba = Vec::with_capacity(sdf.len() * 4);
    for v in &sdf {
        rgba.extend_from_slice(&[*v, *v, *v, 255]);
    }
    let digest = Sha256::digest(&rgba);
    let key: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    let atlas = Arc::new(Image { width: ATLAS_W, height, rgba, srgb: false });
    let key = format!("sdf-{key}");
    marionette_assets::generated::put(&key, atlas.clone());
    FontData {
        glyphs,
        line_height,
        ascent,
        white_uv: [
            (wx as f32 + white as f32 / 2.0) / ATLAS_W as f32,
            (wy as f32 + white as f32 / 2.0) / height as f32,
        ],
        image_name: format!("generated:{key}"),
        atlas,
    }
}

/// Signed distance field of an anti-aliased coverage bitmap: 255 deep inside, 0 far outside,
/// 128 on the edge, reaching those over `spread` pixels (Felzenszwalb distance transform,
/// with partial coverage placing the edge to a fraction of a pixel).
pub fn sdf_from_coverage(cov: &[u8], w: usize, h: usize, spread: f32) -> Vec<u8> {
    const INF: f32 = 1e20;
    let n = w * h;
    let mut outer = vec![INF; n];
    let mut inner = vec![INF; n];
    for i in 0..n {
        let a = cov[i] as f32 / 255.0;
        if a >= 1.0 {
            outer[i] = 0.0;
        } else if a <= 0.0 {
            inner[i] = 0.0;
        } else {
            let d = 0.5 - a;
            outer[i] = if d > 0.0 { d * d } else { 0.0 };
            inner[i] = if d < 0.0 { d * d } else { 0.0 };
        }
    }
    edt(&mut outer, w, h);
    edt(&mut inner, w, h);
    (0..n)
        .map(|i| {
            let dist = outer[i].sqrt() - inner[i].sqrt();
            (255.0 * (0.5 - dist / (2.0 * spread))).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

fn edt(grid: &mut [f32], w: usize, h: usize) {
    let m = w.max(h);
    let (mut f, mut d, mut z) = (vec![0f32; m], vec![0f32; m], vec![0f32; m + 1]);
    let mut v = vec![0usize; m];
    for x in 0..w {
        for y in 0..h {
            f[y] = grid[y * w + x];
        }
        edt1d(&f, &mut d, &mut v, &mut z, h);
        for y in 0..h {
            grid[y * w + x] = d[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&grid[y * w..y * w + w]);
        edt1d(&f, &mut d, &mut v, &mut z, w);
        grid[y * w..y * w + w].copy_from_slice(&d[..w]);
    }
}

fn edt1d(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32], n: usize) {
    const INF: f32 = 1e20;
    let sq = |q: usize| (q * q) as f32;
    let mut k = 0usize;
    v[0] = 0;
    z[0] = -INF;
    z[1] = INF;
    for q in 1..n {
        let mut s = ((f[q] + sq(q)) - (f[v[k]] + sq(v[k]))) / (2.0 * q as f32 - 2.0 * v[k] as f32);
        while s <= z[k] {
            k -= 1;
            s = ((f[q] + sq(q)) - (f[v[k]] + sq(v[k]))) / (2.0 * q as f32 - 2.0 * v[k] as f32);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = INF;
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let dq = q as f32 - v[k] as f32;
        *out = dq * dq + f[v[k]];
    }
}
