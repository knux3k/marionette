# 0072: Unity Burst jobs port to the game's own Rust module; the scanner sees jobs

Date: 2026-11-13. Status: accepted.

## Context

The porting tools (decision 0020) had only seen small hand-written samples. A real slice of a
large VR factory game was ported (kept private): its Burst factory tick (an `IJob` run on the
main thread over flat `NativeArray`s of machine structs and belt slots: machines with seven
input slots, smelters that pick a recipe from what arrives, assemblers, labs, splitters with
filters, storage crates, rate-limited hatches, power scaling, stacked items on fast belts, two
half-tick belt passes, per-machine stats counters), its logic-network tick (sensors, gates,
switches that turn machines off) and its stress-save layout. What happened:

- `port_scan` panicked on the first real folder: C# tuple types (`static readonly (string,
  float)[] table`, `(int a, int b) Split(..)`) were read as parameter lists.
- After that fix the scan reported 12 classes and 2 scripts and said nothing about the hot
  code: C# `struct`s were not scanned, so the three job structs and their data structs were
  invisible; `NativeArray`, `Allocator`, `[BurstCompile]`, `math.*`, `float3` were not in the
  tables and not even reported as unknown. A partial class over many files was reported once
  per file.
- The built-in factory module (decision 0026) covers a simpler model (one fixed recipe, four
  inputs, one output belt per machine); the game's rules did not fit it as data.
- Porting the job by hand to a game Rust module (decision 0062) was direct: `NativeArray<T>`
  to `Vec<T>` in one resource, `Execute` to a `Stage::Logic` system, numbered fields with
  switch accessors to fixed arrays, integer math unchanged. It ran deterministically (same
  state hash on 1 and 4 job threads), items flowed and logic switches turned machines off.

## Decision

- **Recipe**: a Unity job (`IJob`, `IJobFor`, `IJobParallelFor`, `IJobChunk`, `IJobEntity`,
  ...) ports to a system of the game's own Rust module, not to a script and not to job
  TypeScript, unless it is a per-entity number loop that fits `JobData` arrays (decision
  0070). PORTING.md section "Burst jobs" has the mapping.
- **Scanner**: C# `struct`s are scanned; job bases have the new role `job` (summary field
  `jobs`, a next step pointing to the recipe); C# `partial` classes are merged into one class
  (fields and methods of every part, base from whichever part names it); tuple types are
  skipped when looking for a parameter list. The Unity table gains Unity.Collections, Jobs,
  Burst and Mathematics entries (native containers `rewrite`, allocators and job attributes
  `ready`, `math.min/max/sqrt/...` `script`, `math.sin/cos/pow/...` `hazard`), the Mathematics
  value types, `Mathf.*ToInt` and instanced draws.
- **Scaffold**: `port_scaffold` writes `jobs.rs` next to the TypeScript skeletons when there
  are jobs: one Rust struct per job and per scanned struct its fields use (C# types mapped to
  Rust types, enums as `i32`, constants as comments), each method's original code as comments
  with `->` lines, and a real `execute` method. On the ported slice it compiled unchanged.

## Alternatives

- **Translate job C# to Rust automatically**: Burst code is close to Rust, but a translator
  that is right for all of C# is a large project; the skeleton plus an agent did the slice in
  about an hour with every original line in view. Rejected for now.
- **Port jobs to job TypeScript**: it has no structs and no 64-bit integers (decision 0030);
  array-of-struct job data would become dozens of parallel arrays. Kept for per-entity number
  loops only.
- **Grow the built-in factory module to the game's rules**: its rules are the game's own
  (seven slots, recipe picking, hatches); putting them in the engine helps one game. A game
  module keeps them with the game (focus 1).

## Consequences

- Focus 1: the documented path for Burst code is a game Rust module. Focus 2: one scan now
  shows the jobs instead of hiding them; partial classes no longer repeat. Focus 3: the
  ported tick of the stress layout measured 0.12 ms per tick at 4x (all four sites, logic
  included) and 0.2 ms at 8x, one thread, release, cloud container, against a 2 ms budget.
- Gaps found and left open: no automatic C#-to-Rust translation; the TypeScript skeleton of a
  large partial class is long (57 KB for one class); the built-in factory module cannot
  express recipe picking, splitters, crates, power or machine on/off as data.
- Saving was the real budget problem (decision 0073).
