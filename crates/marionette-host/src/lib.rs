//! What the engine holds from the operating system, and how it gives it back (decision 0032).
//!
//! The rule: when an engine process ends, normally or by a crash or a kill, the operating
//! system must be able to reclaim everything it held. Memory, file handles, sockets and GPU
//! objects are reclaimed by the OS on its own. Three things are not, and this crate owns them:
//!
//! - **Child processes.** [`spawn`] starts a child that the OS kills when the engine dies:
//!   on Windows the engine process sits in a kill-on-close job object ([`init`]), so every
//!   descendant dies with it; on Linux the child gets `PR_SET_PDEATHSIG = SIGKILL`, set from
//!   a spawner thread that lives as long as the process.
//! - **Files outside the project.** Temporary files go in this process's run folder
//!   ([`run_dir`], [`temp_dir`]): `<temp>/marionette/runs/<pid>-<n>/`, held by an OS file
//!   lock that dies with the process. [`init`] removes run folders whose owner is gone, so a
//!   crash leaves nothing behind after the next start; [`shutdown`] removes this process's
//!   folder on a clean exit.
//! - **Named OS objects** (shared memory, named pipes, ports, lock files elsewhere). The
//!   engine does not create any; new code must not either without a decision record. See
//!   `docs/ai/MEMORY-AND-CRASHES.md`.
//!
//! [`alloc::Counting`] is an opt-in global allocator that counts live bytes, used by the
//! leak tests and by anything that wants a cheap heap probe.

pub mod alloc;
mod child;
mod run;

pub use child::{contained, spawn};
pub use run::{RUN_ROOT_ENV, TempDir, run_dir, runs_root, shutdown, sweep_stale, sweep_stale_in, temp_dir};

use std::path::PathBuf;

/// What [`init`] did.
#[derive(Debug, Default)]
pub struct InitReport {
    /// Run folders of dead engine processes that were removed.
    pub swept: Vec<PathBuf>,
    /// Whether descendants of this process are tied to its life (see [`contained`]).
    pub contained: bool,
}

/// Call once at the start of every engine process (the CLI does); calling it again is
/// harmless. Ties future child processes to this process's life and removes leftovers of
/// engine processes that died without cleaning up.
pub fn init() -> InitReport {
    let contained = child::contain();
    InitReport {
        swept: sweep_stale(),
        contained,
    }
}
