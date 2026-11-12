// Marionette job TypeScript prelude (decision 0030, docs/ai/JOBS-TS.md).
//
// Job files are ordinary TypeScript that also compiles to WebAssembly. Add this file to your
// editor or `tsc` inputs so the job-only names below type-check:
//   tsc --noEmit --strict --lib es2020 jobs.d.ts my.job.ts
//
// Types job code may use: number (64-bit float), i32 (32-bit integer), boolean,
// Float32Array, Float64Array, Int32Array (parameters only), void (return type).

/** A 32-bit signed integer. At run time in a JavaScript engine it is a plain number. */
type i32 = number;
