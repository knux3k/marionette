//! Repository rules that must hold for every tracked file:
//! - no absolute user paths, links to private chats/artifacts, or other patterns listed in
//!   `docs/ai/forbidden-patterns.txt`;
//! - no email addresses;
//! - every relative link and backticked repo path in `docs/ai/*.md` exists.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Tracked files (via git), or every file outside target/ and .git/ when git is unavailable.
fn files() -> Vec<PathBuf> {
    let root = repo();
    if let Ok(out) = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["ls-files", "-z"])
        .output()
        && out.status.success()
    {
        return out
            .stdout
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| root.join(String::from_utf8_lossy(s).as_ref()))
            .collect();
    }
    let mut all = Vec::new();
    let mut stack = vec![root];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                if name != "target" && name != ".git" {
                    stack.push(p);
                }
            } else {
                all.push(p);
            }
        }
    }
    all
}

fn is_email_at(line: &[u8], at: usize) -> bool {
    let local = |c: u8| c.is_ascii_alphanumeric() || b"._%+-".contains(&c);
    let domain = |c: u8| c.is_ascii_alphanumeric() || b".-".contains(&c);
    let before = line[..at].iter().rev().take_while(|c| local(**c)).count();
    let after: Vec<u8> = line[at + 1..]
        .iter()
        .take_while(|c| domain(**c))
        .copied()
        .collect();
    let Some(dot) = after.iter().rposition(|c| *c == b'.') else {
        return false;
    };
    let tld = &after[dot + 1..];
    before > 0 && dot > 0 && tld.len() >= 2 && tld.iter().all(u8::is_ascii_alphabetic)
}

#[test]
fn no_identifying_details_in_tracked_files() {
    let root = repo();
    let patterns: Vec<String> = std::fs::read_to_string(root.join("docs/ai/forbidden-patterns.txt"))
        .expect("docs/ai/forbidden-patterns.txt exists")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_lowercase)
        .collect();
    let mut problems = Vec::new();
    for f in files() {
        let rel = f
            .strip_prefix(&root)
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        if rel == "docs/ai/forbidden-patterns.txt" || rel == "Cargo.lock" {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        }; // binary files
        for (n, line) in text.lines().enumerate() {
            let lower = line.to_lowercase();
            for p in &patterns {
                if lower.contains(p.as_str()) {
                    problems.push(format!("{rel}:{}: pattern `{p}`", n + 1));
                }
            }
            let bytes = line.as_bytes();
            for (i, b) in bytes.iter().enumerate() {
                if *b == b'@' && is_email_at(bytes, i) {
                    problems.push(format!("{rel}:{}: email address", n + 1));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "identifying details found:\n{}",
        problems.join("\n")
    );
}

#[test]
fn email_detector_works() {
    let sample = format!("x a.b{}example.org y", char::from(64));
    assert!(is_email_at(sample.as_bytes(), 5));
    assert!(!is_email_at(b"uses: actions/checkout@v4", 22));
    assert!(!is_email_at(b"@group(0) @binding(0)", 0));
}

/// Relative markdown links `](path)` and backticked paths that look like repo files.
#[test]
fn every_path_in_ai_docs_exists() {
    let root = repo();
    let dir = root.join("docs/ai");
    let mut missing = Vec::new();
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "md") {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap();
        let mut candidates: Vec<(String, PathBuf)> = Vec::new();
        for part in text.split("](").skip(1) {
            let target = part
                .split(')')
                .next()
                .unwrap_or("")
                .split('#')
                .next()
                .unwrap_or("");
            if !target.is_empty() && !target.contains("://") {
                candidates.push((target.to_string(), p.parent().unwrap().join(target)));
            }
        }
        for (i, part) in text.split('`').enumerate() {
            let looks_like_path = i % 2 == 1
                && !part.contains(' ')
                && !part.contains('*')
                && !part.contains('<')
                && (part.starts_with("crates/")
                    || part.starts_with("docs/")
                    || part.starts_with("examples/")
                    || part.starts_with(".github/")
                    || ["README.md", "CLAUDE.md", "AGENTS.md", "Cargo.toml"].contains(&part));
            if looks_like_path {
                candidates.push((part.to_string(), root.join(part.trim_end_matches('/'))));
            }
        }
        for (shown, path) in candidates {
            if !path.exists() {
                missing.push(format!("{}: {shown}", p.file_name().unwrap().to_string_lossy()));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "paths in docs/ai that do not exist:\n{}",
        missing.join("\n")
    );
}

#[test]
fn generated_reference_docs_are_current() {
    let out = Command::new(env!("CARGO_BIN_EXE_marionette"))
        .current_dir(repo())
        .args(["docs", "--check", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "run `marionette docs` and commit: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// Decision 0104: `unsafe` only where the engine calls C libraries or the OS, each file
/// listed here with what it calls. Every other crate denies `unsafe_code` through
/// `[workspace.lints]`. Adding a file needs the owner's review first (`docs/ai/RULES.md`).
const UNSAFE_ALLOWED: &[(&str, &str)] = &[
    (
        "crates/marionette-cli/src/packaged.rs",
        "Windows console and message box calls",
    ),
    (
        "crates/marionette-host/src/alloc.rs",
        "counting global allocator over the system one",
    ),
    (
        "crates/marionette-host/src/child.rs",
        "OS process groups and job objects",
    ),
    (
        "crates/marionette-host/src/machine.rs",
        "OS idle and CPU time queries",
    ),
    (
        "crates/marionette-openxr/src/gpu.rs",
        "OpenXR and Vulkan device sharing with wgpu",
    ),
    (
        "crates/marionette-openxr/src/lib.rs",
        "loading the OpenXR runtime library",
    ),
    (
        "crates/marionette-render/src/gpu.rs",
        "wgpu's experimental ray query switch",
    ),
    (
        "crates/marionette-script/src/lib.rs",
        "reading and writing QuickJS typed array memory",
    ),
    (
        "crates/marionette-steam/src/ffi.rs",
        "loading the Steam library's C functions",
    ),
    (
        "crates/marionette-steam/src/lib.rs",
        "calling the Steam library's C functions",
    ),
    (
        "crates/marionette-synth/tests/realtime.rs",
        "per-thread counting allocator in a test",
    ),
];

#[test]
fn unsafe_code_only_in_listed_ffi_files() {
    let root = repo();
    let mut found = Vec::new();
    for path in files() {
        // This file names the patterns it looks for.
        if path.extension().is_none_or(|e| e != "rs") || path.ends_with("tests/repo_hygiene.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let uses = text.lines().any(|l| {
            let code = l.split("//").next().unwrap_or("");
            [
                "unsafe {",
                "unsafe impl",
                "unsafe fn",
                "unsafe extern",
                "allow(unsafe_code)",
            ]
            .iter()
            .any(|p| code.contains(p))
        });
        if uses {
            let rel = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            found.push(rel);
        }
    }
    let allowed: Vec<&str> = UNSAFE_ALLOWED.iter().map(|(f, _)| *f).collect();
    let extra: Vec<&String> = found.iter().filter(|f| !allowed.contains(&f.as_str())).collect();
    assert!(
        extra.is_empty(),
        "unsafe code outside the FFI list (decision 0104; needs the owner's review): {extra:?}"
    );
    let stale: Vec<&&str> = allowed
        .iter()
        .filter(|f| !found.iter().any(|x| x == *f))
        .collect();
    assert!(
        stale.is_empty(),
        "listed files no longer hold unsafe; remove them: {stale:?}"
    );
}
