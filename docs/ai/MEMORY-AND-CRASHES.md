# Memory and crashes

Normative rules (decision 0032). Two promises to the project owner:

1. **No leaks.** Repeated work (loading and unloading scenes, hot-reloading scripts, spawning
   and destroying entities, loading assets, rendering, long runs) gives its memory back. Memory
   in use levels off; it never grows with time or with the number of times something was done.
2. **Nothing outlives a crash.** If an engine process or a game on it crashes or is killed,
   the engine leaves nothing that stays in use until reboot. Everything it held is either
   reclaimed by the OS when the process dies or removed by the next engine start.

## What the OS reclaims by itself, and what it does not

Reclaimed when a process dies, whatever the cause: its heap, threads, open files and file
locks, sockets and listening ports, GPU objects and the driver's per-process memory, audio
devices, VR runtime sessions held through the OS (the runtime sees the client disconnect).

Not reclaimed, so the engine must manage them:

| Thing | Rule | Mechanism |
| --- | --- | --- |
| Child processes | Start every one with `marionette_host::spawn` (or `marionette_host::output` to run to completion), never `Command::spawn`, `output` or `status`. | Windows: the engine process is in a kill-on-close job object (`marionette_host::init`), inherited by all descendants. Linux: `PR_SET_PDEATHSIG = SIGKILL`, set on a spawner thread that lives as long as the process. |
| Files outside the project | Write them under `marionette_host::temp_dir(tag)` (or `run_dir()`), never straight into the system temp folder. | Per-process run folder `<temp>/marionette/runs/<pid>-<n>/` held by an OS file lock; `init` sweeps folders whose lock is free; `shutdown` removes this process's folder on a clean exit. `MARIONETTE_RUN_ROOT` moves the root (tests). |
| Named OS objects (named shared memory, named pipes, semaphores, lock files outside the run folder, fixed TCP ports) | Do not create them. If one is ever needed, write a decision record first and make the next start clean it up. | None exist today. |
| Detached processes (meant to outlive the engine, e.g. opening a browser) | Not allowed without a decision record. | - |

Every engine binary calls `marionette_host::init()` first (the CLI does in `main`) and
`marionette_host::shutdown()` on the way out. A panic or kill skips `shutdown`; the next start
sweeps.

## Bounded growth inside a process

Anything that records history must have a bound:

- `EventLog` (`crates/marionette-core/src/module.rs`) keeps the last `EventLog::KEEP` (16,384)
  events as a ring; `dropped` counts the rest. Readers keep absolute positions (`end()`,
  `since(pos)`) or read one tick (`of_tick`). `observe_events` reports `dropped`.
- Session logs keep the last `session::LOG_KEEP` (1,000) entries; `seq` stays absolute.
- `Sim::step` calls `World::clear_trackers()` after every tick. Without it bevy_ecs keeps a
  record of every removed component forever (resources are entities, so taking a resource
  out counts). Systems must not rely on `RemovedComponents` across ticks.
- The renderer's GPU asset cache grows with the assets drawn; call `Renderer::trim(frame)` (or
  `Offscreen::trim`) after switching scenes.
- Kept on purpose, freed with their session: input recordings (one entry per action change),
  named snapshots (`session_snapshot`). Stopping the session frees them.

New caches and logs: give them a bound or a documented owner that frees them, and add them to
the leak test.

## Scripts and game errors fail as data

- Script heap: `marionette_script::MEMORY_LIMIT` (64 MB) per world. Past it, allocation throws
  inside the script; the error becomes a `script_error` event and the tick goes on.
- Work per call: `STEP_BUDGET` interrupt checks (about half a second of looping). An endless
  loop is stopped with `script.budget`; at load time it is a load error.
- Stack: `STACK_LIMIT` (512 KB); deep recursion throws `RangeError`.
- A Rust panic under a script call (engine bug, or an engine function used at module top
  level) is caught at the call and reported as `script.engine_panic`.
- `Agent::call` catches panics in operations: the result is an `agent.panic` diagnostic, the
  operation table stays intact, and a live session named in the input is closed because its
  state may be half-updated. The MCP server keeps serving.

## Proofs (rerun these)

| What | Test |
| --- | --- |
| Heap flat over session load/unload, long runs, hot reload, entity churn, screenshots | `crates/marionette-agent/tests/leaks.rs` (counting allocator; one test per binary) |
| GPU objects come back after repeated setups and scene switches | `gpu_objects_come_back_after_repeated_setups_and_trims` in `crates/marionette-render/tests/render.rs` |
| Children die when the engine is killed; killed run folders are swept | `crates/marionette-host/tests/crash.rs` |
| A CLI run killed mid-simulation is cleaned up by the next start | `crates/marionette-cli/tests/crash.rs` |
| Misbehaving scripts (endless loop, out of memory, deep recursion, engine call at load) | `crates/marionette-script/tests/misbehaving.rs` |
| Panicking operation | `a_panicking_operation_is_an_error_and_the_agent_keeps_serving` in `crates/marionette-agent/src/lib.rs` |
| No leaks in the C script runtime (QuickJS) | CI job `leaks (valgrind)` in `.github/workflows/ci.yml` |

Run the valgrind job locally on Linux:

```bash
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind --leak-check=full --errors-for-leak-kinds=definite --show-leak-kinds=definite --error-exitcode=9 -q" \
  cargo test -p marionette-core -p marionette-host -p marionette-script -- --test-threads=1
```

## Measuring memory

- Rust heap: `marionette_host::alloc::stats()` (`live_bytes`, `peak_bytes`, ...) in binaries
  that install `marionette_host::alloc::Counting` as `#[global_allocator]` (the `marionette`
  binary does). `observe_stats` reports it as `process_heap_bytes`.
- Script heap: `marionette_script::heap_bytes(world)`, `observe_stats.script_heap_bytes`.
- GPU: `Gpu::live_objects()` (buffers, textures, views, bind groups, samplers, pipelines).
- Event log size: `observe_stats.events_held`.
