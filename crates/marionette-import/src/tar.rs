//! A small, bounded tar reader (ustar, GNU long names, pax `path`), enough for
//! `.unitypackage` archives, plus a ustar writer used by tests and tools.
//!
//! The reader never touches the file system: it hands each regular file to a callback with
//! its declared name and a reader limited to its size. Callers decide where (or whether)
//! bytes go, so a hostile name cannot escape anywhere.

use std::io::{self, Read};

/// Limits applied while reading.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Largest single entry, in bytes.
    pub max_entry: u64,
    /// Largest sum of all entries, in bytes.
    pub max_total: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entry: 2 << 30,
            max_total: 16 << 30,
        }
    }
}

/// Why reading stopped.
#[derive(Debug)]
pub enum TarError {
    Io(io::Error),
    Corrupt(String),
    TooLarge(String),
    /// Returned by the callback; reading stops.
    Callback(String),
}

impl std::fmt::Display for TarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TarError::Io(e) => write!(f, "read error: {e}"),
            TarError::Corrupt(s) => write!(f, "corrupt archive: {s}"),
            TarError::TooLarge(s) => write!(f, "too large: {s}"),
            TarError::Callback(s) => write!(f, "{s}"),
        }
    }
}

impl From<io::Error> for TarError {
    fn from(e: io::Error) -> Self {
        TarError::Io(e)
    }
}

fn field_str(b: &[u8]) -> String {
    let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn parse_octal(b: &[u8]) -> Result<u64, TarError> {
    if b.first().is_some_and(|c| c & 0x80 != 0) {
        // Base-256 (GNU) for large sizes.
        let mut v: u64 = (b[0] & 0x7f) as u64;
        for c in &b[1..] {
            v = v
                .checked_mul(256)
                .and_then(|v| v.checked_add(*c as u64))
                .ok_or_else(|| TarError::Corrupt("size overflow".into()))?;
        }
        return Ok(v);
    }
    let s = field_str(b);
    let s = s.trim_matches(|c: char| c == ' ' || c == '\0');
    if s.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(s, 8).map_err(|_| TarError::Corrupt(format!("bad octal field `{s}`")))
}

fn checksum_ok(h: &[u8; 512]) -> bool {
    let Ok(stored) = parse_octal(&h[148..156]) else {
        return false;
    };
    let sum: u64 = h
        .iter()
        .enumerate()
        .map(|(i, b)| if (148..156).contains(&i) { 32 } else { *b as u64 })
        .sum();
    sum == stored
}

fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<bool, TarError> {
    let mut got = 0;
    while got < buf.len() {
        let n = r.read(&mut buf[got..])?;
        if n == 0 {
            if got == 0 {
                return Ok(false);
            }
            return Err(TarError::Corrupt("truncated header".into()));
        }
        got += n;
    }
    Ok(true)
}

fn skip<R: Read>(r: &mut R, n: u64) -> Result<(), TarError> {
    let copied = io::copy(&mut r.take(n), &mut io::sink())?;
    if copied != n {
        return Err(TarError::Corrupt("truncated entry".into()));
    }
    Ok(())
}

fn pad(size: u64) -> u64 {
    (512 - size % 512) % 512
}

fn read_small<R: Read>(r: &mut R, size: u64) -> Result<Vec<u8>, TarError> {
    if size > 1 << 20 {
        return Err(TarError::TooLarge("metadata entry over 1 MiB".into()));
    }
    let mut v = vec![0; size as usize];
    r.read_exact(&mut v)
        .map_err(|_| TarError::Corrupt("truncated metadata entry".into()))?;
    skip(r, pad(size))?;
    Ok(v)
}

fn pax_path(data: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(data);
    let mut rest: &str = &text;
    while !rest.is_empty() {
        let (len, _) = rest.split_once(' ')?;
        let n: usize = len.parse().ok()?;
        let record = rest.get(..n)?;
        rest = &rest[n..];
        let body = record.split_once(' ')?.1.trim_end_matches('\n');
        if let Some(p) = body.strip_prefix("path=") {
            return Some(p.to_string());
        }
    }
    None
}

/// Reads every regular file in the archive, calling `f(name, size, reader)` for each. The
/// callback must read at most `size` bytes (the reader is limited); unread bytes are skipped.
pub fn read_entries<R: Read>(
    mut r: R,
    limits: Limits,
    mut f: impl FnMut(&str, u64, &mut dyn Read) -> Result<(), TarError>,
) -> Result<(), TarError> {
    let mut total: u64 = 0;
    let mut long_name: Option<String> = None;
    let mut header = [0u8; 512];
    loop {
        if !read_full(&mut r, &mut header)? {
            return Ok(()); // EOF without end blocks: accept.
        }
        if header.iter().all(|b| *b == 0) {
            return Ok(());
        }
        if !checksum_ok(&header) {
            return Err(TarError::Corrupt("header checksum mismatch".into()));
        }
        let size = parse_octal(&header[124..136])?;
        let kind = header[156];
        match kind {
            b'L' => {
                let data = read_small(&mut r, size)?;
                long_name = Some(field_str(&data));
                continue;
            }
            b'x' => {
                let data = read_small(&mut r, size)?;
                if let Some(p) = pax_path(&data) {
                    long_name = Some(p);
                }
                continue;
            }
            _ => {}
        }
        let name = long_name.take().unwrap_or_else(|| {
            let base = field_str(&header[0..100]);
            let prefix = if &header[257..262] == b"ustar" {
                field_str(&header[345..500])
            } else {
                String::new()
            };
            if prefix.is_empty() {
                base
            } else {
                format!("{prefix}/{base}")
            }
        });
        let regular = kind == b'0' || kind == 0 || kind == b'7';
        if !regular {
            skip(&mut r, size + pad(size))?;
            continue;
        }
        if size > limits.max_entry {
            return Err(TarError::TooLarge(format!("entry `{name}` is {size} bytes")));
        }
        total = total.saturating_add(size);
        if total > limits.max_total {
            return Err(TarError::TooLarge(format!(
                "archive content over {} bytes",
                limits.max_total
            )));
        }
        let mut limited = (&mut r).take(size);
        f(&name, size, &mut limited)?;
        let left = limited.limit();
        skip(&mut r, left + pad(size))?;
    }
}

/// Writes a ustar archive of regular files (names under 100 bytes). For tests and tools.
pub fn write_ustar(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in entries {
        assert!(name.len() < 100, "write_ustar supports names under 100 bytes");
        let mut h = [0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..107].copy_from_slice(b"0000644");
        h[108..115].copy_from_slice(b"0000000");
        h[116..123].copy_from_slice(b"0000000");
        h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
        h[136..147].copy_from_slice(b"00000000000");
        h[156] = b'0';
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|b| *b as u32).sum();
        h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(data);
        out.resize(out.len() + pad(data.len() as u64) as usize, 0);
    }
    out.resize(out.len() + 1024, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let a = write_ustar(&[("abc/asset", b"hello"), ("abc/pathname", b"Assets/x.png")]);
        let mut seen = Vec::new();
        read_entries(&a[..], Limits::default(), |n, s, r| {
            let mut v = Vec::new();
            r.read_to_end(&mut v)?;
            assert_eq!(v.len() as u64, s);
            seen.push((n.to_string(), v));
            Ok(())
        })
        .unwrap();
        assert_eq!(seen[0], ("abc/asset".into(), b"hello".to_vec()));
        assert_eq!(seen[1].1, b"Assets/x.png");
    }

    #[test]
    fn limits_and_corruption_are_errors() {
        let a = write_ustar(&[("a", &[1u8; 100])]);
        let small = Limits {
            max_entry: 10,
            max_total: 10,
        };
        assert!(matches!(
            read_entries(&a[..], small, |_, _, _| Ok(())),
            Err(TarError::TooLarge(_))
        ));
        let mut bad = a.clone();
        bad[0] ^= 0xff;
        assert!(matches!(
            read_entries(&bad[..], Limits::default(), |_, _, _| Ok(())),
            Err(TarError::Corrupt(_))
        ));
    }
}
