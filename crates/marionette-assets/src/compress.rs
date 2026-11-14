//! GPU block compression of model textures (decision 0283): BC7, which desktop GPUs and the
//! Steam Deck sample directly. It stores 8 bits per texel instead of 32, so textures take a
//! quarter of the memory and bandwidth. GPUs without BC formats keep uncompressed textures. Encoding is slow, so results are kept in a bounded disk
//! cache keyed by the texels; a texture is encoded once per machine.

use crate::Image;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

/// A block format the GPU samples directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Codec {
    /// BC7 (BPTC), 4x4 texels in 16 bytes.
    Bc7,
}

impl Codec {
    fn tag(self) -> u8 {
        match self {
            Codec::Bc7 => 1,
        }
    }
}

/// Block width and height, in texels.
pub const BLOCK: u32 = 4;
/// Bytes per block.
pub const BLOCK_BYTES: u32 = 16;

/// A compressed texture: blocks of level 0 first, then each mip level.
#[derive(Debug, Clone, PartialEq)]
pub struct Blocks {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Vec<u8>>,
}

impl Blocks {
    /// Bytes on the GPU.
    pub fn bytes(&self) -> usize {
        self.levels.iter().map(Vec::len).sum()
    }
}

/// Size of level `level` of a `width` x `height` texture, at least 1.
pub fn level_size(width: u32, height: u32, level: usize) -> (u32, u32) {
    ((width >> level).max(1), (height >> level).max(1))
}

/// Blocks across and down of a level (its size rounded up to whole blocks).
pub fn blocks_of(w: u32, h: u32) -> (u32, u32) {
    (w.div_ceil(BLOCK), h.div_ceil(BLOCK))
}

/// Compresses `image` and its mip levels (`mips`, largest first, as [`crate::mips::chain`]
/// makes them). `None` when level 0 is not a whole number of blocks: GPUs need that, and
/// such textures stay uncompressed.
pub fn compress(image: &Image, mips: &[Vec<u8>], codec: Codec) -> Option<Blocks> {
    if image.width == 0
        || image.height == 0
        || !image.width.is_multiple_of(BLOCK)
        || !image.height.is_multiple_of(BLOCK)
        || image.rgba.len() != (4 * image.width * image.height) as usize
    {
        return None;
    }
    let alpha = image.rgba.as_chunks::<4>().0.iter().any(|t| t[3] != 255);
    let mut levels = Vec::with_capacity(1 + mips.len());
    levels.push(encode(&image.rgba, image.width, image.height, codec, alpha));
    for (i, texels) in mips.iter().enumerate() {
        let (w, h) = level_size(image.width, image.height, i + 1);
        if texels.len() != (4 * w * h) as usize {
            break;
        }
        levels.push(encode(texels, w, h, codec, alpha));
    }
    Some(Blocks {
        codec,
        width: image.width,
        height: image.height,
        levels,
    })
}

/// Encodes one level. Sizes that are not whole blocks (small mips) are padded by repeating
/// the last row and column. Rows of blocks are split across the CPU's threads.
fn encode(texels: &[u8], w: u32, h: u32, codec: Codec, alpha: bool) -> Vec<u8> {
    let (bw, bh) = blocks_of(w, h);
    let (pw, ph) = (bw * BLOCK, bh * BLOCK);
    let padded;
    let data: &[u8] = if (pw, ph) == (w, h) {
        texels
    } else {
        let mut p = vec![0u8; (4 * pw * ph) as usize];
        for y in 0..ph {
            for x in 0..pw {
                let s = (4 * (y.min(h - 1) * w + x.min(w - 1))) as usize;
                let d = (4 * (y * pw + x)) as usize;
                p[d..d + 4].copy_from_slice(&texels[s..s + 4]);
            }
        }
        padded = p;
        &padded
    };
    let row_bytes = (bw * BLOCK_BYTES) as usize;
    let mut out = vec![0u8; row_bytes * bh as usize];
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()) as u32;
    let rows_per = bh.div_ceil(threads.clamp(1, bh));
    std::thread::scope(|scope| {
        for (i, chunk) in out.chunks_mut(row_bytes * rows_per as usize).enumerate() {
            let first = i as u32 * rows_per * BLOCK;
            let rows = (chunk.len() / row_bytes) as u32 * BLOCK;
            let start = (4 * first * pw) as usize;
            let strip = &data[start..start + (4 * rows * pw) as usize];
            scope.spawn(move || {
                let surface = intel_tex_2::RgbaSurface {
                    data: strip,
                    width: pw,
                    height: rows,
                    stride: 4 * pw,
                };
                let settings = match (codec, alpha) {
                    (Codec::Bc7, true) => intel_tex_2::bc7::alpha_basic_settings(),
                    (Codec::Bc7, false) => intel_tex_2::bc7::opaque_basic_settings(),
                };
                intel_tex_2::bc7::compress_blocks_into(&settings, &surface, chunk);
            });
        }
    });
    out
}

/// Format version of cache files; bump when the encoder or its settings change.
const VERSION: u32 = 1;
const MAGIC: &[u8; 4] = b"MTXC";

/// Default size limit of the texture cache.
pub const CACHE_LIMIT: u64 = 2 << 30;

/// The texture cache folder: `MARIONETTE_TEXTURE_CACHE` if set (`off` turns the cache off),
/// else `marionette/textures` in the user's cache folder (`%LOCALAPPDATA%` on Windows,
/// `$XDG_CACHE_HOME` or `~/.cache` elsewhere).
pub fn cache_dir() -> Option<PathBuf> {
    let var = |k| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(v) = var("MARIONETTE_TEXTURE_CACHE") {
        return (v != Path::new("off")).then_some(v);
    }
    let base = if cfg!(windows) {
        var("LOCALAPPDATA")
    } else {
        var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|h| h.join(".cache")))
    }?;
    Some(base.join("marionette").join("textures"))
}

/// [`compress`] through the disk cache in `dir` (none: no cache), which is kept under
/// `limit` bytes by deleting the least recently used files.
pub fn compress_cached(
    image: &Image,
    mips: &[Vec<u8>],
    codec: Codec,
    dir: Option<&Path>,
    limit: u64,
) -> Option<Blocks> {
    let Some(dir) = dir else {
        return compress(image, mips, codec);
    };
    let key = key(image, mips, codec);
    let path = dir.join(format!("{key}.mtxc"));
    if let Some(b) = read(&path, image, codec) {
        // Mark it used, so eviction keeps it.
        if let Ok(f) = std::fs::File::options().write(true).open(&path) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        return Some(b);
    }
    let blocks = compress(image, mips, codec)?;
    if write(dir, &path, &blocks).is_ok() {
        evict(dir, limit);
    }
    Some(blocks)
}

fn key(image: &Image, mips: &[Vec<u8>], codec: Codec) -> String {
    let mut h = Sha256::new();
    h.update(VERSION.to_le_bytes());
    h.update([codec.tag(), image.srgb as u8]);
    h.update(image.width.to_le_bytes());
    h.update(image.height.to_le_bytes());
    h.update(&image.rgba);
    for m in mips {
        h.update((m.len() as u64).to_le_bytes());
        h.update(m);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn read(path: &Path, image: &Image, codec: Codec) -> Option<Blocks> {
    let bytes = std::fs::read(path).ok()?;
    let (head, mut rest) = bytes.split_at_checked(13)?;
    if &head[..4] != MAGIC
        || u32::from_le_bytes(head[4..8].try_into().ok()?) != VERSION
        || head[8] != codec.tag()
    {
        return None;
    }
    let count = u32::from_le_bytes(head[9..13].try_into().ok()?) as usize;
    let mut levels = Vec::with_capacity(count.min(32));
    for level in 0..count {
        let (w, h) = level_size(image.width, image.height, level);
        let (bw, bh) = blocks_of(w, h);
        let (data, more) = rest.split_at_checked((bw * bh * BLOCK_BYTES) as usize)?;
        levels.push(data.to_vec());
        rest = more;
    }
    (rest.is_empty() && !levels.is_empty()).then_some(Blocks {
        codec,
        width: image.width,
        height: image.height,
        levels,
    })
}

/// Writes to a temporary name, then renames, so a crash never leaves a half file.
fn write(dir: &Path, path: &Path, b: &Blocks) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(MAGIC)?;
    f.write_all(&VERSION.to_le_bytes())?;
    f.write_all(&[b.codec.tag()])?;
    f.write_all(&(b.levels.len() as u32).to_le_bytes())?;
    for l in &b.levels {
        f.write_all(l)?;
    }
    drop(f);
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Deletes the least recently used cache files until the folder is under three quarters
/// of `limit`, once it is over `limit`.
fn evict(dir: &Path, limit: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "mtxc"))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= limit {
        return;
    }
    files.sort();
    for (_, len, path) in files {
        if total <= limit / 4 * 3 {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32, alpha: bool) -> Image {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let a = if alpha { (x * 255 / w) as u8 } else { 255 };
                rgba.extend([(x * 255 / w) as u8, (y * 255 / h) as u8, 128, a]);
            }
        }
        Image {
            width: w,
            height: h,
            rgba,
            srgb: true,
        }
    }

    #[test]
    fn levels_are_whole_blocks_and_odd_sizes_are_refused() {
        let img = image(16, 8, false);
        let mips = crate::mips::chain(&img, crate::mips::MipKind::Linear);
        let b = compress(&img, &mips, Codec::Bc7).unwrap();
        let sizes: Vec<usize> = b.levels.iter().map(Vec::len).collect();
        // 16x8, 8x4, 4x2, 2x1, 1x1: 8, 2, 1, 1, 1 blocks.
        assert_eq!(sizes, [128, 32, 16, 16, 16]);
        assert!(compress(&image(10, 8, false), &[], Codec::Bc7).is_none());
    }

    #[test]
    fn cache_returns_the_same_blocks_and_stays_under_its_limit() {
        let tmp = marionette_host::temp_dir("mtxc").unwrap();
        let dir = tmp.path().join("cache");
        let img = image(32, 32, true);
        let fresh = compress(&img, &[], Codec::Bc7).unwrap();
        let a = compress_cached(&img, &[], Codec::Bc7, Some(&dir), CACHE_LIMIT).unwrap();
        let b = compress_cached(&img, &[], Codec::Bc7, Some(&dir), CACHE_LIMIT).unwrap();
        assert_eq!(a, fresh);
        assert_eq!(b, fresh);
        // Over the limit, old files go.
        for i in 0..8u8 {
            let mut other = image(32, 32, false);
            other.rgba[0] = i;
            compress_cached(&other, &[], Codec::Bc7, Some(&dir), 4 * 1040).unwrap();
        }
        let total: u64 = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.metadata().unwrap().len())
            .sum();
        assert!(total <= 4 * 1040, "cache holds {total} bytes");
    }
}
