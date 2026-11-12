//! Moving bytes for the online sources. Requests and answers are plain data
//! (`marionette_providers::http`); a [`Transport`] sends them. The CLI uses [`Curl`], which runs
//! the system `curl` (shipped with Windows 10 and later and with every common Linux), so the
//! engine needs no TLS stack of its own and honours the user's proxy settings. Tests use
//! [`Scripted`]. Downloading CC0 files is free; no spending gate applies.

pub use marionette_providers::http::{HttpRequest, HttpResponse, NoNetwork, Transport};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Identifies the client to the libraries' APIs (Poly Haven asks for a unique agent).
pub const USER_AGENT: &str = "marionette-engine-cc0-pool";
/// Largest single download accepted (bytes).
pub const MAX_DOWNLOAD: u64 = 1 << 30;

/// The system `curl`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Curl;

impl Transport for Curl {
    fn available(&self) -> bool {
        let mut cmd = std::process::Command::new("curl");
        cmd.arg("--version");
        marionette_host::output(cmd).is_ok_and(|o| o.status.success())
    }

    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, String> {
        if !request.url.starts_with("https://") {
            return Err(format!("only https URLs are fetched, not `{}`", request.url));
        }
        let mut cmd = std::process::Command::new("curl");
        cmd.args([
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-redirs",
            "5",
            "--max-time",
            "600",
            "--max-filesize",
            &MAX_DOWNLOAD.to_string(),
            "--user-agent",
            USER_AGENT,
            "--write-out",
            "\n%{http_code}",
        ]);
        for h in &request.headers {
            cmd.arg("--header").arg(format!("{}: {}", h.name, h.value));
        }
        cmd.arg("--").arg(&request.url);
        // Through marionette_host so curl dies with the engine (decision 0032).
        let out = marionette_host::output(cmd).map_err(|e| format!("cannot run curl: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "curl failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let mut body = out.stdout;
        let nl = body
            .iter()
            .rposition(|b| *b == b'\n')
            .ok_or("curl gave no status")?;
        let status: u16 = std::str::from_utf8(&body[nl + 1..])
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .ok_or("curl gave no status")?;
        body.truncate(nl);
        Ok(HttpResponse { status, body })
    }
}

/// Answers from a fixed table (URL to response); unknown URLs are a network error. Records
/// every URL asked for.
#[derive(Debug, Default)]
pub struct Scripted {
    pub answers: BTreeMap<String, HttpResponse>,
    pub asked: Mutex<Vec<String>>,
}

impl Scripted {
    pub fn with(mut self, url: &str, status: u16, body: impl Into<Vec<u8>>) -> Self {
        self.answers.insert(
            url.to_string(),
            HttpResponse {
                status,
                body: body.into(),
            },
        );
        self
    }
}

impl Transport for Scripted {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, String> {
        self.asked.lock().expect("lock").push(request.url.clone());
        self.answers
            .get(&request.url)
            .cloned()
            .ok_or_else(|| format!("no network in this test for {}", request.url))
    }
}

/// GET with the pool's error handling.
pub fn get(t: &dyn Transport, url: &str) -> Result<Vec<u8>, String> {
    if !t.available() {
        return Err("no network transport is available (the system `curl` was not found)".into());
    }
    let r = t.send(&HttpRequest::get(url).header("user-agent", USER_AGENT))?;
    if !(200..300).contains(&r.status) {
        return Err(format!("{url} answered HTTP {}", r.status));
    }
    Ok(r.body)
}

/// The host of an https URL, lowercased.
pub fn host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = &rest[..end];
    if host.contains('@') || host.is_empty() {
        return None;
    }
    Some(host.split(':').next().unwrap_or(host).to_ascii_lowercase())
}

/// True when `url` is https on `domain` or one of its subdomains.
pub fn on_domain(url: &str, domain: &str) -> bool {
    host(url).is_some_and(|h| h == domain || h.ends_with(&format!(".{domain}")))
}

/// MD5 as lowercase hex (Poly Haven publishes MD5 sums; used only to check downloads).
pub fn md5_hex(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
        5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15,
        21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
        0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
        0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
        0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
        0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
        0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
        0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
    ];
    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(K[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    [a0, b0, c0, d0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_and_hosts() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(
            md5_hex(b"The quick brown fox jumps over the lazy dog"),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        assert!(on_domain("https://dl.polyhaven.org/file/x.jpg", "polyhaven.org"));
        assert!(!on_domain("https://evilpolyhaven.org/x", "polyhaven.org"));
        assert!(!on_domain("http://polyhaven.org/x", "polyhaven.org"));
        // URL with user info (written in parts so it does not read as an address).
        let with_user = format!("https://user{}polyhaven.org/x", '@');
        assert!(!on_domain(&with_user, "polyhaven.org"));
        assert_eq!(
            host("https://AmbientCG.com:443/get?x"),
            Some("ambientcg.com".into())
        );
    }
}
