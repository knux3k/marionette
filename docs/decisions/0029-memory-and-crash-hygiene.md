# 0029: Memory and crash hygiene

Date: 2026-11-12. Status: accepted.

## Context

The project owner asked for clean memory allocation with no leaks, and for the engine never to
be responsible for memory or other resources that stay unusable until reboot when the engine
or a game on it crashes. An audit of the code found:

- `Sim::step` never cleared bevy_ecs trackers, so the record of removed components grew every
  tick (the script system takes its resource out each tick): about 30 bytes per tick forever.
- `EventLog` kept every event of a run; an hour of the scripted example held about 72,000
  events (about 48 MB, measured by the stability work). Audio scanned the whole log each tick.
- Session logs grew with every operation on a session.
- The renderer's GPU asset cache never shrank.
- A panic inside an operation emptied the agent's operation table (it is moved out while a
  handler runs), so a long-running MCP server stopped working after one bug.
- A script with an endless loop hung the engine; a script calling an engine function at
  module top level panicked the process.
- Unpacked archives went to the system temp folder and stayed there after a crash. No child
  processes existed yet, but the FBX conversion and provider work will start them.

## Decision

- **New crate `crates/marionette-host`**: what the engine holds from the OS. `spawn` for
  child processes that die with the engine (Windows kill-on-close job object holding the
  engine process; Linux `PR_SET_PDEATHSIG` from a long-lived spawner thread); a per-process
  run folder locked with an OS file lock, swept at the next start when the lock is free;
  `init`/`shutdown`; `alloc::Counting`, a counting global allocator. Depends on `libc` (Unix)
  and `windows-sys` (Windows) only.
- **Rules for all code** in `docs/ai/MEMORY-AND-CRASHES.md`: children through `spawn`, temp
  files through `temp_dir`, no named OS objects, every history-like structure bounded.
- **Bounded logs.** `EventLog` is a ring of 16,384 events with an absolute `dropped` count;
  session logs keep 1,000 entries. Both trims depend on counts only, so determinism holds.
- **`Sim::step` clears trackers** after each tick, as a bevy App does per frame. No system
  reads removal records; all state hashes are unchanged.
- **Script limits.** 64 MB heap, 512 KB stack, and a work budget per call counted in QuickJS
  interrupt checks (deterministic for a given history). Panics under script calls are caught
  at the call. All become `script_error` events or load errors.
- **Panics in operations** become `agent.panic`; a session named in the input is closed.
- **Proof.** A counting-allocator leak test, a GPU object-count test, kill tests for children
  and run folders, and a CI job running the core, host and script tests under valgrind.

## Alternatives considered

- **Sanitizers (LeakSanitizer, AddressSanitizer) on nightly.** Need a nightly toolchain and
  rebuild the C code with instrumentation; valgrind finds the same definite leaks in QuickJS
  on stable and is one package on the CI image. Revisit if valgrind gets too slow.
- **Miri.** Cannot run the C script runtime or wgpu; the unsafe code it could check
  (allocator wrapper, script bridge) is small and covered by tests and valgrind.
- **Assigning each child to a job object after spawning (Windows).** Leaves a window where the
  child can start grandchildren outside the job. Putting the engine process itself in the job
  covers every descendant with no race; per-child assignment remains the fallback when the
  process cannot join a job.
- **Process groups and a kill on exit (Linux).** A crashed or killed process runs no exit
  code, so only a kernel mechanism (parent-death signal) holds. Grandchildren started by
  non-engine tools are not covered; engine binaries use `spawn` for their own children.
- **Run folders named by PID only.** PIDs are reused; the OS file lock is the reliable sign
  that the owner is alive.
- **Event log trimmed by age (ticks).** A tick-based bound does not bound memory when one tick
  produces many events; a count bound does.

## Consequences

- Readers of `EventLog` use `since`/`end`/`of_tick` or iterate; positions are absolute.
- `observe_events` gains `dropped`; `observe_stats` gains `script_heap_bytes`, `events_held`
  and `process_heap_bytes`.
- A script loop that does legitimate heavy work for more than about half a second in one call
  is stopped. Raise `STEP_BUDGET` with a recorded reason if a real game needs it.
- Not covered yet: memory budgets per scene (soak limits live in the stability suite), the
  stability of a window-mode game across scene switches (the window runs one scene today),
  VR runtime sessions (no OpenXR module yet; the OS releases them on process death).
