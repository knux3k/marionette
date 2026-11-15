# 0104: Safe Rust outside C calls, and the owner's code rules

Date: 2026-11-14. Status: accepted. Focuses 1 (modularity) and 3 (performance), decision
0060. Asked for by the owner during the optimization pass (decision 0100).

## Context

The owner set rules for engine code: no `unsafe` without their review (C FFI only), no
`unwrap`/`expect` outside tests, no lazy `.clone()`, flat concrete types before generics and
`Box<dyn>`, data structures only as complex as needed, clippy-clean. A survey of main at
`cbbdfbe` found about 100 `unsafe` sites in 13 files and no crate forbidding it; 177
`expect`, 16 `unwrap` and 3 `panic!` in non-test code; 66 reference-count clones written as
`.clone()`; 41 `Box<dyn>` (13 in the module and operation registry).

## Decision

1. `unsafe` stays only where the engine calls C libraries or the OS: Steam, OpenXR and Vulkan
   sharing, Windows and Unix process and idle calls, the counting allocator, QuickJS typed
   array memory, wgpu's experimental ray query switch (the owner chose to keep it). The files
   are listed with what they call in `UNSAFE_ALLOWED` (`repo_hygiene` test), which also fails
   on stale entries.
2. Every other crate inherits `[workspace.lints] rust.unsafe_code = "deny"` (`deny`, so the
   one test allocator can opt out by name and be listed).
3. Removed, with the owner's approval:
   - Jobs (`marionette-jobs-ts`): chunk calls got raw addresses with `unsafe impl Send/Sync`.
     Writable arrays are now split up front into one piece per chunk, each in its own
     uncontended `Mutex<Option<..>>` that the chunk's call takes; copies into job memory go
     through `bytemuck` byte views. Same results and same speed in `jobs_ts_bench` (checked
     1 thread 15.2 vs 15.4 ns per particle over three alternated runs, within noise).
   - Scripts (`marionette-script`): the world was reached through a raw pointer, and the host
     claimed `Send`/`Sync` for rquickjs saved handles. The world is now moved into the bridge
     for script calls and back out after them, swapping with one empty world kept by the host
     (no allocation per tick); module namespaces live in a JavaScript global object instead
     of saved handles, so the host is plain data that is `Send` and `Sync` by itself. The
     typed array byte copies use `bytemuck`; the QuickJS memory access itself stays (FFI).
     Same state hashes and the same `sim/scripts` timing and allocations per tick.
   - OpenXR vignette: a byte cast became `to_ne_bytes`.
4. The rules are in `docs/ai/CODE-RULES.md`, referenced from RULES.md (rule 16).
5. `unwrap`/`expect` and clone cleanup of older code goes crate by crate in separate changes;
   when a crate is clean it turns on `clippy::unwrap_used` and `clippy::expect_used`.

## Owner review of the QuickJS typed array copy (2026-11-15)

The one `unsafe` left in `marionette-script` (`copy_in`/`copy_out`) was reviewed with the owner,
who chose to keep it. It is one byte copy between a script's typed array buffer (QuickJS memory,
pointer and length from rquickjs `TypedArray::as_raw`) and a Rust `Vec`.

- Why safe Rust cannot do it: rquickjs 0.14 has no safe bulk accessor; `as_bytes` and
  `as_slice` are `unsafe` themselves. The safe route is one JavaScript property read per
  element (`Object::get(i)`).
- Measured (cloud container, release, `f32`): read 10,000 elements 2.75 us vs 421 us safe
  (153x); 100,000 elements 36 us vs 3.9 ms safe (109x). Write: a safe version exists (new typed
  array copy, then the target's JavaScript `set`), 16 us vs 48 us at 100,000 elements (about 3x).
- Invariants: the pointer is valid only while no JavaScript runs (JavaScript may write, detach
  or reallocate the buffer). Both copies run with the script paused and call back into nothing
  while the slice lives; a detached buffer gives `None` and copies nothing; `bytemuck` checks
  sizes, so no length or alignment is taken on trust; the write clamps to the shorter length.
- Any change to these functions, or a new QuickJS memory access, needs the owner's review again.

## Rejected

- `#![forbid(unsafe_code)]` per crate file: the workspace lint covers every target of a crate
  in one place and shows in `Cargo.toml`.
- A non-`Send` script host (`NonSend` resource): worlds move between threads (the MCP server,
  the window loop), where a non-send resource would panic.
