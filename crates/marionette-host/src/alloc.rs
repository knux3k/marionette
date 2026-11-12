//! Allocation counting. A binary (or test binary) opts in with
//!
//! ```ignore
//! #[global_allocator]
//! static ALLOC: marionette_host::alloc::Counting = marionette_host::alloc::Counting;
//! ```
//!
//! and reads [`stats`]. Counting costs a few relaxed atomic adds per allocation. Without the
//! `#[global_allocator]` line [`stats`] returns zeros. Memory that C libraries allocate with
//! their own `malloc` (QuickJS, Vulkan drivers) is not seen here; the script heap is measured
//! by the script runtime and the GPU by wgpu (see `docs/ai/MEMORY-AND-CRASHES.md`).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE_BLOCKS: AtomicU64 = AtomicU64::new(0);
static TOTAL_ALLOCS: AtomicU64 = AtomicU64::new(0);
static PEAK_BYTES: AtomicU64 = AtomicU64::new(0);

/// The system allocator, counted.
pub struct Counting;

/// Heap counters since the process started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AllocStats {
    /// Bytes currently allocated.
    pub live_bytes: u64,
    /// Allocations currently live.
    pub live_blocks: u64,
    /// Allocations ever made (reallocations count once each).
    pub total_allocs: u64,
    /// Highest `live_bytes` since start or the last [`reset_peak`].
    pub peak_bytes: u64,
}

/// Current counters. All zero unless [`Counting`] is the global allocator.
pub fn stats() -> AllocStats {
    AllocStats {
        live_bytes: LIVE_BYTES.load(Relaxed),
        live_blocks: LIVE_BLOCKS.load(Relaxed),
        total_allocs: TOTAL_ALLOCS.load(Relaxed),
        peak_bytes: PEAK_BYTES.load(Relaxed),
    }
}

/// Starts a new peak measurement from the current live bytes.
pub fn reset_peak() {
    PEAK_BYTES.store(LIVE_BYTES.load(Relaxed), Relaxed);
}

fn grow(n: u64) {
    let now = LIVE_BYTES.fetch_add(n, Relaxed) + n;
    PEAK_BYTES.fetch_max(now, Relaxed);
}

// SAFETY: every call forwards to `System` with the same arguments; the counters do not
// allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            TOTAL_ALLOCS.fetch_add(1, Relaxed);
            LIVE_BLOCKS.fetch_add(1, Relaxed);
            grow(layout.size() as u64);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            TOTAL_ALLOCS.fetch_add(1, Relaxed);
            LIVE_BLOCKS.fetch_add(1, Relaxed);
            grow(layout.size() as u64);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE_BLOCKS.fetch_sub(1, Relaxed);
        LIVE_BYTES.fetch_sub(layout.size() as u64, Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            TOTAL_ALLOCS.fetch_add(1, Relaxed);
            let old = layout.size() as u64;
            let new = new_size as u64;
            if new >= old {
                grow(new - old);
            } else {
                LIVE_BYTES.fetch_sub(old - new, Relaxed);
            }
        }
        p
    }
}
