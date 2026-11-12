//! Deterministic data-parallel jobs over flat arrays.
//!
//! Hot simulation loops (factory machines, belt slots, crowds, particles) run as compiled
//! Rust over plain arrays instead of per-entity scripts, spread over CPU cores. The rules
//! that keep this deterministic (decision 0024):
//!
//! 1. **Work is split by the data, never by the thread count.** Chunk sizes and segment
//!    boundaries come from the caller; the same input always gives the same chunks.
//! 2. **A chunk only writes its own slice.** Rust's borrow rules enforce this: each closure
//!    gets a disjoint `&mut` slice.
//! 3. **Results are combined in chunk order.** [`Jobs::map_chunks`] returns one value per
//!    chunk, in order; fold them sequentially (floating-point sums included), so
//!    1 thread and 64 threads give bit-identical results.
//!
//! With one thread every call runs inline on the caller's thread (no pool), which is also
//! what headless tests and the state hash use by default; see [`Jobs::from_env`].

use bevy_ecs::prelude::Resource;
use rayon::prelude::*;
use std::ops::Range;
use std::sync::Arc;

/// Environment variable that sets the worker count (`1` = run inline, `0` or unset = one
/// per available core, capped at [`MAX_THREADS`]).
pub const THREADS_ENV: &str = "MARIONETTE_JOB_THREADS";
/// Upper bound on worker threads.
pub const MAX_THREADS: usize = 64;
/// Below this many elements a call runs inline even with a pool: handing work to other
/// threads costs microseconds, more than small loops take. This only decides *where*
/// chunks run, never how data is chunked, so results are the same either way.
pub const PARALLEL_MIN_LEN: usize = 65_536;

/// A pool of worker threads for deterministic jobs. Cheap to clone (shared pool).
///
/// Insert it as a resource (`world.insert_resource(Jobs::from_env())`) and take
/// `Res<Jobs>` in a system. It is not simulation state: it never enters the state hash, and
/// results never depend on how many threads it has.
#[derive(Resource, Clone)]
pub struct Jobs {
    pool: Option<Arc<rayon::ThreadPool>>,
    threads: usize,
}

impl std::fmt::Debug for Jobs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jobs").field("threads", &self.threads).finish()
    }
}

impl Default for Jobs {
    fn default() -> Self {
        Self::inline()
    }
}

impl Jobs {
    /// Runs every job on the calling thread, in chunk order.
    pub fn inline() -> Self {
        Self {
            pool: None,
            threads: 1,
        }
    }

    /// A pool with `threads` workers (`threads <= 1` is [`Jobs::inline`]).
    pub fn with_threads(threads: usize) -> Self {
        let threads = threads.clamp(1, MAX_THREADS);
        if threads == 1 {
            return Self::inline();
        }
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("marionette-job-{i}"))
            .build()
            .map(Arc::new)
            .ok();
        match pool {
            Some(pool) => Self {
                pool: Some(pool),
                threads,
            },
            // Threads could not be started (restricted environment): run inline.
            None => Self::inline(),
        }
    }

    /// Worker count from [`THREADS_ENV`], or one per core.
    pub fn from_env() -> Self {
        let wanted = std::env::var(THREADS_ENV)
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let threads = if wanted == 0 {
            std::thread::available_parallelism().map_or(1, |n| n.get())
        } else {
            wanted
        };
        Self::with_threads(threads)
    }

    /// One process-wide pool sized by [`Jobs::from_env`], created on first use and shared by
    /// every simulation (loading many scenes does not start many pools).
    pub fn shared() -> Self {
        static SHARED: std::sync::OnceLock<Jobs> = std::sync::OnceLock::new();
        SHARED.get_or_init(Self::from_env).clone()
    }

    /// Number of worker threads (1 when inline).
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Calls `f(chunk_index, chunk)` for every `chunk`-sized piece of `data` (the last may be
    /// shorter). Pieces may run at the same time on different threads; each sees only its
    /// own slice.
    pub fn for_each_chunk_mut<T, F>(&self, data: &mut [T], chunk: usize, f: F)
    where
        T: Send,
        F: Fn(usize, &mut [T]) + Sync + Send,
    {
        let chunk = chunk.max(1);
        match &self.pool {
            Some(pool) if data.len() > chunk && data.len() >= PARALLEL_MIN_LEN => {
                pool.install(|| data.par_chunks_mut(chunk).enumerate().for_each(|(i, c)| f(i, c)))
            }
            _ => data.chunks_mut(chunk).enumerate().for_each(|(i, c)| f(i, c)),
        }
    }

    /// Like [`Jobs::for_each_chunk_mut`] over two arrays of equal length, chunked together
    /// (structure-of-arrays data: `a[i]` and `b[i]` belong to the same item).
    pub fn for_each_chunk_mut2<A, B, F>(&self, a: &mut [A], b: &mut [B], chunk: usize, f: F)
    where
        A: Send,
        B: Send,
        F: Fn(usize, &mut [A], &mut [B]) + Sync + Send,
    {
        assert_eq!(
            a.len(),
            b.len(),
            "for_each_chunk_mut2 needs arrays of equal length"
        );
        let chunk = chunk.max(1);
        match &self.pool {
            Some(pool) if a.len() > chunk && a.len() >= PARALLEL_MIN_LEN => pool.install(|| {
                a.par_chunks_mut(chunk)
                    .zip(b.par_chunks_mut(chunk))
                    .enumerate()
                    .for_each(|(i, (x, y))| f(i, x, y))
            }),
            _ => a
                .chunks_mut(chunk)
                .zip(b.chunks_mut(chunk))
                .enumerate()
                .for_each(|(i, (x, y))| f(i, x, y)),
        }
    }

    /// Splits `data` at the given segment `ends` (ascending; the last must be `data.len()`)
    /// and calls `f(segment_index, segment)` for each segment. For data whose natural units
    /// have different sizes, such as the slots of belts of different lengths.
    pub fn for_each_segment_mut<T, F>(&self, data: &mut [T], ends: &[usize], f: F)
    where
        T: Send,
        F: Fn(usize, &mut [T]) + Sync + Send,
    {
        let len = data.len();
        let segments = split_segments(data, ends);
        match &self.pool {
            Some(pool) if segments.len() > 1 && len >= PARALLEL_MIN_LEN => {
                pool.install(|| segments.into_par_iter().enumerate().for_each(|(i, s)| f(i, s)))
            }
            _ => segments.into_iter().enumerate().for_each(|(i, s)| f(i, s)),
        }
    }

    /// Calls `f(range)` for every `chunk`-sized range of `0..len` and returns the results in
    /// range order. Fold them sequentially for a deterministic reduction.
    pub fn map_chunks<R, F>(&self, len: usize, chunk: usize, f: F) -> Vec<R>
    where
        R: Send,
        F: Fn(Range<usize>) -> R + Sync + Send,
    {
        let chunk = chunk.max(1);
        let ranges: Vec<Range<usize>> = (0..len.div_ceil(chunk))
            .map(|i| i * chunk..((i + 1) * chunk).min(len))
            .collect();
        match &self.pool {
            Some(pool) if ranges.len() > 1 && len >= PARALLEL_MIN_LEN => {
                pool.install(|| ranges.into_par_iter().map(&f).collect())
            }
            _ => ranges.into_iter().map(f).collect(),
        }
    }
}

/// Splits `data` into consecutive segments ending at `ends`.
fn split_segments<'a, T>(mut data: &'a mut [T], ends: &[usize]) -> Vec<&'a mut [T]> {
    let total = data.len();
    let mut out = Vec::with_capacity(ends.len());
    let mut at = 0;
    for &end in ends {
        assert!(
            end >= at && end <= total,
            "segment ends must ascend within the data"
        );
        let (head, tail) = data.split_at_mut(end - at);
        out.push(head);
        data = tail;
        at = end;
    }
    assert_eq!(at, total, "the last segment end must be the data length");
    out
}
