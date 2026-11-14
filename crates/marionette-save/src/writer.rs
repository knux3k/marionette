//! Writing saves on a background thread (decision 0073: the tick pays only for the copy).
//!
//! Bounded: at most [`SaveWriter::QUEUE`] saves wait while one is written. A new save for
//! a slot that is still waiting replaces the waiting one (counted in `replaced`); a new
//! save for another slot when the queue is full waits on the calling thread for room.
//! Dropping the writer finishes every waiting save first, so quitting never loses one.

use crate::Pending;
use crate::store::{SaveStore, check_slot};
use marionette_core::{Diagnostic, Errors, Registry};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// The outcome of one background write.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WriteResult {
    pub slot: String,
    pub tick: u64,
    pub bytes: u64,
    /// Time on the writer thread (scene text, checksum, file), in milliseconds.
    pub ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Default)]
struct State {
    queue: VecDeque<(String, Pending)>,
    busy: bool,
    stop: bool,
    results: VecDeque<WriteResult>,
    replaced: u64,
}

struct Shared {
    state: Mutex<State>,
    cv: Condvar,
}

impl Shared {
    /// A poisoned lock only means a writer thread panicked mid-update; the queue and
    /// results are still well formed, so keep using them rather than panic again.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, st: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.cv.wait(st).unwrap_or_else(PoisonError::into_inner)
    }
}

pub struct SaveWriter {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    store: SaveStore,
}

impl SaveWriter {
    /// Saves waiting at most.
    pub const QUEUE: usize = 4;
    /// Results kept until [`SaveWriter::take_results`].
    pub const RESULTS: usize = 32;

    /// A writer into `store`. `registry` must be the registry of the simulations whose
    /// captures it gets (e.g. `registry_with(sim.modules())`).
    pub fn new(store: SaveStore, registry: Arc<Registry>) -> Result<Self, Errors> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            cv: Condvar::new(),
        });
        // The thread and this handle both own the queue (an `Arc`, so a refcount bump)
        // and the save folder path, which the thread keeps after `new` returns.
        let s = Arc::clone(&shared);
        let st = store.clone();
        let thread = std::thread::Builder::new()
            .name("marionette-save".into())
            .spawn(move || run(&s, &st, &registry))
            .map_err(|e| Diagnostic::error(crate::codes::IO, format!("cannot start the save thread: {e}")))?;
        Ok(Self {
            shared,
            thread: Some(thread),
            store,
        })
    }

    pub fn store(&self) -> &SaveStore {
        &self.store
    }

    /// Queues a save of `pending` into `slot` and returns at once (unless the queue is full
    /// of other slots, see the module docs).
    pub fn submit(&self, slot: &str, pending: Pending) -> Result<(), Errors> {
        check_slot(slot)?;
        let mut st = self.shared.lock();
        if let Some(waiting) = st.queue.iter_mut().find(|(s, _)| s == slot) {
            waiting.1 = pending;
            st.replaced += 1;
            return Ok(());
        }
        while st.queue.len() >= Self::QUEUE {
            st = self.shared.wait(st);
        }
        st.queue.push_back((slot.to_string(), pending));
        self.shared.cv.notify_all();
        Ok(())
    }

    /// Waits until every queued save is on disk.
    pub fn flush(&self) {
        let mut st = self.shared.lock();
        while !st.queue.is_empty() || st.busy {
            st = self.shared.wait(st);
        }
    }

    /// Results since the last call (at most [`SaveWriter::RESULTS`], newest kept).
    pub fn take_results(&self) -> Vec<WriteResult> {
        let mut st = self.shared.lock();
        st.results.drain(..).collect()
    }

    /// Waiting saves that a newer save of the same slot replaced.
    pub fn replaced(&self) -> u64 {
        self.shared.lock().replaced
    }
}

impl Drop for SaveWriter {
    fn drop(&mut self) {
        self.flush();
        self.shared.lock().stop = true;
        self.shared.cv.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(shared: &Shared, store: &SaveStore, registry: &Registry) {
    loop {
        let (slot, pending) = {
            let mut st = shared.lock();
            loop {
                if let Some(job) = st.queue.pop_front() {
                    st.busy = true;
                    shared.cv.notify_all();
                    break job;
                }
                if st.stop {
                    return;
                }
                st = shared.wait(st);
            }
        };
        let tick = pending.state.tick;
        let t = Instant::now();
        let result = pending
            .into_file(registry, unix_now())
            .and_then(|file| store.write(&slot, &file));
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let (bytes, error) = match result {
            Ok(b) => (b, None),
            Err(e) => (0, e.0.first().map(|d| format!("{}: {}", d.code, d.message))),
        };
        let mut st = shared.lock();
        if st.results.len() >= SaveWriter::RESULTS {
            st.results.pop_front();
        }
        st.results.push_back(WriteResult {
            slot,
            tick,
            bytes,
            ms,
            error,
        });
        st.busy = false;
        shared.cv.notify_all();
    }
}

/// Seconds since 1970, for headers (slot menus); 0 if the clock is before 1970.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
