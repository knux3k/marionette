# 0030: Job TypeScript, a numbers-only TypeScript subset compiled to WebAssembly

Date: 2026-11-12. Status: accepted.

## Context

Decision 0024 gave the engine deterministic data-parallel jobs with Rust job bodies (the
counterpart of Unity's job system plus Burst). Game code is TypeScript (decision 0022), run by
an interpreter: right for gameplay logic, about 200 times slower than Rust for hot loops over
many items. Game authors (people and AI agents) should be able to write those loops in the
same language as their scripts and get close to native speed, without Rust and without
giving up determinism, sandboxing or hot reload.

## Decision

- Crate `crates/marionette-jobs-ts`. Job files are `*.job.ts`: ordinary TypeScript restricted
  to a typed, numbers-only subset (guide: `docs/ai/JOBS-TS.md`). The compiler parses with oxc
  (already used by scripts), checks types and writes WebAssembly (`wasm-encoder`) in one pass
  per function. wasmtime (Cranelift) compiles and runs it.
- Types: `number` (64-bit float, exactly JavaScript's), `i32` (declared by the prelude as
  `type i32 = number`), `boolean`, and `Float32Array`, `Float64Array`, `Int32Array` as
  parameters only. No strings, objects, closures, classes, allocation or globals.
- JavaScript semantics are kept wherever they decide a result: `number` math is 64-bit, typed
  array loads widen and stores round (`Float32Array`) or apply ToInt32 (`Int32Array`), `/` is
  always a float division, bitwise operators use ToInt32, `%` on numbers is `fmod`,
  `Math.round` rounds half up, transcendental `Math` functions are the same `libm` ones the
  script runtime uses. Consequence, tested: the same file run by the script interpreter and
  compiled gives bit-identical arrays. Known difference: `i32` `+ - *` wrap on overflow (use
  `Math.imul` for hashing, as in JavaScript).
- Unannotated `let x = 0` is `i32`; if the function stores a fraction in it, the compiler
  widens that one variable to `number` and compiles the function again. Indexes must be `i32`,
  conditions must be `boolean` (no truthiness).
- Running: `JobProgram::run(jobs, function, len, chunk, args)` splits the run with
  `Jobs::map_chunks` (decision 0024 rules: chunks from the data, results in chunk order). Each
  chunk call takes a wasmtime instance from a pool, copies its chunk of every array in, calls,
  copies writable arrays back. Arguments: chunked arrays (writable or read-only), whole
  read-only arrays (copied once per run and instance), numbers, `i32`, booleans and the chunk's
  start index.
- Determinism: WebAssembly arithmetic is exactly specified except NaN bits; the compiler makes
  every stored NaN canonical (and the runtime every returned one), so results do not depend on
  the CPU. No threads or atomics inside a job; relaxed SIMD is deterministic; host math is
  `libm`.
- Safety levels, as Burst's safety checks: `Checked` (default: index checks that stop the job
  with the line, and a per-chunk time limit through epoch interruption) and `Unchecked`
  (neither; still sandboxed in the job's own memory and still deterministic).
- Errors are data with stable codes `jobts.*` (`syntax`, `unsupported`, `type`,
  `unknown_name`, `signature`, `top_level`, `read`, `no_function`, `args`, `trap`, `timeout`,
  `internal`), each with file, line, column and a fix written for an agent to apply.
- Hot reload: `JobLibrary` keeps programs by path and recompiles changed files; a file that no
  longer compiles keeps its last good program and reports the errors. Compiling a job file
  takes about 5 ms.
- Operations: `jobs_check`, `jobs_run` (run a function on JSON data, any thread count),
  `jobs_types` (writes `jobs.d.ts`, also generated into `docs/ai/reference/`).

## Alternatives considered

- AssemblyScript: a TypeScript-like language compiled to WebAssembly, but not TypeScript (its
  own number types change the meaning of ordinary code, its standard library allocates), a
  Node toolchain to ship, and no bit-equality with the script interpreter.
- A JIT for all scripts (V8 or similar): large, hard to sandbox deterministically, and
  per-entity script calls would still dominate.
- Writing Rust job bodies only: fastest, but leaves game authors needing a second language.
- Cranelift directly instead of WebAssembly: saves the copy in and out, but loses the sandbox
  and the well-specified semantics that make determinism easy to argue.
- NaN canonicalization after every operation (wasmtime's option): 2.5 times slower on the
  benchmark; canonicalizing at stores gives the same guarantee for state.

## Measured (`cargo run --release -p marionette-jobs-ts --example jobs_ts_bench`)

Particle step (gravity, drag, bouncy floor), 1,000,000 particles, 4-core cloud container,
best step of 20, all versions bit-identical:

| Version | ns per particle | vs Rust, 1 thread |
| --- | --- | --- |
| Rust job body, 1 thread | 3.0 | 1.0 |
| Rust job body, 4 threads | 0.9 | 0.3 |
| Job TypeScript, checked, 1 thread | 14.8 | 4.9 |
| Job TypeScript, checked, 4 threads | 3.9 | 1.3 |
| Job TypeScript, unchecked, 1 thread | 11.1 | 3.7 |
| Job TypeScript, unchecked, 4 threads | 3.4 | 1.1 |
| Script interpreter (QuickJS-ng), 1 thread | 695 | 230 |

So a job is about 50 times faster than the same code as a script, and with all cores about as
fast as single-threaded Rust. The remaining gap to Rust is the code generator: LLVM optimizes
and can vectorize the Rust loop, Cranelift emits scalar code; copying chunks in and out costs
about 2 ns per particle for six arrays.

## Consequences

- `docs/ai/ARCHITECTURE.md` determinism rules allow job files run through this crate.
- Not yet: running job files from scenes or from scripts (a system that gathers component
  fields into arrays, or a script API that passes typed arrays to a job). This is the next
  step of the workstream (WJ in `docs/ai/WORKSTREAMS.md`).
- Faster later: loop versioning to drop index checks in `for (i < a.length)` loops, SIMD for
  simple loops, zero-copy arrays that live in shared WebAssembly memory (like Unity's
  `NativeArray`).
