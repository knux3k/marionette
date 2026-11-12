//! Job TypeScript: a typed, numbers-only subset of TypeScript compiled to WebAssembly and run
//! as deterministic data-parallel jobs (decision 0030; guide in `docs/ai/JOBS-TS.md`).
//!
//! Scripts (`marionette-script`) are interpreted and touch entities one at a time. Hot loops
//! over many items (particles, crowds, factory machines, belt slots) are written instead as
//! job functions over typed arrays:
//!
//! ```ts
//! export function integrate(pos: Float32Array, vel: Float32Array, dt: number): void {
//!   for (let i = 0; i < pos.length; i++) {
//!     pos[i] = pos[i] + vel[i] * dt;
//!   }
//! }
//! ```
//!
//! The compiler ([`compile`]) turns the file into a WebAssembly module; [`JobProgram`] runs
//! an exported function over chunks of the arrays on the [`marionette_jobs::Jobs`] pool,
//! each chunk in its own sandboxed instance. The same file is also valid TypeScript that a
//! JavaScript engine runs with bit-identical results (`Float32Array` rounding included),
//! which the tests check against the script interpreter.
//!
//! Hot reload: [`JobLibrary`] keeps compiled files by path and recompiles the ones whose
//! text changed; a file that no longer compiles keeps its last good version.

// Errors are `Diagnostic`s returned by value, as elsewhere in the engine; they are rare and
// carry the data an agent needs, so their size does not matter.
#![allow(clippy::result_large_err)]

pub mod compile;
mod runtime;

pub use compile::{Compiled, Elem, FunctionInfo, ParamType, Safety, codes, compile, compile_with};
pub use runtime::{Arg, DEFAULT_TIME_LIMIT_MS, JobProgram, signature};

use bevy_ecs::prelude::Resource;
use marionette_core::Diagnostic;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// TypeScript declarations for editors and `tsc` (the job prelude). A job file type-checks
/// with `tsc --noEmit --lib es2020 jobs.d.ts my.job.ts`.
pub const PRELUDE_DTS: &str = include_str!("jobs.d.ts");

/// Compiled job files by path, with hot reload.
#[derive(Resource, Default)]
pub struct JobLibrary {
    root: PathBuf,
    entries: BTreeMap<String, Entry>,
}

struct Entry {
    source: String,
    program: Arc<JobProgram>,
    /// Problems of the newest text, when it did not compile (the old program still runs).
    pending: Vec<Diagnostic>,
}

/// What [`JobLibrary::reload`] did with one file.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Reloaded {
    pub path: String,
    /// `unchanged`, `reloaded` or `failed` (the previous version keeps running).
    pub status: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<Diagnostic>,
}

impl JobLibrary {
    /// Paths are resolved against `root` (usually the scene's directory).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            entries: BTreeMap::new(),
        }
    }

    fn read(&self, path: &str) -> Result<String, Vec<Diagnostic>> {
        std::fs::read_to_string(self.root.join(path)).map_err(|e| {
            vec![
                Diagnostic::error(codes::READ, format!("cannot read the job file: {e}"))
                    .file(path)
                    .fix("paths are relative to the scene directory"),
            ]
        })
    }

    /// The program for `path`, compiling it on first use.
    pub fn get(&mut self, path: &str) -> Result<Arc<JobProgram>, Vec<Diagnostic>> {
        if let Some(e) = self.entries.get(path) {
            return Ok(e.program.clone());
        }
        let source = self.read(path)?;
        let program = Arc::new(JobProgram::compile(&source, path)?);
        self.entries.insert(
            path.to_string(),
            Entry {
                source,
                program: program.clone(),
                pending: Vec::new(),
            },
        );
        Ok(program)
    }

    /// Re-reads every loaded file and recompiles the changed ones. A file that fails keeps
    /// its previous program; its errors are reported here and by [`JobLibrary::pending`].
    pub fn reload(&mut self) -> Vec<Reloaded> {
        let paths: Vec<String> = self.entries.keys().cloned().collect();
        let mut out = Vec::new();
        for path in paths {
            let result = self.read(&path).and_then(|source| {
                if source == self.entries[&path].source {
                    return Ok(None);
                }
                let program = JobProgram::compile(&source, &path)?;
                Ok(Some((source, program)))
            });
            let entry = self.entries.get_mut(&path).expect("loaded");
            let (status, errors) = match result {
                Ok(None) => ("unchanged", Vec::new()),
                Ok(Some((source, program))) => {
                    entry.source = source;
                    entry.program = Arc::new(program);
                    entry.pending.clear();
                    ("reloaded", Vec::new())
                }
                Err(errors) => {
                    entry.pending = errors.clone();
                    ("failed", errors)
                }
            };
            out.push(Reloaded { path, status, errors });
        }
        out
    }

    /// Errors of files whose newest text did not compile.
    pub fn pending(&self, path: &str) -> &[Diagnostic] {
        self.entries.get(path).map_or(&[], |e| &e.pending)
    }

    /// The directory paths are resolved against.
    pub fn root(&self) -> &Path {
        &self.root
    }
}
