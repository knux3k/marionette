//! High dynamic range images for environment lighting (decision 0091): Radiance `.hdr`
//! (RGBE, decoded here, no dependency) and OpenEXR `.exr` (cargo feature `exr`, on by
//! default). Pixels are linear RGB floats, unclamped.

use std::path::Path;

/// Linear RGB floats, row-major from the top row.
#[derive(Debug, Clone, PartialEq)]
pub struct HdrImage {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<[f32; 3]>,
}

/// True when `name` names an HDR image (`.hdr` or `.exr`, any case).
pub fn is_hdr_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".hdr") || lower.ends_with(".exr")
}

/// Loads an `.hdr` or `.exr` file relative to `base_dir`.
pub fn load_hdr_image(name: &str, base_dir: &Path) -> Result<HdrImage, String> {
    let path = base_dir.join(name);
    if name.to_ascii_lowercase().ends_with(".exr") {
        return load_exr(&path);
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    decode_radiance(&bytes).map_err(|e| format!("cannot decode {}: {e}", path.display()))
}

#[cfg(feature = "exr")]
fn load_exr(path: &Path) -> Result<HdrImage, String> {
    use exr::prelude::*;
    let image = read_first_rgba_layer_from_file(
        path,
        |res, _| (res.width(), vec![[0.0f32; 3]; res.width() * res.height()]),
        |(w, px), pos, (r, g, b, _a): (f32, f32, f32, f32)| {
            px[pos.y() * *w + pos.x()] = [r, g, b];
        },
    )
    .map_err(|e| format!("cannot decode {}: {e}", path.display()))?;
    let size = image.layer_data.size;
    Ok(HdrImage {
        width: size.width() as u32,
        height: size.height() as u32,
        rgb: image.layer_data.channel_data.pixels.1,
    })
}

#[cfg(not(feature = "exr"))]
fn load_exr(path: &Path) -> Result<HdrImage, String> {
    Err(format!(
        "cannot decode {}: this build has no OpenEXR support (cargo feature `exr`); use a .hdr file",
        path.display()
    ))
}

/// Decodes a Radiance RGBE file (`#?RADIANCE` or `#?RGBE`, 32-bit_rle_rgbe, `-Y h +X w`).
pub fn decode_radiance(bytes: &[u8]) -> Result<HdrImage, String> {
    let mut at = 0usize;
    let mut line = || -> Option<&[u8]> {
        let start = at;
        let end = start + bytes[start..].iter().position(|&b| b == b'\n')?;
        at = end + 1;
        Some(&bytes[start..end])
    };
    let magic = line().ok_or("empty file")?;
    if !magic.starts_with(b"#?") {
        return Err("not a Radiance HDR file (missing #? header)".into());
    }
    loop {
        let l = line().ok_or("header has no end")?;
        if l.is_empty() {
            break;
        }
        if let Some(f) = l.strip_prefix(b"FORMAT=")
            && f != b"32-bit_rle_rgbe"
        {
            return Err(format!("unsupported format {}", String::from_utf8_lossy(f)));
        }
    }
    let res = String::from_utf8_lossy(line().ok_or("missing resolution line")?).to_string();
    let parts: Vec<&str> = res.split_whitespace().collect();
    let (h, w) = match parts.as_slice() {
        ["-Y", h, "+X", w] => (
            h.parse::<usize>().map_err(|_| "bad height")?,
            w.parse::<usize>().map_err(|_| "bad width")?,
        ),
        _ => return Err(format!("unsupported orientation `{res}` (only -Y h +X w)")),
    };
    if w == 0 || h == 0 || w > 1 << 15 || h > 1 << 15 {
        return Err(format!("unsupported size {w}x{h}"));
    }
    let data = &bytes[at..];
    let mut pos = 0usize;
    let mut rgbe = vec![[0u8; 4]; w * h];
    let mut byte = || -> Result<u8, String> {
        let b = *data.get(pos).ok_or("file ends early")?;
        pos += 1;
        Ok(b)
    };
    for y in 0..h {
        let row = &mut rgbe[y * w..(y + 1) * w];
        let first = [byte()?, byte()?, byte()?, byte()?];
        let rle = (8..0x8000).contains(&w) && first[0] == 2 && first[1] == 2 && first[2] & 0x80 == 0;
        if !rle {
            // Flat (old style, no run-length encoding).
            row[0] = first;
            for px in row.iter_mut().skip(1) {
                *px = [byte()?, byte()?, byte()?, byte()?];
            }
            continue;
        }
        if ((first[2] as usize) << 8 | first[3] as usize) != w {
            return Err("scanline width mismatch".into());
        }
        // Four channels, each run-length encoded separately.
        for c in 0..4 {
            let mut x = 0;
            while x < w {
                let n = byte()? as usize;
                if n > 128 {
                    let count = n - 128;
                    let v = byte()?;
                    if x + count > w {
                        return Err("bad run".into());
                    }
                    for px in &mut row[x..x + count] {
                        px[c] = v;
                    }
                    x += count;
                } else {
                    if n == 0 || x + n > w {
                        return Err("bad run".into());
                    }
                    for px in &mut row[x..x + n] {
                        px[c] = byte()?;
                    }
                    x += n;
                }
            }
        }
    }
    let rgb = rgbe
        .iter()
        .map(|&[r, g, b, e]| {
            if e == 0 {
                [0.0; 3]
            } else {
                let f = 2f32.powi(e as i32 - 136);
                [r as f32 * f, g as f32 * f, b as f32 * f]
            }
        })
        .collect();
    Ok(HdrImage {
        width: w as u32,
        height: h as u32,
        rgb,
    })
}

/// Encodes linear RGB as a flat (not run-length encoded) Radiance file. For tests and tools.
pub fn encode_radiance(width: u32, height: u32, rgb: &[[f32; 3]]) -> Vec<u8> {
    let mut out = format!("#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y {height} +X {width}\n").into_bytes();
    for &[r, g, b] in rgb {
        let m = r.max(g).max(b);
        if m < 1e-32 {
            out.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        let e = m.log2().floor() as i32 + 1;
        let f = 256.0 / 2f32.powi(e);
        let q = |v: f32| (v * f).clamp(0.0, 255.0) as u8;
        out.extend_from_slice(&[q(r), q(g), q(b), (e + 128) as u8]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radiance_round_trip_keeps_values_above_one() {
        let px = vec![
            [0.0, 0.5, 1.0],
            [4.0, 20.0, 1000.0],
            [0.01, 0.02, 0.03],
            [1.0, 1.0, 1.0],
        ];
        let back = decode_radiance(&encode_radiance(2, 2, &px)).unwrap();
        assert_eq!((back.width, back.height), (2, 2));
        for (a, b) in px.iter().zip(&back.rgb) {
            for c in 0..3 {
                assert!((a[c] - b[c]).abs() <= a[c].abs() * 0.02 + 1e-3, "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn radiance_run_length_scanlines_decode() {
        // One 8-pixel scanline, RLE: each channel is one run of 8 (count 128 + 8).
        let mut f = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 8\n".to_vec();
        f.extend_from_slice(&[2, 2, 0, 8]);
        for v in [128u8, 64, 32, 129] {
            f.extend_from_slice(&[136, v]);
        }
        let img = decode_radiance(&f).unwrap();
        assert_eq!(img.rgb.len(), 8);
        // 128 * 2^(129-136) = 1.0
        assert!((img.rgb[7][0] - 1.0).abs() < 1e-6 && (img.rgb[7][2] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn radiance_rejects_other_files() {
        assert!(decode_radiance(b"\x89PNG\r\n").is_err());
        assert!(decode_radiance(b"#?RADIANCE\n\n+Y 2 +X 2\n").is_err());
    }
}
