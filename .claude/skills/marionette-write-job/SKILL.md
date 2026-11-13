---
name: marionette-write-job
description: Write a Marionette job, a numbers-only TypeScript hot loop compiled to WebAssembly, run in parallel and deterministically over typed arrays. Use when code loops over thousands of items per tick, a script is too slow, or a job fails to compile.
---

# Write a job (hot loop)

Scripts are for per-entity gameplay. A loop over thousands of numbers every tick goes in a
`*.job.ts` file: about 50 times faster than a script, same results on any thread count.

```ts
export function step(py: Float32Array, vy: Float32Array, dt: number): void {
  for (let i = 0; i < py.length; i++) {
    vy[i] = vy[i] - 9.81 * dt;
    py[i] = py[i] + vy[i] * dt;
  }
}
```

## What a job file may contain

- Top level: functions (`export` makes one callable), numeric `const`, `type i32 = number`.
  No global variables: a job keeps no state between calls.
- Types: `number`, `i32`, `boolean`; `Float32Array`, `Float64Array`, `Int32Array` as
  parameters only. Every parameter needs a type.
- Loops, `if`, arithmetic, `Math` functions, calls to functions in the same file.
- Not available: strings, objects, array literals, `new`, closures, classes, `switch`,
  `for...of`, `try`, `null`, `Math.random` (pass random numbers in), the `engine` object.
- Array indexes are `i32`: convert with `(x) | 0`. Conditions are `boolean`: `if (n !== 0)`.
  `let x = 0` is an `i32`; write `0.0` for a `number`.

## Loop

1. `marionette op jobs_check '{"path": "jobs/sim.job.ts"}'` until `ok` is true. Every error
   has a line, a column and a `fix`: apply it.
2. Try it on data: `jobs_run` with `function` and `args` such as
   `[{"f32": [1, 2]}, {"number": 0.5}]`. Run it with `"threads": 1` and `"threads": 4`:
   the arrays must be identical.
3. In a live session `jobs_reload` recompiles changed files; a file that fails keeps its
   last good version.

## Wire it up

Without code: put the arrays on an entity as `JobData`, run steps every tick with
`JobSchedule` (`path`, `function`, `args`, `chunk`), and draw one model copy per element with
`JobDraw`. Step `args` are array names, `in:name` (read only), `all:name` (whole array, read
only), `dt`, `tick`, `len`, `chunk_start`, `rand`, or numbers.

From a script: `engine.jobs.set(name, array)`, `engine.jobs.run(path, fn, args)`,
`engine.jobs.sum(...)`, `engine.jobs.get(name)`.

Each call sees one chunk, so a function may only write the elements it was given. Chunked
arrays of one step must have equal lengths.

## Checks

The scene hash is the same with 1 and several threads (`perf_sim` with `threads`), and
`perf_sim` shows the tick cost you expected.
