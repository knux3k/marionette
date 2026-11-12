//! A small, bounded ZIP reader (stored and deflate entries) for the archives CC0 libraries
//! publish (ambientCG material zips, Kenney and Quaternius packs). It never touches the file
//! system: entries come back as names and bytes, and callers choose where anything goes.

use flate2::read::DeflateDecoder;
use std::io::Read;

/// Limits that keep a hostile archive from exhausting memory.
#[derive(Debug, Clone, Copy)]
pub struct ZipLimits {
    pub max_entries: usize,
    pub max_entry: u64,
    pub max_total: u64,
}

impl Default for ZipLimits {
    fn default() -> Self {
        Self {
            max_entries: 20_000,
            max_entry: 512 << 20,
            max_total: 2 << 30,
        }
    }
}

/// One file of an archive.
#[derive(Debug, Clone)]
pub struct ZipEntry {
    /// Path inside the archive, `/` separators, checked (no absolute paths, no `..`).
    pub name: String,
    pub data: Vec<u8>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Checks an archive path; returns the clean form.
pub fn clean_name(raw: &str) -> Option<String> {
    let n = raw.replace('\\', "/");
    if n.starts_with('/') || n.contains(':') {
        return None;
    }
    let parts: Vec<&str> = n.split('/').filter(|p| !p.is_empty() && *p != ".").collect();
    if parts.is_empty() || parts.contains(&"..") {
        return None;
    }
    Some(parts.join("/"))
}

/// Reads every file entry (folders are skipped). Errors are plain messages.
pub fn read(bytes: &[u8], limits: ZipLimits) -> Result<Vec<ZipEntry>, String> {
    // End of central directory: the last "PK\x05\x06" within 64 KiB + 22 bytes of the end.
    let min = bytes.len().saturating_sub(22 + 65_535);
    let eocd = (min..bytes.len().saturating_sub(21))
        .rev()
        .find(|&i| bytes[i..].starts_with(b"PK\x05\x06"))
        .ok_or("not a ZIP archive (no end of central directory)")?;
    let count = u16_at(bytes, eocd + 10).ok_or("truncated archive")? as usize;
    let cd_offset = u32_at(bytes, eocd + 16).ok_or("truncated archive")? as usize;
    if count == 0xffff || cd_offset == 0xffff_ffff {
        return Err("ZIP64 archives are not supported".into());
    }
    if count > limits.max_entries {
        return Err(format!(
            "archive has {count} entries, more than {}",
            limits.max_entries
        ));
    }
    let mut out = Vec::new();
    let mut total: u64 = 0;
    let mut at = cd_offset;
    for _ in 0..count {
        if !bytes.get(at..).is_some_and(|b| b.starts_with(b"PK\x01\x02")) {
            return Err("broken central directory".into());
        }
        let flags = u16_at(bytes, at + 8).ok_or("truncated")?;
        let method = u16_at(bytes, at + 10).ok_or("truncated")?;
        let csize = u32_at(bytes, at + 20).ok_or("truncated")? as usize;
        let usize_ = u32_at(bytes, at + 24).ok_or("truncated")? as u64;
        let nlen = u16_at(bytes, at + 28).ok_or("truncated")? as usize;
        let elen = u16_at(bytes, at + 30).ok_or("truncated")? as usize;
        let clen = u16_at(bytes, at + 32).ok_or("truncated")? as usize;
        let local = u32_at(bytes, at + 42).ok_or("truncated")? as usize;
        let raw_name = bytes.get(at + 46..at + 46 + nlen).ok_or("truncated name")?;
        let raw_name = String::from_utf8_lossy(raw_name).into_owned();
        at += 46 + nlen + elen + clen;
        if raw_name.ends_with('/') {
            continue;
        }
        if flags & 1 != 0 {
            return Err(format!("`{raw_name}` is encrypted"));
        }
        let name = clean_name(&raw_name).ok_or_else(|| format!("unsafe path `{raw_name}` in archive"))?;
        if usize_ > limits.max_entry {
            return Err(format!("`{name}` is larger than {} bytes", limits.max_entry));
        }
        total += usize_;
        if total > limits.max_total {
            return Err(format!("archive expands to more than {} bytes", limits.max_total));
        }
        if !bytes.get(local..).is_some_and(|b| b.starts_with(b"PK\x03\x04")) {
            return Err(format!("broken local header for `{name}`"));
        }
        let lnlen = u16_at(bytes, local + 26).ok_or("truncated")? as usize;
        let lelen = u16_at(bytes, local + 28).ok_or("truncated")? as usize;
        let start = local + 30 + lnlen + lelen;
        let comp = bytes
            .get(start..start + csize)
            .ok_or_else(|| format!("truncated data for `{name}`"))?;
        let data = match method {
            0 => comp.to_vec(),
            8 => {
                let mut d = Vec::with_capacity(usize_.min(1 << 24) as usize);
                DeflateDecoder::new(comp)
                    .take(usize_ + 1)
                    .read_to_end(&mut d)
                    .map_err(|e| format!("cannot inflate `{name}`: {e}"))?;
                d
            }
            m => {
                return Err(format!(
                    "`{name}` uses compression method {m} (only stored and deflate are read)"
                ));
            }
        };
        if data.len() as u64 != usize_ {
            return Err(format!("`{name}` has the wrong size after decompression"));
        }
        out.push(ZipEntry { name, data });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Writes a stored (uncompressed) ZIP. Used by tests to build archives.
pub fn write_stored(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let crc = |d: &[u8]| {
        let mut c: u32 = 0xffff_ffff;
        for &b in d {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    (c >> 1) ^ 0xedb8_8320
                } else {
                    c >> 1
                };
            }
        }
        !c
    };
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = out.len() as u32;
        let c = crc(data);
        let mut h = Vec::new();
        h.extend_from_slice(b"PK\x03\x04");
        h.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        h.extend_from_slice(&c.to_le_bytes());
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&(name.len() as u16).to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(data);
        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&c.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    let cd_len = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_len.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_bad_archives() {
        let z = write_stored(&[("a/License.txt", b"CC0"), ("a/b.png", b"\x89PNG")]);
        let e = read(&z, ZipLimits::default()).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].name, "a/License.txt");
        assert_eq!(e[0].data, b"CC0");
        assert!(read(b"not a zip", ZipLimits::default()).is_err());
        let evil = write_stored(&[("../x", b"1")]);
        assert!(read(&evil, ZipLimits::default()).is_err());
        let tight = ZipLimits {
            max_entry: 1,
            ..Default::default()
        };
        assert!(read(&z, tight).is_err());
        // Deflate entries.
        use flate2::{Compression, write::DeflateEncoder};
        use std::io::Write;
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&[7u8; 1000]).unwrap();
        let comp = enc.finish().unwrap();
        let mut z = write_stored(&[("x.bin", &comp)]);
        // Patch method to deflate and the uncompressed size to 1000 (local + central).
        z[8] = 8;
        z[22..26].copy_from_slice(&1000u32.to_le_bytes());
        let cd = z.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        z[cd + 10] = 8;
        z[cd + 24..cd + 28].copy_from_slice(&1000u32.to_le_bytes());
        let e = read(&z, ZipLimits::default()).unwrap();
        assert_eq!(e[0].data, vec![7u8; 1000]);
    }
}
