# Job TypeScript

Hot loops written in TypeScript, compiled to WebAssembly, run in parallel and deterministic.
Decision 0030; crate `crates/marionette-jobs-ts`; built on `crates/marionette-jobs`
(decision 0024). Use it when a script would loop over thousands of items every tick
(particles, crowds, factory machines, belt slots, grids). Gameplay logic per entity stays in
scripts (decision 0022).

## A job file

```ts
// examples/jobs/particles.job.ts (shortened)
const GRAVITY = -9.81;

function drag(v: number, k: number): number {
  return v - v * Math.abs(v) * k;
}

export function step(py: Float32Array, vy: Float32Array, dt: number, k: number): void {
  for (let i = 0; i < py.length; i++) {
    let y = drag(vy[i] + GRAVITY * dt, k);
    let h = py[i] + y * dt;
    if (h < 0) {
      h = -h;
      y = -y * 0.8;
    }
    py[i] = h;
    vy[i] = y;
  }
}

export function energy(vy: Float32Array): number {
  let e = 0;
  for (let i = 0; i < vy.length; i++) {
    e += 0.5 * vy[i] * vy[i];
  }
  return e;
}
```

Name job files `*.job.ts`. A job file is valid TypeScript: `tsc --noEmit --strict --lib es2020
jobs.d.ts my.job.ts` type-checks it (`jobs.d.ts`: `marionette op jobs_types '{}'`, or
`docs/ai/reference/jobs.d.ts`).

## What a job file may contain

Top level: `function` declarations (`export` makes them callable from the engine), constants
`const NAME = <number | -number | Math.PI ...>`, `type i32 = number`, `import type ...`.
Nothing else: no global variables (jobs keep no state between calls), no value imports.

Types:

| Type | Meaning | Where |
| --- | --- | --- |
| `number` | 64-bit float, as in JavaScript | everywhere |
| `i32` | 32-bit integer (`type i32 = number` for TypeScript) | everywhere |
| `boolean` | true / false | everywhere |
| `Float32Array`, `Float64Array`, `Int32Array` | the job's data | parameters only |
| `void` | no result | return type |

Every parameter needs a type. A return type is needed when the function returns a value.

Statements: `let`, `const`, `=`, `+= -= *= /= %= <<= >>= >>>= |= &= ^= **=`, `++`, `--`,
`if`/`else`, `for (;;)`, `while`, `do`/`while`, `break`, `continue` (no labels), `return`,
calls, blocks.

Expressions: number and boolean literals, `+ - * / % **`, `< <= > >= == != === !==`,
`& | ^ ~ << >> >>>`, `&& || !` (booleans only), `c ? a : b`, `a[i]`, `a.length`, calls to
functions of the same file, `Infinity`, `NaN`, and these `Math` members:

- inline: `abs sqrt floor ceil trunc round sign fround min max imul hypot`;
- `libm` (identical on every platform, same as scripts): `sin cos tan asin acos atan atan2
  sinh cosh tanh asinh acosh atanh exp expm1 log log1p log2 log10 cbrt pow`;
- constants: `PI E LN2 LN10 LOG2E LOG10E SQRT2 SQRT1_2`.

Not available, each with its own error and fix: strings, objects, arrays literals, `new`,
closures and arrow functions, classes and `this`, `switch`, `for...of`/`for...in`, `try`,
`throw`, `null`/`undefined`, `??`/`?.`, `as`, `Math.random` (pass random numbers in), the
`engine` object, `console`.

## Type rules that differ from loose JavaScript

- `let x = 0` (an integer literal, no `.` or exponent) is an `i32`. If the function stores a
  fraction in it (`x += a[i]`), the compiler widens `x` to `number` automatically. `let x =
  0.0` or `let x: number = 0` is a `number` from the start.
- Array indexes must be `i32`. Convert a `number` with `(x) | 0` (truncates, as in
  JavaScript).
- `/` always gives a `number` (`7 / 2` is `3.5`). `i32 + - *` stay `i32` and wrap on overflow;
  JavaScript would not wrap, so use `Math.imul` for hashing and keep integer sums small.
- Conditions must be `boolean`: write `if (n !== 0)`, not `if (n)`.
- `i32 % 0` stops the job (`jobts.trap`); `number % 0` is `NaN` as in JavaScript.
- Storing into a `Float32Array` rounds to 32-bit float; into an `Int32Array` applies ToInt32.
  Reading widens. This matches every JavaScript engine, so a job file run by the script
  interpreter gives bit-identical arrays (tests `compiled_and_interpreted_runs_are_bit_identical`
  and, operator by operator, `every_operator_matches_the_interpreter` in
  `crates/marionette-jobs-ts/tests/jobs.rs`).

## From a scene (no code)

Arrays live on an entity as `JobData` (saved with the scene, part of the state hash). A
`JobSchedule` runs steps every tick in `Stage::Logic` (after scripts, before physics), entities
in id order, steps in order. `JobDraw` draws one model copy per element. Decision 0070;
example `examples/swarm.scene.json` (recipe `examples/swarm.ops.json`).

```json
"JobData": { "arrays": [ { "name": "py", "values": { "F32": [] } },
                         { "name": "vy", "values": { "F32": [] } } ] },
"JobSchedule": { "steps": [ { "path": "jobs/particles.job.ts", "function": "step",
                              "args": ["py", "vy", "dt", "0.001"], "chunk": 0 } ] }
```

`JobDraw { asset, material, x, y, z, scale }` names three arrays (`z` may be `""`) and draws
copy `i` at `(x[i], y[i], z[i])` through the entity's `Transform`, one instanced batch.

`args`, one per parameter: `name` (the entity's array, chunked, changes kept), `in:name`
(chunked, read only), `all:name` (whole, read only), `dt`, `tick`, `len` (elements in the run),
`chunk_start`, `rand` (seeded, new per run: [0, 1) for `number`, 0 to 2^31 - 1 for `i32`),
numbers, `true`/`false`. `chunk: 0` means 16,384. Chunked arrays must have equal lengths; an
empty array is fine (a script may fill it). The load fails with `jobts.*` diagnostics (entity,
`JobSchedule`, `steps[i]`) when a file does not compile, a function is not exported, or an
argument does not fit. A step that fails during a tick records a `job_error` event (entity,
path, function, code, message, line) and the tick goes on. `jobs_reload '{"session": ...}'`
recompiles changed files of a live session.

## From a script

```ts
engine.jobs.set("vx", new Float32Array(4096));            // create or replace a job array
engine.jobs.run("jobs/swarm.job.ts", "seed", ["px", "vx", engine.jobs.chunkStart, 7]);
const e = engine.jobs.sum("jobs/swarm.job.ts", "energy", [engine.jobs.read("vx")]);
const mine = new Float32Array([1, 2, 3]);                 // a script's own array: changed in place
engine.jobs.run("jobs/t.job.ts", "scale", [mine, 2], { chunk: 1024 });
const copy = engine.jobs.get("vx");                        // a copy, or null
```

Array names mean the current entity's job arrays (used in place); pass an entity as the second
argument of `engine.jobs.read/whole/write` for another entity's. Numbers fit `number` or `i32`
parameters (an `i32` needs a whole number). `run` returns one value per chunk; `sum` adds them
in chunk order. Errors throw `jobts.<code>: message (line n); fix: ...`. Without the `jobs-ts`
cargo feature, `run` throws `jobts.unavailable`. Typings: `docs/ai/reference/marionette.d.ts`.

## Running a job from Rust

```rust
use marionette_jobs::Jobs;
use marionette_jobs_ts::{Arg, JobProgram};

let program = JobProgram::load(path)?;          // or JobProgram::compile(source, name)
program.run(&jobs, "step", py.len(), 16_384, &mut [
    Arg::F32(&mut py), Arg::F32(&mut vy), Arg::Number(dt), Arg::Number(0.001),
])?;
let e = program.run_sum(&jobs, "energy", vy.len(), 16_384, &mut [Arg::F32In(&vy)])?;
```

- The run covers `len` elements in chunks of `chunk`. Each call sees one chunk: every chunked
  array starts at index 0 and has the chunk's length.
- `Arg::F32/F64/I32`: chunked, written back. `Arg::F32In/...`: chunked, read only.
  `Arg::F32All/...`: the whole array, read only (lookup tables, neighbours).
  `Arg::Number`, `Arg::Int`, `Arg::Bool`: scalars. `Arg::ChunkStart`: an `i32` parameter
  receiving the chunk's first index (for global ids or seeds).
- A function that returns a value gives one value per chunk, in chunk order; `run_sum` adds
  them in that order (deterministic for any thread count).
- `JobLibrary::new(dir)`: `get(path)` compiles on first use; `reload()` recompiles changed
  files and keeps the last good version of a file that fails (`status: failed` with errors).
- `Safety::Checked` (default) checks indexes and stops a chunk after 5 s
  (`set_time_limit_ms`). `JobProgram::compile_with(source, name, Safety::Unchecked)` drops both
  for about 25% more speed; a wrong index then reads or writes another array of the same call
  instead of stopping. Still sandboxed and deterministic.
- On an error during a run, arrays may be partly updated.

## From the CLI or MCP (no Rust needed)

```bash
marionette op jobs_check '{"path":"examples/jobs/particles.job.ts"}'
marionette op jobs_run '{"path":"examples/jobs/particles.job.ts","function":"energy",
  "args":[{"f32_in":[1,2,3]}],"chunk":2,"threads":2}'
marionette op jobs_types '{}'
```

`jobs_check` lists functions and signatures, or returns `ok: false` with every error.
`jobs_run` arguments: `{"f32": [...]}`, `{"f64": ...}`, `{"i32": ...}` (written back and
returned in `arrays`), the `_in` and `_all` forms, `{"number": x}`, `{"int": n}`,
`{"bool": b}`, `{"chunk_start": true}`.

## Errors

Every error is a diagnostic with `code`, `file`, `line`, `column`, `message` and `fix`. Codes:

| Code | When |
| --- | --- |
| `jobts.syntax` | not valid TypeScript |
| `jobts.unsupported` | a construct outside the subset (the fix says what to use instead) |
| `jobts.type` | wrong type for the place (index not `i32`, `number` into `i32`, non-boolean condition) |
| `jobts.unknown_name` | not a variable, parameter, constant, function or supported `Math` member |
| `jobts.signature` | parameter or return type missing or not a job type |
| `jobts.top_level` | top-level code other than functions, constants and type-only imports |
| `jobts.read` | the file cannot be read |
| `jobts.no_function` | `run` names a function that is not exported (the fix lists exported ones) |
| `jobts.args` | arguments do not match the parameters, or chunked arrays differ in length |
| `jobts.trap` | index out of range (with index, length, line and chunk), missing return, `i32 % 0` |
| `jobts.timeout` | a chunk ran longer than the time limit |
| `jobts.internal` | the runtime refused the compiled module: an engine bug |

All problems of a file are reported at once (at most one per function).

## Performance

`cargo run --release -p marionette-jobs-ts --example jobs_ts_bench` (numbers in decision 0030):
about 50x faster than the script interpreter; about 4x the time of a Rust job body on one
thread, about equal to one-thread Rust when using all cores. Prefer chunk sizes of 4,096 to
65,536; fewer, larger arrays; `Float32Array` for bulk data (half the copying of
`Float64Array`).

## Determinism

Same inputs give bit-identical outputs for any thread count, chunk scheduling and CPU:
WebAssembly arithmetic is exactly specified, stored and returned NaNs are canonical, math
functions are `libm`, chunks come from the data, results are combined in chunk order. Proofs:
`results_do_not_depend_on_thread_count` (1, 2, 3, 8 threads) and `pinned_hash` (a fixed
SHA-256 checked by CI on Linux and Windows) in `crates/marionette-jobs-ts/tests/jobs.rs`.
