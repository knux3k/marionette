//! Focus 2 (decision 0061): an agent should load as little as a task needs. The docs every
//! session reads stay under a fixed size, and no hand-written doc grows without bound. When a
//! limit fails, split the doc by task or move old detail out (proofs to PROOFS.md, history to
//! SESSION-LOG.md or git); do not raise the limit without a decision record.

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn size(rel: &str) -> usize {
    std::fs::read_to_string(repo().join(rel))
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
        .replace("\r\n", "\n")
        .len()
}

/// What START-HERE.md tells every session to read, plus the rules file the client loads.
const ALWAYS_READ: &[&str] = &[
    "CLAUDE.md",
    "docs/ai/START-HERE.md",
    "docs/ai/RULES.md",
    "docs/ai/FOCUSES.md",
    "docs/ai/STATE.md",
];

/// Bytes for the whole always-read set (about 6,000 tokens).
const ALWAYS_READ_LIMIT: usize = 24 * 1024;
/// Bytes for STATE.md alone: status and UNVERIFIED only.
const STATE_LIMIT: usize = 12 * 1024;
/// Bytes for any other hand-written doc in `docs/ai/` (read one section at a time).
const DOC_LIMIT: usize = 32 * 1024;

#[test]
fn always_read_docs_fit_the_budget() {
    let sizes: Vec<(&str, usize)> = ALWAYS_READ.iter().map(|f| (*f, size(f))).collect();
    let total: usize = sizes.iter().map(|(_, s)| s).sum();
    assert!(
        total <= ALWAYS_READ_LIMIT,
        "always-read docs are {total} bytes, limit {ALWAYS_READ_LIMIT}: {sizes:?}"
    );
    let state = size("docs/ai/STATE.md");
    assert!(
        state <= STATE_LIMIT,
        "docs/ai/STATE.md is {state} bytes, limit {STATE_LIMIT}: move proofs to PROOFS.md, \
         history to SESSION-LOG.md, and drop UNVERIFIED items that are now checked"
    );
}

#[test]
fn every_hand_written_doc_stays_splittable() {
    let dir = repo().join("docs/ai");
    let mut over = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "md") {
            let rel = format!("docs/ai/{}", path.file_name().unwrap().to_string_lossy());
            let s = size(&rel);
            if s > DOC_LIMIT {
                over.push((rel, s));
            }
        }
    }
    assert!(
        over.is_empty(),
        "docs over {DOC_LIMIT} bytes (split them by task): {over:?}"
    );
}

#[test]
fn start_here_names_every_always_read_doc() {
    let text = std::fs::read_to_string(repo().join("docs/ai/START-HERE.md")).unwrap();
    for f in ["RULES.md", "FOCUSES.md", "STATE.md"] {
        assert!(text.contains(&format!("]({f})")), "START-HERE.md must link {f}");
    }
}
