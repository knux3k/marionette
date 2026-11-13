//! A small zip writer (deflate, Unix modes kept, fixed timestamps) so the same folder always
//! gives the same zip bytes. Covers what packages need: plain files, under 4 GiB each and in
//! total (no zip64).

use flate2::Compression;
use flate2::write::DeflateEncoder;
use std::io::{self, Write};
use std::path::Path;

/// 1980-01-01 00:00 in DOS date and time (the zip epoch): fixed so zips are reproducible.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;

struct Entry {
    name: String,
    crc: u32,
    compressed: u32,
    size: u32,
    offset: u32,
    mode: u32,
}

/// Zips every file under `dir` (sorted, under the top folder `top`) into `out`, written
/// through a temporary file and renamed at the end.
pub fn write_dir(dir: &Path, top: &str, out: &Path) -> io::Result<()> {
    let mut files = Vec::new();
    collect(dir, "", &mut files)?;
    files.sort();
    let tmp = out.with_extension("zip.partial");
    let result = write_files(dir, top, &files, &tmp);
    match result {
        Ok(()) => std::fs::rename(&tmp, out),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn collect(root: &Path, rel: &str, out: &mut Vec<String>) -> io::Result<()> {
    let here = if rel.is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    for e in std::fs::read_dir(&here)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        let path = if rel.is_empty() {
            name
        } else {
            format!("{rel}/{name}")
        };
        if e.file_type()?.is_dir() {
            collect(root, &path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

fn too_big() -> io::Error {
    io::Error::other("zip over 4 GiB (zip64 is not supported)")
}

fn mode_of(path: &Path) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = std::fs::metadata(path) {
            return m.permissions().mode() & 0o777;
        }
    }
    let _ = path;
    0o644
}

fn write_files(dir: &Path, top: &str, files: &[String], out: &Path) -> io::Result<()> {
    let mut w = io::BufWriter::new(std::fs::File::create(out)?);
    let mut entries = Vec::new();
    let mut offset: u64 = 0;
    for rel in files {
        let path = dir.join(rel);
        let data = std::fs::read(&path)?;
        let mut crc = flate2::Crc::new();
        crc.update(&data);
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&data)?;
        let packed = enc.finish()?;
        let name = format!("{top}/{rel}");
        let e = Entry {
            name,
            crc: crc.sum(),
            compressed: u32::try_from(packed.len()).map_err(|_| too_big())?,
            size: u32::try_from(data.len()).map_err(|_| too_big())?,
            offset: u32::try_from(offset).map_err(|_| too_big())?,
            mode: mode_of(&path),
        };
        let mut h = Vec::with_capacity(30 + e.name.len());
        h.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        h.extend_from_slice(&20u16.to_le_bytes()); // version needed
        h.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        h.extend_from_slice(&8u16.to_le_bytes()); // deflate
        h.extend_from_slice(&DOS_TIME.to_le_bytes());
        h.extend_from_slice(&DOS_DATE.to_le_bytes());
        h.extend_from_slice(&e.crc.to_le_bytes());
        h.extend_from_slice(&e.compressed.to_le_bytes());
        h.extend_from_slice(&e.size.to_le_bytes());
        h.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(e.name.as_bytes());
        w.write_all(&h)?;
        w.write_all(&packed)?;
        offset += (h.len() + packed.len()) as u64;
        entries.push(e);
    }
    let start = u32::try_from(offset).map_err(|_| too_big())?;
    let mut size: u64 = 0;
    for e in &entries {
        let mut c = Vec::with_capacity(46 + e.name.len());
        c.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        c.extend_from_slice(&((3u16 << 8) | 20).to_le_bytes()); // made by Unix, 2.0
        c.extend_from_slice(&20u16.to_le_bytes());
        c.extend_from_slice(&0x0800u16.to_le_bytes());
        c.extend_from_slice(&8u16.to_le_bytes());
        c.extend_from_slice(&DOS_TIME.to_le_bytes());
        c.extend_from_slice(&DOS_DATE.to_le_bytes());
        c.extend_from_slice(&e.crc.to_le_bytes());
        c.extend_from_slice(&e.compressed.to_le_bytes());
        c.extend_from_slice(&e.size.to_le_bytes());
        c.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
        c.extend_from_slice(&0u16.to_le_bytes()); // extra
        c.extend_from_slice(&0u16.to_le_bytes()); // comment
        c.extend_from_slice(&0u16.to_le_bytes()); // disk
        c.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        c.extend_from_slice(&((0o100000 | e.mode) << 16).to_le_bytes());
        c.extend_from_slice(&e.offset.to_le_bytes());
        c.extend_from_slice(e.name.as_bytes());
        w.write_all(&c)?;
        size += c.len() as u64;
    }
    let count = u16::try_from(entries.len()).map_err(|_| io::Error::other("more than 65,535 files"))?;
    let mut end = Vec::with_capacity(22);
    end.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes());
    end.extend_from_slice(&count.to_le_bytes());
    end.extend_from_slice(&count.to_le_bytes());
    end.extend_from_slice(&u32::try_from(size).map_err(|_| too_big())?.to_le_bytes());
    end.extend_from_slice(&start.to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes());
    w.write_all(&end)?;
    w.into_inner().map_err(|e| e.into_error())?.sync_all()
}
